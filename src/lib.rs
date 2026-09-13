mod archive;
mod artifacts;
mod bridge;
mod core;
mod dom;
mod markdown;
mod model;
mod security;

use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::model::RuntimeInfo;

/// An error produced while decoding an exporter request or encoding its response.
#[derive(Debug)]
pub struct ExportError {
    message: String,
}

impl ExportError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for ExportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ExportError {}

/// Builds a serializable export plan from `ChatGPT` API data and a DOM fallback.
///
/// # Errors
///
/// Returns an error when `input_json` does not match the exporter input schema,
/// or when the resulting plan cannot be serialized as JSON.
pub fn build_export_plan_json(input_json: &str) -> Result<String, ExportError> {
    let input = decode_json(input_json, "exporter input")?;
    encode_json(&core::build_plan(input), "export plan")
}

/// Validates and canonicalizes a `ChatGPT` conversation URL.
///
/// # Errors
///
/// Returns an error when the URL is not a supported HTTPS `ChatGPT` URL.
pub fn conversation_context_json(url: &str) -> Result<String, ExportError> {
    let context = security::conversation_context(url).map_err(ExportError::new)?;
    encode_json(&context, "conversation context")
}

/// Selects an account identifier from the account-check response.
///
/// # Errors
///
/// Returns an error when `payload_json` is not valid JSON or the result cannot
/// be serialized.
pub fn select_account_id_json(
    payload_json: &str,
    workspace_id: &str,
) -> Result<String, ExportError> {
    let payload: Value = decode_json(payload_json, "account response")?;
    let workspace_id = (!workspace_id.trim().is_empty()).then_some(workspace_id.trim());
    encode_json(
        &bridge::select_account_id(&payload, workspace_id),
        "account selection",
    )
}

/// Parses and validates a file-resolver response.
///
/// # Errors
///
/// Returns an error when the response is invalid or contains an untrusted URL.
pub fn parse_resolved_file_json(payload_json: &str) -> Result<String, ExportError> {
    let payload: Value = decode_json(payload_json, "file resolver response")?;
    let file = bridge::resolved_file_from_payload(&payload).map_err(ExportError::new)?;
    encode_json(&file, "resolved file")
}

/// Builds a sanitized archive download plan.
///
/// # Errors
///
/// Returns an error when the input is malformed or archive metadata cannot be
/// serialized.
pub fn build_archive_plan_json(input_json: &str) -> Result<String, ExportError> {
    let input = decode_json(input_json, "archive input")?;
    let plan = archive::build_archive(input).map_err(ExportError::new)?;
    encode_json(&plan, "archive plan")
}

/// Returns build metadata for the loaded `Rust`/`WebAssembly` core.
///
/// # Errors
///
/// Returns an error only if the static runtime metadata cannot be serialized.
pub fn runtime_info_json() -> Result<String, ExportError> {
    encode_json(
        &RuntimeInfo {
            implementation: "rust-wasm",
            version: env!("CARGO_PKG_VERSION"),
            export_schema_version: crate::model::EXPORT_SCHEMA_VERSION,
        },
        "runtime information",
    )
}

/// Redacts credentials and temporary signed-URL data from an error message.
#[must_use]
pub fn sanitize_error_message(message: &str) -> String {
    security::sanitize_error(message)
}

fn decode_json<T: DeserializeOwned>(input: &str, label: &str) -> Result<T, ExportError> {
    serde_json::from_str(input)
        .map_err(|error| ExportError::new(format!("Invalid {label} JSON: {error}")))
}

fn encode_json<T: Serialize>(value: &T, label: &str) -> Result<String, ExportError> {
    serde_json::to_string(value)
        .map_err(|error| ExportError::new(format!("Could not encode {label}: {error}")))
}

fn js_error(error: ExportError) -> JsValue {
    let ExportError { message } = error;
    JsValue::from_str(&message)
}

/// `WebAssembly` entry point for export-plan construction.
///
/// # Errors
///
/// Returns a `JavaScript` error string when the input or output JSON is invalid.
#[wasm_bindgen]
pub fn build_export_plan(input_json: &str) -> Result<String, JsValue> {
    build_export_plan_json(input_json).map_err(js_error)
}

/// `WebAssembly` entry point for conversation URL validation.
///
/// # Errors
///
/// Returns a `JavaScript` error string for unsupported URLs.
#[wasm_bindgen]
pub fn conversation_context(url: &str) -> Result<String, JsValue> {
    conversation_context_json(url).map_err(js_error)
}

/// `WebAssembly` entry point for account selection.
///
/// # Errors
///
/// Returns a `JavaScript` error string when the input or output JSON is invalid.
#[wasm_bindgen]
pub fn select_account_id(payload_json: &str, workspace_id: &str) -> Result<String, JsValue> {
    select_account_id_json(payload_json, workspace_id).map_err(js_error)
}

/// `WebAssembly` entry point for file-resolver response parsing.
///
/// # Errors
///
/// Returns a `JavaScript` error string for malformed or untrusted responses.
#[wasm_bindgen]
pub fn parse_resolved_file(payload_json: &str) -> Result<String, JsValue> {
    parse_resolved_file_json(payload_json).map_err(js_error)
}

/// `WebAssembly` entry point for archive-plan construction.
///
/// # Errors
///
/// Returns a `JavaScript` error string when the input or output JSON is invalid.
#[wasm_bindgen]
pub fn build_archive_plan(input_json: &str) -> Result<String, JsValue> {
    build_archive_plan_json(input_json).map_err(js_error)
}

/// `WebAssembly` entry point for runtime build metadata.
///
/// # Errors
///
/// Returns a `JavaScript` error string if the metadata cannot be serialized.
#[wasm_bindgen]
pub fn runtime_info() -> Result<String, JsValue> {
    runtime_info_json().map_err(js_error)
}

/// `WebAssembly` entry point for error redaction.
#[wasm_bindgen]
#[must_use]
pub fn sanitize_error(message: &str) -> String {
    sanitize_error_message(message)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{build_export_plan_json, conversation_context_json, runtime_info_json};

    #[test]
    fn builds_a_dom_fallback_plan() -> Result<(), Box<dyn std::error::Error>> {
        let input = r#"
        {
          "source_url": "https://chatgpt.com/c/example",
          "exported_at": "2026-08-04T17:00:00Z",
          "structured_conversation": null,
          "dom_conversation": {
            "title": "Example",
            "messages": [
              {"id": "u1", "role": "user", "markdown": "Hello"},
              {"id": "a1", "role": "assistant", "markdown": "Hi"}
            ]
          },
          "dom_artifacts": [],
          "options": {"include_artifacts": true}
        }
        "#;
        let output = build_export_plan_json(input)?;
        let plan: Value = serde_json::from_str(&output)?;
        assert_eq!(plan["extraction"], "dom");
        assert!(
            plan["markdown"]
                .as_str()
                .is_some_and(|text| text.contains("## User"))
        );
        Ok(())
    }

    #[test]
    fn canonicalizes_a_conversation_url() -> Result<(), Box<dyn std::error::Error>> {
        let output = conversation_context_json(
            "https://chatgpt.com/c/conversation-123?model=example#fragment",
        )?;
        let context: Value = serde_json::from_str(&output)?;
        assert_eq!(
            context["source_url"],
            "https://chatgpt.com/c/conversation-123"
        );
        assert_eq!(context["conversation_id"], "conversation-123");
        Ok(())
    }

    #[test]
    fn reports_runtime_build_information() -> Result<(), Box<dyn std::error::Error>> {
        let output = runtime_info_json()?;
        let information: Value = serde_json::from_str(&output)?;
        assert_eq!(information["implementation"], "rust-wasm");
        assert_eq!(information["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(information["export_schema_version"], 1);
        Ok(())
    }
}
