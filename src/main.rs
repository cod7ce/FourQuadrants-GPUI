mod app;
mod native;
mod theme;

use four_quadrants::{export, model, store};

use app::*;
use gpui::{prelude::*, *};
use gpui_component::Root;
use gpui_component_assets::Assets as ComponentAssets;
use std::borrow::Cow;

actions!(application, [Quit, CloseWindow]);

struct ApplicationWorkspace {
    view: Entity<Workspace>,
    window: AnyWindowHandle,
}
impl Global for ApplicationWorkspace {}

fn show_workspace(cx: &mut App) {
    let Some(workspace) = cx.try_global::<ApplicationWorkspace>() else {
        return;
    };
    let view = workspace.view.clone();
    let handle = workspace.window;
    let _ = handle.update(cx, |_, window, cx| {
        view.update(cx, |workspace, cx| workspace.show_main_window(window, cx));
    });
}

fn quit_application(cx: &mut App) {
    // This also runs with no visible windows, when native menus cannot route
    // through a focused view. Always keep the draft/native-editor save gate.
    cx.defer(|cx| {
        let Some(workspace) = cx.try_global::<ApplicationWorkspace>() else {
            return;
        };
        let view = workspace.view.clone();
        let handle = workspace.window;
        let ready = handle
            .update(cx, |_, window, cx| {
                view.update(cx, |workspace, cx| workspace.prepare_close(window, cx))
            })
            .unwrap_or(false);
        if ready {
            cx.quit();
        } else {
            show_workspace(cx);
        }
    });
}

#[derive(Clone, Copy, PartialEq, Action)]
#[action(namespace = application, no_json)]
enum MenuCommand {
    Settings,
    Quit,
    ShowWorkspace,
    CloseWindow,
    NewTask,
    PasteTask,
    ExportMarkdown,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    ToggleSidebar,
    ToggleIsland,
    ShowBoard,
    ShowAgenda,
    ShowCalendar,
    ShowNotes,
}

impl MenuCommand {
    fn action(self) -> Box<dyn Action> {
        match self {
            Self::Settings => Box::new(Settings),
            Self::Quit => Box::new(Quit),
            Self::ShowWorkspace => Box::new(ShowBoard),
            Self::CloseWindow => Box::new(CloseWindow),
            Self::NewTask => Box::new(NewTask),
            Self::PasteTask => Box::new(PasteTask),
            Self::ExportMarkdown => Box::new(ExportMarkdown),
            Self::Undo => Box::new(Undo),
            Self::Redo => Box::new(Redo),
            Self::Cut => Box::new(gpui_component::input::Cut),
            Self::Copy => Box::new(gpui_component::input::Copy),
            Self::Paste => Box::new(gpui_component::input::Paste),
            Self::SelectAll => Box::new(gpui_component::input::SelectAll),
            Self::ToggleSidebar => Box::new(ToggleSidebar),
            Self::ToggleIsland => Box::new(ToggleIsland),
            Self::ShowBoard => Box::new(ShowBoard),
            Self::ShowAgenda => Box::new(ShowAgenda),
            Self::ShowCalendar => Box::new(ShowCalendar),
            Self::ShowNotes => Box::new(ShowNotes),
        }
    }
}

fn init_menu_commands(cx: &mut App) {
    // GPUI 0.2.2 resolves native menu actions through NSApp.mainWindow and
    // excludes GPUIPanel. A distinct global action also works when AppKit keeps
    // the main workspace as mainWindow while the task panel receives keyboard input.
    cx.on_action(|command: &MenuCommand, cx| {
        if *command == MenuCommand::Quit {
            quit_application(cx);
            return;
        }
        if *command == MenuCommand::ShowWorkspace {
            cx.defer(show_workspace);
            return;
        }
        let close = *command == MenuCommand::CloseWindow;
        let workspace_command = matches!(
            command,
            MenuCommand::Settings
                | MenuCommand::NewTask
                | MenuCommand::PasteTask
                | MenuCommand::ExportMarkdown
                | MenuCommand::Undo
                | MenuCommand::Redo
                | MenuCommand::ToggleSidebar
                | MenuCommand::ShowBoard
                | MenuCommand::ShowAgenda
                | MenuCommand::ShowCalendar
                | MenuCommand::ShowNotes
        );
        let action = command.action();
        cx.defer(move |cx| {
            for handle in cx.windows() {
                let (is_active, can_handle) = handle
                    .update(cx, |_, window, cx| {
                        (
                            window.is_window_active(),
                            window.is_action_available(action.as_ref(), cx),
                        )
                    })
                    .unwrap_or((false, false));
                if is_active {
                    // Island has no workspace editors or navigation. Fall back
                    // to the retained main window for those commands; keep local
                    // close/toggle and text-editing commands in their own window.
                    if workspace_command && !can_handle {
                        break;
                    }
                    let _ = handle.update(cx, |_, window, cx| {
                        window.dispatch_action(action, cx);
                    });
                    return;
                }
            }
            if !close {
                show_workspace(cx);
            }
            if let Some(workspace) = cx.try_global::<ApplicationWorkspace>() {
                let handle = workspace.window;
                let _ = handle.update(cx, |_, window, cx| window.dispatch_action(action, cx));
            }
        });
    });
    cx.on_action(|_: &Quit, cx| quit_application(cx));
    // The native menu reads these key equivalents, but this context is never
    // mounted in a view. Keyboard events keep their existing input/workspace bindings.
    cx.bind_keys(
        [
            ("cmd-,", MenuCommand::Settings),
            ("cmd-q", MenuCommand::Quit),
            ("cmd-w", MenuCommand::CloseWindow),
            ("cmd-n", MenuCommand::NewTask),
            ("cmd-shift-v", MenuCommand::PasteTask),
            ("cmd-e", MenuCommand::ExportMarkdown),
            ("cmd-z", MenuCommand::Undo),
            ("cmd-shift-z", MenuCommand::Redo),
            ("cmd-x", MenuCommand::Cut),
            ("cmd-c", MenuCommand::Copy),
            ("cmd-v", MenuCommand::Paste),
            ("cmd-a", MenuCommand::SelectAll),
            ("cmd-b", MenuCommand::ToggleSidebar),
            ("cmd-shift-i", MenuCommand::ToggleIsland),
            ("cmd-1", MenuCommand::ShowBoard),
            ("cmd-2", MenuCommand::ShowAgenda),
            ("cmd-3", MenuCommand::ShowCalendar),
            ("cmd-/", MenuCommand::ShowNotes),
        ]
        .map(|(key, command)| KeyBinding::new(key, command, Some("NativeMenu"))),
    );
}

/// Component icons and project artwork embedded in the binary.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == "app-icon.png" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/app-icon.png"
            ))));
        }
        if path == "icons/carry-forward.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/carry-forward.svg"
            ))));
        }
        if path == "icons/sidebar-toggle.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/sidebar-toggle.svg"
            ))));
        }
        if path == "icons/overview.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/overview.svg"
            ))));
        }
        if path == "icons/this-week.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/this-week.svg"
            ))));
        }
        if path == "icons/calendar-dates.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/calendar-dates.svg"
            ))));
        }
        if path == "icons/share.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/share.svg"
            ))));
        }
        if path == "icons/trash.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/trash.svg"
            ))));
        }
        if path == "icons/notes.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/notes.svg"
            ))));
        }
        if path == "icons/pin.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/pin.svg"
            ))));
        }
        ComponentAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = ComponentAssets.list(path)?;
        for asset in [
            "app-icon.png",
            "icons/carry-forward.svg",
            "icons/sidebar-toggle.svg",
            "icons/overview.svg",
            "icons/this-week.svg",
            "icons/calendar-dates.svg",
            "icons/share.svg",
            "icons/notes.svg",
            "icons/trash.svg",
            "icons/pin.svg",
        ] {
            if asset.starts_with(path) {
                paths.push(asset.into());
            }
        }
        Ok(paths)
    }
}

fn main() {
    if !cfg!(target_os = "macos") {
        eprintln!("This version of FourQuadrants supports macOS only.");
        std::process::exit(1);
    }
    if let Err(error) = cli() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn cli() -> anyhow::Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).is_some_and(|arg| arg == "--validate") {
        let path = args
            .get(2)
            .ok_or_else(|| anyhow::anyhow!("Usage: four-quadrants --validate <backup.json>"))?;
        let doc = store::import(std::path::Path::new(path))?;
        println!(
            "Valid: {} tasks, {} tags, {} weekly notes",
            doc.tasks.len(),
            doc.tags.len(),
            doc.week_notes.len()
        );
        return Ok(());
    }
    let alternate_store = args.iter().any(|arg| arg == "--data");
    let (store, doc) = if let Some(i) = args.iter().position(|a| a == "--data") {
        let path = args
            .get(i + 1)
            .ok_or_else(|| anyhow::anyhow!("--data requires a JSON path"))?;
        let store = store::Store::at(std::path::PathBuf::from(path));
        let doc = store.load()?;
        (store, doc)
    } else {
        store::Store::open_default()?
    };
    let application = Application::new().with_assets(Assets);
    // on_reopen runs outside App::update; enter through the window update now
    // so restoration is flushed even when every document window is hidden.
    application.on_reopen(show_workspace);
    application.run(move |cx| {
        native::configure_workspace_reopen();
        native::set_task_complete_sound_enabled(doc.settings.task_completion_sound);
        if let Err(error) = native::set_application_icon() {
            eprintln!("{error:#}");
        }
        gpui_component::init(cx);
        // Quit is handled by the workspace so active drafts and rich-text sessions can block it.
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseWindow, Some("Workspace")),
            KeyBinding::new("cmd-w", CloseWindow, Some("Island")),
            KeyBinding::new("cmd-n", NewTask, Some("Workspace")),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
            KeyBinding::new("cmd-shift-i", ToggleIsland, None),
            KeyBinding::new("cmd-f", Search, Some("Workspace")),
            KeyBinding::new("cmd-,", Settings, Some("Workspace")),
            KeyBinding::new("cmd-z", Undo, Some("Workspace")),
            KeyBinding::new("cmd-shift-z", Redo, Some("Workspace")),
            KeyBinding::new("cmd-left", Previous, Some("Workspace")),
            KeyBinding::new("cmd-right", Next, Some("Workspace")),
            KeyBinding::new("cmd-t", ThisWeek, Some("Workspace")),
            KeyBinding::new("cmd-e", ExportMarkdown, Some("Workspace")),
            KeyBinding::new("escape", ClosePanel, Some("Workspace")),
            KeyBinding::new("cmd-shift-h", ToggleCompleted, Some("Workspace")),
            KeyBinding::new("cmd-1", ShowBoard, Some("Workspace")),
            KeyBinding::new("cmd-2", ShowAgenda, Some("Workspace")),
            KeyBinding::new("cmd-3", ShowCalendar, Some("Workspace")),
            KeyBinding::new("cmd-/", ShowNotes, Some("Workspace")),
            KeyBinding::new("cmd-shift-v", PasteTask, Some("Workspace")),
            KeyBinding::new("cmd-=", IncreaseFont, Some("Workspace")),
            KeyBinding::new("cmd-+", IncreaseFont, Some("Workspace")),
            KeyBinding::new("cmd--", DecreaseFont, Some("Workspace")),
            KeyBinding::new("cmd-0", ResetFont, Some("Workspace")),
            KeyBinding::new("cmd-w", CloseTaskWindow, Some("TaskDialog")),
            KeyBinding::new("escape", CloseTaskWindow, Some("TaskDialog")),
            KeyBinding::new("cmd-enter", CloseTaskWindow, Some("TaskDialog")),
            KeyBinding::new("cmd-enter", CloseTaskWindow, Some("TaskDialog > Input")),
            KeyBinding::new("cmd-s", SaveTask, Some("TaskDialog")),
            KeyBinding::new("cmd-n", NewTask, Some("TaskDialog")),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("TaskDialog")),
            KeyBinding::new("cmd-f", Search, Some("TaskDialog")),
            KeyBinding::new("cmd-left", Previous, Some("TaskDialog")),
            KeyBinding::new("cmd-right", Next, Some("TaskDialog")),
            KeyBinding::new("cmd-t", ThisWeek, Some("TaskDialog")),
            KeyBinding::new("cmd-e", ExportMarkdown, Some("TaskDialog")),
            KeyBinding::new("cmd-shift-h", ToggleCompleted, Some("TaskDialog")),
            KeyBinding::new("cmd-shift-v", PasteTask, Some("TaskDialog")),
            KeyBinding::new("cmd-z", Undo, Some("TaskDialog")),
            KeyBinding::new("cmd-shift-z", Redo, Some("TaskDialog")),
            KeyBinding::new("cmd-,", Settings, Some("TaskDialog")),
            KeyBinding::new("cmd-1", ShowBoard, Some("TaskDialog")),
            KeyBinding::new("cmd-2", ShowAgenda, Some("TaskDialog")),
            KeyBinding::new("cmd-3", ShowCalendar, Some("TaskDialog")),
            KeyBinding::new("cmd-/", ShowNotes, Some("TaskDialog")),
            KeyBinding::new("cmd-=", IncreaseFont, Some("TaskDialog")),
            KeyBinding::new("cmd-+", IncreaseFont, Some("TaskDialog")),
            KeyBinding::new("cmd--", DecreaseFont, Some("TaskDialog")),
            KeyBinding::new("cmd-0", ResetFont, Some("TaskDialog")),
        ]);
        init_menu_commands(cx);
        let en = doc.settings.language == "en";
        cx.set_menus(vec![
            Menu {
                name: "FourQuadrants".into(),
                items: vec![
                    MenuItem::action("Settings…", MenuCommand::Settings),
                    MenuItem::separator(),
                    MenuItem::action(
                        if en {
                            "Quit FourQuadrants"
                        } else {
                            "退出 FourQuadrants"
                        },
                        MenuCommand::Quit,
                    ),
                ],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("New task", MenuCommand::NewTask),
                    MenuItem::action("Paste as task", MenuCommand::PasteTask),
                    MenuItem::action("Export Markdown…", MenuCommand::ExportMarkdown),
                    MenuItem::separator(),
                    MenuItem::action(
                        if en { "Close Window" } else { "关闭窗口" },
                        MenuCommand::CloseWindow,
                    ),
                ],
            },
            Menu {
                name: "Edit".into(),
                items: vec![
                    MenuItem::action("Undo task change", MenuCommand::Undo),
                    MenuItem::action("Redo task change", MenuCommand::Redo),
                    MenuItem::separator(),
                    MenuItem::os_action("Cut", MenuCommand::Cut, OsAction::Cut),
                    MenuItem::os_action("Copy", MenuCommand::Copy, OsAction::Copy),
                    MenuItem::os_action("Paste", MenuCommand::Paste, OsAction::Paste),
                    MenuItem::os_action("Select all", MenuCommand::SelectAll, OsAction::SelectAll),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action(
                        if en {
                            "Show Main Window"
                        } else {
                            "显示主窗口"
                        },
                        MenuCommand::ShowWorkspace,
                    ),
                    MenuItem::separator(),
                    MenuItem::action("Toggle sidebar", MenuCommand::ToggleSidebar),
                    MenuItem::action("Toggle Island", MenuCommand::ToggleIsland),
                    MenuItem::separator(),
                    MenuItem::action("Overview", MenuCommand::ShowBoard),
                    MenuItem::action("Weekly", MenuCommand::ShowAgenda),
                    MenuItem::action("Monthly", MenuCommand::ShowCalendar),
                    MenuItem::action("Notes", MenuCommand::ShowNotes),
                ],
            },
        ]);
        let bounds = Bounds::centered(None, size(px(1380.), px(900.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("FourQuadrants".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.), px(12.))),
                }),
                window_min_size: Some(size(px(520.), px(560.))),
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| Workspace::new(doc, store, window, cx));
                cx.set_global(ApplicationWorkspace {
                    view: view.clone(),
                    window: window.window_handle(),
                });
                view.update(cx, |view, cx| {
                    view.apply_theme(window, cx);
                    view.start_updater(!alternate_store, window, cx);
                    view.sync_island(cx);
                    view.focus.focus(window);
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        );
        if let Err(error) = result {
            eprintln!("Could not open window: {error:#}");
            cx.quit();
        }
        // macOS keeps the process alive after closing windows. Only an explicit
        // Quit action (or OS termination) ends the app, after the save gate.
        cx.activate(true);
    });
    Ok(())
}
