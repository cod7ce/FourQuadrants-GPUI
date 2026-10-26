mod accent_picker;
mod calendar;
mod confetti;
mod editor;
mod notes;
mod notes_preview;
mod settings;
mod task_dialog;
mod task_focus;
mod task_schedule;
mod updates;

use crate::{
    model::{self, Document, Task},
    store::{self, Store},
    theme::{self, Palette},
};
use anyhow::Result;
use chrono::{Datelike, Duration, NaiveDate};
use editor::TaskEditor;
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, IconNamed, Sizable, Theme, ThemeMode,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
};
use settings::SettingsEditor;
use std::{collections::HashSet, path::PathBuf};

const SIDEBAR_RAIL_WIDTH: f32 = 44.;
const SIDEBAR_RAIL_BUTTON_SIZE: f32 = 32.;
const TASK_CHECKBOX_SIZE: f32 = 15.;
const TASK_TITLE_GAP: f32 = 6.;
const TASK_CONNECTOR_GAP: f32 = 6.;

actions!(
    four_quadrants,
    [
        NewTask,
        ToggleSidebar,
        Search,
        Settings,
        Undo,
        Redo,
        Previous,
        Next,
        ThisWeek,
        ExportMarkdown,
        ClosePanel,
        SaveTask,
        CloseTaskWindow,
        ToggleCompleted,
        ShowBoard,
        ShowAgenda,
        ShowCalendar,
        ShowNotes,
        PasteTask,
        IncreaseFont,
        DecreaseFont,
        ResetFont
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Board,
    Agenda,
    Calendar,
    All,
    Notes,
    Settings,
}

#[derive(Clone)]
struct DragTask {
    id: String,
    title: String,
    color: Hsla,
    palette: Palette,
    hint: &'static str,
}

struct DragPreview {
    task: DragTask,
    offset: Point<Pixels>,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = self.task.palette;
        div().w(px(260.)).h(px(64.)).child(
            div()
                .absolute()
                // GPUI subtracts the original grip offset; compensate inside a
                // wrapper, since the root's own position is supplied by GPUI.
                .left(self.offset.x + px(16.))
                .top(self.offset.y + px(16.))
                .w(px(260.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(8.))
                .border_1()
                .border_color(p.accent.opacity(0.6))
                .shadow_lg()
                .bg(p.bg.alpha(0.98))
                .text_color(p.text)
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .w(px(3.))
                        .h(px(28.))
                        .rounded_full()
                        .bg(self.task.color),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(self.task.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(p.muted)
                                .child(self.task.hint),
                        ),
                )
                .with_animation(
                    "drag-lift",
                    Animation::new(std::time::Duration::from_millis(140))
                        .with_easing(ease_out_quint()),
                    |el, progress| el.opacity(0.55 + 0.45 * progress),
                ),
        )
    }
}

fn start_task_drag(
    workspace: &WeakEntity<Workspace>,
    task: &DragTask,
    offset: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DragPreview> {
    let _ = workspace.update(cx, |this, cx| {
        this.focus.focus(window);
        this.dragging_task = Some(task.id.clone());
        cx.notify();
    });
    cx.new(|_| DragPreview {
        task: task.clone(),
        offset,
    })
}

pub struct Workspace {
    pub doc: Document,
    pub store: Store,
    pub page: Page,
    pub week: NaiveDate,
    pub day: NaiveDate,
    pub selected: Option<String>,
    task_focus: task_focus::TaskFocus,
    pub tag_filter: Option<String>,
    pub search: Entity<InputState>,
    search_open: bool,
    pub week_text: Entity<InputState>,
    note_editor: notes::NoteEditor,
    note_subscription: Subscription,
    pub editor: Option<TaskEditor>,
    main_window: AnyWindowHandle,
    task_dialog: Option<AnyWindowHandle>,
    task_dialog_generation: u64,
    pub settings_editor: SettingsEditor,
    update_status: crate::native::UpdateStatus,
    updater_task: Option<gpui::Task<()>>,
    pub status: String,
    pub error: Option<String>,
    pub focus: FocusHandle,
    pub undo: Vec<Document>,
    pub redo: Vec<Document>,
    collapsed: HashSet<String>,
    sidebar_overlay: bool,
    dragging_task: Option<String>,
    confetti: confetti::Confetti,
    background_key: String,
    background_render: Option<PathBuf>,
    clipboard_text: Option<String>,
    clipboard_candidate: Option<model::ParsedTaskInput>,
    clipboard_generation: u64,
    pub rich_sessions: HashSet<String>,
    pub rich_pending: Vec<(Option<String>, NaiveDate, String)>,
    carry_selection: Option<HashSet<String>>,
    _subscriptions: Vec<Subscription>,
}

pub fn field(
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    multiline: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
            .multi_line(multiline)
    })
}

pub fn text_input(state: &Entity<InputState>) -> Input {
    Input::new(state).min_h(px(32.))
}

pub fn chip(id: impl Into<ElementId>, label: impl Into<SharedString>, p: Palette) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .px_3()
        .py_1()
        .h(rems(2.))
        .min_h(px(32.))
        .rounded(px(6.))
        .text_sm()
        .line_height(rems(1.25))
        .text_color(p.text)
        .cursor_pointer()
        .hover(move |s| s.bg(p.hover).text_color(p.text))
        .when(!label.is_empty(), |el| el.child(label))
}

/// Task list with a forward arrow for carrying unfinished tasks into the selected week.
#[derive(Clone, Copy)]
struct CarryForward;

impl IconNamed for CarryForward {
    fn path(self) -> SharedString {
        "icons/carry-forward.svg".into()
    }
}

/// Hugeicons LayoutAlignRight, matching MonoCode's sidebar toggle.
#[derive(Clone, Copy)]
struct SidebarToggle;

impl IconNamed for SidebarToggle {
    fn path(self) -> SharedString {
        "icons/sidebar-toggle.svg".into()
    }
}

/// Pushpin marking the weekly pinned task box.
#[derive(Clone, Copy)]
struct PinIcon;

impl IconNamed for PinIcon {
    fn path(self) -> SharedString {
        "icons/pin.svg".into()
    }
}

/// Icons ported from the original FourQuadrants SwiftUI paths.
#[derive(Clone, Copy)]
enum AppIcon {
    Overview,
    ThisWeek,
    Calendar,
    Share,
    Notes,
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        match self {
            Self::Overview => "icons/overview.svg",
            Self::ThisWeek => "icons/this-week.svg",
            Self::Calendar => "icons/calendar-dates.svg",
            Self::Share => "icons/share.svg",
            Self::Notes => "icons/notes.svg",
        }
        .into()
    }
}

fn sidebar_button(
    id: impl Into<ElementId>,
    label: &'static str,
    expanded: bool,
    p: Palette,
) -> Stateful<Div> {
    icon_button_container(
        id,
        label,
        Palette {
            muted: if expanded { p.text } else { p.muted },
            hover: p.text.opacity(0.1),
            ..p
        },
    )
    .size(px(if expanded {
        24.
    } else {
        SIDEBAR_RAIL_BUTTON_SIZE
    }))
    .rounded(px(6.))
    .child(Icon::new(SidebarToggle).with_size(px(if expanded { 14. } else { 18. })))
}

pub fn glyph(icon: impl Into<Icon>) -> Icon {
    Icon::new(icon).with_size(px(15.))
}

fn metadata_item(window: &Window, icon: IconName, text: impl Into<SharedString>) -> Div {
    // text_xs is 0.75rem. Match the icon to that size and lift it slightly so its
    // optical center meets the glyph ink instead of the taller line box center.
    let text_size = window.rem_size() * 0.75;
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_xs()
        .line_height(relative(1.))
        .child(
            div()
                .flex_none()
                .relative()
                .top(-text_size * 0.08)
                .child(glyph(icon).with_size(text_size)),
        )
        .child(div().whitespace_nowrap().child(text.into()))
}

pub fn icon_button(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
    label: impl Into<SharedString>,
    p: Palette,
) -> Stateful<Div> {
    icon_button_container(id, label, p).child(glyph(icon))
}

fn titlebar_button(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
    icon_size: f32,
    label: impl Into<SharedString>,
    p: Palette,
) -> Stateful<Div> {
    icon_button_container(id, label, p)
        .size(px(22.))
        .rounded(px(4.))
        // Match visible glyph sizes despite different padding in the SVGs.
        .child(Icon::new(icon).with_size(px(icon_size)))
}

fn icon_button_container(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    p: Palette,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .size(px(28.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .text_color(p.muted)
        .cursor_pointer()
        .hover(move |s| s.bg(p.hover).text_color(p.text))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
}

fn app_logo(p: Palette) -> Div {
    let colors = if p.dark {
        [
            rgb(0xff453a).into(),
            rgb(0x0a84ff).into(),
            rgb(0xff9f0a).into(),
            p.muted,
        ]
    } else {
        [
            rgb(0xff3b30).into(),
            rgb(0x007aff).into(),
            rgb(0xff9500).into(),
            p.muted,
        ]
    };
    div()
        .size(px(26.))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(2.))
        .children(colors.chunks(2).map(|row| {
            div().flex().gap(px(2.)).children(
                row.iter()
                    .map(|color| div().size(px(12.)).rounded(px(3.)).bg(color.opacity(0.9))),
            )
        }))
}

fn task_checkbox(id: impl Into<ElementId>, completed: bool, p: Palette) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(TASK_CHECKBOX_SIZE))
        // Match text_sm's font size and the task title's first-line height.
        .h(rems(0.875 * 1.45))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .size(px(TASK_CHECKBOX_SIZE))
                .flex_shrink_0()
                .rounded(px(5.))
                .border_1()
                .border_color(if completed {
                    p.accent
                } else {
                    p.muted.opacity(0.55)
                })
                .bg(if completed {
                    p.accent
                } else {
                    transparent_black()
                })
                .flex()
                .items_center()
                .justify_center()
                .text_color(p.accent_text)
                .when(completed, |el| {
                    el.child(glyph(IconName::Check).with_size(px(11.)))
                }),
        )
}

enum TaskConnector {
    Stem {
        top: Pixels,
    },
    Branch {
        y: Pixels,
        width: Pixels,
        continues: bool,
    },
}

fn task_connector(connector: TaskConnector, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let scale = window.scale_factor();
            let snap = |value: Pixels| px((f32::from(value) * scale).round() / scale);
            let thickness = px(scale.round().max(1.) / scale);
            let left = snap(bounds.left() + px(14.));
            let right = left + thickness;
            let bottom = snap(bounds.bottom());
            let mut paint = |left, top, right, bottom| {
                if right > left && bottom > top {
                    window.paint_quad(fill(
                        Bounds::from_corners(point(left, top), point(right, bottom)),
                        color,
                    ));
                }
            };
            // Snap shared endpoints in window coordinates. Each pixel belongs to
            // one fill, avoiding alpha buildup where borders and stems used to meet.
            match connector {
                TaskConnector::Stem { top } => {
                    paint(left, snap(bounds.top() + top), right, bottom);
                }
                TaskConnector::Branch {
                    y,
                    width,
                    continues,
                } => {
                    let junction = snap(bounds.top() + y);
                    paint(
                        left,
                        snap(bounds.top()),
                        right,
                        if continues { bottom } else { junction },
                    );
                    paint(
                        right,
                        junction - thickness,
                        snap(bounds.left() + px(14.) + width),
                        junction,
                    );
                }
            }
        },
    )
    .absolute()
    .inset_0()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeeklyGroup {
    Unplanned,
    Overdue,
    Day(NaiveDate),
}

fn weekly_group(
    task: &Task,
    week: NaiveDate,
    today: NaiveDate,
    completed: bool,
) -> Option<WeeklyGroup> {
    // Use the displayed completion state so a newly checked row stays put during confetti.
    let week = model::week_start(week);
    let date = task
        .due_date
        .or(if completed { task.completed_at } else { None });
    let group = if let Some(date) = date {
        let date = date.with_timezone(&chrono::Local).date_naive();
        if date < week || date >= week + Duration::days(7) {
            return None;
        }
        if !completed && date < today {
            WeeklyGroup::Overdue
        } else {
            WeeklyGroup::Day(date)
        }
    } else {
        if model::week_start(task.week_start) != week {
            return None;
        }
        // Legacy completed tasks can lack both timestamps; keep them visible in their assigned week.
        if completed {
            WeeklyGroup::Day(week)
        } else {
            WeeklyGroup::Unplanned
        }
    };
    Some(group)
}

fn weekly_order_date(task: &Task, completed: bool) -> Option<chrono::DateTime<chrono::Utc>> {
    if completed {
        task.completed_at.or(task.due_date)
    } else {
        task.due_date
    }
}

/// Weekly boxes list open tasks by due time and completed tasks by completion
/// time. Tasks without either keep their manual order after the dated ones.
fn weekly_order(a: &Task, a_completed: bool, b: &Task, b_completed: bool) -> std::cmp::Ordering {
    let a_date = weekly_order_date(a, a_completed);
    let b_date = weekly_order_date(b, b_completed);
    match (a_date, b_date) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
    .then_with(|| a.sort_order.total_cmp(&b.sort_order))
}

fn matches_tag_week(task: &Task, tag: &str, week: NaiveDate) -> bool {
    task.tags.iter().any(|id| id == tag)
        && model::week_start(task.week_start) == model::week_start(week)
}

fn week_completion(doc: &Document, week: NaiveDate) -> [(usize, usize); 4] {
    let mut counts = [(0, 0); 4];
    let week = model::week_start(week);
    for task in &doc.tasks {
        if task.parent_id.is_none() && model::week_start(task.week_start) == week {
            let (completed, total) = &mut counts[task.quadrant()];
            *total += 1;
            *completed += usize::from(task.completed);
        }
    }
    counts
}

fn week_labels(week: NaiveDate, en: bool) -> (String, String, String) {
    let start = model::week_start(week);
    let end = start + Duration::days(6);
    let iso_week = start.iso_week();
    if en {
        (
            format!("{} · Week {}", iso_week.year(), iso_week.week()),
            start.format("%b %-d").to_string(),
            end.format("%b %-d").to_string(),
        )
    } else {
        (
            format!("{} 年第 {} 周", iso_week.year(), iso_week.week()),
            format!("{}月{}日", start.month(), start.day()),
            format!("{}月{}日", end.month(), end.day()),
        )
    }
}

impl Workspace {
    pub fn new(doc: Document, store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let week = model::week_start(model::today());
        let search = field(
            "",
            if doc.settings.language == "en" {
                "Search tasks"
            } else {
                "搜索任务"
            },
            false,
            window,
            cx,
        );
        let week_text = field(
            doc.week_notes
                .iter()
                .find(|n| n.week_start == week)
                .map(|n| n.editable_text().to_string())
                .unwrap_or_default(),
            theme::tr(
                &doc.settings,
                "写下想法… 支持 Markdown、粘贴或拖入图片",
                "Write Markdown, paste or drop images…",
            ),
            true,
            window,
            cx,
        );
        let note_editor = notes::NoteEditor::new(
            doc.week_notes.iter().find(|note| note.week_start == week),
            cx,
        );
        let settings_editor = SettingsEditor::new(&doc, window, cx);
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()));
        let note_subscription = cx.subscribe(&week_text, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.save_week_note(cx);
            }
        });
        subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            if this.doc.settings.theme == "system" {
                this.apply_theme(window, cx);
                cx.notify();
            }
        }));
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.save_week_note(cx);
            } else {
                this.check_clipboard(window, cx);
            }
        }));
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| {
                if !this.prepare_close(window, cx) {
                    return false;
                }
                this.dismiss_task_dialog(window, cx);
                crate::native::updater_workspace_closed();
                true
            })
            .unwrap_or(true)
        });
        let mut workspace = Self {
            doc,
            store,
            page: Page::Board,
            week,
            day: model::today(),
            selected: None,
            task_focus: task_focus::TaskFocus::default(),
            tag_filter: None,
            search,
            search_open: false,
            week_text,
            note_editor,
            note_subscription,
            editor: None,
            main_window: window.window_handle(),
            task_dialog: None,
            task_dialog_generation: 0,
            settings_editor,
            update_status: crate::native::UpdateStatus::default(),
            updater_task: None,
            status: String::new(),
            error: None,
            focus: cx.focus_handle(),
            undo: Vec::new(),
            redo: Vec::new(),
            collapsed: HashSet::new(),
            sidebar_overlay: false,
            dragging_task: None,
            confetti: confetti::Confetti::default(),
            background_key: String::new(),
            background_render: None,
            clipboard_text: None,
            clipboard_candidate: None,
            clipboard_generation: 0,
            rich_sessions: HashSet::new(),
            rich_pending: Vec::new(),
            carry_selection: None,
            _subscriptions: subscriptions,
        };
        workspace.reconcile_reminders(cx);
        workspace
    }

    pub fn prepare_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.rich_sessions.is_empty() {
            self.error = Some(
                self.t(
                    "请先保存并关闭富文本窗口，再退出主应用",
                    "Save and close rich text windows before quitting",
                )
                .into(),
            );
            cx.notify();
            return false;
        }
        self.save_editor(window, cx) && self.save_week_note(cx)
    }

    pub fn palette(&self, window: &Window) -> Palette {
        Palette::new(&self.doc.settings, window.appearance())
    }
    pub fn t<'a>(&self, zh: &'a str, en: &'a str) -> &'a str {
        theme::tr(&self.doc.settings, zh, en)
    }
    pub fn change(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Document) -> Result<()>) {
        let mut next = self.doc.clone();
        match f(&mut next)
            .and_then(|_| next.validate())
            .and_then(|_| self.store.save(&next))
        {
            Ok(()) => {
                self.undo.push(self.doc.clone());
                if self.undo.len() > 50 {
                    self.undo.remove(0);
                }
                self.redo.clear();
                let reminders_changed =
                    self.doc
                        .tasks
                        .iter()
                        .any(|old| old.remind_at.is_some() && next.task(&old.id) != Some(old))
                        || next.tasks.iter().any(|new| {
                            new.remind_at.is_some() && self.doc.task(&new.id) != Some(new)
                        });
                let previous_settings = self.doc.settings.clone();
                self.task_focus.reconcile(&self.doc, &next);
                self.doc = next;
                self.sync_completion_preferences(&previous_settings);
                self.error = None;
                self.status = self.t("已保存", "Saved").into();
                if reminders_changed {
                    self.reconcile_reminders(cx);
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        cx.notify();
    }

    fn resize_font(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.change(cx, |doc| {
            doc.settings.font_size = if delta == 0. {
                14.
            } else {
                (doc.settings.font_size + delta).clamp(10., 28.)
            };
            Ok(())
        });
        self.apply_theme(window, cx);
    }

    fn titlebar_height(&self) -> Pixels {
        px((self.doc.settings.font_size * 1.5 + 12.).max(40.))
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = false;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.focus.focus(window);
        cx.notify();
    }

    fn narrow_window(window: &Window) -> bool {
        window.viewport_size().width < px(760.)
    }

    fn toggle_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) {
        if Self::narrow_window(window) {
            self.sidebar_overlay = !self.sidebar_overlay;
            cx.notify();
        } else {
            self.change(cx, |doc| {
                doc.settings.sidebar_collapsed = !doc.settings.sidebar_collapsed;
                Ok(())
            });
        }
    }

    pub fn apply_theme(&mut self, window: &mut Window, cx: &mut App) {
        gpui_component::set_locale(if self.doc.settings.language == "en" {
            "en"
        } else {
            "zh-CN"
        });
        let palette = self.palette(window);
        Theme::change(
            if palette.dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            Some(window),
            cx,
        );
        let theme = Theme::global_mut(cx);
        theme.colors.background = gpui::transparent_black();
        theme.colors.foreground = palette.text;
        theme.colors.border = palette.border;
        theme.colors.input = palette.border;
        theme.colors.ring = palette.accent.opacity(0.55);
        theme.colors.caret = palette.accent;
        theme.colors.selection = palette.accent.opacity(0.22);
        theme.colors.primary = palette.accent;
        theme.colors.primary_hover = palette.accent.opacity(0.85);
        theme.colors.primary_active = palette.accent;
        theme.colors.primary_foreground = palette.accent_text;
        theme.colors.accent = palette.selected;
        theme.colors.accent_foreground = palette.text;
        theme.colors.secondary = palette.card;
        theme.colors.secondary_hover = palette.hover;
        theme.colors.secondary_foreground = palette.text;
        theme.colors.muted = palette.hover;
        theme.colors.muted_foreground = palette.muted.opacity(0.75);
        theme.colors.popover = Hsla {
            a: 1.,
            ..palette.bg
        };
        theme.colors.popover_foreground = palette.text;
        theme.colors.switch = palette.selected;
        theme.colors.switch_thumb = palette.text;
        theme.colors.slider_bar = palette.accent;
        theme.colors.slider_thumb = palette.text;
        theme.colors.scrollbar = gpui::transparent_black();
        theme.colors.scrollbar_thumb = palette.text.opacity(0.14);
        theme.colors.scrollbar_thumb_hover = palette.text.opacity(0.25);
        theme.radius = px(6.);
        theme.radius_lg = px(8.);
        theme.font_family = if self.doc.settings.theme == "terminal" {
            "Menlo".into()
        } else {
            self.doc.settings.font_family.clone().into()
        };
        theme.font_size = px(self.doc.settings.font_size);
        for (input, zh, en) in [
            (&self.search, "搜索任务", "Search tasks"),
            (
                &self.week_text,
                "写下想法… 支持 Markdown、粘贴或拖入图片",
                "Write Markdown, paste or drop images…",
            ),
        ] {
            input.update(cx, |input, cx| {
                input.set_placeholder(theme::tr(&self.doc.settings, zh, en), window, cx);
            });
        }
        if let Err(error) = crate::native::configure_window_material(window, &self.doc.settings) {
            self.error = Some(format!("{error:#}"));
        }
    }

    pub fn open_task(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_task_editor(id, false, window, cx);
    }

    fn activate_task(
        &mut self,
        id: &str,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers().alt {
            if self.save_editor(window, cx)
                && let Some(url) = self.doc.task(id).and_then(|task| task.links.first())
            {
                cx.open_url(url);
            }
        } else if event.is_keyboard() {
            self.open_task(id, window, cx);
        } else if self.doc.task(id).is_some() {
            let open = self
                .task_focus
                .click(id, event.modifiers().shift, event.click_count());
            cx.notify();
            if open {
                self.open_task(id, window, cx);
            }
        }
    }

    fn open_new_task(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_task_editor(id, true, window, cx);
    }

    fn open_task_editor(
        &mut self,
        id: &str,
        blank_title: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.save_editor(window, cx) {
            return;
        }
        if self.doc.task(id).is_some() {
            self.show_task_dialog(id.to_owned(), blank_title, cx);
        }
    }

    pub fn add_task(
        &mut self,
        q: usize,
        parent: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.save_editor(window, cx) {
            return;
        }
        let week = if self.page == Page::Calendar {
            model::week_start(self.day)
        } else {
            self.week
        };
        let schedule = if self.page == Page::Calendar {
            Some(self.day)
        } else if self.page == Page::Agenda {
            Some(if model::week_start(model::today()) == week {
                model::today()
            } else {
                week
            })
        } else {
            None
        };
        let title = self.t("未命名任务", "Untitled task").to_string();
        let tag = if self.page == Page::All {
            self.tag_filter.clone()
        } else {
            None
        };
        let mut id = String::new();
        self.change(cx, |doc| {
            id = doc.add_task(&title, week, q);
            if let Some(tag) = tag {
                doc.task_mut(&id).unwrap().tags.push(tag);
            }
            if let Some(parent) = parent {
                doc.reparent(&id, Some(&parent))?;
            } else if let Some(day) = schedule {
                doc.schedule(&id, day);
            }
            Ok(())
        });
        if self.error.is_none() {
            self.open_new_task(&id, window, cx);
        }
    }

    fn history(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.save_editor(window, cx) || !self.save_week_note(cx) {
            return;
        }
        let target = if redo {
            self.redo.last()
        } else {
            self.undo.last()
        }
        .cloned();
        if let Some(target) = target {
            if let Err(e) = self.store.save(&target) {
                self.error = Some(e.to_string());
            } else {
                if redo {
                    self.redo.pop();
                    self.undo.push(self.doc.clone());
                } else {
                    self.undo.pop();
                    self.redo.push(self.doc.clone());
                }
                let previous_settings = self.doc.settings.clone();
                self.task_focus.reconcile(&self.doc, &target);
                self.doc = target;
                self.sync_completion_preferences(&previous_settings);
                self.reconcile_reminders(cx);
                self.dismiss_task_dialog(window, cx);
                self.focus.focus(window);
                self.apply_theme(window, cx);
                self.load_week_note(window, cx);
                self.status = self.t("已恢复", "Restored").into();
            }
            cx.notify();
        }
    }

    fn navigate(&mut self, amount: i64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.save_week_note(cx) || !self.save_editor(window, cx) {
            return;
        }
        if self.page == Page::Calendar {
            let month = self.day.with_day(1).unwrap();
            self.day = if amount < 0 {
                (month - Duration::days(1)).with_day(1).unwrap()
            } else {
                (month + Duration::days(32)).with_day(1).unwrap()
            };
            self.week = model::week_start(self.day);
        } else {
            self.week += Duration::weeks(amount);
            self.day = self.week;
        }
        self.load_week_note(window, cx);
        cx.notify();
    }

    fn return_to_current_week(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.save_week_note(cx) && self.save_editor(window, cx) {
            self.day = model::today();
            self.week = model::week_start(self.day);
            self.load_week_note(window, cx);
            cx.notify();
        }
    }

    fn load_week_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_note_editor(window, cx);
    }

    pub fn import_file(&mut self, legacy: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.save_editor(window, cx) || !self.save_week_note(cx) {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                if legacy {
                    "选择旧版 SwiftData .store 文件"
                } else {
                    "选择 FourQuadrants JSON 备份"
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<Document>> = async {
                let paths = receiver.await??;
                let Some(path) = paths.and_then(|p| p.into_iter().next()) else {
                    return Ok(None);
                };
                cx.background_executor()
                    .spawn(async move {
                        if legacy {
                            crate::native::import_store(&path).map(Some)
                        } else {
                            store::import(&path).map(Some)
                        }
                    })
                    .await
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(Some(doc)) => {
                    if !this.save_editor(window, cx) || !this.save_week_note(cx) {
                        return;
                    }
                    let mut count = 0;
                    this.change(cx, |current| {
                        count = current.merge(doc)?;
                        Ok(())
                    });
                    if this.error.is_none() {
                        this.load_week_note(window, cx);
                        this.status = format!("已导入 {count} 个任务 / Imported {count} tasks");
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    this.error = Some(format!("导入失败 / Import failed: {e:#}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn export_file(&mut self, markdown: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.save_editor(window, cx) || !self.save_week_note(cx) {
            return;
        }
        let name = format!(
            "FourQuadrants-{}.{}",
            self.week,
            if markdown { "md" } else { "json" }
        );
        let dir = self
            .store
            .path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        let receiver = cx.prompt_for_new_path(&dir, Some(&name));
        let doc = self.doc.clone();
        let week = self.week;
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<bool> = async {
                let Some(path) = receiver.await?? else {
                    return Ok(false);
                };
                cx.background_executor()
                    .spawn(async move {
                        if markdown {
                            std::fs::write(path, crate::export::markdown(&doc, week))?;
                        } else {
                            store::export(&path, &doc)?;
                        }
                        Ok(true)
                    })
                    .await
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(true) => this.status = "导出成功 / Exported".into(),
                    Ok(false) => {}
                    Err(e) => this.error = Some(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn reconcile_reminders(&mut self, cx: &mut Context<Self>) {
        // Demo stores must never schedule or cancel the user's real reminders.
        if crate::store::default_data_path().ok().as_ref() != Some(&self.store.path) {
            return;
        }
        let path = self.store.path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::native::sync_reminders(&path) })
                .await;
            if let Err(error) = result {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("提醒同步失败 / Reminder sync failed: {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn sync_reminders(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if crate::store::default_data_path().ok().as_ref() != Some(&self.store.path) {
            self.status = "演示数据不触发系统通知 / Notifications disabled for demo data".into();
            cx.notify();
            return;
        }
        let path = self.store.path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::native::sync_reminders(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(msg) => this.status = msg,
                    Err(e) => this.error = Some(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn paste_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.parse_clipboard(true, window, cx);
    }

    fn check_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.parse_clipboard(false, window, cx);
    }

    fn parse_clipboard(&mut self, explicit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if text.trim().is_empty()
            || text.len() > 100_000
            || (!explicit && self.clipboard_text.as_ref() == Some(&text))
        {
            return;
        }
        self.clipboard_text = Some(text.clone());
        self.clipboard_generation = self.clipboard_generation.wrapping_add(1);
        let generation = self.clipboard_generation;
        let settings = self.doc.settings.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::native::parse_input(&settings, &text) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.clipboard_generation != generation {
                    return;
                }
                match result {
                    Ok(parsed) if explicit || parsed.issue_key.is_some() => {
                        this.clipboard_candidate = Some(parsed);
                    }
                    Ok(_) => {}
                    Err(error) if explicit => this.error = Some(error.to_string()),
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn create_parsed(&mut self, q: usize, cx: &mut Context<Self>) {
        let Some(parsed) = self.clipboard_candidate.clone() else {
            return;
        };
        let week = self.week;
        self.change(cx, |doc| {
            let title = if parsed.title.is_empty() {
                parsed
                    .issue_key
                    .clone()
                    .or_else(|| parsed.links.first().cloned())
                    .unwrap_or_else(|| "新任务 / New task".into())
            } else {
                parsed.title
            };
            let tag = parsed
                .tag_name
                .filter(|s| !s.trim().is_empty())
                .map(|name| {
                    if let Some(tag) = doc.tags.iter().find(|t| t.name == name) {
                        tag.id.clone()
                    } else {
                        let tag = model::Tag {
                            name,
                            sort_order: doc.tags.len() as f64,
                            ..Default::default()
                        };
                        let id = tag.id.clone();
                        doc.tags.push(tag);
                        id
                    }
                });
            let id = doc.add_task(&title, week, q);
            let task = doc.task_mut(&id).unwrap();
            task.issue_key = parsed.issue_key;
            task.links = parsed.links;
            if let Some(tag) = tag {
                task.tags.push(tag);
            }
            Ok(())
        });
        if self.error.is_none() {
            self.clipboard_candidate = None;
            self.clipboard_generation = self.clipboard_generation.wrapping_add(1);
        }
        cx.notify();
    }

    fn refresh_background(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = format!(
            "{:?}:{}",
            self.doc.settings.background_image, self.doc.settings.background_blur
        );
        if key == self.background_key {
            return;
        }
        self.background_key = key.clone();
        let Some(path) = self.doc.settings.background_image.clone() else {
            self.background_render = None;
            return;
        };
        if self.doc.settings.background_blur == 0. {
            self.background_render = Some(path.into());
            return;
        }
        let blur = self.doc.settings.background_blur;
        let output = self
            .store
            .path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("Backgrounds")
            .join(format!("blur-{}.png", uuid::Uuid::new_v4()));
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            if !this
                .update(cx, |this, _| this.background_key == key)
                .unwrap_or(false)
            {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(dir) = output.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    crate::native::prepare_background(std::path::Path::new(&path), &output, blur)?;
                    Ok::<_, anyhow::Error>(output)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.background_key == key {
                    match result {
                        Ok(path) => this.background_render = Some(path),
                        Err(e) => this.error = Some(e.to_string()),
                    }
                    cx.notify();
                } else if let Ok(path) = result {
                    // This output belongs to an obsolete slider position, not a user image.
                    let _ = std::fs::remove_file(path);
                }
            });
        })
        .detach();
    }

    fn visible(&self, task: &Task, cx: &App) -> bool {
        if self.doc.settings.hide_completed && self.grouped_as_completed(task) {
            return false;
        }
        if let Some(tag) = &self.tag_filter
            && !matches_tag_week(task, tag, self.week)
        {
            return false;
        }
        let query = self.search.read(cx).value().to_lowercase();
        query.is_empty()
            || format!(
                "{} {} {} {} {}",
                task.title,
                task.notes,
                task.issue_key.as_deref().unwrap_or(""),
                task.links.join(" "),
                self.doc
                    .tags
                    .iter()
                    .filter(|tag| task.tags.contains(&tag.id))
                    .map(|tag| tag.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .to_lowercase()
            .contains(&query)
    }

    fn task_link_active(&self, task: &Task, window: &Window, cx: &App) -> bool {
        window.modifiers().alt
            && self
                .editor
                .as_ref()
                .filter(|editor| editor.id == task.id)
                .map_or_else(|| !task.links.is_empty(), |editor| editor.has_link(cx))
    }

    fn tagged_title(&self, task: &Task, window: &Window, cx: &App) -> StyledText {
        let mut text = String::new();
        let mut highlights = Vec::new();
        for tag_id in &task.tags {
            if let Some(tag) = self.doc.tags.iter().find(|tag| &tag.id == tag_id) {
                let start = text.len();
                text.push('#');
                text.push_str(&tag.name);
                highlights.push((
                    start..text.len(),
                    HighlightStyle {
                        color: Some(Hsla::from(rgb(u32::from_str_radix(
                            tag.color_hex.trim_start_matches('#'),
                            16,
                        )
                        .unwrap_or(0x888888)))),
                        ..Default::default()
                    },
                ));
                text.push(' ');
            }
        }
        let title_start = text.len();
        text.push_str(&task.title);
        let link_active = self.task_link_active(task, window, cx);
        if title_start < text.len() && (task.completed || link_active) {
            highlights.push((
                title_start..text.len(),
                HighlightStyle {
                    color: link_active.then(|| self.palette(window).accent),
                    underline: link_active.then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    }),
                    strikethrough: task.completed.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                    ..Default::default()
                },
            ));
        }
        StyledText::new(text).with_highlights(highlights)
    }

    fn task_title(&self, task: &Task, window: &Window, cx: &App) -> StyledText {
        let mut text = String::new();
        let mut highlights = Vec::new();
        for tag_id in &task.tags {
            if let Some(tag) = self.doc.tags.iter().find(|tag| &tag.id == tag_id) {
                let start = text.len();
                text.push_str(&tag.name);
                highlights.push((
                    start..text.len(),
                    HighlightStyle {
                        color: Some(Hsla::from(rgb(u32::from_str_radix(
                            tag.color_hex.trim_start_matches('#'),
                            16,
                        )
                        .unwrap_or(0x888888)))),
                        ..Default::default()
                    },
                ));
                text.push(' ');
            }
        }
        let title_start = text.len();
        text.push_str(&task.title);
        let link_active = self.task_link_active(task, window, cx);
        if title_start < text.len() && (task.completed || link_active) {
            highlights.push((
                title_start..text.len(),
                HighlightStyle {
                    color: link_active.then(|| self.palette(window).accent),
                    underline: link_active.then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    }),
                    strikethrough: task.completed.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                    ..Default::default()
                },
            ));
        }
        StyledText::new(text).with_highlights(highlights)
    }

    fn completion_metadata(&self, task: &Task, p: Palette, window: &Window) -> Option<Div> {
        task.completed
            .then_some(task.completed_at)
            .flatten()
            .map(|date| {
                metadata_item(
                    window,
                    IconName::Check,
                    format!(
                        "{} {}",
                        self.t("完成于", "Completed"),
                        date.with_timezone(&chrono::Local).format("%Y/%m/%d %H:%M")
                    ),
                )
                .text_color(p.muted)
            })
    }

    fn task_metadata(&self, task: &Task, p: Palette, window: &Window) -> Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(p.muted)
            .when_some(task.issue_key.clone(), |el, issue| {
                el.child(div().text_color(p.muted).child(issue))
            })
            .when_some(task.due_date, |el, date| {
                let local = date.with_timezone(&chrono::Local);
                let today = model::today();
                let due_day = local.date_naive();
                let is_near_due =
                    !task.completed && date <= chrono::Utc::now() + Duration::hours(24);
                let due_label = match due_day.signed_duration_since(today).num_days() {
                    0 => format!("{} {}", self.t("今天", "Today"), local.format("%H:%M")),
                    1 => format!("{} {}", self.t("明天", "Tomorrow"), local.format("%H:%M")),
                    _ => local.format("%m/%d %H:%M").to_string(),
                };
                el.child(
                    metadata_item(window, IconName::Calendar, due_label)
                        .text_color(if is_near_due { p.quadrant[0] } else { p.muted }),
                )
            })
            .when(task.remind_at.is_some(), |el| {
                el.child(metadata_item(
                    window,
                    IconName::Bell,
                    self.t("提醒", "Reminder"),
                ))
            })
            .children(self.completion_metadata(task, p, window))
    }

    fn can_drop_before(&self, source: &str, target: &str) -> bool {
        if self.doc.validate_reorder(source, target).is_err() {
            return false;
        }
        // The board pins completed roots below open roots. Do not promise a
        // precise insertion across that boundary when the sort would override it.
        let target = self.doc.task(target).expect("validated target");
        self.page != Page::Board
            || target.parent_id.is_some()
            || self.doc.task(source).is_some_and(|task| {
                self.grouped_as_completed(task) == self.grouped_as_completed(target)
            })
    }

    fn row(
        &self,
        task: &Task,
        depth: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette(window);
        let id = task.id.clone();
        let toggle = id.clone();
        let open = id.clone();
        let fold = id.clone();
        let reorder = id.clone();
        let target = id.clone();
        let workspace = cx.entity().downgrade();
        let drop_workspace = workspace.clone();
        let direct_children = self.doc.children(&id);
        let child_count = direct_children.len();
        let completed_children = direct_children
            .iter()
            .filter(|child| child.completed)
            .count();
        let children = direct_children
            .into_iter()
            .filter(|t| self.visible(t, cx))
            .cloned()
            .collect::<Vec<_>>();
        let is_collapsed = self.collapsed.contains(&id);
        let show_children =
            !is_collapsed && !self.grouped_as_completed(task) && !children.is_empty();
        let branch_y = px(8.) + window.rem_size() * (0.875 * 1.45 / 2.);
        let child_indent = px(TASK_CHECKBOX_SIZE + TASK_TITLE_GAP);
        let branch_width = child_indent - px(TASK_CHECKBOX_SIZE / 2. + TASK_CONNECTOR_GAP);
        let stem_top = branch_y + px(TASK_CHECKBOX_SIZE / 2. + TASK_CONNECTOR_GAP);
        let branch_color = p.muted.opacity(0.22);
        let fold_hint = if is_collapsed {
            self.t("展开子任务", "Expand subtasks")
        } else {
            self.t("收起子任务", "Collapse subtasks")
        };
        let children_hint = if self.doc.settings.language == "en" {
            format!("{fold_hint} · {completed_children} of {child_count} completed")
        } else {
            format!("{fold_hint} · 已完成 {completed_children} / {child_count}")
        };
        let metadata = self.task_metadata(task, p, window);
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .flex_shrink_0()
            .when(
                cx.has_active_drag() && self.dragging_task.as_deref() == Some(&id),
                |el| el.opacity(0.4),
            )
            .child(
                div()
                    .relative()
                    .pb(px(3.))
                    .child(
                        div()
                            .id(SharedString::from(format!("task-{id}")))
                            .flex()
                            .min_w_0()
                            .items_start()
                            .gap(px(TASK_TITLE_GAP))
                            .px(px(6.))
                            .py(px(6.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(transparent_black())
                            .cursor_pointer()
                            .map(|row| self.task_focus.style(row, &id, p))
                            .border_t_2()
                            .can_drop(move |value, _, cx| {
                                value.downcast_ref::<DragTask>().is_some_and(|drag| {
                                    drop_workspace.upgrade().is_some_and(|workspace| {
                                        let this = workspace.read(cx);
                                        this.can_drop_before(&drag.id, &target)
                                    })
                                })
                            })
                            .drag_over::<DragTask>(move |style, _, _, _| {
                                style.bg(p.accent.opacity(0.10)).border_color(p.accent)
                            })
                            .on_click(cx.listener(move |this, event, window, cx| {
                                this.activate_task(&open, event, window, cx);
                            }))
                            .on_drag(
                                DragTask {
                                    id: id.clone(),
                                    title: task.title.clone(),
                                    color: p.quadrant[task.quadrant()],
                                    palette: p,
                                    hint: self
                                        .t("拖动任务 · Esc 取消", "Moving task · Esc to cancel"),
                                },
                                move |drag, offset, window, cx| {
                                    start_task_drag(&workspace, drag, offset, window, cx)
                                },
                            )
                            .on_drop(cx.listener(move |this, drag: &DragTask, _, cx| {
                                cx.stop_propagation();
                                let source = drag.id.clone();
                                if this.can_drop_before(&source, &reorder) {
                                    this.change(cx, |doc| doc.reorder(&source, &reorder));
                                }
                            }))
                            .child(
                                task_checkbox(
                                    SharedString::from(format!("complete-{id}")),
                                    task.completed,
                                    p,
                                )
                                .on_click(cx.listener(
                                    move |this, event, window, cx| {
                                        cx.stop_propagation();
                                        this.toggle_task_complete(&toggle, event, window, cx);
                                    },
                                )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .whitespace_normal()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .line_height(relative(1.45))
                                            .text_color(if task.completed {
                                                p.muted
                                            } else {
                                                p.text
                                            })
                                            .child(self.task_title(task, window, cx)),
                                    )
                                    .when(
                                        task.issue_key.is_some()
                                            || task.due_date.is_some()
                                            || task.remind_at.is_some()
                                            || (task.completed && task.completed_at.is_some()),
                                        |el| el.child(metadata.w_full().gap_x_2().gap_y(px(2.))),
                                    )
                                    .when_some(task.latest_progress_note(), |el, note| {
                                        el.child(
                                            div()
                                                .flex()
                                                .min_w_0()
                                                .items_center()
                                                .gap(px(5.))
                                                .text_xs()
                                                .text_color(p.muted)
                                                .child(glyph(IconName::File).with_size(px(12.)))
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .overflow_hidden()
                                                        .whitespace_nowrap()
                                                        .text_ellipsis()
                                                        .child(note.text.clone()),
                                                ),
                                        )
                                    }),
                            )
                            .when(child_count > 0, |el| {
                                el.child(
                                    div()
                                        .id(SharedString::from(format!("fold-{id}")))
                                        .flex_shrink_0()
                                        .min_h(px(22.))
                                        .px(px(3.))
                                        .flex()
                                        .items_center()
                                        .gap(px(3.))
                                        .rounded(px(4.))
                                        .text_xs()
                                        .text_color(p.muted)
                                        .hover(move |s| s.bg(p.hover).text_color(p.text))
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(children_hint.clone()).build(window, cx)
                                        })
                                        .child(format!("{completed_children}/{child_count}"))
                                        .child(
                                            glyph(if is_collapsed {
                                                IconName::ChevronRight
                                            } else {
                                                IconName::ChevronDown
                                            })
                                            .with_size(px(12.)),
                                        )
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if !this.collapsed.remove(&fold) {
                                                this.collapsed.insert(fold.clone());
                                            }
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .when(show_children, |el| {
                        el.child(task_connector(
                            TaskConnector::Stem { top: stem_top },
                            branch_color,
                        ))
                    }),
            )
            .when(show_children, |el| {
                el.child(
                    div()
                        .min_w_0()
                        .pb(px(if depth == 0 { 2. } else { 0. }))
                        .flex()
                        .flex_col()
                        .children(children.iter().enumerate().map(|(index, child)| {
                            // Align the child checkbox with the parent's title, leaving a gap at each box.
                            div()
                                .relative()
                                .min_w_0()
                                .flex_shrink_0()
                                .pl(child_indent)
                                .child(task_connector(
                                    TaskConnector::Branch {
                                        y: branch_y,
                                        width: branch_width,
                                        continues: index + 1 < children.len(),
                                    },
                                    branch_color,
                                ))
                                .child(self.row(child, depth + 1, window, cx))
                        })),
                )
            })
            .into_any_element()
    }

    fn sidebar(&self, collapsed: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette(window);
        let nav = [
            (
                Page::Board,
                Icon::new(AppIcon::Overview),
                "总览",
                "Overview",
            ),
            (Page::Agenda, Icon::new(AppIcon::ThisWeek), "周历", "Weekly"),
            (
                Page::Calendar,
                Icon::new(AppIcon::Calendar),
                "月历",
                "Monthly",
            ),
            (Page::Notes, Icon::new(AppIcon::Notes), "笔记", "Notes"),
        ];
        let sidebar = div()
            .relative()
            .w(px(if collapsed { SIDEBAR_RAIL_WIDTH } else { 204. }))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .when(!collapsed, |el| el.border_r_1().border_color(p.border))
            .px(px(if collapsed {
                (SIDEBAR_RAIL_WIDTH - SIDEBAR_RAIL_BUTTON_SIZE) / 2.
            } else {
                12.
            }))
            .pb_3()
            .gap(px(3.))
            .when(collapsed, |el| {
                el.child(
                    div()
                        .absolute()
                        .right_0()
                        .top(self.titlebar_height())
                        .bottom_0()
                        .w(px(1.))
                        .bg(p.border),
                )
            })
            .child(
                div()
                    .h(self.titlebar_height())
                    .flex_shrink_0()
                    .window_control_area(WindowControlArea::Drag),
            )
            .when(collapsed, |el| {
                el.child(
                    sidebar_button(
                        "expand-sidebar",
                        self.t("展开侧边栏 · ⌘B", "Expand sidebar · ⌘B"),
                        false,
                        p,
                    )
                    .mb_2()
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx))),
                )
            })
            .when(!collapsed, |el| {
                el.child(
                    div()
                        .h(px(40.))
                        .flex_shrink_0()
                        .px_2()
                        .flex()
                        .gap(px(10.))
                        .items_center()
                        .child(app_logo(p))
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::BOLD)
                                .child(self.t("肆", "Four Quadrants").to_string()),
                        ),
                )
                .child(
                    div()
                        .px_2()
                        .pt_3()
                        .pb_2()
                        .text_size(px(10.))
                        .text_color(p.muted)
                        .child(self.t("工作空间", "WORKSPACE").to_string()),
                )
            });
        let mut navigation = div()
            .id(if collapsed {
                "sidebar-rail-nav"
            } else {
                "sidebar-nav"
            })
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(3.))
            .overflow_y_scroll();
        for (page, icon, zh, en) in nav {
            let active = self.page == page && self.tag_filter.is_none();
            let label = self.t(zh, en).to_owned();
            navigation = navigation.child(
                div()
                    .id(SharedString::from(format!("nav-{en}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .h(px(if collapsed {
                        SIDEBAR_RAIL_BUTTON_SIZE
                    } else {
                        34.
                    }))
                    .flex_shrink_0()
                    .when(collapsed, |el| el.px_0().justify_center())
                    .rounded(px(6.))
                    .text_sm()
                    .cursor_pointer()
                    .bg(if active {
                        p.selected
                    } else {
                        gpui::transparent_black()
                    })
                    .text_color(if active { p.text } else { p.muted })
                    .hover(move |s| s.bg(p.hover).text_color(p.text))
                    .child(
                        div()
                            .text_color(if active { p.accent } else { p.muted })
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(glyph(icon).with_size(px(if collapsed { 18. } else { 15. }))),
                    )
                    .when(!collapsed, |el| el.child(self.t(zh, en).to_string()))
                    .when(collapsed, |el| {
                        el.tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.save_editor(window, cx) {
                            return;
                        }
                        this.page = page;
                        this.tag_filter = None;
                        this.sidebar_overlay = false;
                        cx.notify();
                    })),
            );
        }
        navigation = navigation.child(
            div()
                .flex_shrink_0()
                .when(collapsed, |el| el.h(px(16.)))
                .when(!collapsed, |el| {
                    el.mt_6()
                        .mb_2()
                        .px_3()
                        .text_xs()
                        .text_color(p.muted)
                        .child(self.t("标签", "TAGS").to_string())
                }),
        );
        let mut tags = self.doc.tags.clone();
        tags.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order));
        for tag in tags {
            let id = tag.id.clone();
            let count = self
                .doc
                .tasks
                .iter()
                .filter(|task| task.parent_id.is_none() && matches_tag_week(task, &id, self.week))
                .count();
            let active = self.tag_filter.as_ref() == Some(&id);
            let label = format!("{} · {count}", tag.name);
            navigation = navigation.child(
                div()
                    .id(SharedString::from(format!("tag-{}", tag.id)))
                    .flex()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .flex_shrink_0()
                    .when(collapsed, |el| {
                        el.h(px(SIDEBAR_RAIL_BUTTON_SIZE)).px_0().justify_center()
                    })
                    .rounded_md()
                    .cursor_pointer()
                    .text_sm()
                    .bg(if active {
                        p.selected
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(move |s| s.bg(p.selected))
                    .items_center()
                    .child(
                        div()
                            .size(px(if collapsed { 8. } else { 6. }))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(Hsla::from(rgb(u32::from_str_radix(
                                tag.color_hex.trim_start_matches('#'),
                                16,
                            )
                            .unwrap_or(0x888888)))),
                    )
                    .when(!collapsed, |el| {
                        el.child(tag.name)
                            .child(div().flex_1())
                            .child(div().text_xs().text_color(p.muted).child(count.to_string()))
                    })
                    .when(collapsed, |el| {
                        el.tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.save_editor(window, cx) {
                            return;
                        }
                        this.tag_filter = Some(id.clone());
                        this.page = Page::All;
                        this.sidebar_overlay = false;
                        cx.notify();
                    })),
            );
        }
        sidebar
            .child(navigation)
            .child(
                div()
                    .id("settings")
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(if collapsed {
                        SIDEBAR_RAIL_BUTTON_SIZE
                    } else {
                        40.
                    }))
                    .px_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.hover))
                    .when(collapsed, |el| el.px_0().justify_center())
                    .bg(if self.page == Page::Settings {
                        p.selected
                    } else {
                        transparent_black()
                    })
                    .flex_shrink_0()
                    .text_size(px(13.))
                    .text_color(if self.page == Page::Settings {
                        p.accent
                    } else {
                        p.muted
                    })
                    .child(glyph(IconName::Settings2).with_size(px(if collapsed {
                        18.
                    } else {
                        15.
                    })))
                    .when(!collapsed, |el| {
                        el.child(self.t("设置", "Settings").to_string())
                            .child(div().flex_1())
                    })
                    .when(collapsed, |el| {
                        let label = self.t("设置 · ⌘,", "Settings · ⌘,").to_owned();
                        el.tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        if !this.save_editor(window, cx) {
                            return;
                        }
                        this.page = Page::Settings;
                        this.tag_filter = None;
                        this.sidebar_overlay = false;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn task_actions(&self, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .child(
                titlebar_button(
                    "hide-completed",
                    if self.doc.settings.hide_completed {
                        IconName::EyeOff
                    } else {
                        IconName::Eye
                    },
                    16.,
                    if self.doc.settings.hide_completed {
                        self.t("显示已完成", "Show completed")
                    } else {
                        self.t("隐藏已完成", "Hide completed")
                    },
                    p,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.change(cx, |doc| {
                        doc.settings.hide_completed = !doc.settings.hide_completed;
                        Ok(())
                    })
                })),
            )
            .when(matches!(self.page, Page::Board | Page::Agenda), |el| {
                el.child(
                    titlebar_button(
                        "carry",
                        CarryForward,
                        17.,
                        self.t("结转未完成", "Carry forward"),
                        p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.carry_selection = Some(
                            this.doc
                                .tasks
                                .iter()
                                .filter(|task| {
                                    task.parent_id.is_none()
                                        && !task.completed
                                        && task.week_start < this.week
                                })
                                .map(|task| task.id.clone())
                                .collect(),
                        );
                        cx.notify();
                    })),
                )
                .child(
                    titlebar_button(
                        "export",
                        AppIcon::Share,
                        18.,
                        self.t("导出 Markdown", "Export Markdown"),
                        p,
                    )
                    .on_click(
                        cx.listener(|this, _, window, cx| this.export_file(true, window, cx)),
                    ),
                )
            })
            .into_any_element()
    }

    fn completion_summary(&self, bar_width: Option<Pixels>, p: Palette) -> Div {
        let (completed, total) = week_completion(&self.doc, self.week)
            .into_iter()
            .fold((0, 0), |(done, total), (q_done, q_total)| {
                (done + q_done, total + q_total)
            });
        let ratio = if total == 0 {
            0.
        } else {
            completed as f32 / total as f32
        };
        let label = self.t("完成进度", "Progress");
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(6.))
            .text_sm()
            .child(div().text_color(p.muted).child(label.to_string()))
            .when_some(bar_width, |el, width| {
                el.child(
                    div()
                        .w(width)
                        .h(px(5.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(p.accent.opacity(0.16))
                        .child(
                            div()
                                .h_full()
                                .w(relative(ratio))
                                .rounded_full()
                                .bg(p.accent),
                        ),
                )
            })
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text)
                    .child(format!("{completed}/{total}")),
            )
    }

    fn toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette(window);
        let narrow = Self::narrow_window(window);
        let sidebar_collapsed = narrow || self.doc.settings.sidebar_collapsed;
        let available_width = window.viewport_size().width
            - px(if sidebar_collapsed {
                SIDEBAR_RAIL_WIDTH
            } else {
                204.
            });
        let compact = available_width < px(700.);
        let small_toolbar = available_width < px(280.);
        let active_tag = self
            .tag_filter
            .as_ref()
            .filter(|_| self.page == Page::All)
            .and_then(|id| self.doc.tags.iter().find(|tag| &tag.id == id));
        let heading = active_tag.map_or_else(
            || match self.page {
                Page::Board => self.t("总览", "Overview"),
                Page::Agenda => self.t("周历", "Weekly"),
                Page::Calendar => self.t("月历", "Monthly"),
                Page::All => self.t("所有任务", "All tasks"),
                Page::Notes => self.t("笔记", "Notes"),
                Page::Settings => self.t("设置", "Settings"),
            },
            |tag| tag.name.as_str(),
        );
        let section = if active_tag.is_some() {
            self.t("标签", "Tags")
        } else {
            self.t("工作空间", "Workspace")
        };
        let current_week = model::week_start(model::today());
        let (week_title, week_start_label, week_end_label) =
            week_labels(self.week, self.doc.settings.language == "en");
        let week_title_size = self.doc.settings.font_size;
        let mut week_range_size = week_title_size * 0.875;
        let text_width = |text: &str, font_size: f32, weight: FontWeight| {
            let mut style = window.text_style();
            style.font_family = if self.doc.settings.theme == "terminal" {
                "Menlo".into()
            } else {
                self.doc.settings.font_family.clone().into()
            };
            style.font_weight = weight;
            f32::from(
                window
                    .text_system()
                    .shape_line(
                        text.to_owned().into(),
                        px(font_size),
                        &[style.to_run(text.len())],
                        None,
                    )
                    .width,
            )
        };
        let rem = f32::from(window.rem_size());
        let week_year = self.week.iso_week().year();
        let week_title_width = text_width(
            &if self.doc.settings.language == "en" {
                format!("{week_year} · Week 53")
            } else {
                format!("{week_year} 年第 53 周")
            },
            week_title_size,
            FontWeight::SEMIBOLD,
        );
        let max_date_label = if self.doc.settings.language == "en" {
            "Sep 30"
        } else {
            "12月31日"
        };
        let week_range_width = loop {
            let range_width = text_width(max_date_label, week_range_size, FontWeight::NORMAL) * 2.
                + text_width("-", week_range_size, FontWeight::NORMAL)
                + 8.;
            if range_width <= week_title_width || week_range_size <= 10. {
                break range_width;
            }
            week_range_size -= 0.5;
        };
        let progress_selector_width = week_title_width.max(week_range_width);
        let progress_bar_width = if self.page == Page::Board {
            let small = rem * 0.875;
            let selector = 80. + progress_selector_width;
            let return_button = if self.week != current_week {
                text_width(self.t("本周", "This week"), small, FontWeight::NORMAL) + rem * 2. + 13.
            } else {
                0.
            };
            let (done, total) = week_completion(&self.doc, self.week)
                .into_iter()
                .fold((0, 0), |(done, total), (q_done, q_total)| {
                    (done + q_done, total + q_total)
                });
            let summary_text =
                text_width(self.t("完成进度", "Progress"), small, FontWeight::NORMAL)
                    + text_width(&format!("{done}/{total}"), small, FontWeight::MEDIUM);
            // Reserve the actual text and controls; let the bar shrink before hiding it.
            let room = f32::from(available_width)
                - selector
                - return_button
                - summary_text
                - rem * 2.25
                - 12.
                - 8.;
            (room >= 40.).then(|| px(room.min(140.)))
        } else {
            None
        };
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .child(
                div()
                    .flex()
                    .h(self.titlebar_height())
                    .px_3()
                    // Keep the heading past the native controls as the rail narrows.
                    .when(sidebar_collapsed, |el| el.pl(px(88. - SIDEBAR_RAIL_WIDTH)))
                    .items_center()
                    .gap(px(6.))
                    .when(!sidebar_collapsed, |el| {
                        el.child(
                            sidebar_button(
                                "toggle-sidebar",
                                self.t("收起侧边栏 · ⌘B", "Collapse sidebar · ⌘B"),
                                true,
                                p,
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx)),
                            ),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .h_full()
                            .items_center()
                            .gap_3()
                            .when(small_toolbar, |el| el.hidden())
                            .window_control_area(WindowControlArea::Drag)
                            .when(
                                !sidebar_collapsed && (!compact || active_tag.is_some()),
                                |el| {
                                    el.child(
                                        div()
                                            .text_sm()
                                            .text_color(p.muted)
                                            .child(section.to_string()),
                                    )
                                    .child(div().text_color(p.border).child("/"))
                                },
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(heading.to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .window_control_area(WindowControlArea::Drag),
                    )
                    .when(
                        matches!(
                            self.page,
                            Page::Board | Page::Agenda | Page::Calendar | Page::All
                        ),
                        |el| el.child(self.task_actions(p, cx)),
                    )
                    .when(
                        matches!(
                            self.page,
                            Page::Board | Page::Agenda | Page::Calendar | Page::All
                        ),
                        |el| {
                            el.child(div().w(px(1.)).h(px(14.)).flex_shrink_0().bg(p.border))
                                .child(
                                    titlebar_button(
                                        "new-task",
                                        IconName::Plus,
                                        13.,
                                        self.t("新建任务 · ⌘N", "New task · ⌘N"),
                                        Palette {
                                            text: p.accent_text,
                                            muted: p.accent_text,
                                            hover: p.accent.opacity(0.85),
                                            ..p
                                        },
                                    )
                                    .bg(p.accent)
                                    .on_click(cx.listener(
                                        |this, _, window, cx| this.add_task(0, None, window, cx),
                                    )),
                                )
                        },
                    )
                    .when(!sidebar_collapsed, |el| {
                        el.border_b_1().border_color(p.border)
                    }),
            )
            .when(self.search_open, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_color(p.border)
                        .child(
                            div().flex_1().min_w_0().child(
                                text_input(&self.search)
                                    .prefix(glyph(IconName::Search))
                                    .cleanable(true),
                            ),
                        )
                        .child(
                            icon_button(
                                "close-search",
                                IconName::Close,
                                self.t("关闭搜索 · Esc", "Close search · Esc"),
                                p,
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_search(window, cx)),
                            ),
                        ),
                )
            })
            .when(
                !matches!(self.page, Page::Settings | Page::Calendar),
                |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .min_h(px(64.))
                            .when(self.page == Page::Notes, |el| {
                                el.border_b_1().border_color(p.border)
                            })
                            .px_4()
                            .py_1()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .flex_shrink_0()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        icon_button(
                                            "previous",
                                            IconName::ChevronLeft,
                                            self.t("上一周", "Previous week"),
                                            p,
                                        )
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.navigate(-1, window, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .w(px(progress_selector_width))
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.))
                                            .child(
                                                div()
                                                    .w_full()
                                                    .text_center()
                                                    .text_size(px(week_title_size))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(week_title),
                                            )
                                            .child(
                                                div()
                                                    .w_full()
                                                    .flex()
                                                    .items_center()
                                                    .text_size(px(week_range_size))
                                                    .text_color(p.muted)
                                                    .child(week_start_label)
                                                    .child(div().flex_1().text_center().child("-"))
                                                    .child(week_end_label),
                                            ),
                                    )
                                    .child(
                                        icon_button(
                                            "next",
                                            IconName::ChevronRight,
                                            self.t("下一周", "Next week"),
                                            p,
                                        )
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.navigate(1, window, cx)
                                            }),
                                        ),
                                    ),
                            )
                            .when(self.week != current_week, |el| {
                                el.child(
                                    chip("today", "", p)
                                        .flex_shrink_0()
                                        .whitespace_nowrap()
                                        .child(glyph(IconName::Undo2).with_size(px(13.)))
                                        .child(self.t("本周", "This week").to_string())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.return_to_current_week(window, cx)
                                        })),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .justify_end()
                                    .items_center()
                                    .gap_3()
                                    .when(self.page == Page::Board, |el| {
                                        el.child(self.completion_summary(progress_bar_width, p))
                                    })
                                    .when(self.page == Page::Notes, |el| {
                                        el.child(self.note_controls(window, cx))
                                    }),
                            ),
                    )
                },
            )
            .into_any_element()
    }

    fn board(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let workspace = cx.entity().downgrade();
        canvas(
            move |bounds, window, cx| {
                // Measure the remaining main area after the sidebar and editor are laid out.
                let mut board = workspace
                    .update(cx, |this, cx| this.board_layout(bounds.size, window, cx))
                    .ok()?;
                board.layout_as_root(bounds.size.map(AvailableSpace::Definite), window, cx);
                board.prepaint_at(bounds.origin, window, cx);
                Some(board)
            },
            |_, board, window, cx| {
                if let Some(mut board) = board {
                    board.paint(window, cx);
                }
            },
        )
        .flex_1()
        .min_w_0()
        .min_h_0()
        .into_any_element()
    }

    fn board_layout(
        &self,
        size: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let stacked = size.width < px(840.);
        let mut board = div()
            .id(if stacked {
                "board-stacked"
            } else {
                "board-grid"
            })
            .w(size.width)
            .h(size.height)
            .flex()
            .flex_col()
            .gap_3()
            .p_4();
        if stacked {
            board = board.overflow_y_scroll().children(
                [0, 2, 1, 3]
                    .into_iter()
                    .map(|q| self.quadrant(q, true, window, cx)),
            );
        } else {
            for pair in [[0, 1], [2, 3]] {
                board = board.child(
                    div().flex().flex_1().min_h_0().gap_3().children(
                        pair.into_iter()
                            .map(|q| self.quadrant(q, false, window, cx)),
                    ),
                );
            }
        }
        board.into_any_element()
    }

    fn completed_heading(&self, count: usize, p: Palette) -> Div {
        div()
            .flex_shrink_0()
            .px_2()
            .pt_3()
            .pb_1()
            .text_xs()
            .text_color(p.muted)
            .child(format!("{} {count}", self.t("已完成", "Completed")))
    }

    fn quadrant(
        &self,
        q: usize,
        stacked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette(window);
        let en = self.doc.settings.language == "en";
        let mut tasks = self
            .doc
            .tasks
            .iter()
            .filter(|t| {
                t.parent_id.is_none()
                    && t.week_start == self.week
                    && t.quadrant() == q
                    && self.visible(t, cx)
            })
            .cloned()
            .collect::<Vec<_>>();
        tasks.sort_by(|a, b| {
            self.grouped_as_completed(a)
                .cmp(&self.grouped_as_completed(b))
                .then(a.sort_order.total_cmp(&b.sort_order))
        });
        let (done, total) = week_completion(&self.doc, self.week)[q];
        let completed = tasks
            .iter()
            .filter(|task| self.grouped_as_completed(task))
            .count();
        div()
            .id(("quadrant", q))
            .when(stacked, |el| el.w_full().min_h(px(200.)).flex_shrink_0())
            .when(!stacked, |el| el.flex_1().min_h_0())
            .min_w_0()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.subtle)
            .overflow_hidden()
            .drag_over::<DragTask>(move |style, _, _, _| {
                style.bg(p.accent.opacity(0.10)).border_color(p.accent)
            })
            .on_drop(cx.listener(move |this, drag: &DragTask, _, cx| {
                cx.stop_propagation();
                let week = this.week;
                let id = drag.id.clone();
                this.change(cx, |doc| {
                    doc.reparent(&id, None)?;
                    doc.move_task(&id, week, q);
                    Ok(())
                });
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .min_h(px(58.))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .w(px(3.))
                            .h(px(24.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(p.quadrant[q]),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(p.text)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(theme::quadrant_name(q, en)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted)
                                    .child(theme::quadrant_description(q, en)),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.quadrant[q])
                            .child(format!("{done}/{total}")),
                    ),
            )
            .child(
                div()
                    .id(("scroll", q))
                    .when(stacked, |el| el.flex_shrink_0())
                    .when(!stacked, |el| el.flex_1().min_h_0().overflow_y_scroll())
                    .min_w_0()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .when(tasks.is_empty(), |el| {
                        el.child(
                            div()
                                .py_8()
                                .text_center()
                                .text_sm()
                                .text_color(p.muted)
                                .child(
                                    self.t("把任务放在合适的位置", "Make room for what matters")
                                        .to_string(),
                                ),
                        )
                    })
                    .children(
                        tasks
                            .iter()
                            .filter(|task| !self.grouped_as_completed(task))
                            .map(|task| self.row(task, 0, window, cx)),
                    )
                    .when(completed > 0, |el| {
                        el.child(self.completed_heading(completed, p))
                    })
                    .children(
                        tasks
                            .iter()
                            .filter(|task| self.grouped_as_completed(task))
                            .map(|task| self.row(task, 0, window, cx)),
                    ),
            )
            .into_any_element()
    }

    fn inbox_tray(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette(window);
        let tasks = self
            .doc
            .tasks
            .iter()
            .filter(|t| {
                t.parent_id.is_none() && !self.grouped_as_completed(t) && t.due_date.is_none()
            })
            .cloned()
            .collect::<Vec<_>>();
        div()
            .min_h(px(78.))
            .max_h(px(160.))
            .flex_shrink_0()
            .border_t_1()
            .border_color(p.border)
            .px_5()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .gap_3()
                    .text_xs()
                    .text_color(p.muted)
                    .child(format!("{} · {}", self.t("收集箱", "Inbox"), tasks.len()))
                    .child(
                        self.t("拖入象限或日期安排任务", "Drag into a quadrant or a day")
                            .to_string(),
                    ),
            )
            .child(
                div()
                    .id("inbox-tray")
                    .min_h_0()
                    .flex()
                    .items_start()
                    .gap_2()
                    .overflow_scroll()
                    .children(tasks.iter().map(|task| {
                        let id = task.id.clone();
                        let workspace = cx.entity().downgrade();
                        div()
                            .id(SharedString::from(format!("inbox-{id}")))
                            .when(
                                cx.has_active_drag() && self.dragging_task.as_deref() == Some(&id),
                                |el| el.opacity(0.4),
                            )
                            .w(px(200.))
                            .flex_shrink_0()
                            .p(px(10.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(p.text.opacity(0.14))
                            .map(|row| self.task_focus.style(row, &id, p))
                            .cursor(if self.task_link_active(task, window, cx) {
                                CursorStyle::PointingHand
                            } else {
                                CursorStyle::OpenHand
                            })
                            .text_sm()
                            .line_height(relative(1.45))
                            .whitespace_normal()
                            .child(
                                div()
                                    .h(rems(0.875 * 1.45 * 2.))
                                    .line_clamp(2)
                                    .child(self.tagged_title(task, window, cx)),
                            )
                            .on_drag(
                                DragTask {
                                    id: id.clone(),
                                    title: task.title.clone(),
                                    color: p.quadrant[task.quadrant()],
                                    palette: p,
                                    hint: self
                                        .t("拖动任务 · Esc 取消", "Moving task · Esc to cancel"),
                                },
                                move |drag, offset, window, cx| {
                                    start_task_drag(&workspace, drag, offset, window, cx)
                                },
                            )
                            .on_click(cx.listener(move |this, event, window, cx| {
                                this.activate_task(&id, event, window, cx)
                            }))
                    })),
            )
            .into_any_element()
    }

    fn weekly_rows(&self, tasks: &[Task], window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.palette(window);
        let completed = tasks
            .iter()
            .filter(|task| self.grouped_as_completed(task))
            .count();
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .children(
                tasks
                    .iter()
                    .filter(|task| !self.grouped_as_completed(task))
                    .map(|task| self.flat_row(task, window, cx)),
            )
            .when(completed > 0, |el| {
                el.child(self.completed_heading(completed, p))
            })
            .children(
                tasks
                    .iter()
                    .filter(|task| self.grouped_as_completed(task))
                    .map(|task| self.flat_row(task, window, cx)),
            )
    }

    fn agenda(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette(window);
        let today = model::today();
        let group =
            |task: &Task| weekly_group(task, self.week, today, self.grouped_as_completed(task));
        let order = |a: &Task, b: &Task| {
            weekly_order(
                a,
                self.grouped_as_completed(a),
                b,
                self.grouped_as_completed(b),
            )
        };
        let mut body = div()
            .id("agenda-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p_5()
            .gap_3();
        let mut pinned = self
            .doc
            .tasks
            .iter()
            .filter(|task| {
                group(task).is_some()
                    && (self.task_focus.contains(&task.id) || self.confetti.holds_pin(task))
                    && self.visible(task, cx)
            })
            .cloned()
            .collect::<Vec<_>>();
        pinned.sort_by(order);
        if !pinned.is_empty() {
            body = body.child(
                div()
                    .id("agenda-pinned")
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap_2()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.subtle)
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .line_height(relative(1.))
                            .text_color(p.muted)
                            .child(
                                div()
                                    .flex_none()
                                    .relative()
                                    .top(px(-self.doc.settings.font_size * 0.08))
                                    .child(
                                        Icon::new(PinIcon)
                                            .with_size(px(self.doc.settings.font_size)),
                                    ),
                            )
                            .child("Pin"),
                    )
                    .child(self.weekly_rows(&pinned, window, cx)),
            );
        }
        let mut overdue = self
            .doc
            .tasks
            .iter()
            .filter(|t| group(t) == Some(WeeklyGroup::Overdue) && self.visible(t, cx))
            .cloned()
            .collect::<Vec<_>>();
        overdue.sort_by(order);
        if !overdue.is_empty() {
            body = body.child(
                div()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(p.quadrant[0].opacity(0.4))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_color(p.quadrant[0]).child(format!(
                        "{} · {}",
                        self.t("逾期", "Overdue"),
                        overdue.len()
                    )))
                    .children(overdue.iter().map(|t| self.flat_row(t, window, cx))),
            );
        }
        let mut unplanned = self
            .doc
            .tasks
            .iter()
            .filter(|task| group(task) == Some(WeeklyGroup::Unplanned) && self.visible(task, cx))
            .cloned()
            .collect::<Vec<_>>();
        unplanned.sort_by(order);
        if !unplanned.is_empty() {
            body =
                body.child(
                    div()
                        .id("agenda-unplanned")
                        .flex()
                        .flex_col()
                        .flex_shrink_0()
                        .gap_2()
                        .p_4()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.subtle)
                        .child(
                            div()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .gap_3()
                                .text_color(p.muted)
                                .child(format!(
                                    "{} · {}",
                                    self.t("未规划", "Unplanned"),
                                    unplanned.len()
                                )),
                        )
                        .child(div().min_w_0().flex().flex_col().gap_2().children(
                            unplanned.iter().map(|task| self.flat_row(task, window, cx)),
                        )),
                );
        }
        for offset in 0..7 {
            let date = self.week + Duration::days(offset);
            let mut tasks = self
                .doc
                .tasks
                .iter()
                .filter(|t| group(t) == Some(WeeklyGroup::Day(date)) && self.visible(t, cx))
                .cloned()
                .collect::<Vec<_>>();
            tasks.sort_by(order);
            // List descendants independently under their own scheduled day.
            body = body.child(
                div()
                    .id(("day", offset as usize))
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap_2()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if date == model::today() {
                        p.accent
                    } else {
                        p.border
                    })
                    .bg(p.subtle)
                    .when(date < model::today(), |el| el.opacity(0.5))
                    .drag_over::<DragTask>(move |style, _, _, _| {
                        style
                            .opacity(1.)
                            .bg(p.accent.opacity(0.10))
                            .border_color(p.accent)
                    })
                    .on_drop(cx.listener(move |this, drag: &DragTask, _, cx| {
                        cx.stop_propagation();
                        let id = drag.id.clone();
                        this.change(cx, |doc| {
                            doc.schedule(&id, date);
                            Ok(())
                        });
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_color(if date == model::today() {
                                p.accent
                            } else {
                                p.muted
                            })
                            .child(date.format("%m.%d").to_string())
                            .child(self.weekday(date)),
                    )
                    .child(
                        self.weekly_rows(&tasks, window, cx)
                            .when(tasks.is_empty(), |el| {
                                el.child(
                                    div().py_2().text_sm().text_color(p.muted).child(
                                        self.t(
                                            "拖入任务安排到这一天",
                                            "Drop a task to schedule it",
                                        )
                                        .to_string(),
                                    ),
                                )
                            }),
                    ),
            );
        }
        body.into_any_element()
    }

    fn flat_row(&self, task: &Task, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette(window);
        let id = task.id.clone();
        let toggle = id.clone();
        let open = id.clone();
        let workspace = cx.entity().downgrade();
        div()
            .id(SharedString::from(format!("flat-{id}")))
            .when(
                cx.has_active_drag() && self.dragging_task.as_deref() == Some(&id),
                |el| el.opacity(0.4),
            )
            .flex()
            .min_w_0()
            .flex_shrink_0()
            .gap(px(6.))
            .items_start()
            .px_2()
            .py_2()
            .rounded(px(4.))
            .map(|row| self.task_focus.style(row, &id, p))
            .cursor_pointer()
            .on_drag(
                DragTask {
                    id,
                    title: task.title.clone(),
                    color: p.quadrant[task.quadrant()],
                    palette: p,
                    hint: self.t("拖动任务 · Esc 取消", "Moving task · Esc to cancel"),
                },
                move |drag, offset, window, cx| {
                    start_task_drag(&workspace, drag, offset, window, cx)
                },
            )
            .on_click(cx.listener(move |this, event, window, cx| {
                this.activate_task(&open, event, window, cx);
            }))
            .child(
                task_checkbox(
                    SharedString::from(format!("flat-toggle-{toggle}")),
                    task.completed,
                    p,
                )
                .on_click(cx.listener(move |this, event, window, cx| {
                    cx.stop_propagation();
                    this.toggle_task_complete(&toggle, event, window, cx);
                })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_sm()
                            .line_height(relative(1.45))
                            .whitespace_normal()
                            .text_color(if task.completed { p.muted } else { p.text })
                            .child(self.tagged_title(task, window, cx)),
                    )
                    .children(self.completion_metadata(task, p, window)),
            )
            .child(
                div()
                    .size_2()
                    .mt(px(6.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(p.quadrant[task.quadrant()]),
            )
            .into_any_element()
    }

    fn weekday(&self, date: NaiveDate) -> String {
        let i = date.weekday().num_days_from_monday() as usize;
        if self.doc.settings.language == "en" {
            ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"][i]
        } else {
            ["周一", "周二", "周三", "周四", "周五", "周六", "周日"][i]
        }
        .into()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_background(window, cx);
        let p = self.palette(window);
        let narrow = Self::narrow_window(window);
        let body = match self.page {
            Page::Board => self.board(window, cx),
            Page::Agenda => self.agenda(window, cx),
            Page::Calendar => self.calendar(window, cx),
            Page::Notes => self.notes_view(window, cx),
            Page::Settings => self.settings_view(window, cx),
            Page::All => {
                let tasks = self
                    .doc
                    .tasks
                    .iter()
                    .filter(|t| self.visible(t, cx))
                    .cloned()
                    .collect::<Vec<_>>();
                // Search results include matching descendants independently.
                let mut list = div()
                    .id("all-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3();
                for task in tasks.iter().filter(|task| !self.grouped_as_completed(task)) {
                    list = list.child(self.flat_row(task, window, cx));
                }
                let completed = tasks
                    .iter()
                    .filter(|task| self.grouped_as_completed(task))
                    .count();
                if completed > 0 {
                    list = list.child(self.completed_heading(completed, p));
                    for task in tasks.iter().filter(|task| self.grouped_as_completed(task)) {
                        list = list.child(self.flat_row(task, window, cx));
                    }
                }
                if tasks.is_empty() {
                    list = list.child(
                        div().py_16().text_center().text_color(p.muted).child(
                            self.t(
                                "还没有任务，开始记录第一件事",
                                "Nothing here yet. Start with one task.",
                            )
                            .to_string(),
                        ),
                    );
                }
                list.into_any_element()
            }
        };
        let mut root = div()
            .relative()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.text)
            .font_family(if self.doc.settings.theme == "terminal" {
                "Menlo".into()
            } else {
                self.doc.settings.font_family.clone()
            })
            .text_size(px(self.doc.settings.font_size))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    if cx.stop_active_drag(window) {
                        this.dragging_task = None;
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    this.clipboard_generation = this.clipboard_generation.wrapping_add(1);
                    if this.clipboard_candidate.take().is_some()
                        || this.carry_selection.take().is_some()
                    {
                        cx.stop_propagation();
                        cx.notify();
                    } else if this.search_open {
                        this.close_search(window, cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .on_action(
                cx.listener(|this, _: &IncreaseFont, window, cx| this.resize_font(1., window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DecreaseFont, window, cx| this.resize_font(-1., window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ResetFont, window, cx| this.resize_font(0., window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NewTask, window, cx| this.add_task(0, None, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleSidebar, window, cx| this.toggle_sidebar(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Search, window, cx| {
                this.search_open = true;
                this.search.update(cx, |i, cx| i.focus(window, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Settings, _, cx| {
                this.page = Page::Settings;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.history(false, window, cx)))
            .on_action(cx.listener(|this, _: &Redo, window, cx| this.history(true, window, cx)))
            .on_action(cx.listener(|this, _: &Previous, window, cx| this.navigate(-1, window, cx)))
            .on_action(cx.listener(|this, _: &Next, window, cx| this.navigate(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &ThisWeek, window, cx| {
                    this.return_to_current_week(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ExportMarkdown, window, cx| {
                this.export_file(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClosePanel, window, cx| {
                if cx.stop_active_drag(window) {
                    this.dragging_task = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.search_open {
                    this.close_search(window, cx);
                    return;
                }
                if Self::narrow_window(window) && this.sidebar_overlay {
                    this.sidebar_overlay = false;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::Quit, window, cx| {
                if this.prepare_close(window, cx) {
                    cx.quit();
                } else {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleCompleted, _, cx| {
                this.change(cx, |doc| {
                    doc.settings.hide_completed = !doc.settings.hide_completed;
                    Ok(())
                })
            }))
            .on_action(cx.listener(|this, _: &ShowBoard, _, cx| {
                this.page = Page::Board;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowAgenda, _, cx| {
                this.page = Page::Agenda;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowCalendar, _, cx| {
                this.page = Page::Calendar;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowNotes, _, cx| {
                this.page = Page::Notes;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PasteTask, window, cx| this.paste_task(window, cx)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    // Extend the original sidebar tint across the whole workspace.
                    .bg(p.panel)
                    .child(self.sidebar(narrow || self.doc.settings.sidebar_collapsed, window, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(self.toolbar(window, cx))
                            .when_some(self.error.clone(), |el, error| {
                                el.child(
                                    div()
                                        .mx_4()
                                        .mt_3()
                                        .px_3()
                                        .py_2()
                                        .rounded(px(6.))
                                        .border_1()
                                        .border_color(p.quadrant[0].opacity(0.5))
                                        .bg(p.quadrant[0].opacity(0.10))
                                        .flex()
                                        .items_start()
                                        .gap_2()
                                        .text_sm()
                                        .text_color(p.text)
                                        .child(div().flex_1().min_w_0().child(format!("⚠ {error}")))
                                        .child(
                                            icon_button(
                                                "dismiss-error",
                                                IconName::Close,
                                                self.t("关闭提示", "Dismiss"),
                                                p,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.error = None;
                                                    cx.notify();
                                                }),
                                            ),
                                        ),
                                )
                            })
                            .child(body)
                            .when(
                                self.doc.settings.show_inbox
                                    && matches!(
                                        self.page,
                                        Page::Board | Page::Agenda | Page::Calendar
                                    ),
                                |el| el.child(self.inbox_tray(window, cx)),
                            ),
                    ),
            )
            .when(narrow || self.doc.settings.sidebar_collapsed, |el| {
                // One line spans the rail, main content and any open detail pane.
                el.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(self.titlebar_height() - px(1.))
                        .h(px(1.))
                        .bg(p.border),
                )
            });
        if narrow && self.sidebar_overlay {
            root =
                root.child(
                    div()
                        .id("sidebar-overlay")
                        .absolute()
                        .inset_0()
                        .flex()
                        .bg(black().opacity(0.3))
                        .occlude()
                        .child(
                            div()
                                .w(px(204.))
                                .flex_shrink_0()
                                .h_full()
                                .bg(Hsla { a: 1., ..p.bg })
                                .child(self.sidebar(false, window, cx)),
                        )
                        .child(div().id("dismiss-sidebar").flex_1().h_full().on_click(
                            cx.listener(|this, _, _, cx| {
                                this.sidebar_overlay = false;
                                cx.notify();
                            }),
                        )),
                );
        }
        if let Some(parsed) = &self.clipboard_candidate {
            let candidate = parsed.title.clone();
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::black().opacity(0.35))
                    .occlude()
                    .child(
                        div()
                            .w(px(570.))
                            .p_6()
                            .rounded_xl()
                            .bg(Hsla { a: 1., ..p.bg })
                            .border_1()
                            .border_color(p.border)
                            .shadow_lg()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div().text_lg().child(
                                    self.t("从剪贴板创建任务", "Create a task from clipboard")
                                        .to_string(),
                                ),
                            )
                            .child(
                                div()
                                    .text_color(p.accent)
                                    .child(parsed.issue_key.clone().unwrap_or_default()),
                            )
                            .child(candidate)
                            .child(div().flex().flex_wrap().gap_2().children((0..4).map(|q| {
                                chip(
                                    ("paste-q", q),
                                    theme::quadrant_name(q, self.doc.settings.language == "en"),
                                    p,
                                )
                                .bg(p.quadrant[q].opacity(0.15))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.create_parsed(q, cx)),
                                )
                            })))
                            .child(
                                chip("cancel-clipboard", self.t("取消", "Cancel").to_string(), p)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.clipboard_candidate = None;
                                        this.clipboard_generation =
                                            this.clipboard_generation.wrapping_add(1);
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        if let Some(selection) = &self.carry_selection {
            let tasks = self
                .doc
                .tasks
                .iter()
                .filter(|t| t.parent_id.is_none() && !t.completed && t.week_start < self.week)
                .cloned()
                .collect::<Vec<_>>();
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::black().opacity(0.35))
                    .occlude()
                    .child(
                        div()
                            .w(px(600.))
                            .max_h(px(600.))
                            .p_6()
                            .rounded_xl()
                            .bg(Hsla { a: 1., ..p.bg })
                            .border_1()
                            .border_color(p.border)
                            .shadow_lg()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div().text_lg().child(
                                    self.t("整理历史未完成任务", "Carry unfinished tasks forward")
                                        .to_string(),
                                ),
                            )
                            .child(div().text_sm().text_color(p.muted).child(format!(
                                "{} → {}",
                                self.t("目标周", "Target week"),
                                self.week
                            )))
                            .child(
                                div()
                                    .id("carry-scroll")
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .children(tasks.iter().map(|task| {
                                        let id = task.id.clone();
                                        let children = self.doc.children(&id);
                                        let action = if children.is_empty() {
                                            self.t("移动", "Move")
                                        } else if children.iter().any(|t| !t.completed) {
                                            self.t(
                                                "复制父项并移动未完成子项",
                                                "Copy parent, move unfinished children",
                                            )
                                        } else {
                                            self.t("标记完成", "Mark completed")
                                        };
                                        chip(
                                            SharedString::from(format!("carry-{id}")),
                                            format!(
                                                "{} {} · {}",
                                                if selection.contains(&id) {
                                                    "✓"
                                                } else {
                                                    "○"
                                                },
                                                task.title,
                                                action
                                            ),
                                            p,
                                        )
                                        .justify_start()
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                if let Some(selection) = &mut this.carry_selection
                                                    && !selection.remove(&id)
                                                {
                                                    selection.insert(id.clone());
                                                }
                                                cx.notify();
                                            }),
                                        )
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .child(
                                        chip(
                                            "cancel-carry",
                                            self.t("取消", "Cancel").to_string(),
                                            p,
                                        )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.carry_selection = None;
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        chip(
                                            "commit-carry",
                                            format!(
                                                "{} ({})",
                                                self.t("结转所选", "Carry selected"),
                                                selection.len()
                                            ),
                                            p,
                                        )
                                        .bg(p.selected)
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                if !this.save_editor(window, cx) {
                                                    return;
                                                }
                                                let ids = this
                                                    .carry_selection
                                                    .clone()
                                                    .unwrap_or_default()
                                                    .into_iter()
                                                    .collect::<Vec<_>>();
                                                let week = this.week;
                                                this.change(cx, |doc| {
                                                    doc.carry_selected(&ids, week);
                                                    Ok(())
                                                });
                                                if this.error.is_none() {
                                                    this.carry_selection = None;
                                                }
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            ),
                    ),
            );
        }
        self.window_background_layer()
            .child(root.relative())
            .children(self.confetti.overlay(window, p))
    }
}

impl Workspace {
    fn window_background_layer(&self) -> Div {
        let mut outer = div().relative().size_full().overflow_hidden();
        if let Some(path) = &self.background_render {
            let fit = match self.doc.settings.background_fill.as_str() {
                "fit" => ObjectFit::Contain,
                "stretch" => ObjectFit::Fill,
                _ => ObjectFit::Cover,
            };
            let image = if self.doc.settings.background_fill == "tile" {
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_wrap()
                    .children((0..80).map(|_| {
                        img(path.clone())
                            .w(px(240.))
                            .h(px(180.))
                            .object_fit(ObjectFit::Cover)
                            .into_any_element()
                    }))
                    .into_any_element()
            } else {
                img(path.clone())
                    .size_full()
                    .object_fit(fit)
                    .into_any_element()
            };
            outer = outer
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .opacity(self.doc.settings.background_opacity)
                        .child(image),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(gpui::black().opacity(self.doc.settings.background_scrim)),
                );
        }
        outer
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WeeklyGroup, matches_tag_week, week_completion, week_labels, weekly_group, weekly_order,
    };
    use crate::model::{Document, Task};
    use chrono::{Duration, Local, NaiveDate, TimeZone, Utc};

    #[test]
    fn weekly_groups_keep_tasks_in_their_date_or_unplanned_box() {
        let week = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let today = week + Duration::days(4);
        let stamp = |day: NaiveDate| {
            Local
                .from_local_datetime(&day.and_hms_opt(12, 0, 0).unwrap())
                .single()
                .unwrap()
                .with_timezone(&Utc)
        };
        let overdue = today - Duration::days(1);
        for (due, completed, completion, expected) in [
            (None, false, None, WeeklyGroup::Unplanned),
            (Some(overdue), false, None, WeeklyGroup::Overdue),
            (Some(today), false, None, WeeklyGroup::Day(today)),
            (Some(overdue), true, Some(today), WeeklyGroup::Day(overdue)),
            (None, true, Some(today), WeeklyGroup::Day(today)),
            (None, true, None, WeeklyGroup::Day(week)),
        ] {
            let task = Task {
                id: "pin".into(),
                parent_id: Some("parent".into()),
                week_start: week,
                due_date: due.map(stamp),
                completed,
                completed_at: completion.map(stamp),
                ..Default::default()
            };
            assert_eq!(weekly_group(&task, week, today, completed), Some(expected));
        }
    }

    #[test]
    fn weekly_groups_respect_date_week_and_completion_feedback() {
        let week = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let next_week = week + Duration::weeks(1);
        let timestamp = Local
            .from_local_datetime(&next_week.and_hms_opt(12, 0, 0).unwrap())
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let mut task = Task {
            week_start: week,
            completed: true,
            completed_at: Some(timestamp),
            ..Default::default()
        };
        // Completion in another week moves only after feedback settles.
        assert_eq!(
            weekly_group(&task, week, week, false),
            Some(WeeklyGroup::Unplanned)
        );
        assert_eq!(weekly_group(&task, week, week, true), None);
        assert_eq!(
            weekly_group(&task, next_week, week, true),
            Some(WeeklyGroup::Day(next_week))
        );
        task.due_date = Some(timestamp);
        assert_eq!(weekly_group(&task, week, week, false), None);
        assert_eq!(
            weekly_group(&task, next_week, week, false),
            Some(WeeklyGroup::Day(next_week))
        );
        task.due_date = None;
        task.completed_at = None;
        for completed in [false, true] {
            assert_eq!(weekly_group(&task, next_week, week, completed), None);
        }
    }

    #[test]
    fn weekly_order_uses_completion_for_done_and_due_for_open() {
        let date = |hour| Utc.with_ymd_and_hms(2026, 10, 24, hour, 0, 0).unwrap();
        let mut tasks = [
            Task {
                id: "late".into(),
                due_date: Some(date(17)),
                completed_at: Some(date(8)),
                sort_order: 0.,
                ..Default::default()
            },
            Task {
                id: "early".into(),
                due_date: Some(date(9)),
                completed_at: Some(date(18)),
                sort_order: 1.,
                ..Default::default()
            },
            Task {
                id: "middle".into(),
                due_date: Some(date(12)),
                completed_at: Some(date(12)),
                sort_order: 2.,
                ..Default::default()
            },
        ];
        tasks.sort_by(|a, b| weekly_order(a, false, b, false));
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["early", "middle", "late"]
        );
        tasks.sort_by(|a, b| weekly_order(a, true, b, true));
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["late", "middle", "early"]
        );
        // A checked row still uses its due time until the confetti settles.
        tasks[0].completed = true;
        tasks.sort_by(|a, b| weekly_order(a, false, b, false));
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["early", "middle", "late"]
        );
    }

    #[test]
    fn weekly_order_keeps_manual_ties_and_places_missing_dates_last() {
        let date = Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap();
        let mut tasks = [
            Task {
                id: "no-date".into(),
                sort_order: -10.,
                ..Default::default()
            },
            Task {
                id: "tie-second".into(),
                due_date: Some(date),
                sort_order: 2.,
                ..Default::default()
            },
            Task {
                id: "tie-first".into(),
                due_date: Some(date),
                sort_order: 1.,
                ..Default::default()
            },
        ];
        tasks.sort_by(|a, b| weekly_order(a, false, b, false));
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["tie-first", "tie-second", "no-date"]
        );
        // Legacy completed tasks without completed_at use their due date.
        tasks.sort_by(|a, b| weekly_order(a, true, b, true));
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["tie-first", "tie-second", "no-date"]
        );
    }

    #[test]
    fn weekly_completion_moves_unplanned_tasks_to_their_local_completion_day() {
        let week = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let day = week + Duration::days(2);
        let timestamp = Local
            .from_local_datetime(&day.and_hms_opt(0, 15, 0).unwrap())
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let mut task = Task {
            week_start: week,
            ..Default::default()
        };
        assert_eq!(
            weekly_group(&task, week, day, false),
            Some(WeeklyGroup::Unplanned)
        );
        assert_eq!(
            weekly_group(&task, week + Duration::weeks(1), day, false),
            None
        );

        task.completed = true;
        task.completed_at = Some(timestamp);
        assert_eq!(
            weekly_group(&task, week, day, true),
            Some(WeeklyGroup::Day(day))
        );
        // A completed task from an earlier assigned week still appears on its actual completion day.
        task.week_start = week - Duration::weeks(1);
        assert_eq!(
            weekly_group(&task, week, day, true),
            Some(WeeklyGroup::Day(day))
        );
        assert_eq!(weekly_group(&task, task.week_start, day, true), None);
        task.parent_id = Some("parent".into());
        assert_eq!(
            weekly_group(&task, week, day, true),
            Some(WeeklyGroup::Day(day))
        );
    }

    #[test]
    fn weekly_due_date_takes_priority_across_weeks_and_preserves_overdue_feedback() {
        let week = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let today = week + Duration::days(4);
        let due = week + Duration::days(1);
        let timestamp = |day: NaiveDate| {
            Local
                .from_local_datetime(&day.and_hms_opt(23, 45, 0).unwrap())
                .single()
                .unwrap()
                .with_timezone(&Utc)
        };
        let mut task = Task {
            week_start: week,
            completed: true,
            completed_at: Some(timestamp(today)),
            due_date: Some(timestamp(due)),
            ..Default::default()
        };
        assert_eq!(
            weekly_group(&task, week, today, true),
            Some(WeeklyGroup::Day(due))
        );
        // During confetti, keep the checked row in the overdue section rather than moving early.
        assert_eq!(
            weekly_group(&task, week, today, false),
            Some(WeeklyGroup::Overdue)
        );
        let next_due = due + Duration::weeks(1);
        task.due_date = Some(timestamp(next_due));
        assert_eq!(weekly_group(&task, week, today, true), None);
        assert_eq!(
            weekly_group(&task, week + Duration::weeks(1), today, true),
            Some(WeeklyGroup::Day(next_due))
        );
        assert_eq!(
            weekly_group(&task, week + Duration::weeks(1), today, false),
            Some(WeeklyGroup::Day(next_due))
        );
    }

    #[test]
    fn weekly_unplanned_completion_waits_for_feedback_and_undated_legacy_tasks_stay_visible() {
        let week = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let today = week + Duration::days(4);
        let mut task = Task {
            week_start: today,
            completed: true,
            ..Default::default()
        };
        assert_eq!(
            weekly_group(&task, week, today, false),
            Some(WeeklyGroup::Unplanned)
        );
        // Missing legacy timestamps cannot identify a completion day; fall back to the assigned Monday.
        assert_eq!(
            weekly_group(&task, week, today, true),
            Some(WeeklyGroup::Day(week))
        );
        task.completed_at = Some(
            Local
                .from_local_datetime(&today.and_hms_opt(12, 0, 0).unwrap())
                .single()
                .unwrap()
                .with_timezone(&Utc),
        );
        assert_eq!(
            weekly_group(&task, week, today, false),
            Some(WeeklyGroup::Unplanned)
        );
        assert_eq!(
            weekly_group(&task, week, today, true),
            Some(WeeklyGroup::Day(today))
        );
        task.completed = false;
        assert_eq!(
            weekly_group(&task, week, today, false),
            Some(WeeklyGroup::Unplanned)
        );
    }

    #[test]
    fn tag_tasks_follow_the_selected_week_across_year_boundaries() {
        let week = NaiveDate::from_ymd_opt(2025, 12, 29).unwrap();
        let mut doc = Document::default();
        for (title, date, tags, completed, parent_id) in [
            (
                "previous",
                week - chrono::Duration::weeks(1),
                vec!["work"],
                false,
                None,
            ),
            ("current", week, vec!["work"], false, None),
            ("other-tag", week, vec!["personal"], false, None),
            ("child", week, vec!["work"], false, Some("parent")),
            ("done", week, vec!["work"], true, None),
            (
                "imported-midweek",
                week + chrono::Duration::days(4),
                vec!["work"],
                false,
                None,
            ),
            (
                "next",
                week + chrono::Duration::weeks(1),
                vec!["work"],
                false,
                None,
            ),
        ] {
            doc.tasks.push(Task {
                title: title.into(),
                week_start: date,
                tags: tags.into_iter().map(str::to_owned).collect(),
                completed,
                parent_id: parent_id.map(str::to_owned),
                ..Default::default()
            });
        }
        for (selected_week, expected) in [
            (week - chrono::Duration::weeks(1), vec!["previous"]),
            (week, vec!["current", "child", "done", "imported-midweek"]),
            (week + chrono::Duration::weeks(1), vec!["next"]),
        ] {
            let actual = doc
                .tasks
                .iter()
                .filter(|task| matches_tag_week(task, "work", selected_week))
                .map(|task| task.title.as_str())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
        assert_eq!(
            doc.tasks
                .iter()
                .filter(|task| task.parent_id.is_none() && matches_tag_week(task, "work", week))
                .count(),
            3
        );
    }

    #[test]
    fn completion_counts_week_roots_independently_of_visibility_and_children() {
        let week = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let mut doc = Document::default();
        assert_eq!(week_completion(&doc, week), [(0, 0); 4]);
        let pending = doc.add_task("Pending root", week, 0);
        let done = doc.add_task("Completed root", week, 0);
        doc.task_mut(&done).unwrap().completed = true;
        doc.tasks.push(Task {
            id: "completed-child".into(),
            parent_id: Some(pending),
            completed: true,
            week_start: week,
            urgent: true,
            important: true,
            ..Default::default()
        });
        let other_quadrant = doc.add_task("Completed blue root", week, 1);
        doc.task_mut(&other_quadrant).unwrap().completed = true;
        let other_week = doc.add_task("Previous week", week - chrono::Duration::weeks(1), 0);
        doc.task_mut(&other_week).unwrap().completed = true;
        // Week membership is calendar-based, including imported midweek values.
        doc.add_task("Friday root", week + chrono::Duration::days(4), 2);
        let expected = [(1, 2), (1, 1), (0, 1), (0, 0)];
        assert_eq!(week_completion(&doc, week), expected);
        doc.settings.hide_completed = true;
        assert_eq!(week_completion(&doc, week), expected);
        doc.task_mut(&done).unwrap().completed = false;
        assert_eq!(week_completion(&doc, week)[0], (0, 2));
    }

    #[test]
    fn week_heading_matches_original_and_uses_iso_week_year_at_boundaries() {
        for (date, title, start, end) in [
            ("2026-10-19", "2026 年第 43 周", "10月19日", "10月25日"),
            ("2026-10-25", "2026 年第 43 周", "10月19日", "10月25日"),
            ("2025-12-29", "2026 年第 1 周", "12月29日", "1月4日"),
            ("2027-01-01", "2026 年第 53 周", "12月28日", "1月3日"),
            ("2027-01-04", "2027 年第 1 周", "1月4日", "1月10日"),
        ] {
            let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
            assert_eq!(
                week_labels(date, false),
                (title.into(), start.into(), end.into())
            );
        }
        assert_eq!(
            week_labels(NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(), true),
            ("2026 · Week 43".into(), "Oct 19".into(), "Oct 25".into()),
        );
    }
}
