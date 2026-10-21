mod app;
mod native;
mod theme;

use four_quadrants::{export, model, store};

use app::*;
use gpui::{prelude::*, *};
use gpui_component::Root;
use gpui_component_assets::Assets as ComponentAssets;
use std::borrow::Cow;

actions!(application, [Quit]);

#[derive(Clone, Copy, PartialEq, Action)]
#[action(namespace = application, no_json)]
enum MenuCommand {
    Settings,
    Quit,
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
        let action = command.action();
        cx.defer(move |cx| {
            for handle in cx.windows() {
                let is_active = handle
                    .update(cx, |_, window, _| window.is_window_active())
                    .unwrap_or(false);
                if is_active {
                    let _ = handle.update(cx, |_, window, cx| {
                        window.dispatch_action(action, cx);
                    });
                    break;
                }
            }
        });
    });
    // The native menu reads these key equivalents, but this context is never
    // mounted in a view. Keyboard events keep their existing input/workspace bindings.
    cx.bind_keys(
        [
            ("cmd-,", MenuCommand::Settings),
            ("cmd-q", MenuCommand::Quit),
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
    Application::new().with_assets(Assets).run(move |cx| {
        if let Err(error) = native::set_application_icon() {
            eprintln!("{error:#}");
        }
        gpui_component::init(cx);
        // Quit is handled by the workspace so active drafts and rich-text sessions can block it.
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-n", NewTask, Some("Workspace")),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
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
        cx.set_menus(vec![
            Menu {
                name: "FourQuadrants".into(),
                items: vec![
                    MenuItem::action("Settings…", MenuCommand::Settings),
                    MenuItem::separator(),
                    MenuItem::action("Quit", MenuCommand::Quit),
                ],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("New task", MenuCommand::NewTask),
                    MenuItem::action("Paste as task", MenuCommand::PasteTask),
                    MenuItem::action("Export Markdown…", MenuCommand::ExportMarkdown),
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
                    MenuItem::action("Toggle sidebar", MenuCommand::ToggleSidebar),
                    MenuItem::separator(),
                    MenuItem::action("Overview", MenuCommand::ShowBoard),
                    MenuItem::action("Week agenda", MenuCommand::ShowAgenda),
                    MenuItem::action("Calendar", MenuCommand::ShowCalendar),
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
                view.update(cx, |view, cx| {
                    view.apply_theme(window, cx);
                    view.start_updater(!alternate_store, window, cx);
                    view.focus.focus(window);
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        );
        if let Err(error) = result {
            eprintln!("Could not open window: {error:#}");
            cx.quit();
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
    Ok(())
}
