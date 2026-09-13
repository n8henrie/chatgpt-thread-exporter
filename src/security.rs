use crate::model::ConversationContext;

const CHATGPT_ORIGINS: [&str; 2] = ["https://chatgpt.com", "https://chat.openai.com"];
const MAX_ERROR_LENGTH: usize = 350;
const MAX_PATH_SEGMENT_LENGTH: usize = 120;

pub(crate) fn conversation_context(value: &str) -> Result<ConversationContext, String> {
    let canonical = strip_query_and_fragment(value.trim());
    let Some((origin, path)) = split_chatgpt_url(canonical) else {
        return Err(String::from(
            "This exporter only runs on HTTPS ChatGPT web conversations",
        ));
    };

    let id = conversation_id_from_path(path);
    let api_path = id.as_deref().map(|identifier| {
        format!(
            "/backend-api/conversation/{}",
            percent_encode_component(identifier)
        )
    });
    Ok(ConversationContext {
        source_url: format!("{origin}{path}"),
        origin: origin.to_owned(),
        id,
        api_path,
    })
}

pub(crate) fn conversation_id_from_url(value: &str) -> Option<String> {
    let (_, path) = split_chatgpt_url(strip_query_and_fragment(value.trim()))?;
    conversation_id_from_path(path)
}

pub(crate) fn trusted_https_url(value: &str) -> bool {
    let value = value.trim();
    if value.contains('\\') || value.chars().any(char::is_control) {
        return false;
    }
    let Some(authority_and_path) = value.strip_prefix("https://") else {
        return false;
    };
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if authority.is_empty()
        || authority.contains(['@', ':', '\\', '%', '[', ']'])
        || !authority.is_ascii()
        || authority.chars().any(char::is_control)
    {
        return false;
    }

    let host = authority.to_ascii_lowercase();
    is_trusted_artifact_host(&host)
}

fn is_trusted_artifact_host(host: &str) -> bool {
    host == "chatgpt.com"
        || host.ends_with(".chatgpt.com")
        || host == "openai.com"
        || host.ends_with(".openai.com")
        || host == "cdn.oaistatic.com"
        || host == "oaiusercontent.com"
        || host.ends_with(".oaiusercontent.com")
        || host == "oaidalleapiprodscus.blob.core.windows.net"
}

pub(crate) fn normalize_file_id(pointer: &str) -> Option<String> {
    let value = pointer.trim();
    let stripped = ["sediment://", "file-service://", "file://"]
        .iter()
        .find_map(|prefix| value.strip_prefix(prefix))
        .unwrap_or(value)
        .split(['?', '#'])
        .next()
        .unwrap_or_default();

    let valid = (5..=512).contains(&stripped.len()) && is_safe_api_identifier(stripped);
    valid.then_some(stripped.to_owned())
}

pub(crate) fn normalize_sandbox_path(pointer: &str) -> Option<String> {
    let value = pointer.trim();
    let without_scheme = value
        .strip_prefix("sandbox:")
        .or_else(|| value.starts_with("/mnt/data/").then_some(value))?;
    let raw_path = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let path = percent_decode(raw_path);

    if !path.starts_with("/mnt/data/")
        || path.len() > 4_096
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return None;
    }

    let relative = &path["/mnt/data/".len()..];
    if relative.is_empty()
        || relative
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return None;
    }

    Some(path)
}

pub(crate) fn build_file_download_path(file_id: &str) -> String {
    format!(
        "/backend-api/files/download/{}?post_id=&inline=false",
        percent_encode_component(file_id)
    )
}

pub(crate) fn build_sandbox_download_path(
    conversation_id: &str,
    message_id: &str,
    pointer: &str,
) -> Option<String> {
    let conversation_id = nonempty_identifier(conversation_id)?;
    let message_id = nonempty_identifier(message_id)?;
    let sandbox_path = normalize_sandbox_path(pointer)?;

    Some(format!(
        "/backend-api/conversation/{}/interpreter/download?message_id={}&sandbox_path={}",
        percent_encode_component(conversation_id),
        percent_encode_component(message_id),
        percent_encode_component(&sandbox_path)
    ))
}

pub(crate) fn is_allowed_artifact_api_path(path: &str) -> bool {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return false;
    }

    let path_only = path.split('?').next().unwrap_or_default();
    if let Some(file_id) = path_only.strip_prefix("/backend-api/files/download/") {
        return is_safe_api_identifier(file_id);
    }

    let Some(remainder) = path_only.strip_prefix("/backend-api/conversation/") else {
        return false;
    };
    let Some(conversation_id) = remainder.strip_suffix("/interpreter/download") else {
        return false;
    };
    is_safe_api_identifier(conversation_id)
}

pub(crate) fn absolute_chatgpt_url(source_url: &str, path: &str) -> Option<String> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return None;
    }
    let (origin, _) = split_chatgpt_url(strip_query_and_fragment(source_url.trim()))?;
    Some(format!("{origin}{path}"))
}

pub(crate) fn safe_archive_path(value: &str) -> String {
    let segments = value
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(sanitize_path_segment)
        .collect::<Vec<_>>();
    if segments.is_empty() {
        String::from("untitled")
    } else {
        segments.join("/")
    }
}

pub(crate) fn sanitize_path_segment(value: &str) -> String {
    let mut output = String::new();
    let mut output_length = 0_usize;
    let mut pending_space = false;

    for character in value.chars() {
        if output_length >= MAX_PATH_SEGMENT_LENGTH {
            break;
        }
        if character.is_control() {
            continue;
        }
        if character.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        if pending_space {
            output.push(' ');
            output_length += 1;
            pending_space = false;
            if output_length >= MAX_PATH_SEGMENT_LENGTH {
                break;
            }
        }
        if matches!(character, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            output.push('-');
            output_length += 1;
        } else {
            output.push(character);
            output_length += 1;
        }
    }

    let mut cleaned = output.trim_matches(['.', ' ']).to_owned();
    if cleaned.is_empty() {
        return String::from("untitled");
    }
    if is_windows_reserved_filename(&cleaned) {
        cleaned.insert(0, '_');
    }
    cleaned
}

pub(crate) fn compact_timestamp(value: &str) -> String {
    let compact = value
        .chars()
        .filter(|character| character.is_ascii_digit() || matches!(character, 'T' | 'Z'))
        .take(32)
        .collect::<String>();
    if compact.len() >= 16 {
        compact
    } else {
        String::from("export")
    }
}

pub(crate) fn sanitize_error(value: &str) -> String {
    let mut output = redact_url_queries(value);
    output = redact_bearer_tokens(&output);
    output = redact_sensitive_parameters(&output);
    output = redact_jwt_tokens(&output);
    output
        .chars()
        .map(|character| {
            if matches!(character, '\r' | '\n' | '\t') {
                ' '
            } else {
                character
            }
        })
        .take(MAX_ERROR_LENGTH)
        .collect()
}

pub(crate) fn inline_base64_size(value: &str) -> Option<u64> {
    if value.is_empty()
        || !value.len().is_multiple_of(4)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return None;
    }
    let padding = value.as_bytes().iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 || value[..value.len().saturating_sub(padding)].contains('=') {
        return None;
    }
    let groups = u64::try_from(value.len() / 4).ok()?;
    groups
        .checked_mul(3)?
        .checked_sub(u64::try_from(padding).ok()?)
}

fn split_chatgpt_url(value: &str) -> Option<(&'static str, &str)> {
    if value.contains('\\') || value.chars().any(char::is_control) {
        return None;
    }
    CHATGPT_ORIGINS.iter().find_map(|origin| {
        value.strip_prefix(origin).and_then(|path| {
            (path.is_empty() || path.starts_with('/')).then_some((*origin, path))
        })
    })
}

fn conversation_id_from_path(path: &str) -> Option<String> {
    let marker = "/c/";
    let start = path.find(marker)? + marker.len();
    let raw = path[start..].split('/').next().unwrap_or_default();
    let candidate = percent_decode(raw);
    let valid = !candidate.is_empty()
        && candidate.len() <= 128
        && candidate
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    valid.then_some(candidate)
}

fn strip_query_and_fragment(value: &str) -> &str {
    let query = value.find('?').unwrap_or(value.len());
    let fragment = value.find('#').unwrap_or(value.len());
    &value[..query.min(fragment)]
}

fn is_safe_api_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

fn nonempty_identifier(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= 512
        && !trimmed.chars().any(char::is_control);
    valid.then_some(trimmed)
}

pub(crate) fn percent_encode_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    output
}

pub(crate) fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (
                hex_value(bytes[index + 1]),
                hex_value(bytes[index + 2]),
            ) {
                output.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn is_windows_reserved_filename(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn redact_url_queries(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while cursor < value.len() {
        let remaining = &value[cursor..];
        let next_http = match (remaining.find("http://"), remaining.find("https://")) {
            (Some(http), Some(https)) => Some(http.min(https)),
            (Some(http), None) => Some(http),
            (None, Some(https)) => Some(https),
            (None, None) => None,
        };
        let Some(relative_start) = next_http else {
            output.push_str(&value[cursor..]);
            break;
        };
        let start = cursor + relative_start;
        output.push_str(&value[cursor..start]);
        let end = value[start..]
            .find(char::is_whitespace)
            .map_or(value.len(), |offset| start + offset);
        let token = &value[start..end];
        let sensitive = token.find('?').or_else(|| token.find('#'));
        if let Some(index) = sensitive {
            output.push_str(&token[..index]);
            output.push_str("?[redacted]");
        } else {
            output.push_str(token);
        }
        cursor = end;
    }
    output
}

fn redact_bearer_tokens(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let lower = value.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(relative_start) = lower[cursor..].find("bearer ") {
        let start = cursor + relative_start;
        output.push_str(&value[cursor..start]);
        output.push_str("Bearer [redacted]");
        let token_start = start + "bearer ".len();
        let token_end = value[token_start..]
            .find(char::is_whitespace)
            .map_or(value.len(), |offset| token_start + offset);
        cursor = token_end;
    }
    output.push_str(&value[cursor..]);
    output
}

fn redact_sensitive_parameters(value: &str) -> String {
    const KEYS: [&str; 11] = [
        "access_token",
        "authorization",
        "key-pair-id",
        "policy",
        "sig",
        "signature",
        "token",
        "x-amz-credential",
        "x-amz-security-token",
        "x-amz-signature",
        "x-goog-signature",
    ];

    let mut output = value.to_owned();
    for key in KEYS {
        output = redact_parameter(&output, key);
    }
    output
}

fn redact_parameter(value: &str, key: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let needle = format!("{key}=");
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(relative_start) = lower[cursor..].find(&needle) {
        let start = cursor + relative_start;
        output.push_str(&value[cursor..start]);
        output.push_str(&value[start..start + needle.len()]);
        output.push_str("[redacted]");
        let value_start = start + needle.len();
        let value_end = value[value_start..]
            .find(|character: char| character.is_whitespace() || character == '&')
            .map_or(value.len(), |offset| value_start + offset);
        cursor = value_end;
    }
    output.push_str(&value[cursor..]);
    output
}

fn redact_jwt_tokens(value: &str) -> String {
    value
        .split_inclusive(char::is_whitespace)
        .map(|token| {
            let trimmed = token.trim_end_matches(char::is_whitespace);
            let suffix = &token[trimmed.len()..];
            let jwt_like = trimmed.starts_with("eyJ")
                && trimmed.len() >= 30
                && trimmed.matches('.').count() == 2
                && trimmed.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "._-".contains(character)
                });
            if jwt_like {
                format!("[redacted-jwt]{suffix}")
            } else {
                token.to_owned()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        build_sandbox_download_path, conversation_context, inline_base64_size,
        is_allowed_artifact_api_path, normalize_sandbox_path, safe_archive_path,
        sanitize_error, sanitize_path_segment, trusted_https_url,
    };

    #[test]
    fn parses_supported_conversation_urls() -> Result<(), String> {
        let context = conversation_context(
            "https://chatgpt.com/g/example/c/conversation-123?model=test#fragment",
        )?;
        assert_eq!(context.source_url, "https://chatgpt.com/g/example/c/conversation-123");
        assert_eq!(context.id.as_deref(), Some("conversation-123"));
        assert_eq!(
            context.api_path.as_deref(),
            Some("/backend-api/conversation/conversation-123")
        );
        Ok(())
    }

    #[test]
    fn validates_artifact_hosts_conservatively() {
        assert!(trusted_https_url("https://files.oaiusercontent.com/report.pdf"));
        assert!(trusted_https_url("https://cdn.oaistatic.com/image.png"));
        assert!(!trusted_https_url("https://openai.com%40attacker.example/report.pdf"));
        assert!(!trusted_https_url("https://attacker.example/report.pdf"));
        assert!(!trusted_https_url("https://chatgpt.com/\\attacker.example/file"));
    }

    #[test]
    fn validates_artifact_api_paths_conservatively() {
        assert!(is_allowed_artifact_api_path(
            "/backend-api/files/download/file-123?inline=false"
        ));
        assert!(is_allowed_artifact_api_path(
            "/backend-api/conversation/conversation-123/interpreter/download?message_id=message-456"
        ));
        assert!(!is_allowed_artifact_api_path(
            "/backend-api/conversation/conversation-123/other/interpreter/download"
        ));
        assert!(!is_allowed_artifact_api_path(
            "/backend-api/files/download/file-123%2Fescape"
        ));
        assert!(!is_allowed_artifact_api_path("//attacker.example/file"));
    }

    #[test]
    fn normalizes_sandbox_paths_and_rejects_traversal() {
        assert_eq!(
            normalize_sandbox_path("sandbox:/mnt/data/model%20fit.png"),
            Some(String::from("/mnt/data/model fit.png"))
        );
        assert_eq!(
            normalize_sandbox_path("sandbox:/mnt/data/%2e%2e/private.txt"),
            None
        );
        assert_eq!(
            build_sandbox_download_path(
                "conversation-123",
                "message-456",
                "sandbox:/mnt/data/model%20fit.png"
            ),
            Some(String::from(concat!(
                "/backend-api/conversation/conversation-123/interpreter/download?",
                "message_id=message-456&",
                "sandbox_path=%2Fmnt%2Fdata%2Fmodel%20fit.png"
            )))
        );
    }

    #[test]
    fn sanitizes_archive_paths() {
        assert_eq!(
            safe_archive_path("../unsafe/../../report.pdf"),
            "untitled/unsafe/untitled/untitled/report.pdf"
        );
        assert_eq!(sanitize_path_segment("CON.txt"), "_CON.txt");
    }

    #[test]
    fn redacts_credentials() {
        assert_eq!(
            sanitize_error("failed https://example.test/a?sig=secret&token=secret"),
            "failed https://example.test/a?[redacted]"
        );
        assert_eq!(
            sanitize_error("authorization failed: Bearer abc.def.ghi"),
            "authorization failed: Bearer [redacted]"
        );
    }

    #[test]
    fn estimates_base64_payload_size() {
        assert_eq!(inline_base64_size("aGVsbG8="), Some(5));
        assert_eq!(inline_base64_size("not base64"), None);
    }
}
