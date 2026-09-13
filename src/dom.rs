use std::collections::BTreeMap;

use crate::model::DomNode;

#[derive(Clone, Copy)]
struct RenderContext {
    list_depth: usize,
}

pub(crate) fn render_dom(nodes: &[DomNode]) -> String {
    normalize_markdown(
        &nodes
            .iter()
            .map(|node| render_node(node, RenderContext { list_depth: 0 }))
            .collect::<String>(),
    )
}

fn render_node(node: &DomNode, context: RenderContext) -> String {
    match node {
        DomNode::Text { text } => collapse_inline_whitespace(text),
        DomNode::Element {
            tag,
            attributes,
            children,
        } => render_element(tag, attributes, children, context),
    }
}

fn render_element(
    raw_tag: &str,
    attributes: &BTreeMap<String, String>,
    children: &[DomNode],
    context: RenderContext,
) -> String {
    let tag = raw_tag.to_ascii_lowercase();
    if matches!(
        tag.as_str(),
        "script" | "style" | "svg" | "button" | "nav" | "textarea"
    ) || attributes.get("aria-hidden").is_some_and(|value| value == "true")
    {
        return String::new();
    }

    if let Some(math) = render_math(&tag, attributes, children) {
        return math;
    }

    match tag.as_str() {
        "br" => String::from("\n"),
        "hr" => String::from("\n\n---\n\n"),
        "pre" => render_preformatted(attributes, children),
        "code" => render_inline_code(&collect_text(children)),
        "a" => render_link(attributes, children, context),
        "img" => {
            let alt = attributes
                .get("alt")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .unwrap_or("image");
            format!("[Image: {}]", escape_markdown_text(alt))
        }
        "strong" | "b" => format!("**{}**", render_children(children, context).trim()),
        "em" | "i" => format!("*{}*", render_children(children, context).trim()),
        "del" | "s" => format!("~~{}~~", render_children(children, context).trim()),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag
                .strip_prefix('h')
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            format!(
                "\n\n{} {}\n\n",
                "#".repeat(level),
                render_children(children, context).trim()
            )
        }
        "blockquote" => {
            let body = normalize_markdown(&render_children(children, context));
            let quoted = body
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n\n{quoted}\n\n")
        }
        "table" => render_table(children),
        "ul" => render_list(false, attributes, children, context),
        "ol" => render_list(true, attributes, children, context),
        "li" => render_children(
            children,
            RenderContext {
                list_depth: context.list_depth.saturating_add(1),
            },
        ),
        "p" | "div" | "section" | "article" | "main" | "header" | "footer"
        | "figure" | "figcaption" | "dl" | "dt" | "dd" => {
            format!("\n\n{}\n\n", render_children(children, context))
        }
        _ => render_children(children, context),
    }
}

fn render_children(children: &[DomNode], context: RenderContext) -> String {
    children
        .iter()
        .map(|child| render_node(child, context))
        .collect()
}

fn render_preformatted(
    attributes: &BTreeMap<String, String>,
    children: &[DomNode],
) -> String {
    let mut text = collect_text(children);
    if text.ends_with('\n') {
        text.pop();
    }
    let language = discover_code_language(attributes, children);
    let fence = "`".repeat(longest_backtick_run(&text).saturating_add(1).max(3));
    format!("\n\n{fence}{language}\n{text}\n{fence}\n\n")
}

fn discover_code_language(
    attributes: &BTreeMap<String, String>,
    children: &[DomNode],
) -> String {
    attributes
        .get("data-language")
        .and_then(|value| safe_language(value))
        .or_else(|| {
            attributes
                .get("class")
                .and_then(|value| language_from_classes(value))
        })
        .or_else(|| {
            children.iter().find_map(|child| match child {
                DomNode::Text { .. } => None,
                DomNode::Element { attributes, .. } => attributes
                    .get("class")
                    .and_then(|value| language_from_classes(value)),
            })
        })
        .unwrap_or_default()
}

fn language_from_classes(value: &str) -> Option<String> {
    value
        .split_whitespace()
        .find_map(|class_name| class_name.strip_prefix("language-"))
        .and_then(safe_language)
}

fn safe_language(value: &str) -> Option<String> {
    let output = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '+' | '.' | '-')
        })
        .take(40)
        .collect::<String>();
    (!output.is_empty()).then_some(output)
}

fn render_inline_code(text: &str) -> String {
    let fence = "`".repeat(longest_backtick_run(text).saturating_add(1));
    let padding = if text.chars().next().is_some_and(char::is_whitespace)
        || text.chars().next_back().is_some_and(char::is_whitespace)
    {
        " "
    } else {
        ""
    };
    format!("{fence}{padding}{text}{padding}{fence}")
}

fn render_link(
    attributes: &BTreeMap<String, String>,
    children: &[DomNode],
    context: RenderContext,
) -> String {
    let rendered = normalize_markdown(&render_children(children, context));
    let label = if rendered.is_empty() {
        collect_text(children).trim().to_owned()
    } else {
        rendered
    };
    let label = if label.is_empty() {
        String::from("link")
    } else {
        label
    };
    let Some(href) = attributes.get("href") else {
        return label;
    };
    if !is_safe_reference_url(href) {
        return label;
    }
    format!(
        "[{}]({})",
        escape_markdown_label(&label),
        escape_markdown_destination(href)
    )
}

fn render_list(
    ordered: bool,
    attributes: &BTreeMap<String, String>,
    children: &[DomNode],
    context: RenderContext,
) -> String {
    let start = attributes
        .get("start")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    let indent = "  ".repeat(context.list_depth);
    let items = children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element { tag, children, .. } if tag.eq_ignore_ascii_case("li") => {
                Some(children)
            }
            _ => None,
        })
        .enumerate()
        .map(|(index, item_children)| {
            let marker = if ordered {
                format!("{}.", start.saturating_add(index))
            } else {
                String::from("-")
            };
            let body = normalize_markdown(&render_children(
                item_children,
                RenderContext {
                    list_depth: context.list_depth.saturating_add(1),
                },
            ));
            let indented = body
                .lines()
                .enumerate()
                .map(|(line_index, line)| {
                    if line_index == 0 {
                        line.to_owned()
                    } else {
                        format!("{indent}  {line}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("{indent}{marker} {indented}")
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", items.join("\n"))
    }
}

fn render_table(children: &[DomNode]) -> String {
    let mut rows = Vec::new();
    collect_table_rows(children, &mut rows);
    if rows.is_empty() {
        return String::new();
    }

    let mut matrix = rows
        .into_iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    normalize_markdown(&render_children(cell, RenderContext { list_depth: 0 }))
                        .replace('|', "\\|")
                        .replace('\n', "<br>")
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let width = matrix.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return String::new();
    }
    for row in &mut matrix {
        row.resize(width, String::new());
    }

    let mut lines = Vec::with_capacity(matrix.len().saturating_add(1));
    lines.push(format!("| {} |", matrix[0].join(" | ")));
    lines.push(format!(
        "| {} |",
        vec!["---"; width].join(" | ")
    ));
    lines.extend(
        matrix
            .iter()
            .skip(1)
            .map(|row| format!("| {} |", row.join(" | "))),
    );
    format!("\n\n{}\n\n", lines.join("\n"))
}

fn collect_table_rows<'a>(nodes: &'a [DomNode], output: &mut Vec<Vec<&'a [DomNode]>>) {
    for node in nodes {
        let DomNode::Element { tag, children, .. } = node else {
            continue;
        };
        if tag.eq_ignore_ascii_case("tr") {
            output.push(
                children
                    .iter()
                    .filter_map(|child| match child {
                        DomNode::Element { tag, children, .. }
                            if tag.eq_ignore_ascii_case("td")
                                || tag.eq_ignore_ascii_case("th") =>
                        {
                            Some(children.as_slice())
                        }
                        _ => None,
                    })
                    .collect(),
            );
        } else {
            collect_table_rows(children, output);
        }
    }
}

fn render_math(
    tag: &str,
    attributes: &BTreeMap<String, String>,
    children: &[DomNode],
) -> Option<String> {
    let classes = attributes.get("class").map_or("", String::as_str);
    let is_display = classes
        .split_whitespace()
        .any(|class_name| class_name == "katex-display");
    let is_math = is_display
        || classes
            .split_whitespace()
            .any(|class_name| class_name == "katex")
        || attributes.contains_key("data-math")
        || tag == "math";
    if !is_math {
        return None;
    }

    let tex = attributes
        .get("data-math")
        .cloned()
        .or_else(|| find_tex_annotation(children))?;
    Some(if is_display {
        format!("\n\n$$\n{tex}\n$$\n\n")
    } else {
        format!("${tex}$")
    })
}

fn find_tex_annotation(nodes: &[DomNode]) -> Option<String> {
    nodes.iter().find_map(|node| match node {
        DomNode::Text { .. } => None,
        DomNode::Element {
            tag,
            attributes,
            children,
        } => {
            let annotation = tag.eq_ignore_ascii_case("annotation")
                && attributes
                    .get("encoding")
                    .is_some_and(|value| value == "application/x-tex");
            if annotation {
                Some(collect_text(children))
            } else {
                find_tex_annotation(children)
            }
        }
    })
}

fn collect_text(nodes: &[DomNode]) -> String {
    let mut output = String::new();
    append_text(nodes, &mut output);
    output
}

fn append_text(nodes: &[DomNode], output: &mut String) {
    for node in nodes {
        match node {
            DomNode::Text { text } => output.push_str(text),
            DomNode::Element { children, .. } => append_text(children, output),
        }
    }
}

fn collapse_inline_whitespace(value: &str) -> String {
    let mut output = String::new();
    let mut previous_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            if !previous_space {
                output.push(' ');
            }
            previous_space = true;
        } else {
            output.push(character);
            previous_space = false;
        }
    }
    output
}

fn normalize_markdown(value: &str) -> String {
    collapse_blank_lines(
        &value
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .trim()
    .to_owned()
}

fn collapse_blank_lines(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut newline_run = 0;
    for character in value.chars() {
        if character == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                output.push(character);
            }
        } else {
            newline_run = 0;
            output.push(character);
        }
    }
    output
}

fn longest_backtick_run(value: &str) -> usize {
    value
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0)
}

fn is_safe_reference_url(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
}

fn escape_markdown_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if "\\`*_{}[]()#+.!|>-".contains(character) {
            output.push('\\');
        }
        output.push(character);
    }
    output
}

fn escape_markdown_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]")
}

fn escape_markdown_destination(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::model::DomNode;

    use super::render_dom;

    #[test]
    fn renders_structured_dom_as_markdown() {
        let nodes = vec![DomNode::Element {
            tag: String::from("div"),
            attributes: BTreeMap::new(),
            children: vec![
                DomNode::Element {
                    tag: String::from("h2"),
                    attributes: BTreeMap::new(),
                    children: vec![DomNode::Text {
                        text: String::from("Heading"),
                    }],
                },
                DomNode::Element {
                    tag: String::from("p"),
                    attributes: BTreeMap::new(),
                    children: vec![DomNode::Text {
                        text: String::from("Hello world"),
                    }],
                },
            ],
        }];

        assert_eq!(render_dom(&nodes), "## Heading\n\nHello world");
    }
}
