use crate::{model::Document, store};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn fq_updater_initialize(enabled: bool);
    fn fq_updater_check();
    fn fq_updater_status() -> *mut std::ffi::c_char;
    fn fq_updater_resume_install(allowed: bool);
    fn fq_updater_set_termination_handler(handler: extern "C" fn() -> bool);
    fn fq_updater_workspace_closed();
    fn free(pointer: *mut std::ffi::c_void);
    fn fq_set_application_icon(bytes: *const u8, length: usize) -> i32;
    fn fq_reduce_motion() -> i32;
    fn fq_prepare_task_complete_sound(bytes: *const u8, length: usize);
    fn fq_play_task_complete_sound();
    fn fq_configure_window_material(
        view: *mut std::ffi::c_void,
        enabled: i32,
        appearance: i32,
        blur_radius: f64,
    ) -> i32;
    fn fq_task_dialog_bounds(
        view: *mut std::ffi::c_void,
        width: f64,
        height: f64,
        bounds: *mut f64,
    ) -> i32;
    fn fq_configure_task_dialog(view: *mut std::ffi::c_void) -> i32;
}

pub const RELEASES_URL: &str = "https://github.com/cod7ce/FourQuadrants-GPUI/releases";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    Ready,
    Checking,
    Available,
    Current,
    Error,
    Disabled,
    Unavailable,
    InstallPending,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct UpdateStatus {
    pub state: UpdateState,
    pub message: String,
    pub version: Option<String>,
    pub can_check: bool,
}

pub fn prepare_task_complete_sound() {
    #[cfg(target_os = "macos")]
    // Copy the embedded PCM at startup; decoding and output preparation run off the UI thread.
    unsafe {
        let sound = include_bytes!("../assets/audio/task-complete.aiff");
        fq_prepare_task_complete_sound(sound.as_ptr(), sound.len());
    };
}

pub fn play_task_complete_sound() {
    #[cfg(target_os = "macos")]
    // Playback is queued behind startup preparation without blocking GPUI's main thread.
    unsafe {
        fq_play_task_complete_sound();
    };
}

pub fn reduce_motion() -> bool {
    #[cfg(target_os = "macos")]
    // Read on GPUI's main thread at each trigger so preference changes take effect.
    unsafe {
        fq_reduce_motion() != 0
    }
    #[cfg(not(target_os = "macos"))]
    false
}

pub fn initialize_updater(enabled: bool) {
    #[cfg(target_os = "macos")]
    // AppKit and Sparkle are initialized on GPUI's main thread.
    unsafe {
        fq_updater_initialize(enabled)
    };
    #[cfg(not(target_os = "macos"))]
    let _ = enabled;
}

pub fn check_for_updates() {
    #[cfg(target_os = "macos")]
    unsafe {
        fq_updater_check()
    };
}

pub fn resume_update_install(allowed: bool) {
    #[cfg(target_os = "macos")]
    unsafe {
        fq_updater_resume_install(allowed)
    };
    #[cfg(not(target_os = "macos"))]
    let _ = allowed;
}

thread_local! {
    static UPDATE_TERMINATION_HANDLER: std::cell::RefCell<Option<Box<dyn FnMut() -> bool>>> =
        std::cell::RefCell::new(None);
}

extern "C" fn prepare_update_termination() -> bool {
    // AppKit enters from outside GPUI's borrow. Unexpected reentrancy or a stale
    // window must cancel termination, and no Rust panic may cross the C ABI.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        UPDATE_TERMINATION_HANDLER.with(|handler| {
            let Ok(mut handler) = handler.try_borrow_mut() else {
                return false;
            };
            handler.as_mut().is_some_and(|handler| handler())
        })
    }))
    .unwrap_or(false)
}

pub fn set_update_termination_handler(handler: impl FnMut() -> bool + 'static) {
    UPDATE_TERMINATION_HANDLER.with(|slot| *slot.borrow_mut() = Some(Box::new(handler)));
    #[cfg(target_os = "macos")]
    unsafe {
        fq_updater_set_termination_handler(prepare_update_termination)
    };
}

pub fn updater_workspace_closed() {
    #[cfg(target_os = "macos")]
    unsafe {
        fq_updater_workspace_closed()
    };
}

pub fn updater_status() -> UpdateStatus {
    #[cfg(target_os = "macos")]
    {
        // The bridge returns a malloc-owned UTF-8 JSON snapshot. Copy/parse it
        // before freeing, so no borrowed native memory escapes this function.
        let pointer = unsafe { fq_updater_status() };
        if !pointer.is_null() {
            let result =
                serde_json::from_slice(unsafe { std::ffi::CStr::from_ptr(pointer).to_bytes() });
            unsafe { free(pointer.cast()) };
            return result.unwrap_or_else(|error| UpdateStatus {
                state: UpdateState::Error,
                message: format!("无法读取更新状态 / Could not read update status: {error}"),
                version: None,
                can_check: false,
            });
        }
    }
    UpdateStatus {
        state: UpdateState::Unavailable,
        message: "无法初始化更新组件 / The updater is unavailable".into(),
        version: None,
        can_check: false,
    }
}

#[cfg(test)]
mod updater_tests {
    use super::{UPDATE_TERMINATION_HANDLER, prepare_update_termination};

    #[test]
    fn termination_preflight_fails_closed_without_crossing_the_ffi_boundary() {
        UPDATE_TERMINATION_HANDLER.with(|slot| *slot.borrow_mut() = None);
        assert!(!prepare_update_termination());
        UPDATE_TERMINATION_HANDLER.with(|slot| *slot.borrow_mut() = Some(Box::new(|| true)));
        assert!(prepare_update_termination());
        UPDATE_TERMINATION_HANDLER.with(|slot| {
            let _active = slot.borrow_mut();
            assert!(!prepare_update_termination());
        });
        UPDATE_TERMINATION_HANDLER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(|| panic!("simulated stale GPUI borrow")));
        });
        assert!(!prepare_update_termination());
        UPDATE_TERMINATION_HANDLER.with(|slot| *slot.borrow_mut() = None);
    }
}

pub fn set_application_icon() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let bytes = include_bytes!("../assets/app-icon.png");
        // Called on the main thread after NSApplication exists; AppKit copies the bytes.
        if unsafe { fq_set_application_icon(bytes.as_ptr(), bytes.len()) } == 0 {
            bail!("无法设置应用图标 / Could not set application icon");
        }
    }
    Ok(())
}

pub fn configure_window_material(
    window: &mut gpui::Window,
    settings: &crate::model::Settings,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let (enabled, blur_radius) = match &settings.window_effects {
            Some(effects) => (effects.transparency > 0., f64::from(effects.blur_radius)),
            None => (settings.glass, -1.),
        };
        window.set_background_appearance(if enabled {
            gpui::WindowBackgroundAppearance::Transparent
        } else {
            gpui::WindowBackgroundAppearance::Opaque
        });
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("Cannot access native window: {error}"))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            bail!("Expected an AppKit window");
        };
        let appearance = match settings.theme_mode() {
            "system" => -1,
            "light" => 0,
            _ => 1,
        };
        // GPUI calls this on the main thread; the borrowed NSView remains alive for
        // this synchronous call. The bridge retains its own material in AppKit.
        let installed = unsafe {
            fq_configure_window_material(
                handle.ns_view.as_ptr(),
                i32::from(enabled),
                appearance,
                blur_radius,
            )
        };
        if installed == 0 {
            // Keep the legacy fallback, but never replace an explicit radius with
            // a fixed blur while claiming the user's requested value was applied.
            if enabled && settings.window_effects.is_none() {
                window.set_background_appearance(gpui::WindowBackgroundAppearance::Blurred);
            }
            bail!("无法应用窗口透明与模糊效果 / Could not apply window transparency and blur");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, settings);
    Ok(())
}

/// Bounds in the parent's display-local, top-left coordinate system. Open the
/// dialog on that same display so GPUI interprets the coordinates correctly.
pub fn task_dialog_bounds(
    parent: &gpui::Window,
    desired: gpui::Size<gpui::Pixels>,
) -> Result<gpui::Bounds<gpui::Pixels>> {
    #[cfg(target_os = "macos")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = HasWindowHandle::window_handle(parent)
            .map_err(|error| anyhow::anyhow!("Cannot access native window: {error}"))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            bail!("Expected an AppKit window");
        };
        let mut bounds = [0.; 4];
        // The borrowed NSView is used synchronously; only copied geometry returns.
        let resolved = unsafe {
            fq_task_dialog_bounds(
                handle.ns_view.as_ptr(),
                f64::from(desired.width),
                f64::from(desired.height),
                bounds.as_mut_ptr(),
            )
        };
        if resolved == 0 {
            bail!("无法确定任务窗口位置 / Could not determine task dialog bounds");
        }
        Ok(gpui::Bounds::new(
            gpui::point(gpui::px(bounds[0] as f32), gpui::px(bounds[1] as f32)),
            gpui::size(gpui::px(bounds[2] as f32), gpui::px(bounds[3] as f32)),
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (parent, desired);
        bail!("Task dialogs require macOS");
    }
}

/// Configures and activates an independent GPUI `WindowKind::PopUp` panel.
pub fn configure_task_dialog(window: &gpui::Window) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow::anyhow!("Cannot access native window: {error}"))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            bail!("Expected an AppKit window");
        };
        // The bridge keeps GPUI's panel class, delegate and render view intact.
        if unsafe { fq_configure_task_dialog(handle.ns_view.as_ptr()) } == 0 {
            bail!("无法配置任务窗口 / Could not configure task dialog");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
    Ok(())
}

fn helper() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let bundled = dir.join("four-quadrants-helper");
        if bundled.is_file() {
            return Ok(bundled);
        }
    }
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/native/four-quadrants-helper");
    if path.is_file() {
        Ok(path)
    } else {
        bail!("原生组件尚未构建，请运行 bash scripts/build-native.sh / Native helper is not built")
    }
}

fn run(args: &[&std::ffi::OsStr]) -> Result<String> {
    let output = Command::new(helper()?)
        .args(args)
        .output()
        .context("启动 macOS 原生组件失败")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("four-quadrants-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir)?;
        Ok(Self(dir))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn import_store(path: &Path) -> Result<Document> {
    let temp = Temporary::new()?;
    let output = temp.0.join("export.json");
    run(&[
        "export-store".as_ref(),
        path.as_os_str(),
        output.as_os_str(),
    ])?;
    store::import(&output)
}

pub fn edit_rich(
    content: Option<String>,
    title: String,
    recovery_dir: PathBuf,
) -> Result<Option<String>> {
    let temp = Temporary::new()?;
    let input = temp.0.join("input.rtfd");
    // The helper saves directly to durable storage. Closing the window, a
    // failed main-document save, or quitting the GPUI process must not discard it.
    std::fs::create_dir_all(&recovery_dir)
        .context("无法创建富文本恢复目录 / Cannot create recovery folder")?;
    let output = recovery_dir.join(format!("{}.rtfd", uuid::Uuid::new_v4()));
    if let Some(content) = content {
        std::fs::write(
            &input,
            STANDARD
                .decode(content)
                .context("富文本数据损坏，未覆盖原内容")?,
        )?;
    }
    run(&[
        "edit-rich".as_ref(),
        input.as_os_str(),
        output.as_os_str(),
        title.as_ref(),
    ])
    .with_context(|| format!("富文本恢复文件 / Recovery file: {}", output.display()))?;
    if output.is_file() {
        Ok(Some(STANDARD.encode(std::fs::read(&output).with_context(
            || {
                format!(
                    "无法读取恢复文件 / Cannot read recovery file: {}",
                    output.display()
                )
            },
        )?)))
    } else {
        Ok(None)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertedNote {
    pub plain_text: String,
    pub source: String,
    pub attachments: Vec<crate::model::NoteAttachment>,
}

/// Converts a copy without changing or opening the original rich document.
pub fn convert_note(content: &str) -> Result<ConvertedNote> {
    let temp = Temporary::new()?;
    let input = temp.0.join("note.rtfd");
    let output = temp.0.join("note.json");
    std::fs::write(
        &input,
        STANDARD
            .decode(content)
            .context("笔记富文本数据损坏 / Invalid rich note data")?,
    )?;
    run(&[
        "note-markdown".as_ref(),
        input.as_os_str(),
        output.as_os_str(),
    ])?;
    serde_json::from_slice(&std::fs::read(output)?)
        .context("无法读取笔记转换结果 / Invalid converted note")
}

pub fn sync_reminders(path: &Path) -> Result<String> {
    static SYNC: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SYNC
        .lock()
        .map_err(|_| anyhow::anyhow!("Reminder synchronization lock failed"))?;
    let raw = run(&["sync-reminders".as_ref(), path.as_os_str()])?;
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    Ok(format!(
        "提醒已同步 / Reminders synced: {}",
        value["scheduled"].as_u64().unwrap_or(0)
    ))
}

pub fn prepare_background(input: &Path, output: &Path, blur: f32) -> Result<()> {
    run(&[
        "prepare-background".as_ref(),
        input.as_os_str(),
        output.as_os_str(),
        blur.to_string().as_ref(),
    ])?;
    Ok(())
}

pub fn parse_input(
    settings: &crate::model::Settings,
    text: &str,
) -> Result<crate::model::ParsedTaskInput> {
    let temp = Temporary::new()?;
    let rules = temp.0.join("rules.json");
    let input = temp.0.join("text.txt");
    let output = temp.0.join("parsed.json");
    std::fs::write(&rules, serde_json::to_vec(settings)?)?;
    std::fs::write(&input, text)?;
    run(&[
        "parse-input".as_ref(),
        rules.as_os_str(),
        input.as_os_str(),
        output.as_os_str(),
    ])?;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(output)?)?;
    Ok(crate::model::ParsedTaskInput {
        title: value["title"].as_str().unwrap_or("").to_string(),
        issue_key: value["issueKey"].as_str().map(str::to_string),
        links: value["links"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        tag_name: value["tagName"]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_string),
    })
}
