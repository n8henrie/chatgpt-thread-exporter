use std::collections::HashSet;
use std::fmt::Write as _;

use serde_json::{Map, Value};

use crate::artifacts::encode_markdown_path;
use crate::model::{ArtifactPlan, NormalizedMessage, SharedMessage};

pub(crate) struct DocumentContext<'a> {
    pub(crate) title: &'a str,
    pub(crate) source_url: &'a str,
    pub(crate) exported_at: &'a str,
    pub(crate) conversation_id: Option<&'a str>,
    pub(crate) extraction: &'a str,
}

#[derive(Debug)]
struct Reference {
    title: String,
    url: String,
}

pub(crate) fn render_document(
    context: &DocumentContext<'_>,
    messages: &[SharedMessage],
    artifacts: &[ArtifactPlan],
    path_prefix: &str,
) -> String {
    let mut output = String::new();
    push_front_matter(&mut output, context);
    let _ = writeln!(output, "# {}\n", escape_heading(context.title));

    for message in messages {
        let message = message.as_ref();
        if !message.renderable {
            continue;
        }
        push_message(&mut output, message, artifacts, path_prefix);
    }

    normalize_document(&output)
}

fn push_front_matter(output: &mut String, context: &DocumentContext<'_>) {
    let _ = writeln!(output, "---");
    let _ = writeln!(output, "title: {}", yaml_string(context.title));
    let _ = writeln!(output, "source_url: {}", yaml_string(context.source_url));
    let _ = writeln!(
        output,
        "conversation_id: {}",
        yaml_string(context.conversation_id.unwrap_or_default())
    );
    let _ = writeln!(output, "exported_at: {}", yaml_string(context.exported_at));
    let _ = writeln!(output, "extraction: {}", yaml_string(context.extraction));
    let _ = writeln!(output, "---\n");
}

fn push_message(
    output: &mut String,
    message: &NormalizedMessage,
    artifacts: &[ArtifactPlan],
    path_prefix: &str,
) {
    let role_label = if message.role.eq_ignore_ascii_case("user") {
        "User"
    } else {
        "ChatGPT"
    };
    let _ = writeln!(output, "## {role_label}\n");
    let _ = writeln!(
        output,
        "<!-- message-id: {} -->\n",
        escape_html_comment(&message.id)
    );

    let message_artifacts: Vec<&ArtifactPlan> = artifacts
        .iter()
        .filter(|artifact| artifact.message_id == message.id)
        .collect();
    let mut body = message.markdown.trim().to_owned();
    for artifact in &message_artifacts {
        let destination = encode_markdown_path(&format!(
            "{path_prefix}{}",
            artifact.relative_path
        ));
        for pointer in std::iter::once(&artifact.pointer).chain(artifact.pointer_aliases.iter()) {
            if !pointer.is_empty() {
                body = body.replace(pointer.as_str(), &destination);
            }
        }
    }
    body = clean_proprietary_markers(&body);
    if body.is_empty() {
        let _ = writeln!(output, "_No textual content was available._\n");
    } else {
        let _ = writeln!(output, "{body}\n");
    }

    let references = collect_references(&message.metadata);
    if !references.is_empty() {
        let _ = writeln!(output, "### References\n");
        for reference in references {
            let _ = writeln!(
                output,
                "- [{}]({})",
                escape_markdown_label(&reference.title),
                escape_markdown_destination(&reference.url)
            );
        }
        output.push('\n');
    }

    let unlinked: Vec<&ArtifactPlan> = message_artifacts
        .into_iter()
        .filter(|artifact| {
            let raw_path = format!("{path_prefix}{}", artifact.relative_path);
            let encoded_path = encode_markdown_path(&raw_path);
            !body.contains(raw_path.as_str()) && !body.contains(encoded_path.as_str())
        })
        .collect();
    if !unlinked.is_empty() {
        let _ = writeln!(output, "### Artifacts\n");
        for artifact in unlinked {
            let destination = encode_markdown_path(&format!(
                "{path_prefix}{}",
                artifact.relative_path
            ));
            let _ = writeln!(
                output,
                "- [{}]({destination}) _({})_",
                escape_markdown_label(&artifact.suggested_name),
                artifact.direction.directory()
            );
        }
        output.push('\n');
    }
}

pub(crate) fn render_structured_content(content: &Value) -> String {
    match content {
        Value::String(text) => clean_proprietary_markers(text),
        Value::Object(map) => render_content_object(map),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => String::new(),
    }
}

fn render_content_object(map: &Map<String, Value>) -> String {
    let content_type = object_string(map, "content_type")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if content_type == "code" {
        return fenced_block(
            first_object_string(map, &["text", "code"]).unwrap_or_default(),
            object_string(map, "language").unwrap_or_default(),
        );
    }
    if content_type == "execution_output" {
        return fenced_block(
            first_object_string(map, &["text", "output"]).unwrap_or_default(),
            "text",
        );
    }

    if let Some(Value::Array(parts)) = map.get("parts") {
        let rendered: Vec<String> = parts
            .iter()
            .map(render_content_part)
            .filter(|text| !text.trim().is_empty())
            .collect();
        if !rendered.is_empty() {
            return clean_proprietary_markers(&rendered.join("\n\n"));
        }
    }

    clean_proprietary_markers(
        first_object_string(map, &["text", "result", "summary", "content"])
            .unwrap_or_default(),
    )
}

fn render_content_part(part: &Value) -> String {
    let Value::Object(map) = part else {
        return part.as_str().unwrap_or_default().to_owned();
    };
    let content_type = object_string(map, "content_type")
        .unwrap_or_default()
        .to_ascii_lowercase();
    match content_type.as_str() {
        "code" => fenced_block(
            first_object_string(map, &["text", "code"]).unwrap_or_default(),
            object_string(map, "language").unwrap_or_default(),
        ),
        "execution_output" => fenced_block(
            first_object_string(map, &["text", "output"]).unwrap_or_default(),
            "text",
        ),
        "tether_quote" => first_object_string(map, &["text", "quote"])
            .unwrap_or_default()
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => first_object_string(map, &["text", "content", "result", "summary"])
            .unwrap_or_default()
            .to_owned(),
    }
}

fn fenced_block(text: &str, language: &str) -> String {
    let value = text.strip_suffix('\n').unwrap_or(text);
    let fence_length = longest_backtick_run(value).saturating_add(1).max(3);
    let fence = "`".repeat(fence_length);
    let safe_language: String = language
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "_+.-".contains(*character))
        .collect();
    format!("{fence}{safe_language}\n{value}\n{fence}")
}

fn longest_backtick_run(value: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for character in value.chars() {
        if character == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn collect_references(value: &Value) -> Vec<Reference> {
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    visit_references(value, &mut output, &mut seen);
    output.truncate(100);
    output
}

fn visit_references(
    value: &Value,
    output: &mut Vec<Reference>,
    seen: &mut HashSet<String>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                visit_references(item, output, seen);
            }
        }
        Value::Object(map) => {
            let url = first_object_string(map, &["url", "source_url", "link"]);
            if let Some(url) = url.filter(|url| is_safe_reference_url(url))
                && seen.insert(url.to_owned())
            {
                let title = first_object_string(map, &["title", "name", "text"])
                    .map_or_else(|| host_from_url(url), ToOwned::to_owned);
                output.push(Reference {
                    title,
                    url: url.to_owned(),
                });
            }
            for child in map.values() {
                visit_references(child, output, seen);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_safe_reference_url(url: &str) -> bool {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return false;
    }

    let lowercase = url.to_ascii_lowercase();
    let host = host_from_url(&lowercase);
    if host == "oaiusercontent.com"
        || host.ends_with(".oaiusercontent.com")
        || host.ends_with(".blob.core.windows.net")
        || host.contains("oaidalle")
    {
        return false;
    }

    let path = lowercase
        .split_once("//")
        .map_or(lowercase.as_str(), |(_, remainder)| remainder);
    if path.contains("/backend-api/files/") || path.contains("/files/download/") {
        return false;
    }

    let Some((_, query_and_fragment)) = lowercase.split_once('?') else {
        return true;
    };
    let query = query_and_fragment.split('#').next().unwrap_or_default();
    !query.split('&').filter_map(|pair| pair.split('=').next()).any(|key| {
        matches!(
            key,
            "token"
                | "access_token"
                | "sig"
                | "signature"
                | "policy"
                | "key-pair-id"
                | "expires"
                | "se"
                | "sp"
                | "sv"
                | "st"
                | "x-amz-algorithm"
                | "x-amz-credential"
                | "x-amz-date"
                | "x-amz-expires"
                | "x-amz-security-token"
                | "x-amz-signature"
                | "x-goog-algorithm"
                | "x-goog-credential"
                | "x-goog-date"
                | "x-goog-expires"
                | "x-goog-signature"
        )
    })
}

fn object_string<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

fn first_object_string<'a>(
    map: &'a Map<String, Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object_string(map, key))
        .filter(|text| !text.trim().is_empty())
}

pub(crate) fn clean_proprietary_markers(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remainder = value;
    while let Some(start) = remainder.find('') {
        output.push_str(&remainder[..start]);
        let marker = &remainder[start..];
        let Some(relative_end) = marker.find('') else {
            output.push_str(marker);
            remainder = "";
            break;
        };
        let end = relative_end + ''.len_utf8();
        if is_proprietary_marker(&marker[..end]) {
            remainder = &marker[end..];
        } else {
            output.push_str(&marker[..end]);
            remainder = &marker[end..];
        }
    }
    output.push_str(remainder);
    normalize_markdown(&output)
}

fn is_proprietary_marker(marker: &str) -> bool {
    [
        "cite",
        "filecite",
        "filenavlist",
        "navlist",
        "i",
        "products",
        "forecast",
        "finance",
        "schedule",
        "standing",
    ]
    .iter()
    .any(|prefix| marker.starts_with(prefix))
}

fn normalize_markdown(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().map(str::trim_end).collect();
    collapse_blank_lines(&lines.join("\n")).trim().to_owned()
}

fn normalize_document(value: &str) -> String {
    let normalized = collapse_blank_lines(value.trim());
    format!("{normalized}\n")
}

fn collapse_blank_lines(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut blank_count = 0;
    for line in value.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                output.push('\n');
            }
        } else {
            blank_count = 0;
            output.push_str(line);
            output.push('\n');
        }
    }
    output.trim_end_matches('\n').to_owned()
}

fn yaml_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(output, "\\u{:04X}", u32::from(character));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn escape_heading(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_owned()
}

fn escape_html_comment(value: &str) -> String {
    value.replace("--", "—").replace(['<', '>'], "")
}

fn escape_markdown_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn escape_markdown_destination(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace(' ', "%20")
}

fn host_from_url(url: &str) -> String {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(url)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{clean_proprietary_markers, is_safe_reference_url, render_structured_content};

    #[test]
    fn renders_code_with_a_safe_fence() {
        let content = json!({
            "content_type": "code",
            "language": "rust",
            "text": "let fence = ```;"
        });
        let rendered = render_structured_content(&content);
        assert!(rendered.starts_with("````rust\n"));
        assert!(rendered.ends_with("\n````"));
    }

    #[test]
    fn removes_chatgpt_citation_markers() {
        assert_eq!(clean_proprietary_markers("Text citesource end"), "Text  end");
    }

    #[test]
    fn excludes_signed_file_urls_from_references() {
        assert!(is_safe_reference_url("https://example.test/article?section=1"));
        assert!(!is_safe_reference_url(
            "https://files.oaiusercontent.com/private/file.pdf?sig=secret&se=2099-01-01"
        ));
        assert!(!is_safe_reference_url(
            "https://storage.example.test/file?x-amz-signature=secret"
        ));
    }
}
