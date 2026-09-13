use std::collections::BTreeMap;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const EXPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
pub(crate) struct ExportInput {
    #[serde(default)]
    pub(crate) source_url: String,
    #[serde(default)]
    pub(crate) exported_at: String,
    #[serde(default)]
    pub(crate) structured_conversation: Option<Conversation>,
    #[serde(default)]
    pub(crate) dom_conversation: DomConversation,
    #[serde(default)]
    pub(crate) dom_artifacts: Vec<DomArtifact>,
    #[serde(default)]
    pub(crate) options: ExportOptions,
    #[serde(default)]
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Conversation {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default, rename = "conversation_id")]
    pub(crate) payload_id: Option<String>,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) current_node: Option<String>,
    #[serde(default)]
    pub(crate) mapping: BTreeMap<String, ConversationNode>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ConversationNode {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) parent: Option<String>,
    #[serde(default)]
    pub(crate) children: Vec<String>,
    #[serde(default)]
    pub(crate) message: Option<Message>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Message {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) author: Author,
    #[serde(default)]
    pub(crate) content: Value,
    #[serde(default)]
    pub(crate) metadata: Value,
    #[serde(default)]
    pub(crate) recipient: Option<String>,
    #[serde(default)]
    pub(crate) create_time: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Author {
    #[serde(default)]
    pub(crate) role: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DomConversation {
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) messages: Vec<DomMessage>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DomMessage {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) role: String,
    #[serde(default)]
    pub(crate) markdown: String,
    #[serde(default)]
    pub(crate) content: Vec<DomNode>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum DomNode {
    Text {
        #[serde(default)]
        text: String,
    },
    Element {
        #[serde(default)]
        tag: String,
        #[serde(default)]
        attributes: BTreeMap<String, String>,
        #[serde(default)]
        children: Vec<Self>,
    },
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DomArtifact {
    #[serde(default)]
    pub(crate) direction: String,
    #[serde(default)]
    pub(crate) message_id: String,
    #[serde(default)]
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) mime_type: Option<String>,
    #[serde(default)]
    pub(crate) size_bytes: Option<u64>,
    #[serde(default)]
    pub(crate) source: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct ExportOptions {
    #[serde(default = "default_true")]
    pub(crate) include_artifacts: bool,
    #[serde(default = "default_true")]
    pub(crate) include_alternate_branches: bool,
    #[serde(default = "default_max_branches")]
    pub(crate) max_alternate_branches: usize,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_artifacts: true,
            include_alternate_branches: true,
            max_alternate_branches: default_max_branches(),
        }
    }
}

const fn default_true() -> bool {
    true
}

const fn default_max_branches() -> usize {
    20
}

#[derive(Debug)]
pub(crate) struct NormalizedMessage {
    pub(crate) id: String,
    pub(crate) role: String,
    pub(crate) markdown: String,
    pub(crate) structured_content: Option<Value>,
    pub(crate) metadata: Value,
    pub(crate) renderable: bool,
}

pub(crate) type SharedMessage = Rc<NormalizedMessage>;
pub(crate) type MessagePath = Vec<SharedMessage>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Direction {
    Submitted,
    Received,
}

impl Direction {
    pub(crate) const fn directory(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Received => "received",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PointerKind {
    Sediment,
    Sandbox,
    Blob,
    Data,
    Https,
    RelativeUrl,
    FileId,
    Unknown,
}

#[derive(Debug)]
pub(crate) struct ArtifactCandidate {
    pub(crate) direction: Direction,
    pub(crate) message_id: String,
    pub(crate) pointer: String,
    pub(crate) pointer_aliases: Vec<String>,
    pub(crate) pointer_kind: PointerKind,
    pub(crate) suggested_name: String,
    pub(crate) mime_type: Option<String>,
    pub(crate) size_bytes: Option<u64>,
    pub(crate) source: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ResolutionRequest {
    Api { path: String },
    Page { url: String },
    Remote { url: String },
    Unresolved { reason: String },
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ArtifactPlan {
    pub(crate) id: String,
    pub(crate) direction: Direction,
    pub(crate) message_id: String,
    pub(crate) pointer: String,
    pub(crate) pointer_aliases: Vec<String>,
    pub(crate) pointer_kind: PointerKind,
    pub(crate) suggested_name: String,
    pub(crate) mime_type: Option<String>,
    pub(crate) size_bytes: Option<u64>,
    pub(crate) relative_path: String,
    pub(crate) source: String,
    pub(crate) resolution: Vec<ResolutionRequest>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct BranchPlan {
    pub(crate) relative_path: String,
    pub(crate) label: String,
    pub(crate) message_count: usize,
    pub(crate) markdown: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ExportPlan {
    pub(crate) schema_version: u32,
    pub(crate) generated_at: String,
    pub(crate) title: String,
    pub(crate) source_url: String,
    pub(crate) conversation_id: Option<String>,
    pub(crate) root_name: String,
    pub(crate) extraction: String,
    pub(crate) markdown: String,
    pub(crate) branches: Vec<BranchPlan>,
    pub(crate) artifacts: Vec<ArtifactPlan>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct AlternatePath {
    pub(crate) label: String,
    pub(crate) messages: MessagePath,
}

#[derive(Debug, Serialize)]
pub(crate) struct ConversationContext {
    pub(crate) source_url: String,
    pub(crate) origin: String,
    #[serde(rename = "conversation_id")]
    pub(crate) id: Option<String>,
    #[serde(rename = "conversation_path")]
    pub(crate) api_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RuntimeInfo {
    pub(crate) implementation: &'static str,
    pub(crate) version: &'static str,
    pub(crate) export_schema_version: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResolvedFile {
    pub(crate) url: String,
    pub(crate) name: Option<String>,
    pub(crate) mime_type: Option<String>,
    pub(crate) size_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArchiveInput {
    pub(crate) plan: ExportPlan,
    #[serde(default)]
    pub(crate) resolutions: Vec<ArtifactResolution>,
    #[serde(default)]
    pub(crate) raw_conversation: Option<Value>,
    #[serde(default)]
    pub(crate) extension_version: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResolutionStatus {
    Resolved,
    ResolvedInline,
    Unresolved,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArtifactResolution {
    #[serde(rename = "artifact_id")]
    pub(crate) id: String,
    pub(crate) status: ResolutionStatus,
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) resolved_url: Option<String>,
    #[serde(default)]
    pub(crate) resolved_name: Option<String>,
    #[serde(default)]
    pub(crate) resolved_mime_type: Option<String>,
    #[serde(default)]
    pub(crate) resolved_size_bytes: Option<u64>,
    #[serde(default)]
    pub(crate) inline_base64: Option<String>,
    #[serde(default)]
    pub(crate) inline_mime_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ArchivePlan {
    pub(crate) jobs: Vec<ArchiveJob>,
    pub(crate) unresolved: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct ArchiveJob {
    pub(crate) path: String,
    #[serde(flatten)]
    pub(crate) source: ArchiveSource,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ArchiveSource {
    Inline {
        base64: String,
        mime_type: String,
    },
    Remote {
        url: String,
    },
    Text {
        text: String,
        mime_type: String,
    },
}

#[cfg(test)]
mod tests {
    use super::Conversation;

    #[test]
    fn deserializes_conversation_identifier() -> Result<(), serde_json::Error> {
        let conversation: Conversation =
            serde_json::from_str(r#"{"conversation_id":"example-id"}"#)?;
        assert_eq!(conversation.payload_id.as_deref(), Some("example-id"));
        Ok(())
    }
}
