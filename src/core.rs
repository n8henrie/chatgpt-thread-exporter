use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use serde_json::Value;

use crate::artifacts::build_plans;
use crate::dom::render_dom;
use crate::markdown::{DocumentContext, render_document, render_structured_content};
use crate::model::{
    AlternatePath, BranchPlan, Conversation, ConversationNode, ExportInput, ExportPlan,
    MessagePath, NormalizedMessage, SharedMessage,
};
use crate::security::conversation_id_from_url;

pub(crate) fn build_plan(input: ExportInput) -> ExportPlan {
    let mut warnings = input.warnings;
    let source_url = input.source_url;
    let generated_at = nonempty_or(input.exported_at, "unknown");
    let options = input.options;

    let selection = match input
        .structured_conversation
        .filter(|conversation| !conversation.mapping.is_empty())
    {
        Some(conversation) => structured_selection(
            &conversation,
            &input.dom_conversation.title,
            &source_url,
            options.include_alternate_branches,
            options.max_alternate_branches.clamp(1, 100),
            &mut warnings,
        ),
        None => dom_selection(&input.dom_conversation, &source_url, &mut warnings),
    };

    if selection.active.iter().all(|message| !message.renderable) {
        warnings.push(String::from(
            "No visible user or assistant messages were discovered.",
        ));
    }

    let artifacts = if options.include_artifacts {
        let mut seen_message_ids = HashSet::new();
        build_plans(
            selection
                .active
                .iter()
                .chain(
                    selection
                        .alternates
                        .iter()
                        .flat_map(|branch| branch.messages.iter()),
                )
                .filter(|message| seen_message_ids.insert(message.id.as_str())),
            &input.dom_artifacts,
            &source_url,
            selection.conversation_id.as_deref(),
        )
    } else {
        Vec::new()
    };

    let markdown = render_document(
        &DocumentContext {
            title: &selection.title,
            source_url: &source_url,
            exported_at: &generated_at,
            conversation_id: selection.conversation_id.as_deref(),
            extraction: selection.extraction,
        },
        &selection.active,
        &artifacts,
        "",
    );
    let branches = build_branch_plans(
        &selection,
        &source_url,
        &generated_at,
        &artifacts,
    );

    let root_name = build_root_name(&selection.title, selection.conversation_id.as_deref());
    let extraction = selection.extraction.to_owned();
    let title = selection.title;
    let conversation_id = selection.conversation_id;

    ExportPlan {
        schema_version: crate::model::EXPORT_SCHEMA_VERSION,
        generated_at,
        title,
        source_url,
        conversation_id,
        root_name,
        extraction,
        markdown,
        branches,
        artifacts,
        warnings: deduplicate_strings(warnings),
    }
}

struct Selection {
    title: String,
    conversation_id: Option<String>,
    extraction: &'static str,
    active: MessagePath,
    alternates: Vec<AlternatePath>,
}

fn structured_selection(
    conversation: &Conversation,
    dom_title: &str,
    source_url: &str,
    include_alternates: bool,
    max_alternates: usize,
    warnings: &mut Vec<String>,
) -> Selection {
    let title = if conversation.title.trim().is_empty() {
        nonempty_or(dom_title.to_owned(), "ChatGPT conversation")
    } else {
        conversation.title.clone()
    };
    let url_conversation_id = conversation_id_from_url(source_url);
    let conversation_id = first_nonempty([
        conversation.payload_id.as_deref(),
        conversation.id.as_deref(),
        url_conversation_id.as_deref(),
    ]);

    let active_node = conversation
        .current_node
        .as_deref()
        .filter(|node_id| conversation.mapping.contains_key(*node_id))
        .or_else(|| {
            warnings.push(String::from(
                "The conversation did not expose a valid current node; the latest leaf was selected.",
            ));
            latest_leaf(&conversation.mapping)
        });
    let mut message_cache = HashMap::new();
    let active = active_node.map_or_else(Vec::new, |node_id| {
        build_path(
            &conversation.mapping,
            node_id,
            warnings,
            &mut message_cache,
        )
    });
    let alternates = if include_alternates {
        build_alternates(
            &conversation.mapping,
            &active,
            max_alternates,
            warnings,
            &mut message_cache,
        )
    } else {
        Vec::new()
    };

    Selection {
        title,
        conversation_id,
        extraction: "api",
        active,
        alternates,
    }
}

fn dom_selection(
    dom: &crate::model::DomConversation,
    source_url: &str,
    warnings: &mut Vec<String>,
) -> Selection {
    warnings.push(String::from(concat!(
        "This archive was built from the rendered page. It may omit unloaded messages, ",
        "hidden branches, or non-rendered metadata."
    )));
    let active = dom
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let role = message.role.to_ascii_lowercase();
            Rc::new(NormalizedMessage {
                id: if message.id.trim().is_empty() {
                    format!("dom-message-{}", index + 1)
                } else {
                    message.id.clone()
                },
                renderable: matches!(role.as_str(), "user" | "assistant"),
                role,
                markdown: if message.markdown.trim().is_empty() {
                    render_dom(&message.content)
                } else {
                    message.markdown.trim().to_owned()
                },
                structured_content: None,
                metadata: Value::Null,
            })
        })
        .collect();
    Selection {
        title: nonempty_or(dom.title.clone(), "ChatGPT conversation"),
        conversation_id: conversation_id_from_url(source_url),
        extraction: "dom",
        active,
        alternates: Vec::new(),
    }
}

fn latest_leaf(mapping: &BTreeMap<String, ConversationNode>) -> Option<&str> {
    mapping
        .iter()
        .filter(|(_, node)| node.children.is_empty())
        .max_by(|(_, left), (_, right)| compare_timestamps(left, right))
        .map(|(node_id, _)| node_id.as_str())
        .or_else(|| mapping.keys().next_back().map(String::as_str))
}

fn compare_timestamps(left: &ConversationNode, right: &ConversationNode) -> Ordering {
    let left_time = left
        .message
        .as_ref()
        .and_then(|message| message.create_time)
        .unwrap_or(f64::NEG_INFINITY);
    let right_time = right
        .message
        .as_ref()
        .and_then(|message| message.create_time)
        .unwrap_or(f64::NEG_INFINITY);
    left_time.total_cmp(&right_time)
}

fn build_path<'a>(
    mapping: &'a BTreeMap<String, ConversationNode>,
    leaf_id: &'a str,
    warnings: &mut Vec<String>,
    cache: &mut HashMap<&'a str, SharedMessage>,
) -> MessagePath {
    let mut reverse_nodes = Vec::new();
    let mut visited = HashSet::new();
    let mut current_id = Some(leaf_id);

    while let Some(node_id) = current_id {
        if !visited.insert(node_id) {
            warnings.push(format!(
                "A cycle was detected in the conversation graph at node {node_id}."
            ));
            break;
        }
        let Some(node) = mapping.get(node_id) else {
            warnings.push(format!(
                "Conversation node {node_id} was referenced but not present."
            ));
            break;
        };
        reverse_nodes.push((node_id, node));
        current_id = node.parent.as_deref();
    }

    reverse_nodes
        .into_iter()
        .rev()
        .map(|(node_id, node)| cached_message(cache, node_id, node))
        .collect()
}

fn cached_message<'a>(
    cache: &mut HashMap<&'a str, SharedMessage>,
    node_id: &'a str,
    node: &ConversationNode,
) -> SharedMessage {
    Rc::clone(
        cache
            .entry(node_id)
            .or_insert_with(|| Rc::new(normalize_node(node_id, node))),
    )
}

fn normalize_node(node_id: &str, node: &ConversationNode) -> NormalizedMessage {
    let Some(message) = node.message.as_ref() else {
        return NormalizedMessage {
            id: node_id.to_owned(),
            role: String::from("unknown"),
            markdown: String::new(),
            structured_content: None,
            metadata: Value::Null,
            renderable: false,
        };
    };

    let role = message.author.role.to_ascii_lowercase();
    let content_type = message
        .content
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let hidden = message
        .metadata
        .get("is_visually_hidden_from_conversation")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let internal_content = matches!(
        content_type.as_str(),
        "thoughts"
            | "reasoning_recap"
            | "model_editable_context"
            | "user_editable_context"
    );
    let recipient_is_visible = message
        .recipient
        .as_deref()
        .is_none_or(|recipient| recipient == "all");
    let renderable = matches!(role.as_str(), "user" | "assistant")
        && !hidden
        && !internal_content
        && recipient_is_visible;
    let id = first_nonempty([
        Some(message.id.as_str()),
        Some(node.id.as_str()),
        Some(node_id),
    ])
    .unwrap_or_else(|| node_id.to_owned());
    let markdown = render_structured_content(&message.content);
    NormalizedMessage {
        id,
        role,
        markdown,
        structured_content: Some(message.content.clone()),
        metadata: message.metadata.clone(),
        renderable,
    }
}

fn build_alternates<'a>(
    mapping: &'a BTreeMap<String, ConversationNode>,
    active: &[SharedMessage],
    max_alternates: usize,
    warnings: &mut Vec<String>,
    cache: &mut HashMap<&'a str, SharedMessage>,
) -> Vec<AlternatePath> {
    let active_signature = path_signature(active);
    let mut signatures = HashSet::from([active_signature]);
    let mut output = Vec::new();

    for leaf_id in mapping
        .iter()
        .filter(|(_, node)| node.children.is_empty())
        .map(|(node_id, _)| node_id)
    {
        let mut path_warnings = Vec::new();
        let messages = build_path(mapping, leaf_id, &mut path_warnings, cache);
        let signature = path_signature(&messages);
        if signature.is_empty()
            || !signatures.insert(signature)
            || messages.iter().all(|message| !message.renderable)
        {
            continue;
        }
        warnings.extend(path_warnings);
        output.push(AlternatePath {
            label: describe_branch(&messages, output.len() + 1),
            messages,
        });
        if output.len() >= max_alternates {
            warnings.push(format!(
                "Alternate branch export was limited to {max_alternates} branches."
            ));
            break;
        }
    }
    output
}

fn describe_branch(messages: &[SharedMessage], ordinal: usize) -> String {
    let summary = messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant" && !message.markdown.trim().is_empty())
        .map(|message| summarize(&message.markdown));
    summary.filter(|value| !value.is_empty()).map_or_else(
        || format!("Alternate branch {ordinal}"),
        |value| format!("Alternate branch {ordinal}: {value}"),
    )
}

fn summarize(value: &str) -> String {
    let mut output = String::new();
    let mut output_len = 0;
    let mut previous_space = false;
    for character in value.chars() {
        let replacement = if "#*_`>\r\n".contains(character) {
            ' '
        } else {
            character
        };
        if replacement.is_whitespace() {
            if !previous_space && !output.is_empty() {
                output.push(' ');
                output_len += 1;
            }
            previous_space = true;
        } else {
            output.push(replacement);
            output_len += 1;
            previous_space = false;
        }
        if output_len >= 70 {
            break;
        }
    }
    output.trim().to_owned()
}

fn path_signature(messages: &[SharedMessage]) -> String {
    let mut signature = String::new();
    for message in messages.iter().filter(|message| message.renderable) {
        if !signature.is_empty() {
            signature.push('\0');
        }
        signature.push_str(&message.id);
    }
    signature
}

fn build_branch_plans(
    selection: &Selection,
    source_url: &str,
    exported_at: &str,
    artifacts: &[crate::model::ArtifactPlan],
) -> Vec<BranchPlan> {
    selection
        .alternates
        .iter()
        .enumerate()
        .map(|(index, branch)| {
            let branch_title = format!("{} — {}", selection.title, branch.label);
            let context = DocumentContext {
                title: &branch_title,
                source_url,
                exported_at,
                conversation_id: selection.conversation_id.as_deref(),
                extraction: selection.extraction,
            };
            BranchPlan {
                relative_path: format!("branches/alternate-{:02}.md", index + 1),
                label: branch.label.clone(),
                message_count: branch
                    .messages
                    .iter()
                    .filter(|message| message.renderable)
                    .count(),
                markdown: render_document(&context, &branch.messages, artifacts, "../"),
            }
        })
        .collect()
}

fn build_root_name(title: &str, conversation_id: Option<&str>) -> String {
    let mut slug = String::new();
    let mut slug_len = 0;
    let mut previous_separator = false;
    for character in title.chars().flat_map(char::to_lowercase) {
        if slug_len >= 70 {
            break;
        }
        if character.is_alphanumeric() {
            slug.push(character);
            slug_len += 1;
            previous_separator = false;
        } else if !previous_separator && !slug.is_empty() {
            slug.push('-');
            slug_len += 1;
            previous_separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    let base = if slug.is_empty() { "conversation" } else { slug };
    conversation_id.map_or_else(
        || base.to_owned(),
        |identifier| {
            let suffix: String = identifier
                .chars()
                .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
                .take(16)
                .collect();
            if suffix.is_empty() {
                base.to_owned()
            } else {
                format!("{base}--{suffix}")
            }
        },
    )
}

fn first_nonempty<const N: usize>(values: [Option<&str>; N]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn nonempty_or(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value
    }
}

fn deduplicate_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::rc::Rc;

    use serde_json::json;

    use super::{build_path, build_root_name, conversation_id_from_url};
    use crate::model::{ConversationNode, SharedMessage};

    #[test]
    fn extracts_conversation_identifier() {
        assert_eq!(
            conversation_id_from_url("https://chatgpt.com/g/example/c/abc-123?x=1"),
            Some(String::from("abc-123"))
        );
    }

    #[test]
    fn builds_a_safe_root_name() {
        assert_eq!(
            build_root_name("A useful / conversation", Some("abc-123")),
            "a-useful-conversation--abc-123"
        );
    }

    #[test]
    fn reuses_normalized_messages_across_branches() -> Result<(), serde_json::Error> {
        let mapping: BTreeMap<String, ConversationNode> = serde_json::from_value(json!({
            "root": {
                "id": "root",
                "parent": null,
                "children": ["left", "right"],
                "message": {
                    "id": "user-message",
                    "author": {"role": "user"},
                    "content": {"content_type": "text", "parts": ["Question"]}
                }
            },
            "left": {
                "id": "left",
                "parent": "root",
                "children": [],
                "message": {
                    "id": "left-message",
                    "author": {"role": "assistant"},
                    "content": {"content_type": "text", "parts": ["Left"]}
                }
            },
            "right": {
                "id": "right",
                "parent": "root",
                "children": [],
                "message": {
                    "id": "right-message",
                    "author": {"role": "assistant"},
                    "content": {"content_type": "text", "parts": ["Right"]}
                }
            }
        }))?;
        let mut warnings = Vec::new();
        let mut cache: HashMap<&str, SharedMessage> = HashMap::new();
        let left = build_path(&mapping, "left", &mut warnings, &mut cache);
        let right = build_path(&mapping, "right", &mut warnings, &mut cache);

        assert!(warnings.is_empty());
        assert_eq!(cache.len(), 3);
        assert!(Rc::ptr_eq(&left[0], &right[0]));
        Ok(())
    }
}
