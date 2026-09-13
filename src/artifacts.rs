use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::model::{
    ArtifactCandidate, ArtifactPlan, Direction, DomArtifact, PointerKind, ResolutionRequest,
    SharedMessage,
};
use crate::security::{
    absolute_chatgpt_url, build_file_download_path, build_sandbox_download_path,
    is_allowed_artifact_api_path, is_windows_reserved_filename, normalize_file_id,
    percent_decode, percent_encode_component, trusted_https_url,
};

struct ScanContext<'a> {
    direction: Direction,
    message_id: &'a str,
    source: &'a str,
}

#[derive(Default)]
struct CandidateMetadata {
    name: Option<String>,
    mime_type: Option<String>,
    size_bytes: Option<u64>,
}

type PointerKey = (Direction, String);
type AliasKey = (Direction, String, String);

pub(crate) fn build_plans<'a>(
    messages: impl IntoIterator<Item = &'a SharedMessage>,
    dom_artifacts: &[DomArtifact],
    source_url: &str,
    conversation_id: Option<&str>,
) -> Vec<ArtifactPlan> {
    let mut candidates = Vec::new();

    for message in messages {
        let message = message.as_ref();
        let Some(content) = &message.structured_content else {
            continue;
        };
        let direction = direction_for_role(&message.role);
        let content_context = ScanContext {
            direction,
            message_id: &message.id,
            source: "structured-content",
        };
        let metadata_context = ScanContext {
            direction,
            message_id: &message.id,
            source: "structured-metadata",
        };
        let mut content_path = vec![String::from("content")];
        scan_value(
            content,
            &content_context,
            &mut content_path,
            &mut candidates,
        );
        let mut metadata_path = vec![String::from("metadata")];
        scan_value(
            &message.metadata,
            &metadata_context,
            &mut metadata_path,
            &mut candidates,
        );
    }

    candidates.extend(dom_artifacts.iter().filter_map(candidate_from_dom));
    assign_paths(
        deduplicate(candidates),
        source_url,
        conversation_id,
    )
}

fn direction_for_role(role: &str) -> Direction {
    if role.eq_ignore_ascii_case("user") {
        Direction::Submitted
    } else {
        Direction::Received
    }
}

fn candidate_from_dom(artifact: &DomArtifact) -> Option<ArtifactCandidate> {
    if !is_likely_dom_artifact(artifact) {
        return None;
    }

    let direction = if artifact.direction.eq_ignore_ascii_case("submitted") {
        Direction::Submitted
    } else {
        Direction::Received
    };
    let pointer = artifact.url.clone();
    let suggested_name = if artifact.name.trim().is_empty() {
        filename_from_pointer(&pointer)
    } else {
        artifact.name.clone()
    };

    Some(ArtifactCandidate {
        direction,
        message_id: artifact.message_id.clone(),
        pointer_kind: classify_pointer(&pointer),
        pointer,
        pointer_aliases: Vec::new(),
        suggested_name: nonempty_or(suggested_name, "artifact"),
        mime_type: artifact.mime_type.clone(),
        size_bytes: artifact.size_bytes,
        source: nonempty_or(artifact.source.clone(), "dom"),
    })
}

fn is_likely_dom_artifact(artifact: &DomArtifact) -> bool {
    let source = artifact.source.to_ascii_lowercase();
    let pointer = artifact.url.trim();
    let kind = classify_pointer(pointer);
    let explicit = source.contains("download") || source.contains("card");
    let image = source.contains("image");
    if explicit {
        return !pointer.is_empty() || !artifact.name.trim().is_empty();
    }
    if image {
        if matches!(kind, PointerKind::Blob | PointerKind::Data) {
            return true;
        }
        if kind == PointerKind::Https && trusted_https_url(pointer) {
            let lower = pointer.to_ascii_lowercase();
            return lower.contains("/backend-api/files/")
                || lower.contains("oaiusercontent")
                || lower.contains("oaidalle");
        }
        return false;
    }
    if matches!(
        kind,
        PointerKind::Sediment
            | PointerKind::Sandbox
            | PointerKind::Blob
            | PointerKind::Data
            | PointerKind::FileId
    ) {
        return true;
    }
    if kind == PointerKind::Https && trusted_https_url(pointer) {
        let lower = pointer.to_ascii_lowercase();
        return lower.contains("/backend-api/files/")
            || lower.contains("/download/")
            || lower.contains("oaiusercontent")
            || lower.contains("oaidalle")
            || has_extension(&filename_from_pointer(pointer));
    }
    kind == PointerKind::RelativeUrl
        && (pointer.contains("/backend-api/files/") || pointer.contains("/download/"))
}

fn scan_value(
    value: &Value,
    context: &ScanContext<'_>,
    path: &mut Vec<String>,
    output: &mut Vec<ArtifactCandidate>,
) {
    match value {
        Value::String(text) => scan_string(text, context, path, output),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(index.to_string());
                scan_value(item, context, path, output);
                path.pop();
            }
        }
        Value::Object(map) => {
            if let Some(candidate) = candidate_from_object(map, context, path) {
                output.push(candidate);
            }
            for (key, child) in map {
                path.push(key.clone());
                scan_value(child, context, path, output);
                path.pop();
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn scan_string(
    text: &str,
    context: &ScanContext<'_>,
    path: &[String],
    output: &mut Vec<ArtifactCandidate>,
) {
    let metadata = CandidateMetadata::default();
    for pointer in extract_embedded_pointers(text) {
        output.push(candidate_from_pointer(
            pointer,
            context,
            path,
            &metadata,
        ));
    }

    if path
        .last()
        .is_some_and(|key| is_pointer_key(key))
        && looks_like_artifact_pointer(text)
    {
        output.push(candidate_from_pointer(
            text.to_owned(),
            context,
            path,
            &metadata,
        ));
    }
}

fn candidate_from_object(
    map: &Map<String, Value>,
    context: &ScanContext<'_>,
    path: &[String],
) -> Option<ArtifactCandidate> {
    let explicit_name = first_object_string(map, &["file_name", "filename"]);
    let raw_mime_type = first_object_string(map, &["mime_type", "media_type", "content_type"]);
    let mime_type = raw_mime_type.filter(|value| value.contains('/'));
    let size_bytes = first_object_u64(map, &["size_bytes", "size", "file_size"]);
    let file_key_context = map.keys().any(|key| is_file_context_key(key));
    let has_file_context = explicit_name.is_some()
        || mime_type.is_some()
        || size_bytes.is_some()
        || file_key_context;
    let name = explicit_name.or_else(|| {
        file_key_context
            .then(|| first_object_string(map, &["name", "title"]))
            .flatten()
    });
    let metadata = CandidateMetadata {
        name,
        mime_type,
        size_bytes,
    };

    let pointer_keys = [
        "asset_pointer",
        "audio_asset_pointer",
        "download_url",
        "download_uri",
        "file_url",
        "image_url",
        "audio_url",
        "video_url",
        "asset_url",
        "download_link",
        "file_path",
        "sandbox_path",
    ];
    let mut pointer = first_object_string(map, &pointer_keys);

    if pointer.is_none() && has_file_context {
        pointer = first_object_string(map, &["url", "file_id"]);
        if pointer.is_none() {
            pointer = object_string(map, "id")
                .filter(|value| looks_like_file_id(value))
                .map(ToOwned::to_owned);
        }
    }

    let pointer = pointer.filter(|value| looks_like_artifact_pointer(value))?;
    Some(candidate_from_pointer(
        pointer,
        context,
        path,
        &metadata,
    ))
}

fn candidate_from_pointer(
    pointer: String,
    context: &ScanContext<'_>,
    path: &[String],
    metadata: &CandidateMetadata,
) -> ArtifactCandidate {
    let discovered_name = filename_from_pointer(&pointer);
    let suggested_name = metadata
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .map_or(discovered_name, ToOwned::to_owned);
    ArtifactCandidate {
        direction: context.direction,
        message_id: context.message_id.to_owned(),
        pointer_kind: classify_pointer(&pointer),
        pointer,
        pointer_aliases: Vec::new(),
        suggested_name: nonempty_or(suggested_name, "artifact"),
        mime_type: metadata.mime_type.clone(),
        size_bytes: metadata.size_bytes,
        source: format!("{}:{}", context.source, path.join(".")),
    }
}

fn first_object_string(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object_string(map, key))
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn object_string<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

fn first_object_u64(map: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_u64))
}

fn deduplicate(candidates: Vec<ArtifactCandidate>) -> Vec<ArtifactCandidate> {
    let mut output: Vec<ArtifactCandidate> = Vec::new();
    let mut pointer_indexes: HashMap<PointerKey, usize> = HashMap::new();
    let mut alias_indexes: HashMap<AliasKey, usize> = HashMap::new();

    for candidate in candidates {
        if candidate.pointer.is_empty() && candidate.suggested_name.is_empty() {
            continue;
        }
        let pointer_key = (!candidate.pointer.is_empty())
            .then(|| (candidate.direction, candidate.pointer.clone()));
        let alias = artifact_alias(&candidate);
        let existing_index = pointer_key
            .as_ref()
            .and_then(|key| pointer_indexes.get(key).copied())
            .or_else(|| alias.as_ref().and_then(|key| alias_indexes.get(key).copied()));

        if let Some(index) = existing_index {
            merge_candidate(&mut output[index], candidate);
            if let Some(key) = pointer_key {
                pointer_indexes.insert(key, index);
            }
            if let Some(key) = artifact_alias(&output[index]) {
                alias_indexes.insert(key, index);
            }
        } else {
            let index = output.len();
            if let Some(key) = pointer_key {
                pointer_indexes.insert(key, index);
            }
            if let Some(key) = alias {
                alias_indexes.insert(key, index);
            }
            output.push(candidate);
        }
    }
    output
}

fn artifact_alias(candidate: &ArtifactCandidate) -> Option<AliasKey> {
    let name = sanitize_filename(&candidate.suggested_name).to_ascii_lowercase();
    if candidate.message_id.is_empty()
        || name.is_empty()
        || matches!(name.as_str(), "artifact" | "artifact.bin")
    {
        return None;
    }
    Some((candidate.direction, candidate.message_id.clone(), name))
}

const fn pointer_preference(kind: PointerKind) -> u8 {
    match kind {
        PointerKind::FileId | PointerKind::Sediment => 70,
        PointerKind::RelativeUrl => 60,
        PointerKind::Https => 50,
        PointerKind::Blob | PointerKind::Data => 40,
        PointerKind::Sandbox => 30,
        PointerKind::Unknown => 0,
    }
}

fn merge_candidate(existing: &mut ArtifactCandidate, mut candidate: ArtifactCandidate) {
    let candidate_preferred =
        pointer_preference(candidate.pointer_kind) > pointer_preference(existing.pointer_kind);
    let mut aliases = std::mem::take(&mut existing.pointer_aliases);
    aliases.append(&mut candidate.pointer_aliases);

    if candidate_preferred {
        aliases.push(std::mem::replace(&mut existing.pointer, candidate.pointer));
        existing.pointer_kind = candidate.pointer_kind;
    } else {
        aliases.push(candidate.pointer);
    }
    aliases.retain(|pointer| !pointer.is_empty() && pointer != &existing.pointer);
    aliases.sort();
    aliases.dedup();
    existing.pointer_aliases = aliases;
    if filename_score(&candidate.suggested_name) > filename_score(&existing.suggested_name) {
        existing.suggested_name = candidate.suggested_name;
    }
    if existing.mime_type.is_none() {
        existing.mime_type = candidate.mime_type;
    }
    if existing.size_bytes.is_none() {
        existing.size_bytes = candidate.size_bytes;
    }
    if !existing
        .source
        .split(';')
        .any(|source| source == candidate.source.as_str())
    {
        existing.source.push(';');
        existing.source.push_str(&candidate.source);
    }
}

fn assign_paths(
    candidates: Vec<ArtifactCandidate>,
    source_url: &str,
    conversation_id: Option<&str>,
) -> Vec<ArtifactPlan> {
    let mut submitted = 0_u32;
    let mut received = 0_u32;

    candidates
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| {
            let ordinal = match candidate.direction {
                Direction::Submitted => {
                    submitted += 1;
                    submitted
                }
                Direction::Received => {
                    received += 1;
                    received
                }
            };
            let clean_name = artifact_name(&candidate);
            let prefixed_name = format!("{ordinal:03}-{clean_name}");
            let resolution = resolution_requests(&candidate, source_url, conversation_id);
            ArtifactPlan {
                id: format!("artifact-{:04}", index + 1),
                direction: candidate.direction,
                message_id: candidate.message_id,
                pointer: candidate.pointer,
                pointer_aliases: candidate.pointer_aliases,
                pointer_kind: candidate.pointer_kind,
                suggested_name: clean_name,
                mime_type: candidate.mime_type,
                size_bytes: candidate.size_bytes,
                relative_path: format!(
                    "{}/{prefixed_name}",
                    candidate.direction.directory()
                ),
                source: candidate.source,
                resolution,
            }
        })
        .collect()
}

fn resolution_requests(
    candidate: &ArtifactCandidate,
    source_url: &str,
    conversation_id: Option<&str>,
) -> Vec<ResolutionRequest> {
    let mut requests = Vec::new();
    for pointer in std::iter::once(candidate.pointer.as_str())
        .chain(candidate.pointer_aliases.iter().map(String::as_str))
    {
        let request = resolution_for_pointer(
            pointer,
            &candidate.message_id,
            source_url,
            conversation_id,
        );
        if !matches!(&request, ResolutionRequest::Unresolved { .. })
            && !requests.contains(&request)
        {
            requests.push(request);
        }
    }
    if requests.is_empty() {
        requests.push(ResolutionRequest::Unresolved {
            reason: String::from("No supported download representation was discovered"),
        });
    }
    requests
}

fn resolution_for_pointer(
    pointer: &str,
    message_id: &str,
    source_url: &str,
    conversation_id: Option<&str>,
) -> ResolutionRequest {
    match classify_pointer(pointer) {
        PointerKind::Sediment | PointerKind::FileId => normalize_file_id(pointer).map_or_else(
            || ResolutionRequest::Unresolved {
                reason: String::from("The file identifier was malformed"),
            },
            |file_id| ResolutionRequest::Api {
                path: build_file_download_path(&file_id),
            },
        ),
        PointerKind::Sandbox => conversation_id
            .and_then(|identifier| {
                build_sandbox_download_path(identifier, message_id, pointer)
            })
            .map_or_else(
                || ResolutionRequest::Unresolved {
                    reason: String::from(
                        "The interpreter file lacked safe conversation, message, or path identifiers",
                    ),
                },
                |path| ResolutionRequest::Api { path },
            ),
        PointerKind::Blob | PointerKind::Data => ResolutionRequest::Page {
            url: pointer.to_owned(),
        },
        PointerKind::Https => {
            if trusted_https_url(pointer) {
                ResolutionRequest::Remote {
                    url: pointer.to_owned(),
                }
            } else {
                ResolutionRequest::Unresolved {
                    reason: String::from("The artifact URL host was not trusted"),
                }
            }
        }
        PointerKind::RelativeUrl => {
            if is_allowed_artifact_api_path(pointer) {
                ResolutionRequest::Api {
                    path: pointer.to_owned(),
                }
            } else {
                absolute_chatgpt_url(source_url, pointer).map_or_else(
                    || ResolutionRequest::Unresolved {
                        reason: String::from("The relative artifact URL was malformed"),
                    },
                    |url| ResolutionRequest::Remote { url },
                )
            }
        }
        PointerKind::Unknown => ResolutionRequest::Unresolved {
            reason: String::from("The artifact pointer type was unsupported"),
        },
    }
}

fn artifact_name(candidate: &ArtifactCandidate) -> String {
    let mut name = sanitize_filename(&candidate.suggested_name);
    if !has_extension(&name) {
        name.push_str(extension_for_mime(candidate.mime_type.as_deref()));
    }
    if name.is_empty() || name == ".bin" {
        format!("artifact{}", extension_for_mime(candidate.mime_type.as_deref()))
    } else {
        name
    }
}

pub(crate) fn classify_pointer(pointer: &str) -> PointerKind {
    let value = pointer.trim();
    if value.starts_with("sediment://") || value.starts_with("file-service://") {
        PointerKind::Sediment
    } else if value.starts_with("sandbox:/") || value.starts_with("/mnt/data/") {
        PointerKind::Sandbox
    } else if value.starts_with("blob:") {
        PointerKind::Blob
    } else if value.starts_with("data:") {
        PointerKind::Data
    } else if value.starts_with("https://") {
        PointerKind::Https
    } else if value.starts_with('/') {
        PointerKind::RelativeUrl
    } else if looks_like_file_id(value) {
        PointerKind::FileId
    } else {
        PointerKind::Unknown
    }
}

fn looks_like_artifact_pointer(value: &str) -> bool {
    classify_pointer(value) != PointerKind::Unknown || value.contains("sandbox:/mnt/data/")
}

fn looks_like_file_id(value: &str) -> bool {
    let candidate = value.trim();
    let prefix_match = candidate.starts_with("file-") || candidate.starts_with("file_");
    let length_match = candidate.len() >= 12;
    (prefix_match || length_match)
        && candidate
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
}

fn is_pointer_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "asset_pointer"
            | "audio_asset_pointer"
            | "download_url"
            | "download_uri"
            | "file_url"
            | "image_url"
            | "audio_url"
            | "video_url"
            | "asset_url"
            | "download_link"
            | "file_path"
            | "file_id"
            | "sandbox_path"
    )
}

fn is_file_context_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    [
        "file",
        "asset",
        "attachment",
        "download",
        "image",
        "audio",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn extract_embedded_pointers(text: &str) -> Vec<String> {
    let mut output = Vec::new();
    for prefix in ["sandbox:/mnt/data/", "sediment://", "file-service://"] {
        let mut offset = 0;
        while let Some(relative_start) = text[offset..].find(prefix) {
            let start = offset + relative_start;
            let content_start = start + prefix.len();
            let mut end = content_start;
            for (relative_index, character) in text[content_start..].char_indices() {
                if is_pointer_terminator(character) {
                    break;
                }
                end = content_start + relative_index + character.len_utf8();
            }
            if end > content_start {
                let pointer = text[start..end]
                    .trim_end_matches(['.', ',', ';', ':'])
                    .to_owned();
                if !output.contains(&pointer) {
                    output.push(pointer);
                }
            }
            offset = end.max(content_start);
            if offset >= text.len() {
                break;
            }
        }
    }
    output
}

fn is_pointer_terminator(character: char) -> bool {
    character.is_whitespace()
        || matches!(character, ')' | ']' | '}' | '>' | '"' | '\'' | '`')
}

fn filename_from_pointer(pointer: &str) -> String {
    let stripped = ["sandbox:/mnt/data/", "sediment://", "file-service://"]
        .iter()
        .find_map(|prefix| pointer.strip_prefix(prefix))
        .unwrap_or(pointer);
    let without_query = stripped
        .split(['?', '#'])
        .next()
        .unwrap_or(stripped)
        .trim_end_matches('/');
    without_query
        .rsplit(['/', '\\'])
        .next()
        .map_or_else(|| String::from("artifact"), percent_decode)
}

fn sanitize_filename(value: &str) -> String {
    let source = filename_from_pointer(value);
    let mut output = String::with_capacity(source.len().min(110));
    let mut previous_separator = false;
    let mut character_count = 0_usize;

    for character in source.chars() {
        if character_count >= 110 {
            break;
        }
        if character.is_alphanumeric() || matches!(character, '.' | '_' | '-') {
            output.push(character);
            previous_separator = false;
            character_count += 1;
        } else if (character.is_whitespace() || !character.is_control())
            && !previous_separator
            && !output.is_empty()
        {
            output.push('-');
            previous_separator = true;
            character_count += 1;
        }
    }
    let mut cleaned = output.trim_matches(['.', '-']).to_owned();
    if is_windows_reserved_filename(&cleaned) {
        cleaned.insert(0, '_');
    }
    cleaned
}

fn filename_score(value: &str) -> usize {
    usize::from(has_extension(value)) * 1_000 + value.len().min(500)
}

fn has_extension(value: &str) -> bool {
    let Some((_, extension)) = value.rsplit_once('.') else {
        return false;
    };
    !extension.is_empty()
        && extension.len() <= 12
        && extension.chars().all(|character| character.is_ascii_alphanumeric())
}

fn extension_for_mime(mime_type: Option<&str>) -> &'static str {
    match mime_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "application/json" => ".json",
        "application/pdf" => ".pdf",
        "application/zip" => ".zip",
        "audio/mpeg" => ".mp3",
        "audio/ogg" => ".ogg",
        "audio/wav" => ".wav",
        "image/gif" => ".gif",
        "image/jpeg" => ".jpg",
        "image/png" => ".png",
        "image/svg+xml" => ".svg",
        "image/webp" => ".webp",
        "text/csv" => ".csv",
        "text/html" => ".html",
        "text/markdown" => ".md",
        "text/plain" => ".txt",
        "video/mp4" => ".mp4",
        _ => ".bin",
    }
}

pub(crate) fn encode_markdown_path(value: &str) -> String {
    value
        .split('/')
        .map(percent_encode_component)
        .collect::<Vec<_>>()
        .join("/")
}

fn nonempty_or(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_pointer, encode_markdown_path, extract_embedded_pointers, sanitize_filename,
    };
    use crate::model::PointerKind;

    #[test]
    fn classifies_common_pointer_types() {
        assert_eq!(classify_pointer("sediment://file-12345"), PointerKind::Sediment);
        assert_eq!(
            classify_pointer("sandbox:/mnt/data/report.pdf"),
            PointerKind::Sandbox
        );
        assert_eq!(
            classify_pointer("/mnt/data/report.pdf"),
            PointerKind::Sandbox
        );
        assert_eq!(
            classify_pointer("https://files.example.test/a"),
            PointerKind::Https
        );
    }

    #[test]
    fn finds_embedded_sandbox_links() {
        let pointers =
            extract_embedded_pointers("See [report](sandbox:/mnt/data/a%20b.pdf). Done.");
        assert_eq!(pointers, vec!["sandbox:/mnt/data/a%20b.pdf"]);
    }

    #[test]
    fn prefixes_windows_reserved_filenames() {
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_filename("normal.txt"), "normal.txt");
    }

    #[test]
    fn encodes_relative_markdown_paths() {
        assert_eq!(
            encode_markdown_path("received/001-a b.pdf"),
            "received/001-a%20b.pdf"
        );
    }
}
