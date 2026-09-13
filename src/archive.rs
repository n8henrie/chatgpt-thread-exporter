use std::collections::HashMap;

use serde_json::{json, Value};

use crate::artifacts::encode_markdown_path;
use crate::model::{
    ArchiveInput, ArchiveJob, ArchivePlan, ArchiveSource, ArtifactPlan, ArtifactResolution,
    Direction, PointerKind, ResolutionRequest, ResolutionStatus,
};
use crate::security::{
    compact_timestamp, inline_base64_size, safe_archive_path, sanitize_error,
    sanitize_path_segment, trusted_https_url,
};

const ARCHIVE_DIRECTORY: &str = "ChatGPT Exports";
const MAX_INLINE_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_INLINE_EXPORT_BYTES: u64 = 32 * 1024 * 1024;

struct ArtifactRecord {
    id: String,
    direction: Direction,
    message_id: String,
    relative_path: String,
    suggested_name: String,
    mime_type: Option<String>,
    size_bytes: Option<u64>,
    source: String,
    pointer_kind: PointerKind,
    status: &'static str,
    error: Option<String>,
}

pub(crate) fn build_archive(input: ArchiveInput) -> Result<ArchivePlan, String> {
    let ArchiveInput {
        plan,
        resolutions,
        raw_conversation,
        extension_version,
    } = input;
    let timestamp = compact_timestamp(&plan.generated_at);
    let root_name = sanitize_path_segment(&plan.root_name);
    let root = format!("{ARCHIVE_DIRECTORY}/{root_name}--{timestamp}");
    let resolution_map = resolutions
        .iter()
        .map(|resolution| (resolution.id.as_str(), resolution))
        .collect::<HashMap<_, _>>();

    let mut inline_bytes = 0_u64;
    let mut artifact_jobs = Vec::new();
    let mut artifact_records = Vec::with_capacity(plan.artifacts.len());
    for artifact in &plan.artifacts {
        let resolution = resolution_map.get(artifact.id.as_str()).copied();
        let (record, job) = artifact_output(&root, artifact, resolution, &mut inline_bytes);
        if let Some(job) = job {
            artifact_jobs.push(job);
        }
        artifact_records.push(record);
    }
    let unresolved = artifact_records
        .iter()
        .filter(|record| !record.status.starts_with("resolved"))
        .count();

    let manifest = build_manifest(&plan, &artifact_records, &root, &extension_version);
    let artifact_index = build_artifact_index(&plan.title, &plan.warnings, &artifact_records);
    let raw_json = raw_conversation
        .map(|value| match value {
            Value::String(value) => Ok(value),
            other => serde_json::to_string_pretty(&other)
                .map(|text| format!("{text}\n"))
                .map_err(|error| format!("Could not serialize raw conversation JSON: {error}")),
        })
        .transpose()?;

    let mut jobs = Vec::with_capacity(
        3 + plan.branches.len() + artifact_jobs.len() + usize::from(raw_json.is_some()),
    );
    jobs.push(text_job(
        &root,
        "conversation.md",
        plan.markdown,
        "text/markdown;charset=utf-8",
    ));
    for branch in plan.branches {
        jobs.push(text_job(
            &root,
            &branch.relative_path,
            branch.markdown,
            "text/markdown;charset=utf-8",
        ));
    }
    if let Some(raw_json) = raw_json {
        jobs.push(text_job(
            &root,
            "raw/conversation.json",
            raw_json,
            "application/json;charset=utf-8",
        ));
    }
    jobs.extend(artifact_jobs);
    jobs.push(text_job(
        &root,
        "manifest.json",
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("Could not serialize archive manifest: {error}"))?
        ),
        "application/json;charset=utf-8",
    ));
    jobs.push(text_job(
        &root,
        "artifacts.md",
        artifact_index,
        "text/markdown;charset=utf-8",
    ));

    Ok(ArchivePlan { jobs, unresolved })
}

fn artifact_output(
    root: &str,
    artifact: &ArtifactPlan,
    resolution: Option<&ArtifactResolution>,
    inline_bytes: &mut u64,
) -> (ArtifactRecord, Option<ArchiveJob>) {
    let relative_path = safe_archive_path(&artifact.relative_path);
    let base_record = || ArtifactRecord {
        id: artifact.id.clone(),
        direction: artifact.direction,
        message_id: artifact.message_id.clone(),
        relative_path: relative_path.clone(),
        suggested_name: resolution
            .and_then(|value| value.resolved_name.as_ref())
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| artifact.suggested_name.clone()),
        mime_type: resolution
            .and_then(|value| value.resolved_mime_type.as_ref())
            .cloned()
            .or_else(|| artifact.mime_type.clone()),
        size_bytes: resolution
            .and_then(|value| value.resolved_size_bytes)
            .or(artifact.size_bytes),
        source: artifact.source.clone(),
        pointer_kind: artifact.pointer_kind,
        status: "unresolved",
        error: None,
    };

    let Some(resolution) = resolution else {
        let mut record = base_record();
        record.error = Some(default_resolution_error(artifact));
        return (record, None);
    };

    match resolution.status {
        ResolutionStatus::Resolved => {
            let Some(url) = resolution
                .resolved_url
                .as_deref()
                .filter(|url| trusted_https_url(url))
            else {
                let mut record = base_record();
                record.error = Some(String::from(
                    "The resolved artifact URL was missing or untrusted",
                ));
                return (record, None);
            };
            let mut record = base_record();
            record.status = "resolved";
            let job = ArchiveJob {
                path: safe_archive_path(&format!("{root}/{relative_path}")),
                source: ArchiveSource::Remote {
                    url: url.to_owned(),
                },
            };
            (record, Some(job))
        }
        ResolutionStatus::ResolvedInline => {
            let Some(base64) = resolution.inline_base64.as_deref() else {
                let mut record = base_record();
                record.error = Some(String::from("The inline artifact payload was missing"));
                return (record, None);
            };
            let Some(size_bytes) = inline_base64_size(base64) else {
                let mut record = base_record();
                record.error = Some(String::from("The inline artifact payload was malformed"));
                return (record, None);
            };
            let within_limits = size_bytes <= MAX_INLINE_ARTIFACT_BYTES
                && inline_bytes
                    .checked_add(size_bytes)
                    .is_some_and(|total| total <= MAX_INLINE_EXPORT_BYTES);
            if !within_limits {
                let mut record = base_record();
                record.error = Some(String::from(
                    "The inline artifact exceeded the archive transfer limit",
                ));
                return (record, None);
            }
            *inline_bytes += size_bytes;
            let mime_type = resolution
                .inline_mime_type
                .as_deref()
                .or(resolution.resolved_mime_type.as_deref())
                .or(artifact.mime_type.as_deref())
                .unwrap_or("application/octet-stream")
                .to_owned();
            let mut record = base_record();
            record.status = "resolved_inline";
            record.size_bytes = Some(size_bytes);
            record.mime_type = Some(mime_type.clone());
            let job = ArchiveJob {
                path: safe_archive_path(&format!("{root}/{relative_path}")),
                source: ArchiveSource::Inline {
                    base64: base64.to_owned(),
                    mime_type,
                },
            };
            (record, Some(job))
        }
        ResolutionStatus::Unresolved => {
            let mut record = base_record();
            record.error = Some(
                resolution
                    .error
                    .as_deref()
                    .map(sanitize_error)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| default_resolution_error(artifact)),
            );
            (record, None)
        }
    }
}

fn default_resolution_error(artifact: &ArtifactPlan) -> String {
    artifact
        .resolution
        .iter()
        .find_map(|request| match request {
            ResolutionRequest::Unresolved { reason } => Some(sanitize_error(reason)),
            ResolutionRequest::Api { .. }
            | ResolutionRequest::Page { .. }
            | ResolutionRequest::Remote { .. } => None,
        })
        .unwrap_or_else(|| String::from("No artifact resolution result was returned"))
}

fn text_job(root: &str, relative_path: &str, text: String, mime_type: &str) -> ArchiveJob {
    ArchiveJob {
        path: safe_archive_path(&format!("{root}/{relative_path}")),
        source: ArchiveSource::Text {
            text,
            mime_type: mime_type.to_owned(),
        },
    }
}

fn build_manifest(
    plan: &crate::model::ExportPlan,
    records: &[ArtifactRecord],
    root: &str,
    extension_version: &str,
) -> Value {
    json!({
        "schema_version": 2,
        "exporter": {
            "name": "ChatGPT Thread Exporter",
            "version": if extension_version.is_empty() { "unknown" } else { extension_version },
            "core": "rust-wasm"
        },
        "exported_at": &plan.generated_at,
        "archive_root": root,
        "title": &plan.title,
        "source_url": &plan.source_url,
        "conversation_id": &plan.conversation_id,
        "extraction": &plan.extraction,
        "warnings": plan.warnings.iter().map(|warning| sanitize_error(warning)).collect::<Vec<_>>(),
        "branches": plan.branches.iter().map(|branch| json!({
            "relative_path": safe_archive_path(&branch.relative_path),
            "label": &branch.label,
            "message_count": branch.message_count
        })).collect::<Vec<_>>(),
        "artifacts": records.iter().map(|record| json!({
            "artifact_id": &record.id,
            "direction": record.direction,
            "message_id": &record.message_id,
            "relative_path": &record.relative_path,
            "suggested_name": &record.suggested_name,
            "mime_type": &record.mime_type,
            "size_bytes": record.size_bytes,
            "source": &record.source,
            "pointer_kind": record.pointer_kind,
            "status": record.status,
            "error": &record.error
        })).collect::<Vec<_>>()
    })
}

fn build_artifact_index(
    title: &str,
    warnings: &[String],
    records: &[ArtifactRecord],
) -> String {
    let mut lines = vec![
        String::from("# Artifact index"),
        String::new(),
        format!("Conversation: {}", escape_markdown_inline(title)),
        String::new(),
        String::from("| Direction | File | Status | Message | Source |"),
        String::from("|---|---|---|---|---|"),
    ];

    if records.is_empty() {
        lines.push(String::from("| — | No artifacts discovered | — | — | — |"));
    } else {
        for record in records {
            let file = if record.status.starts_with("resolved") {
                format!(
                    "[{}]({})",
                    escape_markdown_inline(&record.suggested_name),
                    encode_markdown_path(&record.relative_path)
                )
            } else {
                escape_markdown_inline(&record.suggested_name)
            };
            lines.push(format!(
                "| {} | {file} | {} | {} | {} |",
                record.direction.directory(),
                escape_markdown_inline(record.status),
                escape_markdown_inline(if record.message_id.is_empty() {
                    "—"
                } else {
                    &record.message_id
                }),
                escape_markdown_inline(&record.source)
            ));
            if let Some(error) = &record.error {
                lines.push(format!(
                    "|  | Error: {} |  |  |  |",
                    escape_markdown_inline(error)
                ));
            }
        }
    }

    if !warnings.is_empty() {
        lines.extend([String::new(), String::from("## Export warnings"), String::new()]);
        lines.extend(
            warnings
                .iter()
                .map(|warning| format!("- {}", escape_markdown_inline(&sanitize_error(warning)))),
        );
    }
    lines.push(String::new());
    lines.join("\n")
}

fn escape_markdown_inline(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('`', "\\`")
        .replace('\r', " ")
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::build_archive;
    use crate::model::ArchiveInput;

    #[test]
    fn omits_signed_urls_from_persisted_files() -> Result<(), Box<dyn std::error::Error>> {
        let input: ArchiveInput = serde_json::from_value(json!({
            "plan": {
                "schema_version": 1,
                "generated_at": "2026-08-04T17:00:00.123Z",
                "title": "Example",
                "source_url": "https://chatgpt.com/c/conversation-123",
                "conversation_id": "conversation-123",
                "root_name": "example",
                "extraction": "api",
                "markdown": "# Example\n",
                "branches": [],
                "artifacts": [{
                    "id": "artifact-0001",
                    "direction": "received",
                    "message_id": "message-123",
                    "pointer": "sandbox:/mnt/data/report.pdf",
                    "pointer_aliases": [],
                    "pointer_kind": "sandbox",
                    "suggested_name": "report.pdf",
                    "mime_type": "application/pdf",
                    "size_bytes": 42,
                    "relative_path": "received/001-report.pdf",
                    "source": "fixture",
                    "resolution": [{
                        "kind": "api",
                        "path": "/backend-api/example"
                    }]
                }],
                "warnings": []
            },
            "resolutions": [{
                "artifact_id": "artifact-0001",
                "status": "resolved",
                "resolved_url": "https://files.oaiusercontent.com/report.pdf?sig=synthetic"
            }],
            "extension_version": "0.3.1"
        }))?;
        let archive = build_archive(input)?;
        let persisted = archive
            .jobs
            .iter()
            .filter_map(|job| match &job.source {
                crate::model::ArchiveSource::Text { text, .. } => Some(text.as_str()),
                crate::model::ArchiveSource::Inline { .. }
                | crate::model::ArchiveSource::Remote { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("");
        assert!(!persisted.contains("sig=synthetic"));
        assert_eq!(archive.unresolved, 0);
        Ok(())
    }
}
