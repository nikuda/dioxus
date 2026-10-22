#[cfg(target_os = "linux")]
use std::time::Instant;
use std::{
    collections::HashMap,
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

#[cfg(target_os = "linux")]
use winit::event_loop::ControlFlow;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::{Window, WindowAttributes, WindowId},
};
use wry::{Rect, WebView, WebViewBuilder};

const READY_MESSAGE: &str = "winit-host-ready";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ProbeMode {
    #[default]
    SupportedHost,
    #[cfg(target_os = "linux")]
    RejectDirectWayland,
}

impl ProbeMode {
    fn from_args() -> Result<Self, Box<dyn Error>> {
        match std::env::args().nth(1).as_deref() {
            None => Ok(Self::SupportedHost),
            #[cfg(target_os = "linux")]
            Some("--expect-unsupported-wayland") => Ok(Self::RejectDirectWayland),
            Some(argument) => Err(format!("unsupported argument: {argument}").into()),
        }
    }
}

#[derive(Debug)]
enum HostCommand {
    WebViewReady(WindowId),
    TimedOut,
}

#[derive(Clone)]
struct HostCommandSender {
    sender: Sender<HostCommand>,
    proxy: EventLoopProxy,
}

impl HostCommandSender {
    fn send(&self, command: HostCommand) {
        if self.sender.send(command).is_ok() {
            self.proxy.wake_up();
        }
    }
}

struct HostedWebView {
    // Drop the menu while its GTK hierarchy exists, then the WebView before its host window.
    #[cfg(target_os = "linux")]
    _menu: muda::Menu,
    webview: WebView,
    window: Arc<dyn Window>,
}

struct Probe {
    mode: ProbeMode,
    commands: Receiver<HostCommand>,
    command_sender: HostCommandSender,
    windows: HashMap<WindowId, HostedWebView>,
    passed: Arc<AtomicBool>,
    started: bool,
}

impl Probe {
    fn new(
        mode: ProbeMode,
        commands: Receiver<HostCommand>,
        command_sender: HostCommandSender,
        passed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            mode,
            commands,
            command_sender,
            windows: HashMap::new(),
            passed,
            started: false,
        }
    }

    fn create_supported_host(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
    ) -> Result<(), Box<dyn Error>> {
        let attributes = WindowAttributes::default()
            .with_title("Winit + Wry host probe")
            .with_surface_size(LogicalSize::new(640, 360));
        let window: Arc<dyn Window> = event_loop.create_window(attributes)?.into();
        let window_id = window.id();

        #[cfg(target_os = "linux")]
        require_xlib_window(window.as_ref())?;

        let command_sender = self.command_sender.clone();
        let webview = WebViewBuilder::new()
            .with_html(format!(
                "<!doctype html><meta charset=\"utf-8\"><title>Winit host probe</title>\
                 <body><h1>Winit owns this WebView window</h1>\
                 <script>window.ipc.postMessage('{READY_MESSAGE}')</script>"
            ))
            .with_ipc_handler(move |request| {
                if request.body() == READY_MESSAGE {
                    command_sender.send(HostCommand::WebViewReady(window_id));
                }
            })
            .build_as_child(&window)?;

        #[cfg(target_os = "linux")]
        let menu = attach_linux_menu(&webview)?;

        self.windows.insert(
            window_id,
            HostedWebView {
                #[cfg(target_os = "linux")]
                _menu: menu,
                webview,
                window,
            },
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn prove_direct_wayland_is_rejected(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
    ) -> Result<(), Box<dyn Error>> {
        use wry::Error::UnsupportedWindowHandle;

        let window = event_loop.create_window(
            WindowAttributes::default().with_title("Wry direct Wayland rejection probe"),
        )?;
        require_wayland_window(window.as_ref())?;

        match WebViewBuilder::new()
            .with_html("<!doctype html>")
            .build(&window)
        {
            Err(UnsupportedWindowHandle) => {
                eprintln!("PASS: current Wry rejected the direct Wayland window handle");
                Ok(())
            }
            Err(error) => Err(format!("expected UnsupportedWindowHandle, received {error}").into()),
            Ok(_) => Err("current Wry unexpectedly accepted a direct Wayland window".into()),
        }
    }

    fn fail(&mut self, event_loop: &dyn ActiveEventLoop, error: impl std::fmt::Display) {
        eprintln!("FAIL: {error}");
        self.windows.clear();
        event_loop.exit();
    }
}

impl ApplicationHandler for Probe {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;

        let result = match self.mode {
            ProbeMode::SupportedHost => self.create_supported_host(event_loop),
            #[cfg(target_os = "linux")]
            ProbeMode::RejectDirectWayland => self.prove_direct_wayland_is_rejected(event_loop),
        };

        if let Err(error) = result {
            self.fail(event_loop, error);
        } else if self.mode != ProbeMode::SupportedHost {
            self.passed.store(true, Ordering::Release);
            event_loop.exit();
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        while let Ok(command) = self.commands.try_recv() {
            match command {
                HostCommand::WebViewReady(window_id) if self.windows.contains_key(&window_id) => {
                    eprintln!("PASS: Winit hosted Wry and received WebView IPC");
                    self.passed.store(true, Ordering::Release);
                    self.windows.clear();
                    event_loop.exit();
                }
                HostCommand::WebViewReady(window_id) => {
                    self.fail(event_loop, format!("unknown WebView window {window_id:?}"));
                }
                HostCommand::TimedOut => {
                    self.fail(event_loop, "timed out waiting for WebView IPC");
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.windows.remove(&window_id);
                if self.windows.is_empty() {
                    event_loop.exit();
                }
            }
            WindowEvent::SurfaceResized(size) => {
                let Some(host) = self.windows.get(&window_id) else {
                    return;
                };
                let size = size.to_logical::<u32>(host.window.scale_factor());
                if let Err(error) = host.webview.set_bounds(Rect {
                    position: LogicalPosition::new(0, 0).into(),
                    size: LogicalSize::new(size.width, size.height).into(),
                }) {
                    self.fail(event_loop, error);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        #[cfg(target_os = "linux")]
        {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(16),
            ));
        }

        #[cfg(not(target_os = "linux"))]
        let _ = event_loop;
    }

    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.windows.clear();
    }
}

#[cfg(target_os = "linux")]
fn require_xlib_window(window: &dyn Window) -> Result<(), Box<dyn Error>> {
    use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    match window.window_handle()?.as_raw() {
        RawWindowHandle::Xlib(_) => {
            eprintln!("Winit backend: X11");
            Ok(())
        }
        handle => Err(format!("expected an Xlib window handle, received {handle:?}").into()),
    }
}

#[cfg(target_os = "linux")]
fn require_wayland_window(window: &dyn Window) -> Result<(), Box<dyn Error>> {
    use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    match window.window_handle()?.as_raw() {
        RawWindowHandle::Wayland(_) => Ok(()),
        handle => Err(format!("expected a Wayland window handle, received {handle:?}").into()),
    }
}

#[cfg(target_os = "linux")]
fn attach_linux_menu(webview: &WebView) -> Result<muda::Menu, Box<dyn Error>> {
    use gtk::prelude::*;
    use wry::WebViewExtUnix;

    let gtk_webview = webview.webview();
    let container = gtk_webview
        .parent()
        .and_downcast::<gtk::Box>()
        .ok_or("Wry's WebKit widget is not parented by a GTK box")?;
    let window = gtk_webview
        .toplevel()
        .and_downcast::<gtk::Window>()
        .ok_or("Wry's WebKit widget has no GTK toplevel")?;

    let menu = muda::Menu::new();
    menu.append(&muda::MenuItem::new("Winit host probe", false, None))?;
    menu.init_for_gtk_window(&window, Some(&container))?;
    eprintln!("GTK menu: attached");
    Ok(menu)
}

#[cfg(target_os = "linux")]
fn initialize_linux(mode: ProbeMode) -> Result<EventLoop, Box<dyn Error>> {
    use gtk::prelude::DisplayExtManual;

    match mode {
        ProbeMode::SupportedHost => {
            gtk::gdk::set_allowed_backends("x11");
            winit::platform::x11::register_xlib_error_hook(Box::new(|_display, error| {
                let error = error.cast::<x11_dl::xlib::XErrorEvent>();
                // SAFETY: Winit's hook supplies a valid XErrorEvent pointer for this callback.
                unsafe { (*error).error_code == 170 }
            }));
        }
        ProbeMode::RejectDirectWayland => gtk::gdk::set_allowed_backends("wayland"),
    }
    gtk::init()?;

    let display = gtk::gdk::Display::default().ok_or("GDK did not open a display")?;
    let gdk_backend = display.backend();
    let expected_backend = match mode {
        ProbeMode::SupportedHost => gdk_backend.is_x11(),
        ProbeMode::RejectDirectWayland => gdk_backend.is_wayland(),
    };
    if !expected_backend {
        return Err(format!("GDK selected the wrong backend: {gdk_backend:?}").into());
    }
    eprintln!("GDK backend: {gdk_backend:?}");

    let mut builder = EventLoop::builder();
    match mode {
        ProbeMode::SupportedHost => {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }
        ProbeMode::RejectDirectWayland => {
            use winit::platform::wayland::EventLoopBuilderExtWayland;
            builder.with_wayland();
        }
    }
    Ok(builder.build()?)
}

#[cfg(not(target_os = "linux"))]
fn initialize_event_loop(_mode: ProbeMode) -> Result<EventLoop, Box<dyn Error>> {
    Ok(EventLoop::new()?)
}

#[cfg(target_os = "linux")]
fn initialize_event_loop(mode: ProbeMode) -> Result<EventLoop, Box<dyn Error>> {
    initialize_linux(mode)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mode = ProbeMode::from_args()?;
    let event_loop = initialize_event_loop(mode)?;
    let proxy = event_loop.create_proxy();
    let (sender, commands) = mpsc::channel();
    let command_sender = HostCommandSender { sender, proxy };
    if mode == ProbeMode::SupportedHost {
        let timeout_sender = command_sender.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(10));
            timeout_sender.send(HostCommand::TimedOut);
        });
    }
    let passed = Arc::new(AtomicBool::new(false));
    event_loop.run_app(Probe::new(mode, commands, command_sender, passed.clone()))?;
    if passed.load(Ordering::Acquire) {
        Ok(())
    } else {
        Err("probe exited without satisfying its assertion".into())
    }
}
