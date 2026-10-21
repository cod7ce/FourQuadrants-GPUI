use super::*;
use crate::model::{MarkdownNote, NoteAttachment, WeekNote};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{collections::HashMap, path::Path, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NoteMode {
    Write,
    Split,
    Preview,
}

pub(super) struct NoteEditor {
    baseline: MarkdownNote,
    pub(super) attachments: Vec<NoteAttachment>,
    pub(super) images: Entity<NoteImages>,
    pub(super) generation: u64,
    loading: bool,
    converted: bool,
    conversion_error: Option<String>,
    picking_image: bool,
    mode: NoteMode,
}

impl NoteEditor {
    pub(super) fn new(note: Option<&WeekNote>, cx: &mut App) -> Self {
        let baseline = note
            .and_then(|note| note.markdown.clone())
            .unwrap_or_else(|| MarkdownNote {
                source: note.map(|note| note.text.clone()).unwrap_or_default(),
                attachments: Vec::new(),
            });
        Self {
            images: NoteImages::entity(&baseline.attachments, cx),
            attachments: baseline.attachments.clone(),
            baseline,
            generation: 0,
            loading: false,
            converted: false,
            conversion_error: None,
            picking_image: false,
            mode: NoteMode::Split,
        }
    }
}

/// TextView inherits this cache, resolving portable attachment IDs without a server.
pub(super) struct NoteImages {
    pub(super) images: HashMap<String, Arc<Image>>,
    rendered: HashMap<u64, Arc<RenderImage>>,
}

impl NoteImages {
    fn entity(attachments: &[NoteAttachment], cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|_| {
            let mut cache = Self {
                images: HashMap::new(),
                rendered: HashMap::new(),
            };
            cache.extend(attachments);
            cache
        });
        cx.observe_release(&entity, |cache, cx| {
            for image in cache.images.values() {
                image.clone().remove_asset(cx);
            }
            let rendered = std::mem::take(&mut cache.rendered);
            // Defer until the current window is back in App.windows as well.
            cx.defer(move |cx| {
                for image in rendered.into_values() {
                    cx.drop_image(image, None);
                }
            });
        })
        .detach();
        entity
    }

    fn extend(&mut self, attachments: &[NoteAttachment]) {
        for attachment in attachments {
            if self.images.contains_key(&attachment.id) {
                continue;
            }
            if let (Some(format), Ok(bytes)) = (
                ImageFormat::from_mime_type(&attachment.mime_type),
                STANDARD.decode(&attachment.data),
            ) {
                self.images.insert(
                    attachment.id.clone(),
                    Arc::new(Image::from_bytes(format, bytes)),
                );
            }
        }
    }
}

impl ImageCache for NoteImages {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<std::result::Result<Arc<RenderImage>, ImageCacheError>> {
        let id = match resource {
            Resource::Uri(uri) => uri.strip_prefix("attachment:"),
            _ => None,
        };
        if let Some(image) = id.and_then(|id| self.images.get(id)) {
            let rendered = image.clone().use_render_image(window, cx)?;
            self.rendered.insert(image.id(), rendered.clone());
            return Some(Ok(rendered));
        }
        Some(Err(ImageCacheError::Asset(
            "请将图片插入笔记以便离线保存 / Insert an image to save it with this note".into(),
        )))
    }
}

fn prepared_legacy(note: &WeekNote, converted: crate::native::ConvertedNote) -> MarkdownNote {
    let normalize = |text: &str| {
        text.replace('\u{fffc}', "")
            .replace("\r\n", "\n")
            .trim()
            .to_string()
    };
    let source = if note.text.trim().is_empty()
        || normalize(&note.text) == normalize(&converted.plain_text)
    {
        converted.source
    } else if converted.source.trim().is_empty() {
        note.text.clone()
    } else {
        format!(
            "{}\n\n---\n\n## 原富文本内容 / Original rich text\n\n{}",
            note.text, converted.source
        )
    };
    MarkdownNote {
        source,
        attachments: converted.attachments,
    }
}

fn attachment_from_bytes(name: String, bytes: Vec<u8>) -> Result<NoteAttachment> {
    anyhow::ensure!(
        bytes.len() <= model::MAX_NOTE_ATTACHMENT_BYTES,
        "图片不能超过 20 MB / Image exceeds 20 MB"
    );
    let mime_type = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if bytes.starts_with(b"BM") {
        "image/bmp"
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        "image/tiff"
    } else {
        anyhow::bail!("请选择 PNG、JPEG、WebP、GIF、BMP 或 TIFF 图片 / Unsupported image format")
    };
    Ok(NoteAttachment {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        mime_type: mime_type.into(),
        data: STANDARD.encode(bytes),
    })
}

fn attachment_markdown(attachment: &NoteAttachment) -> String {
    let name = attachment
        .name
        .replace(['\n', '\r'], " ")
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    format!("\n\n![{name}](attachment:{})\n\n", attachment.id)
}

fn draft_with_attachments(source: String, attachments: &[NoteAttachment]) -> MarkdownNote {
    let attachments = attachments
        .iter()
        .filter(|attachment| {
            !attachment.mime_type.starts_with("image/")
                || source.contains(&format!("attachment:{}", attachment.id))
        })
        .cloned()
        .collect();
    // Keep omitted images in the editor cache for text undo, but not in the saved note.
    MarkdownNote {
        source,
        attachments,
    }
}

fn format_selection(source: &str, range: std::ops::Range<usize>, kind: &str) -> String {
    let units: Vec<u16> = source.encode_utf16().collect();
    let selected =
        String::from_utf16_lossy(&units[range.start.min(units.len())..range.end.min(units.len())]);
    match kind {
        "heading" => format!(
            "\n## {}\n",
            if selected.is_empty() {
                "标题"
            } else {
                &selected
            }
        ),
        "bold" => format!(
            "**{}**",
            if selected.is_empty() {
                "加粗文字"
            } else {
                &selected
            }
        ),
        "italic" => format!(
            "*{}*",
            if selected.is_empty() {
                "斜体文字"
            } else {
                &selected
            }
        ),
        "list" => format!(
            "\n{}\n",
            if selected.is_empty() {
                "- 列表项".into()
            } else {
                selected
                    .lines()
                    .map(|line| format!("- {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        ),
        "check" => format!(
            "\n- [ ] {}\n",
            if selected.is_empty() {
                "待办事项"
            } else {
                &selected
            }
        ),
        "link" => format!(
            "[{}](https://)",
            if selected.is_empty() {
                "链接文字"
            } else {
                &selected
            }
        ),
        _ => format!("`{selected}`"),
    }
}

impl Workspace {
    pub(super) fn reset_note_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.note_editor.generation.wrapping_add(1);
        let mode = self.note_editor.mode;
        self.note_editor = NoteEditor::new(
            self.doc
                .week_notes
                .iter()
                .find(|note| note.week_start == self.week),
            cx,
        );
        self.note_editor.generation = generation;
        self.note_editor.mode = mode;
        let source = self.note_editor.baseline.source.clone();
        // InputState::set_value keeps its undo history; each loaded note needs a fresh input.
        self.week_text = field(
            source,
            "写下想法… 支持 Markdown、粘贴或拖入图片 / Write Markdown…",
            true,
            window,
            cx,
        );
        self.note_subscription = cx.subscribe(&self.week_text, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.save_week_note(cx);
            }
        });
    }

    fn note_draft(&self, cx: &App) -> MarkdownNote {
        let source = self.week_text.read(cx).value().to_string();
        if source == self.note_editor.baseline.source {
            return self.note_editor.baseline.clone();
        }
        draft_with_attachments(source, &self.note_editor.attachments)
    }

    pub fn save_week_note(&mut self, cx: &mut Context<Self>) -> bool {
        let draft = self.note_draft(cx);
        if draft == self.note_editor.baseline {
            return true;
        }
        let week = self.week;
        let saved = draft.clone();
        self.change(cx, move |doc| {
            doc.schema_version = model::SCHEMA_VERSION;
            if let Some(note) = doc
                .week_notes
                .iter_mut()
                .find(|note| note.week_start == week)
            {
                if note.markdown.is_none() && (!note.text.is_empty() || note.rich_content.is_some())
                {
                    let original = note.clone();
                    if !doc.archived_week_notes.contains(&original) {
                        doc.archived_week_notes.push(original);
                    }
                }
                note.markdown = Some(saved);
            } else {
                doc.week_notes.push(WeekNote {
                    week_start: week,
                    text: String::new(),
                    rich_content: None,
                    markdown: Some(saved),
                });
            }
            Ok(())
        });
        if self.error.is_none() {
            self.note_editor.baseline = draft;
        }
        cx.notify();
        self.error.is_none()
    }

    fn prepare_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_editor.converted || self.note_editor.loading {
            return;
        }
        self.note_editor.converted = true;
        let Some(note) = self
            .doc
            .week_notes
            .iter()
            .find(|note| {
                note.week_start == self.week
                    && note.markdown.is_none()
                    && note.rich_content.is_some()
            })
            .cloned()
        else {
            return;
        };
        self.note_editor.loading = true;
        let generation = self.note_editor.generation;
        let week = self.week;
        cx.spawn_in(window, async move |this, cx| {
            let original = note.clone();
            let result = cx.background_executor().spawn(async move {
                let converted = crate::native::convert_note(note.rich_content.as_deref().unwrap())?;
                let prepared = prepared_legacy(&note, converted);
                prepared.validate()?;
                Ok::<_, anyhow::Error>(prepared)
            }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.week != week || this.note_editor.generation != generation { return; }
                this.note_editor.loading = false;
                if this.doc.week_notes.iter().find(|note| note.week_start == week) != Some(&original) { return; }
                match result {
                    Ok(prepared) => {
                        this.note_editor.attachments = prepared.attachments.clone();
                        this.note_editor.images = NoteImages::entity(&prepared.attachments, cx);
                        this.note_editor.baseline = prepared.clone();
                        this.week_text.update(cx, |input, cx| input.set_value(prepared.source, window, cx));
                    }
                    Err(error) => this.note_editor.conversion_error = Some(format!("旧富文本暂时无法转换，原件仍完整保留 / Original rich note retained: {error:#}")),
                }
                cx.notify();
            });
        }).detach();
    }

    fn insert_note_images(
        &mut self,
        attachments: Vec<NoteAttachment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.note_editor.loading {
            return;
        }
        let mut draft = self.note_draft(cx);
        let insertion = attachments
            .iter()
            .map(attachment_markdown)
            .collect::<String>();
        draft.attachments.extend(attachments.iter().cloned());
        if let Err(error) = draft.validate() {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.note_editor
            .images
            .update(cx, |cache, _| cache.extend(&attachments));
        self.note_editor.attachments.extend(attachments);
        self.week_text.update(cx, |input, cx| {
            input.replace(insertion, window, cx);
            input.focus(window, cx);
        });
        self.save_week_note(cx);
    }

    fn read_note_images(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() || self.note_editor.loading {
            return;
        }
        let generation = self.note_editor.generation;
        let week = self.week;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                let mut attachments = Vec::new();
                let mut size = 0;
                for path in paths {
                    let metadata = std::fs::metadata(&path)?;
                    anyhow::ensure!(metadata.is_file() && metadata.len() <= model::MAX_NOTE_ATTACHMENT_BYTES as u64, "图片不是文件或超过 20 MB / Image is not a file or exceeds 20 MB");
                    let bytes = std::fs::read(&path)?;
                    size += bytes.len();
                    anyhow::ensure!(size <= model::MAX_NOTE_ATTACHMENTS_BYTES, "图片总大小超过 50 MB / Images exceed 50 MB");
                    attachments.push(attachment_from_bytes(path.file_name().unwrap_or_default().to_string_lossy().into_owned(), bytes)?);
                }
                Ok::<_, anyhow::Error>(attachments)
            }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.week != week || this.note_editor.generation != generation {
                    this.error = Some("笔记已切换，请在目标笔记中重新插入图片 / Note changed; insert the image again".into());
                } else {
                    match result {
                        Ok(attachments) => this.insert_note_images(attachments, window, cx),
                        Err(error) => this.error = Some(error.to_string()),
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn choose_note_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_editor.picking_image || self.note_editor.loading {
            return;
        }
        self.note_editor.picking_image = true;
        let generation = self.note_editor.generation;
        let week = self.week;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(self.t("插入图片", "Insert image").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = receiver.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.note_editor.picking_image = false;
                match result {
                    Ok(Ok(Some(paths)))
                        if this.week == week && this.note_editor.generation == generation =>
                    {
                        this.read_note_images(paths, window, cx)
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => this.error = Some(error.to_string()),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn format_note(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_editor.loading {
            return;
        }
        self.week_text.update(cx, |input, cx| {
            let selection = input
                .selected_text_range(false, window, cx)
                .map(|selection| selection.range)
                .unwrap_or(0..0);
            let insertion = format_selection(&input.value(), selection, kind);
            input.replace(insertion, window, cx);
            input.focus(window, cx);
        });
        self.save_week_note(cx);
    }

    pub(super) fn save_note_attachment(
        &mut self,
        attachment: NoteAttachment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_new_path(
            self.store.path.parent().unwrap_or(Path::new(".")),
            Some(&attachment.name),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<()> = async {
                if let Some(path) = receiver.await?? {
                    let bytes = STANDARD.decode(attachment.data)?;
                    cx.background_executor()
                        .spawn(async move { std::fs::write(path, bytes) })
                        .await?;
                }
                Ok(())
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.error = Some(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn narrow_notes(&self, window: &Window) -> bool {
        let sidebar_width = if Self::narrow_window(window) || self.doc.settings.sidebar_collapsed {
            SIDEBAR_RAIL_WIDTH
        } else {
            204.
        };
        window.viewport_size().width - px(sidebar_width) < px(900.)
    }

    pub(super) fn note_controls(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette(window);
        let narrow = self.narrow_notes(window);
        let mode = self.note_editor.mode;
        let unsaved = self.note_draft(cx) != self.note_editor.baseline;
        let mut modes = div().flex().flex_shrink_0().gap_1();
        for (value, zh, en) in [
            (NoteMode::Write, "编辑", "Write"),
            (NoteMode::Split, "对照", "Split"),
            (NoteMode::Preview, "预览", "Preview"),
        ] {
            if narrow && value == NoteMode::Split {
                continue;
            }
            let selected =
                mode == value || (narrow && mode == NoteMode::Split && value == NoteMode::Write);
            modes = modes.child(
                div()
                    .id(SharedString::from(format!("note-mode-{en}")))
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .text_xs()
                    .cursor_pointer()
                    .bg(if selected {
                        p.selected
                    } else {
                        transparent_black()
                    })
                    .text_color(if selected { p.text } else { p.muted })
                    .hover(move |el| el.bg(p.hover))
                    .child(self.t(zh, en))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.note_editor.mode = value;
                        cx.notify();
                    })),
            );
        }
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .child(modes)
            .child(
                div()
                    .text_xs()
                    .text_color(if unsaved { p.quadrant[0] } else { p.muted })
                    .child(if self.note_editor.loading {
                        self.t("正在整理旧笔记…", "Preparing note…")
                    } else if unsaved {
                        self.t("未保存", "Unsaved")
                    } else {
                        self.t("已保存", "Saved")
                    }),
            )
            .when(unsaved, |el| {
                el.child(
                    chip("retry-save-week-note", self.t("重试", "Retry"), p)
                        .text_xs()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.save_week_note(cx);
                        })),
                )
            })
            .into_any_element()
    }

    pub fn notes_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.prepare_note(window, cx);
        let p = self.palette(window);
        let source = self.week_text.read(cx).value().to_string();
        let narrow = self.narrow_notes(window);
        let mode = self.note_editor.mode;
        let write = mode != NoteMode::Preview;
        let preview = mode == NoteMode::Preview || (mode == NoteMode::Split && !narrow);
        let mut tools = div().flex().flex_wrap().items_center().gap_1();
        for (kind, label, hint) in [
            ("heading", "H", "标题 / Heading"),
            ("bold", "B", "加粗 / Bold"),
            ("italic", "I", "斜体 / Italic"),
            ("list", "≡", "列表 / List"),
            ("check", "☑", "待办 / Checklist"),
            ("link", "↗", "链接 / Link"),
            ("code", "‹›", "行内代码 / Code"),
        ] {
            tools = tools.child(
                icon_button_container(SharedString::from(format!("note-format-{kind}")), hint, p)
                    .size(px(26.))
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(label)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.format_note(kind, window, cx)),
                    ),
            );
        }
        tools = tools
            .child(div().w(px(1.)).h(px(14.)).mx_1().bg(p.border))
            .child(
                div()
                    .id("note-image")
                    .px_2()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded(px(4.))
                    .text_xs()
                    .text_color(p.muted)
                    .cursor_pointer()
                    .hover(move |el| el.bg(p.hover).text_color(p.text))
                    .child(glyph(IconName::Plus).with_size(px(13.)))
                    .child(self.t("图片", "Image"))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.choose_note_image(window, cx)),
                    ),
            );
        let parsed = markdown::to_mdast(&source, &markdown::ParseOptions::gfm());
        let mut panes = div().flex_1().min_h_0().min_w_0().flex().gap(px(18.));
        if write {
            panes = panes.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(preview, |el| {
                        el.child(div().text_xs().text_color(p.muted).child("Markdown"))
                    })
                    .child(
                        Input::new(&self.week_text)
                            .appearance(false)
                            .disabled(self.note_editor.loading)
                            .h_full()
                            .text_size(px(self.doc.settings.font_size.max(14.))),
                    ),
            )
        }
        if preview {
            panes = panes.child(
                div()
                    .image_cache(self.note_editor.images.clone())
                    .id("note-live-preview")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .when(write, |el| {
                        el.border_l_1().border_color(p.border).pl(px(18.))
                    })
                    .when(source.trim().is_empty(), |el| {
                        el.child(div().text_sm().text_color(p.muted).child(self.t(
                            "写下想法，或插入一张图片…",
                            "Write a thought or insert an image…",
                        )))
                    })
                    .when(!source.trim().is_empty(), |el| match &parsed {
                        Ok(node) => el.child(self.note_preview_block(node, &source, window, cx)),
                        Err(_) => el.child(div().whitespace_normal().child(source)),
                    }),
            );
        }
        let mut extras = div().flex().flex_wrap().gap_1();
        for attachment in self
            .note_editor
            .attachments
            .iter()
            .filter(|attachment| !attachment.mime_type.starts_with("image/"))
        {
            let attachment = attachment.clone();
            extras = extras.child(
                chip(
                    SharedString::from(format!("note-attachment-{}", attachment.id)),
                    format!("↓ {}", attachment.name),
                    p,
                )
                .text_xs()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_note_attachment(attachment.clone(), window, cx)
                })),
            );
        }
        for (index, _) in self
            .doc
            .archived_week_notes
            .iter()
            .enumerate()
            .filter(|(_, note)| note.week_start == self.week)
        {
            extras = extras.child(
                chip(
                    ("restore-note", index),
                    format!(
                        "{} {}",
                        self.t("恢复保留版本", "Restore version"),
                        index + 1
                    ),
                    p,
                )
                .text_xs()
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !this.save_week_note(cx) {
                        return;
                    }
                    this.change(cx, |doc| {
                        let note = doc.archived_week_notes.remove(index);
                        if let Some(current) = doc
                            .week_notes
                            .iter_mut()
                            .find(|current| current.week_start == note.week_start)
                        {
                            doc.archived_week_notes
                                .push(std::mem::replace(current, note));
                        } else {
                            doc.week_notes.push(note);
                        }
                        Ok(())
                    });
                    if this.error.is_none() {
                        this.load_week_note(window, cx);
                    }
                })),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .px(px(20.))
                    .py(px(12.))
                    .gap_3()
                    .when(write, |el| el.child(tools))
                    .when(
                        self.rich_pending.iter().any(|(task, week, _)| {
                            task.as_ref()
                                .map_or(*week == self.week, |id| self.doc.task(id).is_none())
                        }),
                        |el| el.child(self.rich_recovery_view("notes", None, self.week, p, cx)),
                    )
                    .when_some(self.note_editor.conversion_error.clone(), |el, error| {
                        el.child(div().text_xs().text_color(p.quadrant[0]).child(error))
                    })
                    .child(panes)
                    .child(extras)
                    .child(div().text_xs().text_color(p.muted).child(self.t(
                        "支持 Markdown · 粘贴或拖入图片 · 自动保存",
                        "Markdown · Paste or drop images · Autosaved",
                    ))),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.read_note_images(paths.paths().to_vec(), window, cx)
            }))
            .capture_action(
                cx.listener(|this, _: &gpui_component::input::Paste, window, cx| {
                    if this.note_editor.mode == NoteMode::Preview || this.note_editor.loading {
                        cx.propagate();
                        return;
                    }
                    let images = cx
                        .read_from_clipboard()
                        .map(|item| {
                            item.into_entries()
                                .filter_map(|entry| match entry {
                                    ClipboardEntry::Image(image) => Some(image),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if images.is_empty() {
                        cx.propagate();
                        return;
                    }
                    let result = images
                        .into_iter()
                        .map(|image| attachment_from_bytes("粘贴的图片".into(), image.bytes))
                        .collect::<Result<Vec<_>>>();
                    match result {
                        Ok(attachments) => this.insert_note_images(attachments, window, cx),
                        Err(error) => {
                            this.error = Some(error.to_string());
                            cx.notify();
                        }
                    }
                }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{attachment_from_bytes, draft_with_attachments, format_selection, prepared_legacy};
    use crate::model::{NoteAttachment, WeekNote};

    #[test]
    fn deleting_image_releases_saved_bytes_but_preserves_undo_pool() {
        let image = NoteAttachment {
            id: "image-id".into(),
            mime_type: "image/png".into(),
            data: "original image".into(),
            ..Default::default()
        };
        let file = NoteAttachment {
            id: "file-id".into(),
            mime_type: "application/octet-stream".into(),
            data: "original file".into(),
            ..Default::default()
        };
        let mut pool = vec![image.clone(), file.clone()];
        assert_eq!(
            draft_with_attachments("Text only".into(), &pool).attachments,
            vec![file]
        );
        pool.push(NoteAttachment {
            id: "new-image".into(),
            mime_type: "image/png".into(),
            ..Default::default()
        });
        let restored = draft_with_attachments("![old](attachment:image-id)".into(), &pool);
        assert_eq!(restored.attachments[0], image);
        assert_eq!(pool.len(), 3);
    }

    #[test]
    fn legacy_conversion_preserves_independently_edited_text() {
        let note = WeekNote {
            text: "New plain text".into(),
            rich_content: Some("original bytes".into()),
            ..WeekNote::default()
        };
        let prepared = prepared_legacy(
            &note,
            crate::native::ConvertedNote {
                plain_text: "Old rich text".into(),
                source: "**Old rich text**".into(),
                attachments: vec![],
            },
        );
        assert!(prepared.source.starts_with("New plain text\n"));
        assert!(prepared.source.ends_with("**Old rich text**"));
        assert_eq!(note.rich_content.as_deref(), Some("original bytes"));
    }

    #[test]
    fn identical_rich_plain_text_is_not_duplicated() {
        let note = WeekNote {
            text: "Hello \u{fffc}".into(),
            ..WeekNote::default()
        };
        let prepared = prepared_legacy(
            &note,
            crate::native::ConvertedNote {
                plain_text: "Hello \u{fffc}".into(),
                source: "Hello ![image](attachment:id)".into(),
                attachments: vec![],
            },
        );
        assert_eq!(prepared.source, "Hello ![image](attachment:id)");
    }

    #[test]
    fn formatting_uses_utf16_selection_for_cjk_and_emoji() {
        assert_eq!(format_selection("甲😀乙", 1..3, "bold"), "**😀**");
        assert_eq!(format_selection("一\n二", 0..3, "list"), "\n- 一\n- 二\n");
    }

    #[test]
    fn unsupported_image_is_rejected_without_changing_source() {
        assert!(attachment_from_bytes("bad.png".into(), b"not an image".to_vec()).is_err());
    }
}
