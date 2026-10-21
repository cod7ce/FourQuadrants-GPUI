use super::{Workspace, chip};
use gpui::{
    AnyElement, Context, FontWeight, ObjectFit, SharedString, Window, div, img, prelude::*, px,
};
use gpui_component::{
    ActiveTheme,
    text::{TextView, TextViewStyle},
};
use markdown::mdast::{Definition, Node};
use std::collections::HashMap;

type Definitions<'a> = HashMap<String, &'a Definition>;

#[derive(Debug, PartialEq)]
enum PreviewPart {
    Markdown(String),
    Image { url: String, alt: String },
    Attachment { url: String, label: String },
}

fn reference_key(identifier: &str) -> String {
    identifier
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn collect_definitions<'a>(node: &'a Node, definitions: &mut Definitions<'a>) {
    if let Node::Definition(definition) = node {
        definitions
            .entry(reference_key(&definition.identifier))
            .or_insert(definition);
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_definitions(child, definitions);
        }
    }
}

/// AST text is already decoded. Escape it before passing it through Markdown again,
/// including HTML and autolink delimiters. Soft line breaks become display spaces.
fn literal_markdown(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            '\n' | '\r' => escaped.push(' '),
            c if c.is_ascii_punctuation() => {
                escaped.push('\\');
                escaped.push(c);
            }
            c => escaped.push(c),
        }
    }
    escaped
}

fn inline_code(value: &str) -> String {
    let length = value.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1;
    let fence = "`".repeat(length);
    if value.chars().all(|c| c == ' ') {
        format!("{fence}{value}{fence}")
    } else {
        format!("{fence} {value} {fence}")
    }
}

fn destination(url: &str) -> String {
    let escaped = url
        .replace('&', "&amp;")
        .replace('<', "%3C")
        .replace('>', "%3E")
        .replace('\\', "%5C")
        .replace(['\n', '\r'], "%0A");
    format!("<{escaped}>")
}

fn push_markdown(parts: &mut Vec<PreviewPart>, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(PreviewPart::Markdown(previous)) = parts.last_mut() {
        previous.push_str(&text);
    } else {
        parts.push(PreviewPart::Markdown(text));
    }
}

fn wrap_parts(parts: Vec<PreviewPart>, prefix: &str, suffix: &str) -> Vec<PreviewPart> {
    parts
        .into_iter()
        .map(|part| match part {
            PreviewPart::Markdown(text) if !text.trim().is_empty() => {
                // A native image can split a mark in the middle of its text. Keep
                // edge spaces outside the delimiters so each fragment stays valid.
                let start = text.len() - text.trim_start().len();
                let end = text.trim_end().len();
                PreviewPart::Markdown(format!(
                    "{}{prefix}{}{suffix}{}",
                    &text[..start],
                    &text[start..end],
                    &text[end..],
                ))
            }
            part => part,
        })
        .collect()
}

fn link_parts(children: &[Node], url: &str, definitions: &Definitions<'_>) -> Vec<PreviewPart> {
    if url.starts_with("attachment:") {
        let mut parts = inline_parts(children, definitions);
        parts.retain(|part| !matches!(part, PreviewPart::Markdown(_)));
        parts.push(PreviewPart::Attachment {
            url: url.into(),
            label: children.iter().map(Node::to_string).collect(),
        });
        parts
    } else {
        wrap_parts(
            inline_parts(children, definitions),
            "[",
            &format!("]({})", destination(url)),
        )
    }
}

fn inline_parts(children: &[Node], definitions: &Definitions<'_>) -> Vec<PreviewPart> {
    let mut parts = Vec::new();
    for child in children {
        let children = match child {
            Node::Text(text) => vec![PreviewPart::Markdown(literal_markdown(&text.value))],
            Node::Html(html) => vec![PreviewPart::Markdown(literal_markdown(&html.value))],
            Node::InlineCode(code) => vec![PreviewPart::Markdown(inline_code(&code.value))],
            Node::Break(_) => vec![PreviewPart::Markdown("  \n".into())],
            Node::Emphasis(emphasis) => {
                wrap_parts(inline_parts(&emphasis.children, definitions), "*", "*")
            }
            Node::Strong(strong) => {
                wrap_parts(inline_parts(&strong.children, definitions), "**", "**")
            }
            Node::Delete(delete) => {
                wrap_parts(inline_parts(&delete.children, definitions), "~~", "~~")
            }
            Node::Link(link) => link_parts(&link.children, &link.url, definitions),
            Node::LinkReference(link) => match definitions.get(&reference_key(&link.identifier)) {
                Some(definition) => link_parts(&link.children, &definition.url, definitions),
                None => inline_parts(&link.children, definitions),
            },
            Node::Image(image) => vec![PreviewPart::Image {
                url: image.url.clone(),
                alt: image.alt.clone(),
            }],
            Node::ImageReference(image) => vec![PreviewPart::Image {
                url: definitions
                    .get(&reference_key(&image.identifier))
                    .map(|definition| definition.url.clone())
                    .unwrap_or_default(),
                alt: image.alt.clone(),
            }],
            Node::FootnoteReference(note) => vec![PreviewPart::Markdown(literal_markdown(
                &format!("[{}]", note.identifier),
            ))],
            _ => child
                .children()
                .map(|children| inline_parts(children, definitions))
                .unwrap_or_default(),
        };
        for part in children {
            match part {
                PreviewPart::Markdown(text) => push_markdown(&mut parts, text),
                part => parts.push(part),
            }
        }
    }
    parts
}

/// Keep lists and attachments in native layout; only serialize decoded inline AST
/// for TextView, so container prefixes and document-wide references are not lost.
impl Workspace {
    pub(super) fn note_preview_block(
        &self,
        node: &Node,
        _source: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut definitions = Definitions::new();
        collect_definitions(node, &mut definitions);
        self.note_preview_node(node, &definitions, window, cx)
    }

    fn note_preview_node(
        &self,
        node: &Node,
        definitions: &Definitions<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette(window);
        let offset = node
            .position()
            .map(|position| position.start.offset)
            .unwrap_or(0);
        match node {
            Node::Root(root) => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .children(
                    root.children
                        .iter()
                        .filter(|node| !matches!(node, Node::Definition(_)))
                        .map(|node| self.note_preview_node(node, definitions, window, cx)),
                )
                .into_any_element(),
            Node::List(list) => {
                let mut rows = div().w_full().flex().flex_col().gap_2();
                for (index, node) in list.children.iter().enumerate() {
                    let Node::ListItem(item) = node else { continue };
                    let marker = if let Some(checked) = item.checked {
                        if checked {
                            "☑".to_string()
                        } else {
                            "☐".to_string()
                        }
                    } else if list.ordered {
                        format!("{}.", list.start.unwrap_or(1) as usize + index)
                    } else {
                        "•".to_string()
                    };
                    let body = div().flex_1().min_w_0().flex().flex_col().gap_2().children(
                        item.children
                            .iter()
                            .filter(|node| !matches!(node, Node::Definition(_)))
                            .map(|node| self.note_preview_node(node, definitions, window, cx)),
                    );
                    rows = rows.child(
                        div()
                            .w_full()
                            .flex()
                            .items_start()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(20.))
                                    .flex_shrink_0()
                                    .text_color(if item.checked == Some(true) {
                                        p.accent
                                    } else {
                                        p.muted
                                    })
                                    .child(marker),
                            )
                            .child(body),
                    );
                }
                rows.into_any_element()
            }
            Node::Blockquote(quote) => div()
                .w_full()
                .border_l_2()
                .border_color(p.accent.opacity(0.35))
                .pl_3()
                .text_color(p.muted)
                .flex()
                .flex_col()
                .gap_2()
                .children(
                    quote
                        .children
                        .iter()
                        .filter(|node| !matches!(node, Node::Definition(_)))
                        .map(|node| self.note_preview_node(node, definitions, window, cx)),
                )
                .into_any_element(),
            Node::Paragraph(paragraph) => self.note_preview_parts(
                inline_parts(&paragraph.children, definitions),
                offset,
                None,
                window,
                cx,
            ),
            Node::Heading(heading) => self.note_preview_parts(
                inline_parts(&heading.children, definitions),
                offset,
                Some(heading.depth),
                window,
                cx,
            ),
            Node::Code(code) => {
                let length = code
                    .value
                    .split(|c| c != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0)
                    .max(2)
                    + 1;
                let fence = "`".repeat(length);
                let language = code
                    .lang
                    .as_deref()
                    .unwrap_or("")
                    .replace(['`', '\n', '\r'], "");
                self.note_preview_text(
                    &format!("{fence}{language}\n{}\n{fence}", code.value),
                    format!("{offset}-code"),
                    window,
                    cx,
                )
            }
            Node::Table(table) => {
                let mut rows = div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(p.border);
                for (index, row) in table.children.iter().enumerate() {
                    let Some(cells) = row.children() else {
                        continue;
                    };
                    rows =
                        rows.child(
                            div()
                                .w_full()
                                .flex()
                                .items_start()
                                .when(index == 0, |row| {
                                    row.bg(p.subtle).font_weight(FontWeight::SEMIBOLD)
                                })
                                .children(cells.iter().enumerate().map(|(column, cell)| {
                                    let alignment = match table.align.get(column) {
                                        Some(markdown::mdast::AlignKind::Center) => {
                                            gpui::TextAlign::Center
                                        }
                                        Some(markdown::mdast::AlignKind::Right) => {
                                            gpui::TextAlign::Right
                                        }
                                        _ => gpui::TextAlign::Left,
                                    };
                                    div().flex_1().min_w_0().p_2().text_align(alignment).child(
                                        self.note_preview_node(cell, definitions, window, cx),
                                    )
                                })),
                        );
                }
                rows.into_any_element()
            }
            Node::ThematicBreak(_) => div()
                .w_full()
                .h(px(1.))
                .my_2()
                .bg(p.border)
                .into_any_element(),
            Node::Definition(_) => div().into_any_element(),
            Node::FootnoteDefinition(note) => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(format!("[{}]", note.identifier))
                .children(
                    note.children
                        .iter()
                        .map(|node| self.note_preview_node(node, definitions, window, cx)),
                )
                .into_any_element(),
            _ => self.note_preview_parts(
                inline_parts(std::slice::from_ref(node), definitions),
                offset,
                None,
                window,
                cx,
            ),
        }
    }

    fn note_preview_parts(
        &self,
        parts: Vec<PreviewPart>,
        offset: usize,
        heading: Option<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette(window);
        let mut blocks = div().w_full().flex().flex_col().gap_2();
        for (index, part) in parts.into_iter().enumerate() {
            let key = format!("{offset}-{index}");
            match part {
                PreviewPart::Markdown(text) => {
                    if text.trim().is_empty() {
                        continue;
                    }
                    let text = heading
                        .map(|depth| format!("{} {text}", "#".repeat(depth as usize)))
                        .unwrap_or(text);
                    blocks = blocks.child(self.note_preview_text(&text, key, window, cx));
                }
                PreviewPart::Image { url, alt } => {
                    let attachment = url.strip_prefix("attachment:").and_then(|id| {
                        self.note_editor
                            .attachments
                            .iter()
                            .find(|attachment| attachment.id == id)
                    });
                    if let Some(attachment) = attachment {
                        if self
                            .note_editor
                            .images
                            .read(cx)
                            .images
                            .contains_key(&attachment.id)
                        {
                            blocks = blocks.child(
                                img(gpui::SharedUri::from(format!(
                                    "attachment:{}",
                                    attachment.id
                                )))
                                .max_w_full()
                                .max_h(px(320.))
                                .object_fit(ObjectFit::Contain)
                                .rounded(px(6.)),
                            );
                        } else {
                            blocks = blocks.child(
                                div()
                                    .text_sm()
                                    .text_color(p.muted)
                                    .child(format!("图片无法预览：{}", attachment.name)),
                            );
                        }
                    } else {
                        let message = self.t(
                            "图片尚未附加，请使用“图片”按钮插入",
                            "Image not attached; use Insert image",
                        );
                        blocks = blocks.child(div().text_sm().text_color(p.muted).child(
                            if alt.is_empty() {
                                message.to_string()
                            } else {
                                format!("{alt} · {message}")
                            },
                        ));
                    }
                }
                PreviewPart::Attachment { url, label } => {
                    let attachment = url
                        .strip_prefix("attachment:")
                        .and_then(|id| {
                            self.note_editor
                                .attachments
                                .iter()
                                .find(|attachment| attachment.id == id)
                        })
                        .cloned();
                    if let Some(attachment) = attachment {
                        blocks = blocks.child(
                            chip(
                                SharedString::from(format!("preview-download-{key}")),
                                format!("↓ {}", attachment.name),
                                p,
                            )
                            .text_sm()
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.save_note_attachment(attachment.clone(), window, cx)
                                },
                            )),
                        );
                    } else {
                        blocks = blocks.child(div().text_sm().text_color(p.muted).child(format!(
                            "{label} · {}",
                            self.t("附件不存在", "Attachment unavailable")
                        )));
                    }
                }
            }
        }
        blocks.into_any_element()
    }

    fn note_preview_text(
        &self,
        text: &str,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette(window);
        let style = TextViewStyle {
            heading_base_font_size: px(self.doc.settings.font_size.max(14.)),
            highlight_theme: cx.theme().highlight_theme.clone(),
            is_dark: p.dark,
            ..Default::default()
        };
        TextView::markdown(
            SharedString::from(format!(
                "note-block-{}-{}-{key}",
                self.week, self.note_editor.generation
            )),
            text.to_owned(),
            window,
            cx,
        )
        .style(style)
        .selectable(true)
        .w_full()
        .h_auto()
        .flex_shrink_0()
        .text_size(px(self.doc.settings.font_size.max(14.)))
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Definitions, PreviewPart, collect_definitions, inline_parts};
    use markdown::{ParseOptions, mdast::Node, to_mdast};

    fn parse(source: &str) -> Node {
        to_mdast(source, &ParseOptions::gfm()).unwrap()
    }

    fn first_paragraph(node: &Node) -> &Node {
        if matches!(node, Node::Paragraph(_)) {
            return node;
        }
        node.children()
            .unwrap()
            .iter()
            .find_map(|child| {
                if matches!(
                    child,
                    Node::Paragraph(_) | Node::Blockquote(_) | Node::List(_) | Node::ListItem(_)
                ) {
                    Some(first_paragraph(child))
                } else {
                    None
                }
            })
            .unwrap()
    }

    fn paragraph_parts(root: &Node) -> Vec<PreviewPart> {
        let mut definitions = Definitions::new();
        collect_definitions(root, &mut definitions);
        inline_parts(first_paragraph(root).children().unwrap(), &definitions)
    }

    #[test]
    fn resolves_document_references_and_nested_reference_images() {
        let root = parse(
            "[docs][D] **![picture][IMG]** [file][F]\n\n[d]: https://example.com\n[img]: attachment:image-id\n[f]: attachment:file-id",
        );
        let parts = paragraph_parts(&root);
        assert_eq!(
            parts,
            vec![
                PreviewPart::Markdown("[docs](<https://example.com>) ".into()),
                PreviewPart::Image {
                    url: "attachment:image-id".into(),
                    alt: "picture".into()
                },
                PreviewPart::Markdown(" ".into()),
                PreviewPart::Attachment {
                    url: "attachment:file-id".into(),
                    label: "file".into()
                },
            ]
        );
    }

    #[test]
    fn preserves_marks_on_both_sides_of_an_inline_image() {
        let root = parse("**before ![picture](attachment:image-id) after**");
        let parts = paragraph_parts(&root);
        assert_eq!(parts[0], PreviewPart::Markdown("**before** ".into()));
        assert_eq!(parts[2], PreviewPart::Markdown(" **after**".into()));
        for part in [&parts[0], &parts[2]] {
            let PreviewPart::Markdown(text) = part else {
                panic!()
            };
            let parsed = parse(text);
            assert!(
                first_paragraph(&parsed)
                    .children()
                    .unwrap()
                    .iter()
                    .any(|node| matches!(node, Node::Strong(_)))
            );
        }
    }

    #[test]
    fn multiline_quote_and_nested_list_text_do_not_reparse_container_markers() {
        for source in ["> one\n> two", "- outer\n  - one\n    two"] {
            let root = parse(source);
            let paragraph = if source.starts_with('-') {
                let Node::Root(root) = &root else { panic!() };
                let Node::List(list) = &root.children[0] else {
                    panic!()
                };
                let Node::ListItem(item) = &list.children[0] else {
                    panic!()
                };
                first_paragraph(&item.children[1])
            } else {
                first_paragraph(&root)
            };
            let parts = inline_parts(paragraph.children().unwrap(), &Definitions::new());
            assert_eq!(parts, vec![PreviewPart::Markdown("one two".into())]);
        }
    }

    #[test]
    fn raw_html_stays_literal_and_attachment_links_never_reach_text_view() {
        let root = parse(
            "before <img src=\"https://example.com/image.png\"> after **[file](attachment:file-id)**",
        );
        let parts = paragraph_parts(&root);
        assert!(
            matches!(parts.last(), Some(PreviewPart::Attachment { url, .. }) if url == "attachment:file-id")
        );
        let PreviewPart::Markdown(text) = &parts[0] else {
            panic!()
        };
        let reparsed = parse(text);
        fn has_html_or_image(node: &Node) -> bool {
            matches!(node, Node::Html(_) | Node::Image(_))
                || node
                    .children()
                    .is_some_and(|children| children.iter().any(has_html_or_image))
        }
        assert!(!has_html_or_image(&reparsed));
        assert!(!text.contains("attachment:"));
    }
}
