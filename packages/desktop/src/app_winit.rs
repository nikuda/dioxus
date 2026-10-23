use super::App;
use crate::{
    AppEvent, WindowEvent, commands,
    config::Config,
    desktop_state::DesktopAppContext,
    event_handlers::{WinitEvent, WinitEventCallback},
    ipc::UserWindowEvent,
    waker::create_dom_waker,
    windowing::WindowId,
};
use dioxus_core::VirtualDom;
use std::{rc::Rc, sync::mpsc::Receiver};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, StartCause, WindowEvent as WinitWindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum DesktopHostError {
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Linux(#[from] crate::linux_host::LinuxHostError),
    #[cfg(not(target_os = "linux"))]
    #[error("failed to initialize the Winit event loop: {0}")]
    Winit(#[from] winit::error::EventLoopError),
}

pub(super) struct State {
    surfaces_initialized: bool,
    exit_requested: bool,
    commands: Receiver<UserWindowEvent>,
    event_handler: Option<WinitEventCallback>,
    float_all: bool,
    show_devtools: bool,
    tray_icon_show_window_on_click: bool,
}

impl App {
    pub fn new(
        mut cfg: Config,
        virtual_dom: VirtualDom,
    ) -> Result<(EventLoop, Self), DesktopHostError> {
        #[cfg(target_os = "linux")]
        let event_loop = crate::linux_host::initialize(
            cfg.event_loop_builder.take().unwrap_or_default(),
            cfg.disable_dma_buf_on_wayland,
        )?;

        #[cfg(not(target_os = "linux"))]
        let event_loop = cfg.event_loop_builder.take().unwrap_or_default().build()?;

        let (proxy, commands) = commands::channel(event_loop.create_proxy());
        let dom_waker = create_dom_waker(proxy.clone());
        let app_context = Rc::new(DesktopAppContext::new(proxy));
        let host = State {
            surfaces_initialized: false,
            exit_requested: false,
            commands,
            event_handler: cfg.winit_event_handler.take(),
            float_all: false,
            show_devtools: false,
            tray_icon_show_window_on_click: cfg.tray_icon_show_window_on_click,
        };
        let app = Self::from_host(cfg, virtual_dom, dom_waker, app_context, host);

        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        app.set_global_hotkey_handler();
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        app.set_menubar_receiver();
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        app.set_tray_icon_receiver();
        #[cfg(debug_assertions)]
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        app.connect_preserve_window_state_handler();

        Ok((event_loop, app))
    }

    pub(crate) fn request_exit(&mut self) {
        if !self.host.exit_requested {
            self.dispatch_app_event(AppEvent::ExitRequested);
            self.host.exit_requested = true;
        }
    }

    fn dispatch_winit_event(&mut self, event: WinitEvent<'_>) {
        self.app_context
            .event_handlers
            .apply_winit(event, &self.app_context);
        if let Some(mut handler) = self.host.event_handler.take() {
            let app_context = self.app_context.clone();
            self.dom.in_runtime(|| handler(event, &app_context));
            self.host.event_handler = Some(handler);
        }
    }

    fn handle_command(&mut self, event_loop: &dyn ActiveEventLoop, event: UserWindowEvent) {
        match event {
            UserWindowEvent::Poll => self.poll_vdom(),
            UserWindowEvent::NewWindow => self.handle_new_window(event_loop),
            UserWindowEvent::RequestWindowClose(id) => self.handle_close_requested(id),
            UserWindowEvent::DestroyWindow(id) => self.destroy_window(id),
            UserWindowEvent::Shutdown => self.request_exit(),

            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            UserWindowEvent::GlobalHotKeyEvent(event) => self.handle_global_hotkey(event),
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            UserWindowEvent::MudaMenuEvent(event) => {
                self.app_context.event_handlers.apply_menu(&event);
                self.handle_menu_event(event);
            }
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            UserWindowEvent::TrayMenuEvent(event) => {
                self.app_context.event_handlers.apply_tray_menu(&event);
                self.handle_tray_menu_event(event);
            }
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            UserWindowEvent::TrayIconEvent(event) => {
                self.app_context.event_handlers.apply_tray_icon(&event);
                self.handle_tray_icon_event(event);
            }

            #[cfg(all(feature = "devtools", debug_assertions))]
            UserWindowEvent::HotReloadEvent(msg) => self.handle_hot_reload_msg(msg),

            UserWindowEvent::WindowsDragDrop(id) => {
                if let Some(app_webview) = self.webviews.get(&id) {
                    _ = app_webview
                        .desktop_context
                        .webview
                        .evaluate_script("window.interpreter.handleWindowsDragDrop();");
                }
            }
            UserWindowEvent::WindowsDragLeave(id) => {
                if let Some(app_webview) = self.webviews.get(&id) {
                    _ = app_webview
                        .desktop_context
                        .webview
                        .evaluate_script("window.interpreter.handleWindowsDragLeave();");
                }
            }
            UserWindowEvent::WindowsDragOver(id, x_pos, y_pos) => {
                if let Some(app_webview) = self.webviews.get(&id) {
                    _ = app_webview
                        .desktop_context
                        .webview
                        .evaluate_script(&format!(
                            "window.interpreter.handleWindowsDragOver({x_pos}, {y_pos});"
                        ));
                }
            }

            UserWindowEvent::Ipc { id, msg } => match msg.method() {
                crate::ipc::IpcMethod::Initialize => self.handle_initialize_msg(id),
                crate::ipc::IpcMethod::UserEvent => {}
                crate::ipc::IpcMethod::Query => self.handle_query_msg(msg, id),
                crate::ipc::IpcMethod::BrowserOpen => self.handle_browser_open(msg),
                crate::ipc::IpcMethod::Other(_) => {}
            },
        }

        if self.host.exit_requested {
            event_loop.exit();
        }
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn handle_global_hotkey(&self, event: global_hotkey::GlobalHotKeyEvent) {
        self.app_context.shortcut_manager.call_handlers(event);
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn handle_menu_event(&mut self, event: muda::MenuEvent) {
        match event.id().0.as_str() {
            "dioxus-float-top" => {
                for app_webview in self.webviews.values() {
                    app_webview
                        .desktop_context
                        .window
                        .set_window_level(if self.host.float_all {
                            winit::window::WindowLevel::AlwaysOnTop
                        } else {
                            winit::window::WindowLevel::Normal
                        });
                }
                self.host.float_all = !self.host.float_all;
            }
            "dioxus-toggle-dev-tools" => {
                self.host.show_devtools = !self.host.show_devtools;
                for app_webview in self.webviews.values() {
                    let webview = &app_webview.desktop_context.webview;
                    if self.host.show_devtools {
                        webview.open_devtools();
                    } else {
                        webview.close_devtools();
                    }
                }
            }
            _ => {}
        }
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn handle_tray_menu_event(&mut self, event: tray_icon::menu::MenuEvent) {
        _ = event;
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn handle_tray_icon_event(&mut self, event: tray_icon::TrayIconEvent) {
        if let tray_icon::TrayIconEvent::Click { button, .. } = event
            && button == tray_icon::MouseButton::Left
            && self.host.tray_icon_show_window_on_click
        {
            for app_webview in self.webviews.values() {
                app_webview.desktop_context.window.set_visible(true);
                app_webview.desktop_context.window.focus_window();
            }
        }
    }

    fn handle_new_window(&mut self, event_loop: &dyn ActiveEventLoop) {
        for pending_webview in self.app_context.drain_pending_webviews() {
            let app_webview =
                pending_webview.create_window(&mut self.dom, &self.app_context, event_loop);
            let id = app_webview.desktop_context.window.id();
            self.webviews.insert(id, app_webview);
        }
    }

    fn handle_start_cause_init(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.provide_app_context();
        self.rebuild_dom();
        self.handle_new_window(event_loop);
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn set_global_hotkey_handler(&self) {
        let receiver = self.app_context.proxy.clone();
        global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(move |event| {
            _ = receiver.send_event(UserWindowEvent::GlobalHotKeyEvent(event));
        }));
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn set_menubar_receiver(&self) {
        let receiver = self.app_context.proxy.clone();
        muda::MenuEvent::set_event_handler(Some(move |event| {
            _ = receiver.send_event(UserWindowEvent::MudaMenuEvent(event));
        }));
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn set_tray_icon_receiver(&self) {
        let receiver = self.app_context.proxy.clone();
        tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
            _ = receiver.send_event(UserWindowEvent::TrayIconEvent(event));
        }));

        let receiver = self.app_context.proxy.clone();
        tray_icon::menu::MenuEvent::set_event_handler(Some(move |event| {
            _ = receiver.send_event(UserWindowEvent::TrayMenuEvent(event));
        }));
    }

    /// Wire up a signal receiver that preserves window state before shutdown.
    #[cfg(debug_assertions)]
    fn connect_preserve_window_state_handler(&self) {
        #[cfg(unix)]
        {
            let target = self.app_context.proxy.clone();
            std::thread::spawn(move || {
                use signal_hook::consts::{SIGINT, SIGTERM};
                let signals = signal_hook::iterator::Signals::new([SIGTERM, SIGINT]);
                if let Ok(mut signals) = signals {
                    for _ in signals.forever() {
                        if target.send_event(UserWindowEvent::Shutdown).is_err() {
                            std::process::exit(0);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            });
        }
    }
}

impl ApplicationHandler for App {
    fn new_events(&mut self, event_loop: &dyn ActiveEventLoop, cause: StartCause) {
        event_loop.set_control_flow(ControlFlow::Wait);
        self.dispatch_winit_event(WinitEvent::NewEvents(cause));
    }

    fn resumed(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.dispatch_winit_event(WinitEvent::Resumed);
        self.dispatch_app_event(AppEvent::Resumed);
    }

    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.dispatch_winit_event(WinitEvent::CanCreateSurfaces);
        if !self.host.surfaces_initialized {
            self.host.surfaces_initialized = true;
            self.handle_start_cause_init(event_loop);
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.dispatch_winit_event(WinitEvent::ProxyWakeUp);
        while let Ok(command) = self.host.commands.try_recv() {
            self.handle_command(event_loop, command);
            if event_loop.exiting() {
                break;
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WinitWindowEvent,
    ) {
        self.dispatch_winit_event(WinitEvent::WindowEvent {
            window_id,
            event: &event,
        });
        if let Some(event) = WindowEvent::from_winit(&event) {
            self.dispatch_window_event(window_id, event);
        }

        match event {
            WinitWindowEvent::CloseRequested => self.handle_close_requested(window_id),
            WinitWindowEvent::Destroyed => self.window_destroyed(window_id),
            WinitWindowEvent::SurfaceResized(new_size) => self.resize_window(window_id, new_size),
            _ => {}
        }

        if self.host.exit_requested {
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        device_id: Option<DeviceId>,
        event: DeviceEvent,
    ) {
        self.dispatch_winit_event(WinitEvent::DeviceEvent {
            device_id,
            event: &event,
        });
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.dispatch_winit_event(WinitEvent::AboutToWait);
        if self.host.exit_requested {
            event_loop.exit();
        }
    }

    fn suspended(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.dispatch_winit_event(WinitEvent::Suspended);
        self.dispatch_app_event(AppEvent::Suspended);
    }

    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.dispatch_winit_event(WinitEvent::DestroySurfaces);
        self.handle_loop_destroyed();
        self.webviews.clear();
        self.host.surfaces_initialized = false;
    }

    fn memory_warning(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.dispatch_winit_event(WinitEvent::MemoryWarning);
        self.dispatch_app_event(AppEvent::MemoryWarning);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.dispatch_app_event(AppEvent::Exiting);
        self.handle_loop_destroyed();
        self.webviews.clear();
    }
}
