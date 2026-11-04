use super::*;
use gpui_component::Root;
use std::{
    collections::HashMap,
    time::{Duration as TimeDuration, Instant},
};

mod pixels;

const MORPH: TimeDuration = TimeDuration::from_millis(300);
const ROW_HEIGHT: f32 = 56.;
const MAX_ROWS: usize = 5;
const TITLE_FONT_SIZE: f32 = 13.;
const TITLE_MAX_LINES: usize = 2;
/// The virtual island sits above the menu bar's lower edge on notchless screens.
const VIRTUAL_CAP_HEIGHT: f32 = 26.;

fn preview_expanded() -> bool {
    cfg!(debug_assertions)
        && std::env::var("FOUR_QUADRANTS_PREVIEW_ISLAND").as_deref() == Ok("expanded")
        && std::env::args().any(|arg| arg == "--data")
}

/// Pins are global choices, independent of the main window's current filters.
pub(super) fn island_tasks<'a>(doc: &'a Document, feedback: &confetti::Confetti) -> Vec<&'a Task> {
    let mut tasks: Vec<_> = doc
        .tasks
        .iter()
        .filter(|task| {
            (!task.completed && doc.pinned_task_ids.contains(&task.id)) || feedback.holds_pin(task)
        })
        .collect();
    tasks.sort_by(|a, b| {
        a.sort_order
            .total_cmp(&b.sort_order)
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    tasks
}

fn reconcile_selection(ids: &[String], selected: &mut Option<String>, previous_index: usize) {
    if !selected.as_ref().is_some_and(|id| ids.contains(id)) {
        *selected = ids
            .get(previous_index.min(ids.len().saturating_sub(1)))
            .cloned();
    }
}

fn title_needs_tooltip(
    title: &str,
    width: Pixels,
    text_style: &TextStyle,
    text_system: &WindowTextSystem,
) -> bool {
    if title.is_empty() || width <= px(0.) {
        return false;
    }

    let font_size = px(TITLE_FONT_SIZE);
    let run = text_style.to_run(title.len());
    let Ok(lines) = text_system.shape_text(
        title.to_owned().into(),
        font_size,
        &[run],
        Some(width),
        None,
    ) else {
        return false;
    };

    lines
        .iter()
        .map(|line| line.wrap_boundaries().len() + 1)
        .sum::<usize>()
        > TITLE_MAX_LINES
}

/// All dimensions are logical points. The camera strip stays empty in both states.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    compact: Size<Pixels>,
    expanded: Size<Pixels>,
    camera_width: Pixels,
    cap_height: Pixels,
}

impl Layout {
    fn new(display: crate::native::IslandDisplay, count: usize) -> Self {
        let physical = display.notch_height > 0. && display.notch_width > 0.;
        let camera_width = if physical {
            display.notch_width + 4.
        } else {
            72.
        };
        let cap_height = if display.notch_height > 0. {
            display.notch_height
        } else {
            display.menu_bar_height.clamp(20., VIRTUAL_CAP_HEIGHT)
        };
        let compact_width = (camera_width + 112.).min(display.width);
        let expanded_width = 440_f32.max(compact_width + 48.).min(display.width);
        // Header and list live entirely below the camera exclusion zone.
        let body_height = 36. + ROW_HEIGHT * count.clamp(1, MAX_ROWS) as f32 + 8.;
        Self {
            compact: size(px(compact_width), px(cap_height)),
            expanded: size(
                px(expanded_width),
                px((cap_height + body_height).min(display.height)),
            ),
            camera_width: px(camera_width),
            cap_height: px(cap_height),
        }
    }
}

#[derive(Clone, Copy)]
struct Morph {
    from: f32,
    to: f32,
    start: Instant,
}
impl Morph {
    fn value(self, now: Instant) -> f32 {
        let t = (now.saturating_duration_since(self.start).as_secs_f32() / MORPH.as_secs_f32())
            .clamp(0., 1.);
        self.from + (self.to - self.from) * (1. - (1. - t).powi(4))
    }
}

fn island_palette(settings: &model::Settings) -> Palette {
    let mut settings = settings.clone();
    settings.theme = "dark".into();
    let p = Palette::new(&settings, WindowAppearance::Dark);
    Palette {
        bg: black(),
        text: white(),
        muted: rgb(0x9b9ba3).into(),
        border: white().opacity(0.1),
        hover: white().opacity(0.09),
        selected: p.accent.opacity(0.16),
        ..p
    }
}

/// A flush top edge with small shoulders, broadening into a rounded bottom.
/// Drawn in GPUI so both the outline and content can share the same morph clock.
fn notch_surface(progress: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let x = bounds.left();
            let y = bounds.top();
            let w = bounds.size.width;
            let h = bounds.size.height;
            let shoulder = px(7. + 3. * progress);
            let radius = px(12. + 10. * progress).min(h / 2.);
            let mut path = PathBuilder::fill();
            path.move_to(point(x, y));
            path.line_to(point(x + w, y));
            path.curve_to(
                point(x + w - shoulder, y + shoulder),
                point(x + w - shoulder, y),
            );
            path.line_to(point(x + w - shoulder, y + h - radius));
            path.curve_to(
                point(x + w - shoulder - radius, y + h),
                point(x + w - shoulder, y + h),
            );
            path.line_to(point(x + shoulder + radius, y + h));
            path.curve_to(
                point(x + shoulder, y + h - radius),
                point(x + shoulder, y + h),
            );
            path.line_to(point(x + shoulder, y + shoulder));
            path.curve_to(point(x, y), point(x + shoulder, y));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, black());
            }
        },
    )
    .absolute()
    .size_full()
}

struct Island {
    workspace: WeakEntity<Workspace>,
    selected: Option<String>,
    selected_index: usize,
    layout: Layout,
    expanded: bool,
    hovered: bool,
    morph: Option<Morph>,
    hover_task: Option<gpui::Task<()>>,
    resize_task: Option<gpui::Task<()>>,
    celebration: bool,
    celebration_task: Option<gpui::Task<()>>,
    mascot_generation: u64,
    juggle_generation: Option<u64>,
    _idle_task: gpui::Task<()>,
    counter_generation: u64,
    last_count: usize,
    title_overflow: HashMap<String, (String, bool)>,
    native_error: Option<String>,
    focus: FocusHandle,
    list_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl Island {
    fn new(
        workspace: &Entity<Workspace>,
        layout: Layout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observed = cx.observe_in(workspace, window, |this, workspace, window, cx| {
            let owner = workspace.read(cx);
            let tasks = island_tasks(&owner.doc, &owner.confetti);
            let count = tasks.iter().filter(|task| !task.completed).count();
            let ids: Vec<_> = tasks.iter().map(|task| task.id.clone()).collect();
            if count != this.last_count {
                this.last_count = count;
                this.counter_generation = this.counter_generation.wrapping_add(1);
                this.mascot_generation = this.mascot_generation.wrapping_add(1);
                this.juggle_generation = None;
            }
            reconcile_selection(&ids, &mut this.selected, this.selected_index);
            this.selected_index = ids
                .iter()
                .position(|id| Some(id) == this.selected.as_ref())
                .unwrap_or(0);
            this.refresh_layout(window, cx);
            cx.notify();
        });
        // Native screen-parameter updates move the panel first; resize then
        // refreshes safe-area dimensions (unplugging a notched display included).
        let bounds = cx.observe_window_bounds(window, |this, window, cx| {
            this.refresh_layout(window, cx);
        });
        let selected = island_tasks(&workspace.read(cx).doc, &workspace.read(cx).confetti)
            .first()
            .map(|task| task.id.clone());
        // Sleep between brief juggling sequences; retaining the task here cancels it when
        // the Island closes. The completed pose does not request more frames.
        let idle_task = cx.spawn_in(window, async move |this, cx| {
            let mut interval = [18, 26, 22].into_iter().cycle();
            loop {
                cx.background_executor()
                    .timer(TimeDuration::from_secs(interval.next().unwrap()))
                    .await;
                if this
                    .update_in(cx, |this, _, cx| {
                        if this.hovered
                            || this.expanded
                            || this.morph.is_some()
                            || this.celebration
                            || crate::native::reduce_motion()
                        {
                            return;
                        }
                        this.mascot_generation = this.mascot_generation.wrapping_add(1);
                        this.juggle_generation = Some(this.mascot_generation);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            workspace: workspace.downgrade(),
            selected,
            selected_index: 0,
            layout,
            expanded: preview_expanded(),
            hovered: false,
            morph: None,
            hover_task: None,
            resize_task: None,
            celebration: false,
            celebration_task: None,
            mascot_generation: 0,
            juggle_generation: None,
            _idle_task: idle_task,
            counter_generation: 0,
            last_count: island_tasks(&workspace.read(cx).doc, &workspace.read(cx).confetti)
                .iter()
                .filter(|task| !task.completed)
                .count(),
            title_overflow: HashMap::new(),
            native_error: None,
            focus: cx.focus_handle(),
            list_scroll: ScrollHandle::new(),
            _subscriptions: vec![observed, bounds],
        }
    }

    #[cfg(debug_assertions)]
    fn preview_hover_cycles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::env::var("FOUR_QUADRANTS_PREVIEW_ISLAND").as_deref() != Ok("cycle")
            || !std::env::args().any(|arg| arg == "--data")
        {
            return;
        }
        // Exercise the real hover timers, renderer and native resizing without
        // synthesizing OS input or requiring Accessibility permission.
        cx.spawn_in(window, async move |this, cx| {
            for cycle in 0..3 {
                for hovered in [true, false] {
                    if this.update_in(cx, |this, window, cx| this.hover(hovered, window, cx)).is_err() {
                        eprintln!("Island preview FAILED: window closed before hover");
                        return;
                    }
                    // Timer delivery can be delayed by layout/rendering. Wait for
                    // the actual settled state; never panic from a GUI probe.
                    let deadline = Instant::now() + TimeDuration::from_secs(10);
                    loop {
                        cx.background_executor().timer(TimeDuration::from_millis(100)).await;
                        let state = this.update_in(cx, |this, window, _| {
                            let expected = if hovered { this.layout.expanded } else { this.layout.compact };
                            let actual = window.viewport_size();
                            (this.expanded == hovered && this.morph.is_none() && actual == expected, actual)
                        });
                        match state {
                            Ok((true, actual)) => {
                                eprintln!("Island preview cycle {cycle}: expanded={hovered}, size={actual:?}");
                                break;
                            }
                            Ok((false, actual)) if Instant::now() >= deadline => {
                                eprintln!("Island preview FAILED: cycle {cycle} did not settle, expanded={hovered}, size={actual:?}");
                                return;
                            }
                            Err(error) => {
                                eprintln!("Island preview FAILED: {error}");
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }).detach();
    }

    fn refresh_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let Ok(display) = crate::native::island_display(window) else {
            return;
        };
        let next = Layout::new(
            display,
            island_tasks(&workspace.read(cx).doc, &workspace.read(cx).confetti).len(),
        );
        if next != self.layout {
            self.layout = next;
            self.resize_native(
                if self.expanded || self.morph.is_some() {
                    next.expanded
                } else {
                    next.compact
                },
                window,
                cx,
            );
            cx.notify();
        }
    }

    fn progress(&self) -> f32 {
        self.morph
            .map(|m| m.value(Instant::now()))
            .unwrap_or(if self.expanded { 1. } else { 0. })
    }

    fn set_expanded(&mut self, expanded: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.hover_task = None;
        if self.expanded == expanded {
            return;
        }
        let from = self.progress();
        self.expanded = expanded;
        if expanded {
            self.juggle_generation = None;
        }
        self.resize_task = None;
        let motion = !crate::native::reduce_motion();
        self.morph = motion.then_some(Morph {
            from,
            to: if expanded { 1. } else { 0. },
            start: Instant::now(),
        });
        if expanded || !motion {
            self.resize_native(
                if expanded {
                    self.layout.expanded
                } else {
                    self.layout.compact
                },
                window,
                cx,
            );
        }
        if motion {
            self.resize_task = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(MORPH).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.morph = None;
                    if !this.expanded {
                        this.resize_native(this.layout.compact, window, cx);
                    }
                    cx.notify();
                });
            }));
        }
        cx.notify();
    }

    fn resize_native(
        &mut self,
        dimensions: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The native bridge queues/coalesces AppKit geometry outside this borrow.
        if let Err(error) = crate::native::resize_island(window, dimensions) {
            self.native_error = Some(format!("{error:#}"));
            cx.notify();
        }
    }

    fn hover(&mut self, hovered: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.hovered == hovered {
            return;
        }
        self.hovered = hovered;
        if hovered {
            self.juggle_generation = None;
            self.mascot_generation = self.mascot_generation.wrapping_add(1);
            cx.notify();
        }
        self.hover_task = None;
        if hovered == self.expanded {
            return;
        }
        self.hover_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(TimeDuration::from_millis(if hovered { 140 } else { 420 }))
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.hovered == hovered {
                    this.set_expanded(hovered, window, cx);
                }
            });
        }));
    }

    fn select(&mut self, id: String, index: usize, cx: &mut Context<Self>) {
        self.selected = Some(id);
        self.selected_index = index;
        self.list_scroll.scroll_to_item(index);
        cx.notify();
    }

    fn show_workspace(&self, task: Option<String>, cx: &mut Context<Self>) {
        let weak = self.workspace.clone();
        cx.defer(move |cx| {
            let Some(workspace) = weak.upgrade() else {
                return;
            };
            let main = workspace.read(cx).main_window;
            let _ = main.update(cx, |_, window, cx| {
                if let Some(id) = task {
                    workspace.update(cx, |workspace, cx| workspace.open_task(&id, window, cx));
                } else {
                    workspace.update(cx, |workspace, cx| workspace.show_main_window(window, cx));
                }
            });
        });
    }

    fn complete(
        &mut self,
        id: &str,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.workspace.update(cx, |workspace, cx| {
            let was_open = workspace.doc.task(id).is_some_and(|task| !task.completed);
            if !was_open {
                return false;
            }
            workspace.toggle_task_complete(id, event, window, cx);
            workspace.doc.task(id).is_some_and(|task| task.completed)
        });
        if matches!(result, Ok(true)) {
            self.celebration = true;
            self.juggle_generation = None;
            self.mascot_generation = self.mascot_generation.wrapping_add(1);
            self.counter_generation = self.counter_generation.wrapping_add(1);
            self.celebration_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(TimeDuration::from_millis(1100))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.celebration = false;
                    this.counter_generation = this.counter_generation.wrapping_add(1);
                    cx.notify();
                });
            }));
            cx.notify();
        }
    }

    fn row(
        &self,
        task: &Task,
        index: usize,
        owner: &Workspace,
        p: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let doc = &owner.doc;
        let (title_text, styled_title) = owner.task_title_with_text(task, p.accent, window, cx);
        let id = task.id.clone();
        let done = id.clone();
        let active = self.selected.as_ref() == Some(&id);
        let color = p.quadrant[task.quadrant()];
        let completed = task.completed;
        let en = doc.settings.language == "en";
        let parent = task.parent_id.as_deref().and_then(|id| doc.task(id));
        let due = task.due_date.map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%m/%d")
                .to_string()
        });
        let tooltip_visible = self
            .title_overflow
            .get(&task.id)
            .is_some_and(|(title, truncated)| title == &title_text && *truncated);
        let tooltip_title = title_text.clone();
        let measured_title = title_text;
        let measured_id = task.id.clone();
        let measured_view = cx.weak_entity();
        div()
            .id(SharedString::from(format!("island-row-{id}")))
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(9.))
            .bg(if active {
                p.selected
            } else {
                transparent_black()
            })
            .hover(move |s| s.bg(p.hover))
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.focus.focus(window);
                if event.modifiers().alt {
                    let _ = this.workspace.update(cx, |workspace, cx| {
                        workspace.activate_task(&id, event, window, cx);
                    });
                    return;
                }
                this.select(id.clone(), index, cx);
                if event.click_count() == 2 {
                    this.show_workspace(Some(id.clone()), cx);
                }
            }))
            .child(
                div()
                    .id(SharedString::from(format!("island-done-{}", task.id)))
                    .size(px(28.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .hover(move |s| s.bg(color.opacity(0.18)))
                    .child(
                        div()
                            .size(px(15.))
                            .rounded_full()
                            .border_1()
                            .border_color(color)
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(completed, |el| {
                                el.bg(color)
                                    .text_color(p.bg)
                                    .child(glyph(IconName::Check).with_size(px(11.)))
                            }),
                    )
                    .tooltip(move |window, cx| {
                        Tooltip::new(match (en, completed) {
                            (true, true) => "Completed",
                            (true, false) => "Complete",
                            (false, true) => "已完成",
                            (false, false) => "完成",
                        })
                        .build(window, cx)
                    })
                    .on_click(cx.listener(move |this, event, window, cx| {
                        cx.stop_propagation();
                        this.complete(&done, event, window, cx);
                    })),
            )
            .child(
                div()
                    .on_children_prepainted(move |children, window, cx| {
                        let Some(title_bounds) = children.first() else {
                            return;
                        };
                        let mut text_style = window.text_style();
                        text_style.font_size = px(TITLE_FONT_SIZE).into();
                        text_style.white_space = WhiteSpace::Normal;
                        text_style.line_clamp = None;
                        text_style.text_overflow = None;
                        let truncated = title_needs_tooltip(
                            &measured_title,
                            title_bounds.size.width,
                            &text_style,
                            window.text_system(),
                        );
                        let unchanged = measured_view
                            .read_with(cx, |this, _| {
                                matches!(
                                    this.title_overflow.get(&measured_id),
                                    Some((title, current)) if title == &measured_title && *current == truncated
                                )
                            })
                            .unwrap_or(false);
                        if !unchanged {
                            let measured_view = measured_view.clone();
                            let measured_id = measured_id.clone();
                            let measured_title = measured_title.clone();
                            cx.defer(move |cx| {
                                let _ = measured_view.update(cx, |this, cx| {
                                    let unchanged = matches!(
                                        this.title_overflow.get(&measured_id),
                                        Some((title, current)) if title == &measured_title && *current == truncated
                                    );
                                    if !unchanged {
                                        this.title_overflow.insert(
                                            measured_id,
                                            (measured_title, truncated),
                                        );
                                        cx.notify();
                                    }
                                });
                            });
                        }
                    })
                    .id(SharedString::from(format!("island-title-{}", task.id)))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when(tooltip_visible, |el| {
                        el.tooltip(move |window, cx| {
                            let title = tooltip_title.clone();
                            // Include the tooltip's padding, border and outer margins.
                            let max_width = (window.viewport_size().width - px(48.))
                                .max(px(1.))
                                .min(px(420.));
                            Tooltip::element(move |_, _| {
                                div()
                                    .max_w(max_width)
                                    .whitespace_normal()
                                    .child(title.clone())
                            })
                            .build(window, cx)
                        })
                    })
                    .child(
                        div()
                            .text_size(px(TITLE_FONT_SIZE))
                            .line_height(px(18.))
                            .overflow_hidden()
                            .line_clamp(TITLE_MAX_LINES)
                            .text_ellipsis()
                            .child(styled_title),
                    )
                    .when_some(parent, |el, parent| {
                        el.child(
                            div()
                                .text_size(px(10.))
                                .text_color(p.muted)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(parent.title.clone()),
                        )
                    }),
            )
            .when_some(due, |el, due| {
                el.child(
                    div()
                        .text_size(px(10.))
                        .text_color(
                            if task.due_date.is_some_and(|date| date < chrono::Utc::now()) {
                                p.quadrant[0]
                            } else {
                                p.muted
                            },
                        )
                        .child(due),
                )
            })
    }
}

impl Render for Island {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(workspace) = self.workspace.upgrade() else {
            return div().into_any_element();
        };
        let owner = workspace.read(cx);
        let doc = &owner.doc;
        let p = island_palette(&doc.settings);
        let en = doc.settings.language == "en";
        let tr = |zh: &'static str, english: &'static str| if en { english } else { zh };
        let tasks = island_tasks(doc, &owner.confetti);
        let count = tasks.iter().filter(|task| !task.completed).count();
        let motion = !crate::native::reduce_motion();
        let progress = self.progress();
        if self.morph.is_some() {
            window.request_animation_frame();
        }
        let layout = self.layout;
        let width =
            layout.compact.width + (layout.expanded.width - layout.compact.width) * progress;
        let height =
            layout.compact.height + (layout.expanded.height - layout.compact.height) * progress;
        let move_view = cx.weak_entity();
        let exit_view = cx.weak_entity();
        let tracking = canvas(
            |bounds, _, _| bounds,
            move |_, bounds, window, _| {
                let view = move_view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = view.update(cx, |this, cx| {
                            let hit = if this.expanded {
                                Bounds::new(
                                    point(
                                        (window.viewport_size().width - this.layout.expanded.width)
                                            / 2.,
                                        px(0.),
                                    ),
                                    this.layout.expanded,
                                )
                            } else {
                                bounds
                            };
                            this.hover(hit.contains(&event.position), window, cx);
                        });
                    }
                });
                let view = exit_view.clone();
                window.on_mouse_event(move |_: &MouseExitEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = view.update(cx, |this, cx| this.hover(false, window, cx));
                    }
                });
            },
        )
        .absolute()
        .size_full();
        // The center spacer is the real camera width, not a fixed guess. Nothing
        // interactive or legible is ever placed behind the physical camera.
        let cap = div()
            .id("island-cap")
            .h(layout.cap_height)
            .pb(px(2.))
            .flex_shrink_0()
            .px(px(14.))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, window, cx| {
                this.focus.focus(window);
                this.set_expanded(true, window, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(pixels::mascot(
                        count == 0,
                        self.celebration,
                        self.mascot_generation,
                        self.juggle_generation,
                        motion,
                        p,
                    )),
            )
            // Width-only spacer; a full-height child would compete with the
            // 1px optical lift applied by the cap's bottom padding.
            .child(div().w(layout.camera_width).flex_shrink_0())
            .child(
                div()
                    .id("island-pixel-count")
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(move |window, cx| {
                        Tooltip::new(if en {
                            format!("{count} pinned tasks")
                        } else {
                            format!("{count} 个 Pin 任务")
                        })
                        .build(window, cx)
                    })
                    .child(pixels::counter(
                        count,
                        self.celebration,
                        self.counter_generation,
                        motion,
                        p,
                    )),
            );
        let mut panel = div()
            .id("island-surface")
            .relative()
            .w(width)
            .h(height)
            .flex_shrink_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(notch_surface(progress))
            .child(tracking)
            .child(cap);
        if progress > 0.01 {
            let error = self.native_error.clone().or_else(|| owner.error.clone());
            let list = div()
                .id("island-list")
                .track_scroll(&self.list_scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(
                    tasks
                        .iter()
                        .enumerate()
                        .map(|(index, task)| self.row(task, index, owner, p, window, cx)),
                )
                .when(tasks.is_empty(), |el| {
                    el.child(
                        div()
                            .h(px(ROW_HEIGHT))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .child(tr("把重要的事 Pin 到这里", "Pin what matters here")),
                            )
                            .child(div().text_size(px(11.)).text_color(p.muted).child(tr(
                                "在任务上 Shift＋单击即可",
                                "Shift-click any task to pin it",
                            ))),
                    )
                });
            let content = div()
                .w(layout.expanded.width)
                .h(layout.expanded.height - layout.cap_height)
                .flex_shrink_0()
                .px(px(20.))
                .pb(px(8.))
                .flex()
                .flex_col()
                .opacity(((progress - 0.1) / 0.9).clamp(0., 1.))
                .child(
                    div()
                        .h(px(36.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(self.celebration, |el| {
                            el.child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(tr("又完成了一件事", "One more thing, done")),
                            )
                        })
                        .child(div().flex_1())
                        .child(
                            icon_button(
                                "island-main",
                                IconName::ExternalLink,
                                tr("返回主窗口", "Show workspace"),
                                p,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.show_workspace(None, cx))),
                        )
                        .child(
                            icon_button(
                                "island-close",
                                IconName::Close,
                                tr("关闭 Island", "Turn off Island"),
                                p,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let _ = this.workspace.update(cx, |workspace, cx| {
                                    workspace.change(cx, |doc| {
                                        doc.settings.island_enabled = false;
                                        Ok(())
                                    })
                                });
                            })),
                        ),
                )
                .when_some(error, |el, error| {
                    el.child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.quadrant[0])
                            .child(error),
                    )
                })
                .child(list);
            panel = panel.child(content);
        }
        div()
            .key_context("Island")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .justify_center()
            .font_family(if doc.settings.font_family.is_empty() {
                ".AppleSystemUIFont".into()
            } else {
                doc.settings.font_family.clone()
            })
            .text_color(p.text)
            .text_size(px(12.))
            .on_action(cx.listener(|this, _: &ToggleIsland, _, cx| {
                let _ = this
                    .workspace
                    .update(cx, |workspace, cx| workspace.toggle_island(cx));
            }))
            .on_action(cx.listener(|this, _: &crate::CloseWindow, window, cx| {
                this.set_expanded(false, window, cx);
            }))
            .on_action(|_: &crate::Quit, _, cx| crate::quit_application(cx))
            .on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => this.set_expanded(false, window, cx),
                    "down" | "up" => {
                        if let Some(workspace) = this.workspace.upgrade() {
                            let tasks =
                                island_tasks(&workspace.read(cx).doc, &workspace.read(cx).confetti);
                            if !tasks.is_empty() {
                                let index = if event.keystroke.key == "down" {
                                    (this.selected_index + 1) % tasks.len()
                                } else {
                                    (this.selected_index + tasks.len() - 1) % tasks.len()
                                };
                                this.select(tasks[index].id.clone(), index, cx);
                            }
                        }
                    }
                    "enter" => {
                        if let Some(id) = this.selected.clone() {
                            this.show_workspace(Some(id), cx);
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(panel)
            .children(owner.confetti.overlay(window, p))
            .into_any_element()
    }
}

impl Workspace {
    pub(super) fn toggle_island(&mut self, cx: &mut Context<Self>) {
        self.change(cx, |doc| {
            doc.settings.island_enabled = !doc.settings.island_enabled;
            Ok(())
        });
    }

    pub(super) fn close_island(&mut self, cx: &mut Context<Self>) {
        self.island_generation = self.island_generation.wrapping_add(1);
        if let Some(handle) = self.island.take() {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    pub fn sync_island(&mut self, cx: &mut Context<Self>) {
        if !self.doc.settings.island_enabled {
            self.close_island(cx);
            return;
        }
        if self.island.is_some() {
            return;
        }
        self.island_generation = self.island_generation.wrapping_add(1);
        let generation = self.island_generation;
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            let Some(workspace) = weak.upgrade() else {
                return;
            };
            if workspace.read(cx).island_generation != generation
                || !workspace.read(cx).doc.settings.island_enabled
            {
                return;
            }
            let main = workspace.read(cx).main_window;
            let count = island_tasks(&workspace.read(cx).doc, &workspace.read(cx).confetti).len();
            let placement = main
                .update(cx, |_, window, cx| {
                    let display = crate::native::island_display(window)?;
                    let display_id = cx
                        .displays()
                        .iter()
                        .find(|screen| u32::from(screen.id()) == display.id)
                        .map(|screen| screen.id());
                    let layout = Layout::new(display, count);
                    crate::native::island_bounds(window, layout.compact)
                        .map(|bounds| (bounds, display_id, layout))
                })
                .and_then(|result| result);
            let result = placement.and_then(|(bounds, display_id, layout)| {
                let owner = workspace.clone();
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        display_id,
                        kind: WindowKind::PopUp,
                        titlebar: None,
                        focus: false,
                        show: false,
                        is_resizable: false,
                        is_minimizable: false,
                        is_movable: false,
                        window_background: WindowBackgroundAppearance::Transparent,
                        ..Default::default()
                    },
                    move |window, cx| {
                        window.set_window_title("Island · FourQuadrants");
                        let handle = window.window_handle();
                        owner.update(cx, |workspace, _| workspace.island = Some(handle));
                        let view = cx.new(|cx| Island::new(&owner, layout, window, cx));
                        let weak_view = view.downgrade();
                        cx.defer(move |cx| {
                            let _ = handle.update(cx, |_, window, cx| {
                                let result =
                                    crate::native::configure_island(window).and_then(|_| {
                                        crate::native::resize_island(
                                            window,
                                            if preview_expanded() {
                                                layout.expanded
                                            } else {
                                                layout.compact
                                            },
                                        )
                                    });
                                #[cfg(debug_assertions)]
                                if result.is_ok() {
                                    let _ = weak_view.update(cx, |this, cx| {
                                        this.preview_hover_cycles(window, cx)
                                    });
                                }
                                if let Err(error) = result {
                                    let _ = weak_view.update(cx, |this, cx| {
                                        this.native_error = Some(format!("{error:#}"));
                                        cx.notify();
                                    });
                                }
                            });
                        });
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .map(|_| ())
            });
            if let Err(error) = result {
                workspace.update(cx, |workspace, cx| {
                    workspace.error = Some(format!("Island: {error:#}"));
                    cx.notify();
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Document, Layout, MAX_ROWS, MORPH, Morph, ROW_HEIGHT, Task, island_tasks, model,
        reconcile_selection,
    };
    use crate::native::IslandDisplay;
    use chrono::Duration;
    use gpui::{px, size};
    use std::time::Instant;

    fn display(notched: bool) -> IslandDisplay {
        IslandDisplay {
            id: 1,
            width: 1512.,
            height: 940.,
            notch_width: if notched { 184. } else { 0. },
            notch_height: if notched { 37. } else { 0. },
            menu_bar_height: 24.,
        }
    }

    #[test]
    fn camera_is_reserved_and_list_height_is_bounded() {
        let layout = Layout::new(display(true), 3);
        assert_eq!(layout.camera_width, px(188.));
        assert_eq!(layout.cap_height, px(37.));
        assert_eq!(layout.compact, size(px(300.), px(37.)));
        assert_eq!(layout.expanded, size(px(440.), px(249.)));
        assert_eq!(
            Layout::new(display(true), 100).expanded,
            Layout::new(display(true), 5).expanded
        );
        assert_eq!(
            Layout::new(display(true), 0).expanded,
            Layout::new(display(true), 1).expanded
        );
        let virtual_island = Layout::new(display(false), 3);
        assert_eq!(virtual_island.compact, size(px(184.), px(24.)));
        let short_notch = IslandDisplay {
            notch_height: 28.,
            ..display(true)
        };
        assert_eq!(Layout::new(short_notch, 3).compact.height, px(28.));
        let tall_menu = IslandDisplay {
            menu_bar_height: 30.,
            ..display(false)
        };
        assert_eq!(Layout::new(tall_menu, 3).compact.height, px(26.));
        let small_menu = IslandDisplay {
            menu_bar_height: 22.,
            ..display(false)
        };
        assert_eq!(Layout::new(small_menu, 3).compact.height, px(22.));
        let tiny = IslandDisplay {
            width: 320.,
            height: 220.,
            ..display(false)
        };
        assert_eq!(Layout::new(tiny, 100).expanded, size(px(320.), px(220.)));
    }

    #[test]
    fn expanded_height_shrinks_to_one_row_as_pinned_tasks_disappear() {
        for notched in [false, true] {
            let screen = display(notched);
            for count in (2..=MAX_ROWS).rev() {
                let before = Layout::new(screen, count);
                let after = Layout::new(screen, count - 1);
                assert_eq!(
                    before.expanded.height - after.expanded.height,
                    px(ROW_HEIGHT)
                );
                assert_eq!(before.expanded.width, after.expanded.width);
                assert_eq!(before.compact, after.compact);
            }
            let single = Layout::new(screen, 1);
            assert_eq!(
                single.expanded.height,
                single.cap_height + px(36. + ROW_HEIGHT + 8.)
            );
            assert_eq!(Layout::new(screen, 0).expanded, single.expanded);
        }
    }

    #[test]
    fn pins_cross_weeks_and_filters_but_exclude_completed_tasks() {
        let mut doc = Document::default();
        for (id, completed, order) in [
            ("later", false, 2.),
            ("first", false, 1.),
            ("done", true, 0.),
            ("ordinary", false, 0.),
        ] {
            doc.tasks.push(Task {
                id: id.into(),
                completed,
                sort_order: order,
                week_start: model::today() - Duration::weeks(order as i64),
                ..Task::default()
            });
        }
        doc.pinned_task_ids = ["later", "first", "done"].map(String::from).into();
        doc.settings.hide_completed = true;
        assert_eq!(
            island_tasks(&doc, &super::confetti::Confetti::default())
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "later"]
        );
        doc.toggle_complete("first");
        assert_eq!(
            island_tasks(&doc, &super::confetti::Confetti::default())[0].id,
            "later"
        );
    }

    #[test]
    fn selection_survives_reordering_and_moves_to_neighbor_on_removal() {
        let mut selected = Some("b".into());
        reconcile_selection(&["c".into(), "b".into(), "a".into()], &mut selected, 0);
        assert_eq!(selected.as_deref(), Some("b"));
        reconcile_selection(&["c".into(), "a".into()], &mut selected, 1);
        assert_eq!(selected.as_deref(), Some("a"));
        reconcile_selection(&[], &mut selected, 0);
        assert!(selected.is_none());
    }

    #[test]
    fn morph_can_reverse_without_jumping_and_has_bounded_endpoints() {
        let start = Instant::now();
        let opening = Morph {
            from: 0.,
            to: 1.,
            start,
        };
        let middle = start + MORPH / 3;
        let closing = Morph {
            from: opening.value(middle),
            to: 0.,
            start: middle,
        };
        assert_eq!(opening.value(start), 0.);
        assert_eq!(opening.value(middle), closing.value(middle));
        assert_eq!(opening.value(start + MORPH * 2), 1.);
        assert_eq!(closing.value(middle + MORPH * 2), 0.);
    }

    #[test]
    fn old_settings_do_not_open_a_window_and_preference_round_trips() {
        let mut settings: model::Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.island_enabled);
        settings.island_enabled = true;
        let restored: model::Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(restored.island_enabled);
    }
}
