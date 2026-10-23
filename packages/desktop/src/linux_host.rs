use std::{cell::RefCell, os::fd::AsRawFd, rc::Rc, time::Duration};

use gtk::glib::{self, ControlFlow, IOCondition};
use winit::{
    event_loop::{EventLoop, EventLoopBuilder, pump_events::EventLoopExtPumpEvents},
    platform::x11::EventLoopBuilderExtX11,
};

use crate::app::App;

#[derive(Debug, thiserror::Error)]
/// A failure to prepare or initialize Dioxus Desktop's Linux host.
pub enum LinuxHostError {
    /// The X11/XWayland display required by this release is unavailable.
    #[error(
        "Dioxus Desktop requires an X11 or XWayland display in this release; DISPLAY is not set"
    )]
    MissingX11Display,
    /// GTK could not initialize its selected X11 backend.
    #[error("failed to initialize GTK's X11 backend: {0}")]
    Gtk(String),
    /// GTK initialized a backend which current Wry cannot pair with Winit.
    #[error("GTK selected {0}; Dioxus Desktop requires the X11 backend in this release")]
    UnsupportedGdkBackend(String),
    /// Winit could not construct its X11 event loop.
    #[error("failed to initialize Winit's X11 event loop: {0}")]
    Winit(#[from] winit::error::EventLoopError),
    /// GTK initialization was deferred until application workers may exist.
    #[error(
        "the Linux desktop host was not prepared before application threads started; call Config::prepare_linux_host() before launching the application"
    )]
    UnpreparedLinuxHost,
    /// The process environment does not contain the configured WebKit workaround.
    #[error(
        "the WebKit DMA-BUF workaround requires process preparation before application threads start; call Config::prepare_linux_host() or set WEBKIT_DISABLE_DMABUF_RENDERER=1 before launching the process"
    )]
    UnpreparedWebKitEnvironment,
}

pub(crate) fn initialize(
    mut builder: EventLoopBuilder,
    disable_dma_buf: bool,
) -> Result<EventLoop, LinuxHostError> {
    validate_prepared_host(disable_dma_buf)?;
    register_xlib_error_hook();

    builder.with_x11();
    Ok(builder.build()?)
}

fn validate_prepared_host(disable_dma_buf: bool) -> Result<(), LinuxHostError> {
    use gtk::prelude::DisplayExtManual;

    if !gtk::is_initialized_main_thread() {
        return Err(LinuxHostError::UnpreparedLinuxHost);
    }
    if !webkit_environment_prepared(disable_dma_buf) {
        return Err(LinuxHostError::UnpreparedWebKitEnvironment);
    }

    let display = gtk::gdk::Display::default().ok_or(LinuxHostError::MissingX11Display)?;
    let backend = display.backend();
    if !backend.is_x11() {
        return Err(LinuxHostError::UnsupportedGdkBackend(format!(
            "{backend:?}"
        )));
    }
    tracing::debug!(
        gdk_backend = ?backend,
        winit_backend = "x11",
        "initialized the Dioxus Desktop Linux host"
    );

    Ok(())
}

fn register_xlib_error_hook() {
    winit::platform::x11::register_xlib_error_hook(Box::new(|_display, error| {
        let error = error.cast::<x11_dl::xlib::XErrorEvent>();
        // SAFETY: Winit supplies a valid XErrorEvent pointer. Wry's X11 child
        // teardown can report GLXBadWindow (170) after the child is gone.
        unsafe { (*error).error_code == 170 }
    }));
}

/// Prepare the supported GTK backend and apply process-environment workarounds.
///
/// # Safety
///
/// The caller must run this on the process main thread and ensure no other
/// thread can read or write the process environment while it runs. It must
/// also run before GTK, Winit, or WebKit is initialized.
pub(crate) unsafe fn prepare(disable_dma_buf: bool) -> Result<(), LinuxHostError> {
    if gtk::is_initialized() {
        return validate_prepared_host(disable_dma_buf);
    }
    if std::env::var_os("DISPLAY").is_none() {
        return Err(LinuxHostError::MissingX11Display);
    }

    gtk::gdk::set_allowed_backends("x11");
    if dma_buf_workaround_required(disable_dma_buf) {
        // SAFETY: upheld by the caller contract above.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
    gtk::init().map_err(|error| LinuxHostError::Gtk(error.to_string()))?;
    validate_prepared_host(disable_dma_buf)
}

fn dma_buf_workaround_required(disable_dma_buf: bool) -> bool {
    dma_buf_workaround_required_for(
        disable_dma_buf,
        std::path::Path::new("/dev/dri").exists(),
        std::env::var_os("XDG_SESSION_TYPE").as_deref(),
    )
}

fn webkit_environment_prepared(disable_dma_buf: bool) -> bool {
    webkit_environment_prepared_for(
        disable_dma_buf,
        std::path::Path::new("/dev/dri").exists(),
        std::env::var_os("XDG_SESSION_TYPE").as_deref(),
        std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").as_deref(),
    )
}

fn dma_buf_workaround_required_for(
    disable_dma_buf: bool,
    dri_available: bool,
    session_type: Option<&std::ffi::OsStr>,
) -> bool {
    disable_dma_buf && dri_available && session_type == Some(std::ffi::OsStr::new("wayland"))
}

fn webkit_environment_prepared_for(
    disable_dma_buf: bool,
    dri_available: bool,
    session_type: Option<&std::ffi::OsStr>,
    dma_buf_renderer: Option<&std::ffi::OsStr>,
) -> bool {
    !dma_buf_workaround_required_for(disable_dma_buf, dri_available, session_type)
        || dma_buf_renderer == Some(std::ffi::OsStr::new("1"))
}

/// Drive Winit from GLib's blocking main loop without a periodic polling timer.
///
/// Winit exposes a pollable file descriptor on Linux specifically for external
/// event-loop integration. Registering it as a GLib source lets GTK/WebKit and
/// Winit share one main thread; Winit callbacks still run synchronously through
/// `ApplicationHandler`, and its proxy wakes the registered descriptor.
pub(crate) fn run(mut event_loop: EventLoop, mut app: App) -> ! {
    use winit::event_loop::pump_events::PumpStatus;

    struct Host {
        // Drop application WebViews/windows before the event-loop display.
        app: App,
        event_loop: EventLoop,
        exit_code: i32,
    }

    if let PumpStatus::Exit(code) = event_loop.pump_app_events(Some(Duration::ZERO), &mut app) {
        drop(app);
        drop(event_loop);
        std::process::exit(code);
    }

    let raw_fd = event_loop.as_raw_fd();
    let host = Rc::new(RefCell::new(Host {
        app,
        event_loop,
        exit_code: 0,
    }));
    let weak_host = Rc::downgrade(&host);

    let _source = glib::source::unix_fd_add_local(
        raw_fd,
        IOCondition::IN | IOCondition::ERR | IOCondition::HUP,
        move |_, _| {
            let Some(host) = weak_host.upgrade() else {
                return ControlFlow::Break;
            };
            // GTK teardown may briefly iterate the same GLib context while a
            // Winit callback is still active. The outer pump remains the
            // owner; ignore that nested notification and let GLib retry it.
            let Ok(mut host) = host.try_borrow_mut() else {
                return ControlFlow::Continue;
            };
            let Host {
                app,
                event_loop,
                exit_code,
            } = &mut *host;
            if let PumpStatus::Exit(code) = event_loop.pump_app_events(Some(Duration::ZERO), app) {
                *exit_code = code;
                gtk::main_quit();
                ControlFlow::Break
            } else {
                ControlFlow::Continue
            }
        },
    );

    gtk::main();
    let exit_code = host.borrow().exit_code;
    drop(host);
    std::process::exit(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dma_buf_workaround_only_applies_to_enabled_wayland_dri_hosts() {
        let wayland = Some(std::ffi::OsStr::new("wayland"));

        assert!(dma_buf_workaround_required_for(true, true, wayland));
        assert!(!dma_buf_workaround_required_for(false, true, wayland));
        assert!(!dma_buf_workaround_required_for(true, false, wayland));
        assert!(!dma_buf_workaround_required_for(
            true,
            true,
            Some(std::ffi::OsStr::new("x11")),
        ));
    }

    #[test]
    fn required_dma_buf_workaround_must_be_prepared_before_host_initialization() {
        let wayland = Some(std::ffi::OsStr::new("wayland"));

        assert!(!webkit_environment_prepared_for(true, true, wayland, None,));
        assert!(webkit_environment_prepared_for(
            true,
            true,
            wayland,
            Some(std::ffi::OsStr::new("1")),
        ));
        assert!(webkit_environment_prepared_for(false, true, wayland, None,));
    }
}
