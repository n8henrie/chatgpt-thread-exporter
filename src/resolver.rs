use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::bridge::resolved_file_from_payload;
use crate::model::{
    ArtifactResolution, MAX_INLINE_ARTIFACT_BYTES, MAX_INLINE_EXPORT_BYTES,
    ResolutionRequest, ResolutionStatus,
};
use crate::security::{
    inline_base64_size, is_allowed_artifact_api_path, sanitize_error, trusted_https_url,
};

const CONCURRENCY: usize = 4;

#[derive(Deserialize)]
struct Input {
    id: String,
    resolution: VecDeque<ResolutionRequest>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RequestKind {
    Api,
    Page,
}

struct Pending {
    requests: VecDeque<ResolutionRequest>,
    active: Option<RequestKind>,
    reserved: Option<u64>,
    result: ArtifactResolution,
}

#[derive(Serialize)]
struct Task {
    index: usize,
    #[serde(flatten)]
    request: ResolutionRequest,
}

/// Owns request scheduling and ordered results; the browser performs only I/O.
#[wasm_bindgen]
pub struct ArtifactResolver {
    pending: Vec<Pending>,
    ready: VecDeque<usize>,
    active: usize,
    remaining_bytes: u64,
}

#[wasm_bindgen]
impl ArtifactResolver {
    /// Starts an independent resolver for one export.
    ///
    /// # Errors
    /// Returns an error when the artifact plan is invalid JSON.
    #[wasm_bindgen(constructor)]
    pub fn new(artifacts_json: &str) -> Result<Self, String> {
        let inputs: Vec<Input> = serde_json::from_str(artifacts_json)
            .map_err(|_| String::from("Invalid artifact resolution plan"))?;
        let ready = (0..inputs.len()).collect();
        let pending = inputs
            .into_iter()
            .map(|input| Pending {
                requests: input.resolution,
                active: None,
                reserved: None,
                result: ArtifactResolution {
                    id: input.id,
                    status: ResolutionStatus::Unresolved,
                    error: Some(String::from(
                        "No downloadable artifact representation was found",
                    )),
                    resolved_url: None,
                    resolved_name: None,
                    resolved_mime_type: None,
                    resolved_size_bytes: None,
                    inline_base64: None,
                    inline_mime_type: None,
                },
            })
            .collect();
        Ok(Self {
            pending,
            ready,
            active: 0,
            remaining_bytes: MAX_INLINE_EXPORT_BYTES,
        })
    }

    /// Maximum number of concurrent browser requests for this export.
    #[must_use]
    pub fn worker_count(&self) -> usize {
        CONCURRENCY.min(self.pending.len())
    }

    /// Takes the next request without exceeding the concurrency limit.
    ///
    /// # Errors
    /// Returns an error if internal state or request serialization is invalid.
    pub fn next_request(&mut self) -> Result<Option<String>, String> {
        if self.active >= CONCURRENCY {
            return Ok(None);
        }
        while let Some(index) = self.ready.pop_front() {
            let pending = self.pending.get_mut(index).ok_or("Invalid artifact index")?;
            while let Some(request) = pending.requests.pop_front() {
                let kind = match &request {
                    ResolutionRequest::Api { path } if is_allowed_artifact_api_path(path) => {
                        RequestKind::Api
                    }
                    ResolutionRequest::Page { url }
                        if url.starts_with("blob:") || url.starts_with("data:") =>
                    {
                        RequestKind::Page
                    }
                    ResolutionRequest::Remote { url } if trusted_https_url(url) => {
                        pending.result.status = ResolutionStatus::Resolved;
                        pending.result.resolved_url = Some(url.clone());
                        pending.result.error = None;
                        break;
                    }
                    ResolutionRequest::Unresolved { reason } => {
                        pending.result.error = Some(sanitize_error(reason));
                        continue;
                    }
                    _ => {
                        pending.result.error = Some(String::from("Untrusted artifact request"));
                        continue;
                    }
                };
                let json = serde_json::to_string(&Task { index, request })
                    .map_err(|_| String::from("Could not encode artifact request"))?;
                pending.active = Some(kind);
                self.active += 1;
                return Ok(Some(json));
            }
        }
        Ok(None)
    }

    /// Reports a browser response and schedules the next fallback on failure.
    ///
    /// # Errors
    /// Returns an error for a completion without a matching active request.
    pub fn complete(&mut self, index: usize, response_json: &str) -> Result<(), String> {
        let pending = self.pending.get_mut(index).ok_or("Invalid artifact index")?;
        let kind = pending
            .active
            .take()
            .ok_or("Artifact request is not active")?;
        self.active -= 1;
        let reserved = pending.reserved.take();
        let result = apply_response(pending, kind, response_json, reserved);
        if let Err(error) = result {
            self.remaining_bytes += reserved.unwrap_or_default();
            pending.result.error = Some(sanitize_error(&error));
            self.ready.push_front(index);
        }
        Ok(())
    }

    /// Reserves decoded inline bytes before the browser starts reading them.
    ///
    /// # Errors
    /// Returns an error for an inactive request, duplicate reservation, or exceeded limit.
    pub fn reserve_inline(&mut self, index: usize, bytes: u64) -> Result<(), String> {
        let pending = self.pending.get_mut(index).ok_or("Invalid artifact index")?;
        if pending.active != Some(RequestKind::Page) || pending.reserved.is_some() {
            return Err(String::from("Invalid inline reservation"));
        }
        if bytes > MAX_INLINE_ARTIFACT_BYTES || bytes > self.remaining_bytes {
            return Err(String::from(
                "The in-page artifact exceeded the inline transfer limit",
            ));
        }
        self.remaining_bytes -= bytes;
        pending.reserved = Some(bytes);
        Ok(())
    }

    /// Returns results in the original artifact order, regardless of completion order.
    ///
    /// # Errors
    /// Returns an error if requests remain or results cannot be serialized.
    pub fn finish(&self) -> Result<String, String> {
        if self.active != 0 || !self.ready.is_empty() {
            return Err(String::from("Artifact resolution is still running"));
        }
        let results: Vec<_> = self.pending.iter().map(|pending| &pending.result).collect();
        serde_json::to_string(&results)
            .map_err(|_| String::from("Could not encode artifact resolutions"))
    }
}

fn apply_response(
    pending: &mut Pending,
    kind: RequestKind,
    json: &str,
    reserved: Option<u64>,
) -> Result<(), String> {
    let response: Value =
        serde_json::from_str(json).map_err(|_| String::from("Invalid artifact response"))?;
    if let Some(error) = response.get("error").and_then(Value::as_str) {
        return Err(sanitize_error(error));
    }
    match kind {
        RequestKind::Api => {
            let file = resolved_file_from_payload(&response["value"])?;
            pending.result.status = ResolutionStatus::Resolved;
            pending.result.resolved_url = Some(file.url);
            pending.result.resolved_name = file.name;
            pending.result.resolved_mime_type = file.mime_type;
            pending.result.resolved_size_bytes = file.size_bytes;
        }
        RequestKind::Page => {
            let base64 = response["inline_base64"]
                .as_str()
                .ok_or("Missing inline data")?;
            let bytes = inline_base64_size(base64).ok_or("Invalid inline data")?;
            if reserved != Some(bytes) {
                return Err(String::from("Inline data did not match its reservation"));
            }
            pending.result.status = ResolutionStatus::ResolvedInline;
            pending.result.inline_base64 = Some(base64.to_owned());
            pending.result.inline_mime_type = Some(
                response["inline_mime_type"]
                    .as_str()
                    .unwrap_or("application/octet-stream")
                    .to_owned(),
            );
            pending.result.resolved_size_bytes = Some(bytes);
        }
    }
    pending.result.error = None;
    Ok(())
}
