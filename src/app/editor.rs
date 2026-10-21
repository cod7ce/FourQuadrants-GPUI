use super::task_schedule::SchedulePicker;
use super::*;
use crate::model::{ProgressNote, WeekNote};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use gpui_component::{
    calendar::Date,
    date_picker::{DatePicker, DatePickerState},
};

pub struct TaskEditor {
    pub id: String,
    title: Entity<InputState>,
    issue: Entity<InputState>,
    notes: Entity<InputState>,
    due: Entity<SchedulePicker>,
    reminder: Entity<SchedulePicker>,
    week: Entity<DatePickerState>,
    links: Entity<InputState>,
    progress: Entity<InputState>,
    baseline: TaskDraft,
    original_title: String,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, PartialEq)]
struct TaskDraft {
    title: String,
    issue: String,
    notes: String,
    due: String,
    reminder: String,
    week: String,
    links: String,
}

fn editor_date(value: Option<DateTime<Utc>>) -> String {
    value
        .map(|date| {
            date.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

impl TaskDraft {
    fn from_task(task: &Task) -> Self {
        Self {
            title: task.title.clone(),
            issue: task.issue_key.clone().unwrap_or_default(),
            notes: task.notes.clone(),
            due: editor_date(task.due_date),
            reminder: editor_date(task.remind_at),
            week: task.week_start.to_string(),
            links: task.links.join("\n"),
        }
    }

    fn refresh_pristine(
        &mut self,
        baseline: &mut Self,
        latest: &Self,
        original_title: &mut String,
    ) {
        // A new task can display an empty title while the model holds its placeholder.
        // Only an actual model title change should replace that pristine empty input.
        if self.title == baseline.title && latest.title != *original_title {
            self.title.clone_from(&latest.title);
            baseline.title.clone_from(&latest.title);
            original_title.clone_from(&latest.title);
        }
        for (value, baseline, latest) in [
            (&mut self.issue, &mut baseline.issue, &latest.issue),
            (&mut self.notes, &mut baseline.notes, &latest.notes),
            (&mut self.due, &mut baseline.due, &latest.due),
            (&mut self.reminder, &mut baseline.reminder, &latest.reminder),
            (&mut self.week, &mut baseline.week, &latest.week),
            (&mut self.links, &mut baseline.links, &latest.links),
        ] {
            if value == baseline {
                value.clone_from(latest);
                baseline.clone_from(latest);
            }
        }
    }

    // Compare against the text originally displayed, not the latest model value.
    // In particular, displaying a timestamp to minute precision is not an edit.
    fn apply(&self, baseline: &Self, task: &mut Task, progress: &str) -> Result<()> {
        if self.title != baseline.title {
            anyhow::ensure!(
                !self.title.trim().is_empty(),
                "请输入任务标题 / A title is required"
            );
            task.title = self.title.trim().to_string();
        }
        if self.issue != baseline.issue {
            task.issue_key = (!self.issue.trim().is_empty()).then(|| self.issue.trim().to_string());
        }
        if self.notes != baseline.notes {
            task.notes = self.notes.clone();
        }
        if self.due != baseline.due {
            task.due_date = parse_date(&self.due)?;
        }
        if self.reminder != baseline.reminder {
            task.remind_at = parse_date(&self.reminder)?;
        }
        if self.week != baseline.week {
            task.week_start = model::week_start(
                NaiveDate::parse_from_str(self.week.trim(), "%Y-%m-%d")
                    .map_err(|_| anyhow::anyhow!("周日期应为 YYYY-MM-DD / Invalid week date"))?,
            );
        }
        if self.links != baseline.links {
            task.links = self
                .links
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
        }
        if !progress.is_empty() {
            task.progress_notes.push(ProgressNote {
                text: progress.to_string(),
                ..Default::default()
            });
        }
        Ok(())
    }
}

impl TaskEditor {
    pub fn new(task: &Task, blank_title: bool, window: &mut Window, cx: &mut App) -> Self {
        // A freshly created task is persisted as "Untitled task" so it survives an
        // immediate close, but the editor starts empty with that title as placeholder:
        // typing directly renames it without relying on asynchronous SelectAll dispatch.
        let mut baseline = TaskDraft::from_task(task);
        let title_value = if blank_title {
            baseline.title.clear();
            String::new()
        } else {
            task.title.clone()
        };
        let mut this = Self {
            id: task.id.clone(),
            title: field(title_value, task.title.clone(), true, window, cx),
            issue: field(
                task.issue_key.clone().unwrap_or_default(),
                "工单编号 / Issue key",
                false,
                window,
                cx,
            ),
            notes: field(
                task.notes.clone(),
                "备注 / Plain text notes",
                true,
                window,
                cx,
            ),
            due: cx.new(|cx| SchedulePicker::new(&editor_date(task.due_date), 18, window, cx)),
            reminder: cx.new(|cx| SchedulePicker::new(&editor_date(task.remind_at), 9, window, cx)),
            week: cx.new(|cx| {
                let mut picker = DatePickerState::new(window, cx).date_format("%Y-%m-%d");
                picker.set_date(task.week_start, window, cx);
                picker
            }),
            links: field(
                task.links.join("\n"),
                "每行一个链接 / One URL per line",
                true,
                window,
                cx,
            ),
            progress: field("", "添加进展 / Add progress", true, window, cx),
            baseline,
            original_title: task.title.clone(),
            _subscriptions: Vec::new(),
        };
        let handle = window.window_handle();
        for input in [
            &this.title,
            &this.issue,
            &this.notes,
            &this.links,
            &this.progress,
        ] {
            this._subscriptions.push(cx.observe(input, move |_, cx| {
                let _ = handle.update(cx, |_, window, _| window.refresh());
            }));
        }
        for picker in [&this.due, &this.reminder] {
            this._subscriptions.push(cx.observe(picker, move |_, cx| {
                let _ = handle.update(cx, |_, window, _| window.refresh());
            }));
        }
        this._subscriptions
            .push(cx.observe(&this.week, move |_, cx| {
                let _ = handle.update(cx, |_, window, _| window.refresh());
            }));
        this.focus_title(window, cx);
        this
    }

    pub(super) fn focus_title(&self, window: &mut Window, cx: &mut App) {
        self.title.update(cx, |title, cx| title.focus(window, cx));
    }

    pub(super) fn has_link(&self, cx: &App) -> bool {
        self.links
            .read(cx)
            .value()
            .lines()
            .any(|line| !line.trim().is_empty())
    }

    fn draft(&self, cx: &App) -> TaskDraft {
        TaskDraft {
            title: self.title.read(cx).value().to_string(),
            issue: self.issue.read(cx).value().to_string(),
            notes: self.notes.read(cx).value().to_string(),
            due: self.due.read(cx).value(cx),
            reminder: self.reminder.read(cx).value(cx),
            week: self
                .week
                .read(cx)
                .date()
                .start()
                .map(|date| model::week_start(date).to_string())
                .unwrap_or_else(|| self.baseline.week.clone()),
            links: self.links.read(cx).value().to_string(),
        }
    }

    pub(super) fn refresh_pristine(&mut self, task: &Task, window: &mut Window, cx: &mut App) {
        let mut draft = self.draft(cx);
        draft.refresh_pristine(
            &mut self.baseline,
            &TaskDraft::from_task(task),
            &mut self.original_title,
        );
        self.refresh_inputs(&draft, window, cx);
    }

    fn refresh_inputs(&self, draft: &TaskDraft, window: &mut Window, cx: &mut App) {
        for (input, value) in [
            (&self.title, &draft.title),
            (&self.issue, &draft.issue),
            (&self.notes, &draft.notes),
            (&self.links, &draft.links),
        ] {
            if input.read(cx).value().as_ref() != value.as_str() {
                input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            }
        }
        for (picker, value) in [(&self.due, &draft.due), (&self.reminder, &draft.reminder)] {
            if picker.read(cx).value(cx) != *value {
                picker.update(cx, |picker, cx| picker.set_value(value, window, cx));
            }
        }
        if let Ok(date) = NaiveDate::parse_from_str(&draft.week, "%Y-%m-%d")
            && self.week.read(cx).date().start() != Some(date)
        {
            self.week.update(cx, |picker, cx| {
                picker.set_date(Date::Single(Some(date)), window, cx)
            });
        }
    }

    fn accept_saved(&mut self, task: &Task, window: &mut Window, cx: &mut App) {
        let saved = TaskDraft::from_task(task);
        self.refresh_inputs(&saved, window, cx);
        if !self.progress.read(cx).value().is_empty() {
            self.progress
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.original_title.clone_from(&saved.title);
        self.baseline = saved;
    }
}

fn parse_date(text: &str) -> Result<Option<DateTime<Utc>>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M")
        .or_else(|_| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map(|date| date.and_hms_opt(0, 0, 0).unwrap())
        })
        .map_err(|_| anyhow::anyhow!("日期格式应为 YYYY-MM-DD HH:MM / Invalid date format"))?;
    let date = Local.from_local_datetime(&naive).single().ok_or_else(|| {
        anyhow::anyhow!("该本地时间不存在或存在夏令时歧义 / Ambiguous local time")
    })?;
    Ok(Some(date.with_timezone(&Utc)))
}

fn label(text: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_size(px(11.))
        .text_color(p.muted)
        .child(text.into())
}

fn property_input(state: &Entity<InputState>) -> Input {
    text_input(state).rounded(px(6.)).px(px(8.))
}

fn quiet_row(
    id: impl Into<ElementId>,
    icon: IconName,
    text: impl Into<SharedString>,
    p: Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(6.))
        .px(px(6.))
        .py(px(4.))
        .text_size(px(12.))
        .text_color(p.muted)
        .cursor_pointer()
        .hover(move |s| s.bg(p.hover).text_color(p.text))
        .child(glyph(icon))
        .child(text.into())
}

fn apply_rich_result(
    doc: &mut Document,
    task_id: Option<&str>,
    week: NaiveDate,
    original: &Option<String>,
    content: &str,
) -> Result<()> {
    if let Some(id) = task_id {
        let task = doc.task_mut(id).ok_or_else(|| {
            anyhow::anyhow!("任务已删除，富文本已保留 / Task was deleted; rich text retained")
        })?;
        anyhow::ensure!(
            &task.rich_content == original,
            "正文已被其他操作更新，未覆盖 / Rich text changed while the editor was open"
        );
        task.rich_content = Some(content.to_string());
    } else if let Some(note) = doc
        .week_notes
        .iter_mut()
        .find(|note| note.week_start == week)
    {
        anyhow::ensure!(
            &note.rich_content == original,
            "笔记已被其他操作更新，未覆盖 / Weekly rich text changed while the editor was open"
        );
        note.rich_content = Some(content.to_string());
    } else {
        anyhow::ensure!(
            original.is_none(),
            "原笔记已被移除，富文本已保留 / Original note was removed; rich text retained"
        );
        doc.week_notes.push(WeekNote {
            week_start: week,
            text: String::new(),
            rich_content: Some(content.to_string()),
            markdown: None,
        });
    }
    Ok(())
}

impl Workspace {
    pub fn save_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(editor) = &self.editor else {
            return true;
        };
        let id = editor.id.clone();
        let Some(current) = self.doc.task(&id).cloned() else {
            self.editor = None;
            return true;
        };
        let dialog = self
            .task_dialog
            .filter(|dialog| dialog.window_id() != window.window_handle().window_id());
        // Reject an unavailable dialog before persisting progress. The refresh below
        // runs synchronously, so a retry cannot append an already-saved note.
        if let Some(dialog) = dialog
            && let Err(error) = dialog.update(cx, |_, _, _| {})
        {
            self.error = Some(format!(
                "无法更新任务窗口，编辑内容已保留 / Could not update task window; edits retained: {error}"
            ));
            cx.notify();
            return false;
        }
        let draft = editor.draft(cx);
        let progress = editor.progress.read(cx).value().trim().to_string();
        let mut saved = current.clone();
        if let Err(error) = draft.apply(&editor.baseline, &mut saved, &progress) {
            self.error = Some(error.to_string());
            cx.notify();
            return false;
        }
        if saved != current {
            self.change(cx, |doc| {
                if saved.week_start != current.week_start {
                    doc.move_task(&id, saved.week_start, saved.quadrant());
                }
                *doc.task_mut(&id)
                    .ok_or_else(|| anyhow::anyhow!("Task was deleted"))? = saved;
                Ok(())
            });
            if self.error.is_some() {
                return false;
            }
        }
        if let (Some(editor), Some(task)) = (&mut self.editor, self.doc.task(&id).cloned()) {
            if let Some(dialog) = dialog {
                if let Err(error) = dialog.update(cx, |_, dialog_window, cx| {
                    editor.accept_saved(&task, dialog_window, cx);
                }) {
                    self.error = Some(format!(
                        "任务已保存，但无法更新任务窗口 / Task saved, but its window could not be updated: {error}"
                    ));
                    cx.notify();
                    return false;
                }
            } else {
                editor.accept_saved(&task, window, cx);
            }
        }
        self.error = None;
        self.status = self.t("已保存", "Saved").into();
        cx.notify();
        true
    }

    fn edit_rich_document(
        &mut self,
        task_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.save_editor(window, cx) || !self.save_week_note(cx) {
            return;
        }
        let week = self.week;
        let (content, title) = match &task_id {
            Some(id) => {
                let Some(task) = self.doc.task(id) else {
                    return;
                };
                (task.rich_content.clone(), task.title.clone())
            }
            None => (
                self.doc
                    .week_notes
                    .iter()
                    .find(|n| n.week_start == week)
                    .and_then(|n| n.rich_content.clone()),
                format!("{} · {}", self.t("笔记", "Notes"), week),
            ),
        };
        let key = task_id
            .as_ref()
            .map(|id| format!("task:{id}"))
            .unwrap_or_else(|| format!("week:{week}"));
        if !self.rich_sessions.insert(key.clone()) {
            self.status = "该文档已在富文本窗口中打开 / This document is already open".into();
            cx.notify();
            return;
        }
        let recovery_dir = self.rich_recovery_dir();
        let original = content.clone();
        self.status = self
            .t("富文本编辑器已打开", "Rich text editor opened")
            .into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor()
                .spawn(async move { crate::native::edit_rich(content, title, recovery_dir) }).await;
            let _ = this.update(cx, |this, cx| {
                this.rich_sessions.remove(&key);
                match result {
                    Ok(Some(content)) => {
                        this.change(cx, |doc| apply_rich_result(doc, task_id.as_deref(), week, &original, &content));
                        if let Some(error) = this.error.clone() {
                            let pending = (task_id, week, content);
                            if !this.rich_pending.contains(&pending) { this.rich_pending.push(pending); }
                            this.error = Some(format!("{error}；富文本已保留，请在任务详情或笔记恢复 / Rich text retained for recovery"));
                        }
                    }
                    Ok(None) => this.status = "未更改富文本 / Rich text unchanged".into(),
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }).detach();
    }

    fn rich_recovery_dir(&self) -> PathBuf {
        self.store
            .path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("Recovery")
    }

    pub(super) fn rich_recovery_view(
        &self,
        scope: &'static str,
        task_id: Option<&str>,
        week: NaiveDate,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let directory = self.rich_recovery_dir();
        let mut view = div().flex().flex_col().gap_2();
        let pending: Vec<_> = self
            .rich_pending
            .iter()
            .filter(|(pending_task, pending_week, _)| {
                if scope == "task" {
                    pending_task.as_deref() == task_id
                } else {
                    match pending_task {
                        None => *pending_week == week,
                        // Keep a deleted task's output recoverable from the weekly notes view.
                        Some(id) => self.doc.task(id).is_none(),
                    }
                }
            })
            .cloned()
            .collect();
        for (index, pending) in pending.into_iter().enumerate() {
            let title = pending
                .0
                .as_ref()
                .and_then(|id| self.doc.task(id))
                .map(|task| task.title.clone())
                .unwrap_or_else(|| format!("{} · {}", self.t("富文本", "Rich text"), pending.1));
            view = view.child(
                div()
                    .p(px(10.))
                    .rounded(px(6.))
                    .bg(p.subtle)
                    .border_1()
                    .border_color(p.border)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_size(px(11.)).text_color(p.muted).child(format!(
                        "{}: {title}",
                        self.t("尚未写入的富文本", "Unsaved rich text")
                    )))
                    .child(
                        quiet_row(
                            SharedString::from(format!("{scope}-recover-rich-{index}")),
                            IconName::Undo2,
                            self.t("恢复为新任务", "Recover as a new task"),
                            p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let Some(position) =
                                this.rich_pending.iter().position(|entry| entry == &pending)
                            else {
                                return;
                            };
                            this.change(cx, |doc| {
                                let id = doc.add_task(
                                    &format!("恢复 / Recovered · {title}"),
                                    pending.1,
                                    0,
                                );
                                doc.task_mut(&id).unwrap().rich_content = Some(pending.2.clone());
                                Ok(())
                            });
                            if this.error.is_none() {
                                this.rich_pending.remove(position);
                            }
                            cx.notify();
                        })),
                    ),
            );
        }
        view.child(
            quiet_row(
                SharedString::from(format!("{scope}-rich-recovery-folder")),
                IconName::FolderOpen,
                self.t("打开富文本恢复目录", "Open rich text recovery folder"),
                p,
            )
            .on_click(move |_, _, cx| cx.reveal_path(&directory)),
        )
        .into_any_element()
    }

    pub fn editor_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette(window);
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        let Some(task) = self.doc.task(&editor.id).cloned() else {
            return div().into_any_element();
        };
        let id = task.id.clone();
        let delete = id.clone();
        let rich = id.clone();
        let parent = id.clone();
        let child = id.clone();
        let save_status = if self.error.is_some() {
            self.t("保存未完成", "Save incomplete")
        } else if editor.draft(cx) != editor.baseline
            || !editor.progress.read(cx).value().is_empty()
        {
            self.t("有未保存的更改", "Unsaved changes")
        } else {
            self.t("已保存", "Saved")
        };
        let status = if task.completed {
            self.t("已完成", "Completed")
        } else {
            self.t("进行中", "In progress")
        };
        let mut content = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .when_some(
                task.parent_id.as_ref().and_then(|id| self.doc.task(id)),
                |el, parent_task| {
                    let parent_id = parent_task.id.clone();
                    let parent_title = parent_task.title.clone();
                    el.child(
                        div()
                            .id("parent-context")
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .mb_2()
                            .py_1()
                            .px_1()
                            .rounded(px(4.))
                            .text_xs()
                            .text_color(p.muted)
                            .cursor_pointer()
                            .map(|row| self.task_focus.style(row, &parent_id, p))
                            .tooltip(move |window, cx| {
                                Tooltip::new(parent_title.clone()).build(window, cx)
                            })
                            .child(glyph(IconName::ChevronLeft))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .when(self.task_link_active(parent_task, window, cx), |el| {
                                        el.text_color(p.accent).underline()
                                    })
                                    .child(format!(
                                        "{} · {}",
                                        self.t("父任务", "Parent task"),
                                        parent_task.title
                                    )),
                            )
                            .on_click(cx.listener(move |this, event, window, cx| {
                                this.activate_task(&parent_id, event, window, cx);
                            })),
                    )
                },
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        Input::new(&editor.title)
                            .appearance(false)
                            .px(px(0.))
                            .h(px(58.))
                            .text_size(px(20.)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.muted)
                            .child(format!("{} · {}", task.week_start, status)),
                    ),
            );

        let mut properties = div()
            .w(px(340.))
            .flex_shrink_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(10.))
            .pl(px(20.))
            .border_l_1()
            .border_color(p.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(label(self.t("工单编号", "Issue key"), p))
                    .child(property_input(&editor.issue)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(label(
                        self.t("所属周 · 选择周内任意日期", "Week · select any day"),
                        p,
                    ))
                    .child(
                        DatePicker::new(&editor.week)
                            .number_of_months(1)
                            .cleanable(false)
                            .w_full(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(label(self.t("截止时间 · 本地时间", "Due · local time"), p))
                    .child(editor.due.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(label(
                        self.t("提醒时间 · 本地时间", "Reminder · local time"),
                        p,
                    ))
                    .child(editor.reminder.clone()),
            );

        let mut quadrants = div().grid().grid_cols(2).gap(px(4.));
        for q in 0..4 {
            let id = id.clone();
            let selected = task.quadrant() == q;
            quadrants = quadrants.child(
                chip(
                    ("editor-q", q),
                    theme::quadrant_name(q, self.doc.settings.language == "en"),
                    p,
                )
                .min_w_0()
                .px(px(6.))
                .text_size(px(12.))
                .bg(if selected { p.selected } else { p.subtle })
                .text_color(p.quadrant[q])
                .border_1()
                .border_color(if selected {
                    p.quadrant[q].opacity(0.4)
                } else {
                    p.border
                })
                .font_weight(if selected {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change(cx, |doc| {
                        let task = doc
                            .task_mut(&id)
                            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;
                        (task.urgent, task.important) = model::quadrant_flags(q);
                        Ok(())
                    })
                })),
            );
        }
        let mut tags = div().flex().flex_wrap().gap_1();
        for tag in self.doc.tags.clone() {
            let id = id.clone();
            let selected = task.tags.contains(&tag.id);
            let tag_id = tag.id.clone();
            tags = tags.child(
                chip(
                    SharedString::from(format!("edit-tag-{}", tag.id)),
                    format!("#{}", tag.name),
                    p,
                )
                .bg(if selected { p.selected } else { p.subtle })
                .text_color(if selected { p.text } else { p.muted })
                .border_1()
                .border_color(if selected {
                    p.accent.opacity(0.35)
                } else {
                    p.border
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change(cx, |doc| {
                        let task = doc
                            .task_mut(&id)
                            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;
                        if task.tags.contains(&tag_id) {
                            task.tags.retain(|t| t != &tag_id);
                        } else {
                            task.tags.push(tag_id.clone());
                        }
                        Ok(())
                    })
                })),
            );
        }
        properties = properties.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .flex_shrink_0()
                .child(label(self.t("象限", "Quadrant"), p))
                .child(quadrants)
                .child(label(self.t("标签", "Tags"), p))
                .child(
                    div()
                        .id("task-tag-options")
                        .max_h(px(64.))
                        .overflow_y_scroll()
                        .child(tags),
                ),
        );

        let rich_label = if task.rich_content.is_some() {
            self.t("编辑富文本与图片", "Edit rich text & images")
        } else {
            self.t("添加富文本与图片", "Add rich text & images")
        };
        let mut details = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .flex_shrink_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(label(self.t("任务详情", "Description"), p))
                    .child(
                        quiet_row("rich-task", IconName::BookOpen, rich_label, p)
                            .text_color(if task.rich_content.is_some() {
                                p.accent
                            } else {
                                p.muted
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_rich_document(Some(rich.clone()), window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .rounded(px(6.))
                    .bg(p.subtle)
                    .px(px(6.))
                    .py(px(2.))
                    .child(
                        Input::new(&editor.notes)
                            .appearance(false)
                            .px(px(0.))
                            .h(px(110.))
                            .text_size(px(13.)),
                    ),
            )
            .child(label(self.t("链接（每行一条）", "Links (one per line)"), p))
            .child(
                div()
                    .rounded(px(6.))
                    .bg(p.subtle)
                    .px(px(6.))
                    .py(px(2.))
                    .child(
                        Input::new(&editor.links)
                            .appearance(false)
                            .px(px(0.))
                            .h(px(62.))
                            .text_size(px(12.)),
                    ),
            );
        let mut saved_links = div()
            .id("task-saved-links")
            .max_h(px(40.))
            .overflow_y_scroll();
        for (n, url) in task.links.iter().enumerate() {
            let url = url.clone();
            let shown = url.clone();
            saved_links = saved_links.child(
                div()
                    .id(("open-link", n))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(4.))
                    .py(px(2.))
                    .text_size(px(11.))
                    .text_color(p.muted)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(p.text))
                    .child(glyph(IconName::ExternalLink))
                    .child(div().flex_1().min_w_0().truncate().child(shown))
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }
        if !task.links.is_empty() {
            details = details.child(saved_links);
        }
        content = content.child(details);

        let mut progress_history = div()
            .id("task-progress-history")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(6.));
        let mut progress = task.progress_notes.clone();
        progress.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        for note in progress {
            let task_id = id.clone();
            let note_id = note.id.clone();
            let hover_group = SharedString::from(format!("progress-row-{}", note.id));
            progress_history = progress_history.child(
                div()
                    .group(hover_group.clone())
                    .flex_shrink_0()
                    .flex()
                    .items_start()
                    .gap_2()
                    .py(px(2.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div().text_size(px(10.)).text_color(p.muted).child(
                                    note.created_at
                                        .with_timezone(&Local)
                                        .format("%m-%d %H:%M")
                                        .to_string(),
                                ),
                            )
                            .child(div().text_size(px(13.)).child(note.text)),
                    )
                    .child(
                        icon_button(
                            SharedString::from(format!("delete-note-{}", note.id)),
                            Icon::new(IconName::Delete).path("icons/trash.svg"),
                            self.t("删除进展", "Delete progress"),
                            p,
                        )
                        .invisible()
                        .group_hover(hover_group, |style| style.visible())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.change(cx, |doc| {
                                if let Some(task) = doc.task_mut(&task_id) {
                                    task.progress_notes.retain(|n| n.id != note_id);
                                }
                                Ok(())
                            })
                        })),
                    ),
            );
        }
        let progress_group = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(6.))
            .child(label(self.t("进展记录", "Progress"), p))
            .child(
                div()
                    .flex_shrink_0()
                    .rounded(px(6.))
                    .bg(p.subtle)
                    .px(px(6.))
                    .py(px(2.))
                    .child(
                        Input::new(&editor.progress)
                            .appearance(false)
                            .px(px(0.))
                            .h(px(54.))
                            .text_size(px(13.)),
                    ),
            );
        content = content.child(progress_group.child(progress_history));

        let direct_subtasks = self.doc.children(&id);
        let completed_subtasks = direct_subtasks.iter().filter(|sub| sub.completed).count();
        let mut subtasks = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(label(
                        self.t("子任务 · 最多三级", "Subtasks · up to three levels"),
                        p,
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted)
                            .child(format!("{completed_subtasks}/{}", direct_subtasks.len())),
                    ),
            );
        let mut subtask_list = div()
            .id("task-subtask-list")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.));
        for sub in &direct_subtasks {
            let id = sub.id.clone();
            let complete = id.clone();
            let delete = id.clone();
            let completed = sub.completed;
            let hover_group = SharedString::from(format!("subtask-row-{id}"));
            subtask_list = subtask_list.child(
                div()
                    .id(SharedString::from(format!("sub-open-{id}")))
                    .group(hover_group.clone())
                    .flex()
                    .min_w_0()
                    .flex_shrink_0()
                    .items_start()
                    .gap_1()
                    .py(px(2.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .map(|row| self.task_focus.style(row, &id, p))
                    .on_click(cx.listener(move |this, event, window, cx| {
                        this.activate_task(&id, event, window, cx)
                    }))
                    .child(
                        task_checkbox(
                            SharedString::from(format!("sub-complete-{complete}")),
                            completed,
                            p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.change(cx, |doc| {
                                doc.toggle_complete(&complete);
                                Ok(())
                            })
                        })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .line_height(relative(1.45))
                            .whitespace_normal()
                            .text_color(if completed { p.muted } else { p.text })
                            .child(self.task_title(sub, window, cx)),
                    )
                    .child(
                        icon_button(
                            SharedString::from(format!("sub-delete-{delete}")),
                            Icon::new(IconName::Delete).path("icons/trash.svg"),
                            self.t("删除子任务", "Delete subtask"),
                            p,
                        )
                        .size(px(22.))
                        .invisible()
                        .group_hover(hover_group, |style| style.visible())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.change(cx, |doc| {
                                doc.delete_task(&delete);
                                Ok(())
                            })
                        })),
                    ),
            );
        }
        subtasks = subtasks
            .when(!direct_subtasks.is_empty(), |el| el.child(subtask_list))
            .child(
                quiet_row(
                    "add-subtask",
                    IconName::Plus,
                    self.t("添加子任务", "Add subtask"),
                    p,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    let q = this.doc.task(&child).map(Task::quadrant).unwrap_or(0);
                    this.add_task(q, Some(child.clone()), window, cx);
                })),
            )
            .when(task.parent_id.is_some(), |el| {
                el.child(
                    quiet_row(
                        "promote-task",
                        IconName::ArrowUp,
                        self.t("提升为顶层任务", "Move to top level"),
                        p,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(cx, |doc| doc.reparent(&parent, None))
                    })),
                )
            })
            .child(
                div()
                    .pt_2()
                    .text_size(px(10.))
                    .text_color(p.muted)
                    .child(format!(
                        "{} {}",
                        self.t("创建于", "Created"),
                        task.created_at
                            .with_timezone(&Local)
                            .format("%Y-%m-%d %H:%M")
                    )),
            );
        properties = properties.child(subtasks);
        content = content.child(
            div()
                .id("task-rich-recovery")
                .max_h(px(72.))
                .flex_shrink_0()
                .overflow_y_scroll()
                .child(self.rich_recovery_view("task", Some(&id), self.week, p, cx)),
        );
        let content = div()
            .flex_1()
            .min_h_0()
            .flex()
            .gap(px(20.))
            .px(px(20.))
            .py(px(16.))
            .child(content)
            .child(properties);

        div()
            .size_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(p.panel)
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.muted)
                            .on_mouse_down(MouseButton::Left, |_, window, _| {
                                window.start_window_move();
                            })
                            .child(self.t("任务详情", "Task details")),
                    )
                    .child(
                        icon_button("editor-close", IconName::Close, self.t("关闭", "Close"), p)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_task_dialog(window, cx);
                            })),
                    ),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .mx(px(16.))
                        .mt_3()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(p.quadrant[0].opacity(0.5))
                        .bg(p.quadrant[0].opacity(0.10))
                        .text_size(px(12.))
                        .text_color(p.quadrant[0])
                        .whitespace_normal()
                        .child(error),
                )
            })
            .child(content)
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(p.muted)
                            .child(save_status),
                    )
                    .child(
                        chip("delete-task", self.t("删除", "Delete"), p)
                            .bg(p.quadrant[0].opacity(0.10))
                            .text_color(p.quadrant[0])
                            .px(px(14.))
                            .py(px(5.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.change(cx, |doc| {
                                    doc.delete_task(&delete);
                                    Ok(())
                                });
                                if this.error.is_none() {
                                    this.dismiss_task_dialog(window, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("save-task")
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(px(32.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .bg(p.accent)
                            .text_color(p.accent_text)
                            .px(px(14.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .hover(move |style| style.bg(p.accent.opacity(0.85)))
                            .child(self.t("保存", "Save"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_editor(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{TaskDraft, apply_rich_result, parse_date};
    use crate::model::{Document, ProgressNote, Task, WeekNote};
    use chrono::{DateTime, Duration, Local, NaiveDate, Utc};

    #[test]
    fn pristine_refresh_follows_model_changes_without_losing_edits() {
        let date = DateTime::parse_from_rfc3339("2026-10-18T09:30:47Z")
            .unwrap()
            .with_timezone(&Utc);
        let original = Task {
            title: "Before".into(),
            issue_key: Some("ABC-1".into()),
            notes: "original notes".into(),
            due_date: Some(date),
            remind_at: Some(date),
            links: vec!["https://old.example".into()],
            ..Task::default()
        };
        let mut baseline = TaskDraft::from_task(&original);
        let mut draft = baseline.clone();
        draft.notes = "local notes".into();
        draft.reminder = "2026-11-01 08:15".into();
        let mut original_title = original.title.clone();
        let mut latest_task = original.clone();
        latest_task.title = "Updated elsewhere".into();
        latest_task.issue_key = Some("ABC-2".into());
        latest_task.notes = "external notes".into();
        latest_task.due_date = None;
        latest_task.remind_at = None;
        latest_task.week_start += Duration::weeks(1);
        latest_task.links = vec!["https://new.example".into()];
        let latest = TaskDraft::from_task(&latest_task);

        draft.refresh_pristine(&mut baseline, &latest, &mut original_title);

        assert_eq!(draft.title, latest.title);
        assert_eq!(draft.issue, latest.issue);
        assert_eq!(draft.due, latest.due);
        assert_eq!(draft.week, latest.week);
        assert_eq!(draft.links, latest.links);
        assert_eq!(baseline.title, latest.title);
        assert_eq!(baseline.issue, latest.issue);
        assert_eq!(baseline.due, latest.due);
        assert_eq!(baseline.week, latest.week);
        assert_eq!(baseline.links, latest.links);
        assert_eq!(draft.notes, "local notes");
        assert_eq!(draft.reminder, "2026-11-01 08:15");
        assert_eq!(baseline.notes, original.notes);
        assert_eq!(baseline.reminder, TaskDraft::from_task(&original).reminder);

        let mut saved = latest_task.clone();
        draft
            .apply(&baseline, &mut saved, "pending progress")
            .unwrap();
        assert_eq!(saved.title, latest_task.title);
        assert_eq!(saved.issue_key, latest_task.issue_key);
        assert_eq!(saved.due_date, None);
        assert_eq!(saved.week_start, latest_task.week_start);
        assert_eq!(saved.links, latest_task.links);
        assert_eq!(saved.notes, "local notes");
        assert_eq!(saved.remind_at, parse_date("2026-11-01 08:15").unwrap());
        assert_eq!(saved.progress_notes[0].text, "pending progress");
    }

    #[test]
    fn pristine_refresh_preserves_new_title_placeholder_until_model_title_changes() {
        let task = Task {
            title: "Untitled task".into(),
            ..Task::default()
        };
        let mut baseline = TaskDraft::from_task(&task);
        baseline.title.clear();
        let mut draft = baseline.clone();
        let mut original_title = task.title.clone();
        let mut latest = TaskDraft::from_task(&task);
        latest.week = "2026-10-26".into();

        draft.refresh_pristine(&mut baseline, &latest, &mut original_title);
        assert!(draft.title.is_empty());
        assert!(baseline.title.is_empty());
        assert_eq!(draft.week, latest.week);

        latest.title = "Renamed elsewhere".into();
        draft.refresh_pristine(&mut baseline, &latest, &mut original_title);
        assert_eq!(draft.title, latest.title);
        assert_eq!(baseline.title, latest.title);
        assert_eq!(original_title, latest.title);

        latest.title = task.title;
        draft.refresh_pristine(&mut baseline, &latest, &mut original_title);
        assert_eq!(draft.title, latest.title);
        assert_eq!(baseline.title, latest.title);
    }

    #[test]
    fn pristine_refresh_defers_external_title_while_local_title_is_dirty() {
        let task = Task {
            title: "Before".into(),
            ..Task::default()
        };
        let mut baseline = TaskDraft::from_task(&task);
        let mut draft = baseline.clone();
        draft.title = "Local edit".into();
        let mut original_title = task.title.clone();
        let mut latest = TaskDraft::from_task(&task);
        latest.title = "External edit".into();

        draft.refresh_pristine(&mut baseline, &latest, &mut original_title);
        assert_eq!(draft.title, "Local edit");
        assert_eq!(baseline.title, task.title);

        draft.title.clone_from(&baseline.title);
        draft.refresh_pristine(&mut baseline, &latest, &mut original_title);
        assert_eq!(draft.title, latest.title);
        assert_eq!(baseline.title, latest.title);
    }

    #[test]
    fn title_edit_preserves_external_schedule_and_rich_progress() {
        let original = Task {
            title: "Before".into(),
            due_date: Some(
                DateTime::parse_from_rfc3339("2026-10-18T09:30:47Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            ..Task::default()
        };
        let baseline = TaskDraft::from_task(&original);
        let mut draft = baseline.clone();
        draft.title = "After".into();
        let mut current = original.clone();
        current.week_start += Duration::weeks(1);
        current.due_date = original.due_date.map(|date| date + Duration::days(8));
        current.rich_content = Some("external rich text".into());
        current.progress_notes.push(ProgressNote {
            text: "external progress".into(),
            ..Default::default()
        });
        let expected = current.clone();
        draft
            .apply(&baseline, &mut current, "new progress")
            .unwrap();
        assert_eq!(current.title, "After");
        assert_eq!(current.week_start, expected.week_start);
        assert_eq!(current.due_date, expected.due_date);
        assert_eq!(current.rich_content, expected.rich_content);
        assert_eq!(current.progress_notes[0], expected.progress_notes[0]);
        assert_eq!(current.progress_notes[1].text, "new progress");
    }

    #[test]
    fn untouched_dates_preserve_seconds_and_carry_clears() {
        let date = DateTime::parse_from_rfc3339("2026-10-18T09:30:47Z")
            .unwrap()
            .with_timezone(&Utc);
        let original = Task {
            title: "Task".into(),
            due_date: Some(date),
            remind_at: Some(date),
            ..Default::default()
        };
        let baseline = TaskDraft::from_task(&original);
        let mut current = original.clone();
        baseline.apply(&baseline, &mut current, "").unwrap();
        assert_eq!(current, original);
        current.week_start += Duration::weeks(1);
        current.due_date = None;
        current.remind_at = None;
        let carried = current.clone();
        baseline.apply(&baseline, &mut current, "").unwrap();
        assert_eq!(current, carried);
        let mut changed = baseline.clone();
        changed.due = "invalid date".into();
        assert!(changed.apply(&baseline, &mut current, "").is_err());
    }

    #[test]
    fn explicit_field_edits_still_apply_to_latest_model() {
        let original = Task {
            title: "Task".into(),
            issue_key: Some("ABC-1".into()),
            notes: "old".into(),
            due_date: Some(
                DateTime::parse_from_rfc3339("2026-10-18T09:30:47Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            ..Task::default()
        };
        let baseline = TaskDraft::from_task(&original);
        let mut draft = baseline.clone();
        draft.issue.clear();
        draft.notes = "new notes".into();
        draft.due = "2026-11-01 08:15".into();
        draft.week = "2026-10-28".into();
        draft.links = " https://one.example \n\nhttps://two.example ".into();
        let mut current = original;
        draft.apply(&baseline, &mut current, "").unwrap();
        assert_eq!(current.issue_key, None);
        assert_eq!(current.notes, "new notes");
        assert_eq!(
            current.links,
            vec!["https://one.example", "https://two.example"]
        );
        assert_eq!(
            current
                .due_date
                .unwrap()
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-11-01 08:15"
        );
        assert_eq!(
            current.week_start,
            NaiveDate::from_ymd_opt(2026, 10, 26).unwrap()
        );
    }

    #[test]
    fn rich_task_conflict_and_deleted_target_do_not_overwrite() {
        let mut doc = Document::default();
        let week = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let id = doc.add_task("Task", week, 0);
        doc.task_mut(&id).unwrap().rich_content = Some("newer".into());
        let before = doc.clone();
        assert!(
            apply_rich_result(&mut doc, Some(&id), week, &Some("old".into()), "stale").is_err()
        );
        assert_eq!(doc, before);
        apply_rich_result(&mut doc, Some(&id), week, &Some("newer".into()), "edited").unwrap();
        assert_eq!(
            doc.task(&id).unwrap().rich_content.as_deref(),
            Some("edited")
        );
        doc.delete_task(&id);
        assert!(apply_rich_result(&mut doc, Some(&id), week, &None, "stale").is_err());
        assert!(doc.tasks.is_empty());
    }

    #[test]
    fn rich_week_conflict_preserves_newer_content_and_plain_text() {
        let mut doc = Document::default();
        let week = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        doc.week_notes.push(WeekNote {
            week_start: week,
            text: "plain".into(),
            rich_content: Some("newer".into()),
            markdown: None,
        });
        let before = doc.clone();
        assert!(apply_rich_result(&mut doc, None, week, &None, "stale").is_err());
        assert_eq!(doc, before);
        apply_rich_result(&mut doc, None, week, &Some("newer".into()), "edited").unwrap();
        assert_eq!(doc.week_notes[0].text, "plain");
        assert_eq!(doc.week_notes[0].rich_content.as_deref(), Some("edited"));
        doc.week_notes.clear();
        assert!(apply_rich_result(&mut doc, None, week, &Some("old".into()), "stale").is_err());
    }
    #[test]
    fn invalid_calendar_date_is_rejected() {
        assert!(parse_date("2026-02-30").is_err());
        assert!(parse_date("tomorrow").is_err());
    }
    #[test]
    fn empty_date_clears_schedule() {
        assert!(parse_date("  ").unwrap().is_none());
    }
    #[test]
    fn valid_local_date_roundtrips() {
        let value = parse_date("2026-10-18 09:30").unwrap().unwrap();
        assert_eq!(
            value
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2026-10-18 09:30"
        );
    }
}
