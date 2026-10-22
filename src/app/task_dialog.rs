use super::*;
use gpui_component::Root;

struct TaskDialog {
    workspace: Entity<Workspace>,
    settings: model::Settings,
    focus: FocusHandle,
    material_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl TaskDialog {
    fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = workspace.read(cx).doc.settings.clone();
        let material_error = crate::native::configure_window_material(window, &settings)
            .err()
            .map(|error| format!("{error:#}"));
        let observed = cx.observe_in(&workspace, window, |this, workspace, window, cx| {
            workspace.update(cx, |workspace, cx| {
                if workspace.task_dialog != Some(window.window_handle()) {
                    return;
                }
                let task = workspace
                    .editor
                    .as_ref()
                    .and_then(|editor| workspace.doc.task(&editor.id))
                    .cloned();
                if let (Some(editor), Some(task)) = (&mut workspace.editor, task) {
                    if workspace.doc.settings.language != this.settings.language {
                        editor.refresh_placeholders(&workspace.doc.settings, window, cx);
                    }
                    editor.refresh_pristine(&task, window, cx);
                } else {
                    workspace.dismiss_task_dialog(window, cx);
                }
            });
            let settings = workspace.read(cx).doc.settings.clone();
            if settings != this.settings {
                this.material_error = crate::native::configure_window_material(window, &settings)
                    .err()
                    .map(|error| format!("{error:#}"));
                this.settings = settings;
            }
            cx.notify();
        });
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.settings.theme == "system" {
                this.workspace
                    .update(cx, |workspace, cx| workspace.apply_theme(window, cx));
                cx.notify();
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                let workspace = this.workspace.downgrade();
                let handle = window.window_handle();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        let _ = workspace.update(cx, |workspace, cx| {
                            if workspace.task_dialog == Some(handle) {
                                workspace.save_editor(window, cx);
                            }
                        });
                    });
                });
            }
        });
        let weak = workspace.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |workspace, cx| {
                if !workspace.save_editor(window, cx) {
                    return false;
                }
                workspace.clear_task_dialog(cx);
                true
            })
            .unwrap_or(true)
        });
        let handle = window.window_handle();
        let weak = workspace.downgrade();
        cx.on_release(move |_, cx| {
            cx.defer(move |cx| {
                let _ = weak.update(cx, |workspace, cx| {
                    if workspace.task_dialog == Some(handle) {
                        workspace.clear_task_dialog(cx);
                    }
                });
            });
        })
        .detach();
        Self {
            workspace,
            settings,
            focus: cx.focus_handle(),
            material_error,
            _subscriptions: vec![observed, appearance, activation],
        }
    }

    fn forward(&self, action: Box<dyn Action>, cx: &mut Context<Self>) {
        let main = self.workspace.read(cx).main_window;
        let focus = self.workspace.read(cx).focus.clone();
        cx.defer(move |cx| {
            let _ = main.update(cx, |_, window, cx| {
                window.activate_window();
                focus.focus(window);
                window.dispatch_action(action, cx);
            });
        });
    }
}

impl Render for TaskDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.workspace.clone();
        let content = workspace.update(cx, |workspace, cx| workspace.editor_view(window, cx));
        let p = workspace.read(cx).palette(window);
        let root =
            div()
                .key_context("TaskDialog")
                .track_focus(&self.focus)
                .size_full()
                .flex()
                .flex_col()
                .bg(p.bg)
                .text_color(p.text)
                .font_family(if self.settings.theme == "terminal" {
                    "Menlo".into()
                } else {
                    self.settings.font_family.clone()
                })
                .text_size(px(self.settings.font_size))
                .on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))
                .on_action(cx.listener(|this, _: &CloseTaskWindow, window, cx| {
                    this.workspace
                        .update(cx, |workspace, cx| workspace.close_task_dialog(window, cx));
                }))
                .on_action(cx.listener(|this, _: &SaveTask, window, cx| {
                    this.workspace.update(cx, |workspace, cx| {
                        workspace.save_editor(window, cx);
                    });
                }))
                .on_action(cx.listener(|this, _: &NewTask, window, cx| {
                    this.workspace
                        .update(cx, |workspace, cx| workspace.add_task(0, None, window, cx));
                }))
                .on_action(cx.listener(|this, _: &crate::Quit, window, cx| {
                    let ready = this
                        .workspace
                        .update(cx, |workspace, cx| workspace.prepare_close(window, cx));
                    if ready {
                        cx.quit();
                    } else {
                        cx.stop_propagation();
                    }
                }))
                .on_action(cx.listener(|this, _: &Undo, _, cx| this.forward(Box::new(Undo), cx)))
                .on_action(cx.listener(|this, _: &Redo, _, cx| this.forward(Box::new(Redo), cx)))
                .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                    this.forward(Box::new(ToggleSidebar), cx)
                }))
                .on_action(
                    cx.listener(|this, _: &Search, _, cx| this.forward(Box::new(Search), cx)),
                )
                .on_action(
                    cx.listener(|this, _: &Previous, _, cx| this.forward(Box::new(Previous), cx)),
                )
                .on_action(cx.listener(|this, _: &Next, _, cx| this.forward(Box::new(Next), cx)))
                .on_action(
                    cx.listener(|this, _: &ThisWeek, _, cx| this.forward(Box::new(ThisWeek), cx)),
                )
                .on_action(cx.listener(|this, _: &ExportMarkdown, _, cx| {
                    this.forward(Box::new(ExportMarkdown), cx)
                }))
                .on_action(cx.listener(|this, _: &ToggleCompleted, _, cx| {
                    this.forward(Box::new(ToggleCompleted), cx)
                }))
                .on_action(
                    cx.listener(|this, _: &PasteTask, _, cx| this.forward(Box::new(PasteTask), cx)),
                )
                .on_action(
                    cx.listener(|this, _: &Settings, _, cx| this.forward(Box::new(Settings), cx)),
                )
                .on_action(
                    cx.listener(|this, _: &ShowBoard, _, cx| this.forward(Box::new(ShowBoard), cx)),
                )
                .on_action(
                    cx.listener(|this, _: &ShowAgenda, _, cx| {
                        this.forward(Box::new(ShowAgenda), cx)
                    }),
                )
                .on_action(cx.listener(|this, _: &ShowCalendar, _, cx| {
                    this.forward(Box::new(ShowCalendar), cx)
                }))
                .on_action(
                    cx.listener(|this, _: &ShowNotes, _, cx| this.forward(Box::new(ShowNotes), cx)),
                )
                .on_action(cx.listener(|this, _: &IncreaseFont, _, cx| {
                    this.forward(Box::new(IncreaseFont), cx)
                }))
                .on_action(cx.listener(|this, _: &DecreaseFont, _, cx| {
                    this.forward(Box::new(DecreaseFont), cx)
                }))
                .on_action(
                    cx.listener(|this, _: &ResetFont, _, cx| this.forward(Box::new(ResetFont), cx)),
                )
                .when_some(self.material_error.clone(), |el, error| {
                    el.child(
                        div()
                            .px_4()
                            .pt(px(44.))
                            .text_sm()
                            .text_color(p.quadrant[0])
                            .child(error),
                    )
                })
                .child(content);
        workspace
            .read(cx)
            .window_background_layer()
            .child(root.relative())
            .children(workspace.read(cx).confetti.overlay(window, p))
    }
}

impl Workspace {
    pub(super) fn show_task_dialog(
        &mut self,
        id: String,
        blank_title: bool,
        cx: &mut Context<Self>,
    ) {
        self.task_dialog_generation = self.task_dialog_generation.wrapping_add(1);
        let generation = self.task_dialog_generation;
        let workspace = cx.entity();
        // open_window draws its root immediately, after the Workspace borrow must end.
        cx.defer(move |cx| {
            if workspace.read(cx).task_dialog_generation != generation {
                return;
            }
            if let Some(handle) = workspace.read(cx).task_dialog {
                let result = handle.update(cx, |_, window, cx| {
                    workspace.update(cx, |workspace, cx| {
                        if !workspace.save_editor(window, cx) {
                            return;
                        }
                        if workspace
                            .editor
                            .as_ref()
                            .is_some_and(|editor| editor.id == id)
                        {
                            if let Some(editor) = &workspace.editor {
                                editor.focus_title(window, cx);
                            }
                        } else if let Some(task) = workspace.doc.task(&id) {
                            workspace.editor = Some(TaskEditor::new(
                                task,
                                blank_title,
                                &workspace.doc.settings,
                                window,
                                cx,
                            ));
                            workspace.selected = Some(id.clone());
                        }
                        window.activate_window();
                        cx.notify();
                    });
                });
                if result.is_ok() {
                    return;
                }
            }
            let main = workspace.read(cx).main_window;
            let placement = main
                .update(cx, |_, window, cx| {
                    crate::native::task_dialog_bounds(window, size(px(960.), px(760.)))
                        .map(|bounds| (bounds, window.display(cx).map(|display| display.id())))
                })
                .and_then(|result| result);
            let (bounds, display_id) = match placement {
                Ok(placement) => placement,
                Err(error) => {
                    workspace.update(cx, |workspace, cx| {
                        workspace.error = Some(format!(
                            "无法打开任务窗口 / Cannot open task window: {error:#}"
                        ));
                        cx.notify();
                    });
                    return;
                }
            };
            let owner = workspace.clone();
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    display_id,
                    kind: WindowKind::PopUp,
                    titlebar: Some(TitlebarOptions {
                        title: Some("任务详情 · FourQuadrants".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(12.))),
                    }),
                    window_min_size: Some(size(
                        px(820.).min(bounds.size.width),
                        px(700.).min(bounds.size.height),
                    )),
                    window_background: WindowBackgroundAppearance::Transparent,
                    is_minimizable: false,
                    ..Default::default()
                },
                move |window, cx| {
                    let handle = window.window_handle();
                    let weak = owner.downgrade();
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            if let Err(error) = crate::native::configure_task_dialog(window) {
                                let _ = weak.update(cx, |workspace, cx| {
                                    workspace.error = Some(format!("{error:#}"));
                                    cx.notify();
                                });
                            }
                        });
                    });
                    owner.update(cx, |workspace, cx| {
                        workspace.task_dialog = Some(window.window_handle());
                        if let Some(task) = workspace.doc.task(&id) {
                            workspace.editor = Some(TaskEditor::new(
                                task,
                                blank_title,
                                &workspace.doc.settings,
                                window,
                                cx,
                            ));
                            workspace.selected = Some(id);
                        }
                        cx.notify();
                    });
                    let view = cx.new(|cx| TaskDialog::new(owner, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            );
            if let Err(error) = result {
                workspace.update(cx, |workspace, cx| {
                    workspace.error = Some(format!(
                        "无法打开任务窗口 / Cannot open task window: {error:#}"
                    ));
                    cx.notify();
                });
            } else {
                cx.activate(true);
            }
        });
    }

    fn clear_task_dialog(&mut self, cx: &mut Context<Self>) {
        self.task_dialog_generation = self.task_dialog_generation.wrapping_add(1);
        self.task_dialog = None;
        self.editor = None;
        self.selected = None;
        let main = self.main_window;
        let focus = self.focus.clone();
        cx.defer(move |cx| {
            let _ = main.update(cx, |_, window, _| {
                window.activate_window();
                focus.focus(window);
            });
        });
        cx.notify();
    }

    pub(super) fn dismiss_task_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.task_dialog;
        self.clear_task_dialog(cx);
        if let Some(handle) = handle {
            if handle == window.window_handle() {
                window.remove_window();
            } else {
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                });
            }
        }
    }

    pub(super) fn close_task_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.save_editor(window, cx) {
            self.dismiss_task_dialog(window, cx);
        }
    }
}
