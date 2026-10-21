use super::*;
use crate::native::{self, UpdateState, UpdateStatus};

impl Workspace {
    pub fn start_updater(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        if enabled {
            let workspace = cx.weak_entity();
            let handle = window.window_handle();
            let mut app = cx.to_async();
            native::set_update_termination_handler(move || {
                handle
                    .update(&mut app, |_, window, cx| {
                        workspace
                            .update(cx, |this, cx| this.prepare_close(window, cx))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            });
        }
        native::initialize_updater(enabled);
        self.update_status = native::updater_status();
        if matches!(
            self.update_status.state,
            UpdateState::Disabled | UpdateState::Unavailable
        ) {
            return;
        }
        self.updater_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        this.refresh_updater(window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn refresh_updater(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut status = native::updater_status();
        if status.state == UpdateState::InstallPending {
            // The same gate as Quit: validation/write failures retain drafts, and
            // open native editors must finish before Sparkle can relaunch.
            let saved = self.prepare_close(window, cx);
            native::resume_update_install(saved);
            status = native::updater_status();
        }
        if status != self.update_status {
            self.update_status = status;
            cx.notify();
        }
    }

    pub(super) fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_status.can_check {
            native::check_for_updates();
            self.refresh_updater(window, cx);
        }
    }
}

impl Default for UpdateStatus {
    fn default() -> Self {
        Self {
            state: UpdateState::Unavailable,
            message: String::new(),
            version: None,
            can_check: false,
        }
    }
}
