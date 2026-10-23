#[cfg(any(target_os = "android", target_os = "ios"))]
#[path = "app_tao.rs"]
mod app_tao;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[path = "app_winit.rs"]
mod app_winit;

#[cfg(any(target_os = "android", target_os = "ios"))]
use app_tao as host;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use app_winit as host;

use crate::{
    AppEvent, WindowEvent,
    config::{Config, WindowCloseBehaviour},
    desktop_state::{DesktopAppContext, WindowCloseRequestResult},
    event_handlers::AppEventCallback,
    ipc::{IpcMessage, UserWindowEvent},
    query::QueryResult,
    webview::WebviewInstance,
    windowing::{PhysicalSize, WindowId, surface_size},
};
use dioxus_core::{RenderTargetId, ScopeId, VirtualDom, provide_context};
use futures_util::{FutureExt, pin_mut};
use std::{
    collections::{BTreeMap, HashMap},
    rc::Rc,
    task::Waker,
    time::Duration,
};

/// The single top-level object that manages the event loop, VirtualDom, and running windows.
pub(crate) struct App {
    pub(crate) dom: VirtualDom,
    pub(crate) initial_dom_rebuild_done: bool,

    /// Hot-reload messages that arrived before the initial DOM rebuild. Applying them (and
    /// polling the vdom) before `rebuild` runs corrupts the initial render, so they are queued
    /// and replayed right after the first rebuild.
    #[cfg(all(feature = "devtools", debug_assertions))]
    pub(crate) pending_hot_reload_msgs: Vec<dioxus_devtools::DevserverMsg>,

    pub(crate) exit_on_last_window_close: bool,
    pub(crate) webviews: HashMap<WindowId, WebviewInstance>,
    pub(crate) dom_waker: Waker,
    pub(crate) app_event_handler: Option<AppEventCallback>,
    host: host::State,

    /// App-wide state shared by every desktop window.
    pub(crate) app_context: Rc<DesktopAppContext>,
}

impl App {
    fn from_host(
        mut cfg: Config,
        virtual_dom: VirtualDom,
        dom_waker: Waker,
        app_context: Rc<DesktopAppContext>,
        host: host::State,
    ) -> Self {
        let app = Self {
            exit_on_last_window_close: cfg.exit_on_last_window_close,
            webviews: HashMap::new(),
            dom: virtual_dom,
            initial_dom_rebuild_done: false,
            #[cfg(all(feature = "devtools", debug_assertions))]
            pending_hot_reload_msgs: Vec::new(),
            dom_waker,
            app_event_handler: cfg.app_event_handler.take(),
            host,
            app_context,
        };

        dioxus_html::set_event_converter(Box::new(crate::events::SerializedHtmlEventConverter));
        #[cfg(all(feature = "devtools", debug_assertions))]
        app.connect_hotreload();
        app
    }

    fn dispatch_app_event(&mut self, event: AppEvent) {
        self.app_context
            .event_handlers
            .apply_app(event, &self.app_context);
        if let Some(mut handler) = self.app_event_handler.take() {
            let app_context = self.app_context.clone();
            self.dom.in_runtime(|| handler(event, &app_context));
            self.app_event_handler = Some(handler);
        }
    }

    fn dispatch_window_event(&mut self, window_id: WindowId, event: WindowEvent) {
        self.app_context
            .event_handlers
            .apply_window(window_id, event, &self.app_context);
    }

    #[cfg(all(feature = "devtools", debug_assertions))]
    pub fn connect_hotreload(&self) {
        let proxy = self.app_context.proxy.clone();
        dioxus_devtools::connect(move |msg| {
            _ = proxy.send_event(UserWindowEvent::HotReloadEvent(msg));
        })
    }

    pub fn handle_close_requested(&mut self, id: WindowId) {
        let Some(window) = self.webviews.get(&id) else {
            // If the window is not found, we can just return
            return;
        };

        match window.desktop_context.close_behaviour.get() {
            // If the window is just set to hide when closed, we can just hide it
            WindowCloseBehaviour::WindowHides => {
                window.desktop_context.window.set_visible(false);
            }

            // Component-owned windows render out before native teardown. Root
            // and unowned windows can be dropped here.
            WindowCloseBehaviour::WindowCloses => {
                #[cfg(debug_assertions)]
                self.persist_window_state();

                match window.desktop_context.request_window_close() {
                    WindowCloseRequestResult::DeferredToComponent => {}
                    WindowCloseRequestResult::CloseImmediately => self.destroy_window(id),
                }
            }
        };
    }

    pub fn window_destroyed(&mut self, id: WindowId) {
        self.destroy_window(id);
    }

    /// Tear down one webview after its Dioxus owner has released the target, or
    /// immediately for root/unowned windows.
    pub(crate) fn destroy_window(&mut self, id: WindowId) {
        let Some(app_webview) = self.webviews.remove(&id) else {
            return;
        };
        app_webview.desktop_context.notify_window_destroyed();
        let target_id = app_webview.target_id();
        // A component-owned `Window` reclaims its own target when its portal is
        // torn down, so this is usually a no-op. When the OS destroys a window
        // (or the app is shutting down) before that teardown runs the portal is
        // still mounted; `remove_render_target` leaves such a target in place and
        // it is reclaimed when the runtime is dropped.
        self.dom.runtime().remove_render_target(target_id);

        if self.exit_on_last_window_close && self.webviews.is_empty() {
            self.request_exit();
        } else {
            // Removing the webview drops its `WryQueue`, including any
            // `edits_in_progress` receiver that was gating `poll_vdom`. The
            // webview's pending ack can no longer wake the event loop, so
            // schedule a poll to resume any VDOM work that was waiting on it
            // (for example effects queued by the window's `onclose`).
            self.schedule_poll();
        }
    }

    pub fn resize_window(&self, id: WindowId, size: PhysicalSize<u32>) {
        // TODO: the app layer should avoid directly manipulating the webview webview instance internals.
        // Window creation and modification is the responsibility of the webview instance so it makes sense to
        // encapsulate that there.
        if let Some(app_webview) = self.webviews.get(&id) {
            use wry::Rect;

            _ = app_webview.desktop_context.webview.set_bounds(Rect {
                position: wry::dpi::Position::Logical(wry::dpi::LogicalPosition::new(0.0, 0.0)),
                size: wry::dpi::Size::Physical(wry::dpi::PhysicalSize::new(
                    size.width,
                    size.height,
                )),
            });
        }
    }

    fn provide_app_context(&mut self) {
        let app_context = self.app_context.clone();
        self.dom.in_scope(ScopeId::ROOT, || {
            provide_context(app_context);
        });
    }

    pub fn handle_browser_open(&mut self, msg: IpcMessage) {
        if let Some(temp) = msg.params().as_object() {
            if temp.contains_key("href") {
                if let Some(href) = temp.get("href").and_then(|v| v.as_str()) {
                    if let Err(err) = webbrowser::open(href) {
                        tracing::error!("Failed to open URL: {}", err);
                    }
                }
            }
        }
    }

    /// The webview is finally loaded. Rebuild once, then start polling the
    /// shared VDOM.
    pub fn handle_initialize_msg(&mut self, id: WindowId) {
        if !self.webviews.contains_key(&id) {
            return;
        }

        if !self.initial_dom_rebuild_done {
            self.rebuild_dom();
            self.send_touched_edits();

            #[cfg(all(feature = "devtools", debug_assertions))]
            if self.initial_dom_rebuild_done {
                for msg in std::mem::take(&mut self.pending_hot_reload_msgs) {
                    self.handle_hot_reload_msg(msg);
                }
            }
        }

        self.schedule_poll();
    }

    pub fn handle_query_msg(&mut self, msg: IpcMessage, id: WindowId) {
        let Ok(result) = serde_json::from_value::<QueryResult>(msg.params()) else {
            return;
        };

        let Some(view) = self.webviews.get(&id) else {
            return;
        };

        view.desktop_context.query.send(result);
    }

    #[cfg(all(feature = "devtools", debug_assertions))]
    pub fn handle_hot_reload_msg(&mut self, msg: dioxus_devtools::DevserverMsg) {
        use std::time::Duration;

        use dioxus_devtools::DevserverMsg;

        // Amount of time that toats should be displayed.
        const TOAST_TIMEOUT: Duration = Duration::from_secs(2);
        const TOAST_TIMEOUT_LONG: Duration = Duration::from_secs(3600); // Duration::MAX is too long for JS.

        if !self.initial_dom_rebuild_done {
            self.pending_hot_reload_msgs.push(msg);
            return;
        }

        match msg {
            DevserverMsg::HotReload(hr_msg) => {
                let mut patch_error = None;

                if !self.webviews.is_empty() {
                    {
                        // This is a place where wry says it's threadsafe but it's actually not.
                        // If we're patching the app, we want to make sure it's not going to progress in the interim.
                        #[cfg(target_os = "android")]
                        let _lock = crate::android_sync_lock::android_runtime_lock();

                        if let Err(err) = dioxus_devtools::try_apply_changes(&self.dom, &hr_msg) {
                            tracing::error!("Failed to apply hot-patch: {err}");
                            patch_error = Some(err);
                        }
                    }

                    self.poll_vdom();
                }

                if !hr_msg.assets.is_empty() {
                    for app_webview in self.webviews.values_mut() {
                        app_webview.kick_stylsheets();
                    }
                }

                if hr_msg.jump_table.is_some()
                    && hr_msg.for_build_id == Some(dioxus_cli_config::build_id())
                {
                    match patch_error {
                        None => self.send_toast_to_all(
                            "Hot-patch success!",
                            &format!("App successfully patched in {} ms", hr_msg.ms_elapsed),
                            "success",
                            TOAST_TIMEOUT,
                            false,
                        ),
                        Some(err) => self.send_toast_to_all(
                            "Hot-patch failed!",
                            &format!("Failed to apply hot-patch: {err}"),
                            "error",
                            TOAST_TIMEOUT_LONG,
                            false,
                        ),
                    }
                }
            }
            DevserverMsg::FullReloadCommand => {
                self.send_toast_to_all(
                    "Successfully rebuilt.",
                    "Your app was rebuilt successfully and without error.",
                    "success",
                    TOAST_TIMEOUT,
                    true,
                );
            }
            DevserverMsg::FullReloadStart => self.send_toast_to_all(
                "Your app is being rebuilt.",
                "A non-hot-reloadable change occurred and we must rebuild.",
                "info",
                TOAST_TIMEOUT_LONG,
                false,
            ),
            DevserverMsg::FullReloadFailed => self.send_toast_to_all(
                "Oops! The build failed.",
                "We tried to rebuild your app, but something went wrong.",
                "error",
                TOAST_TIMEOUT_LONG,
                false,
            ),
            DevserverMsg::HotPatchStart => self.send_toast_to_all(
                "Hot-patching app...",
                "Hot-patching modified Rust code.",
                "info",
                TOAST_TIMEOUT_LONG,
                false,
            ),
            DevserverMsg::Shutdown => {
                self.request_exit();
            }
            _ => {}
        }
    }

    #[cfg(all(feature = "devtools", debug_assertions))]
    fn send_toast_to_all(
        &self,
        header_text: &str,
        message: &str,
        level: &str,
        duration: Duration,
        after_reload: bool,
    ) {
        for app_webview in self.webviews.values() {
            app_webview.show_toast(header_text, message, level, duration, after_reload);
        }
    }

    fn schedule_poll(&self) {
        _ = self.app_context.proxy.send_event(UserWindowEvent::Poll);
    }

    /// Poll the shared VirtualDom until it is pending.
    ///
    /// The app-level waker is connected to the event loop, so async work wakes
    /// the app by scheduling another [`UserWindowEvent::Poll`].
    pub fn poll_vdom(&mut self) {
        let dom_waker = self.dom_waker.clone();
        let mut cx = std::task::Context::from_waker(&dom_waker);

        loop {
            if self.poll_webview_queues(&mut cx) {
                return;
            }

            {
                // lock the hack-ed in lock sync wry has some thread-safety issues with event handlers and async tasks
                #[cfg(target_os = "android")]
                let _lock = crate::android_sync_lock::android_runtime_lock();
                let fut = self.dom.wait_for_work();
                pin_mut!(fut);

                match fut.poll_unpin(&mut cx) {
                    std::task::Poll::Ready(_) => {}
                    std::task::Poll::Pending => return,
                }
            }

            // lock the hack-ed in lock sync wry has some thread-safety issues with event handlers
            #[cfg(target_os = "android")]
            let _lock = crate::android_sync_lock::android_runtime_lock();

            self.render_dom_immediate();
            self.send_touched_edits();
        }
    }

    /// Build the writer for one render pass: every webview's `WryQueue` keyed
    /// by its target id, with each queue's `touched` flag cleared so we can
    /// detect which targets receive writes during the pass.
    fn dom_writer(&self) -> BTreeMap<RenderTargetId, crate::edits::WryQueue> {
        self.webviews
            .values()
            .map(|app_webview| {
                app_webview.edits.wry_queue.clear_touched();
                (app_webview.target_id(), app_webview.edits.wry_queue.clone())
            })
            .collect()
    }

    fn rebuild_dom(&mut self) {
        let mut writer = self.dom_writer();
        let has_render_targets = !writer.is_empty();
        self.dom.rebuild(&mut writer);
        if has_render_targets {
            self.initial_dom_rebuild_done = true;
        }
    }

    fn render_dom_immediate(&mut self) {
        let mut writer = self.dom_writer();
        self.dom.render_immediate(&mut writer);
    }

    /// Flush queued edits for every webview whose `WryQueue` was touched during
    /// the preceding render pass. The diff writes directly into each registered
    /// queue (the `WriteMutations` impl on `WryQueue`), and `dom_writer` cleared
    /// every `touched` flag at the start of the pass, so a touched queue means
    /// "this webview received new edits to send".
    fn send_touched_edits(&self) {
        for app_webview in self.webviews.values() {
            if app_webview.edits.wry_queue.is_touched() {
                app_webview.edits.wry_queue.send_edits();
            }
        }
    }

    fn poll_webview_queues(&self, cx: &mut std::task::Context<'_>) -> bool {
        let mut has_pending_edits = false;

        for app_webview in self.webviews.values() {
            if app_webview
                .edits
                .wry_queue
                .poll_new_edits_location(cx)
                .is_ready()
            {
                _ = app_webview
                    .desktop_context
                    .webview
                    .evaluate_script(&format!(
                        "window.interpreter.waitForRequest(\"{edits_path}\", \"{expected_key}\");",
                        edits_path = app_webview.edits.wry_queue.edits_path(),
                        expected_key = app_webview.edits.wry_queue.required_server_key()
                    ));
            }

            if app_webview
                .edits
                .wry_queue
                .poll_edits_flushed(cx)
                .is_pending()
            {
                has_pending_edits = true;
            }
        }

        has_pending_edits
    }

    /// Do our best to preserve state about the window when the event loop is destroyed
    ///
    /// This will attempt to save the window position, size, and monitor into the environment before
    /// closing. This way, when the app is restarted, it can attempt to restore the window to the same
    /// position and size it was in before, making a better DX.
    pub(crate) fn handle_loop_destroyed(&self) {
        #[cfg(debug_assertions)]
        self.persist_window_state();
    }

    #[cfg(debug_assertions)]
    fn persist_window_state(&self) {
        if let Some(app_webview) = self.webviews.values().next() {
            let window = &app_webview.desktop_context.window;

            let Some(monitor) = window.current_monitor() else {
                return;
            };

            let Ok(position) = window.outer_position() else {
                return;
            };
            let (x, y) = if cfg!(target_os = "macos") {
                let position = position.to_logical::<i32>(window.scale_factor());
                (position.x, position.y)
            } else {
                (position.x, position.y)
            };

            let size = surface_size(window.as_ref());

            let (width, height) = if cfg!(target_os = "macos") {
                let size = size.to_logical::<u32>(window.scale_factor());
                (size.width, size.height)
            } else {
                (size.width, size.height)
            };

            let Some(monitor_name) = monitor.name() else {
                return;
            };

            let state = PreservedWindowState {
                x,
                y,
                width: width.max(200),
                height: height.max(200),
                monitor: monitor_name.to_string(),
            };

            // Yes... I know... we're loading a file that might not be ours... but it's a debug feature
            if let Ok(state) = serde_json::to_string(&state) {
                _ = std::fs::write(restore_file(), state);
            }
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreservedWindowState {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) monitor: String,
}

/// Return the location of a tempfile with our window state in it such that we can restore it later
pub(crate) fn restore_file() -> std::path::PathBuf {
    let dir = dioxus_cli_config::session_cache_dir().unwrap_or_else(std::env::temp_dir);
    dir.join("window-state.json")
}
