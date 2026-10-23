use std::rc::Rc;

use crate::{
    AppEvent, DesktopContext, EventHandlerId, HotKeyState, ShortcutHandle, ShortcutRegistryError,
    WindowEvent, assets::*, desktop_state::DesktopAppContext, shortcut::IntoAccelerator, window,
};
use dioxus_core::{Runtime, consume_context, use_hook, use_hook_with_cleanup};

use dioxus_hooks::use_callback;
use wry::RequestAsyncResponder;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::WinitEvent;
#[cfg(any(target_os = "android", target_os = "ios"))]
use crate::ipc::UserWindowEvent;
#[cfg(any(target_os = "android", target_os = "ios"))]
use tao::{event::Event, event_loop::EventLoopWindowTarget};

/// Get an imperative handle to the current window
pub fn use_window() -> DesktopContext {
    use_hook(consume_context::<DesktopContext>)
}

/// Get a handle to the desktop application.
pub fn use_app() -> Rc<DesktopAppContext> {
    use_hook(consume_context::<Rc<DesktopAppContext>>)
}

/// Register a cross-platform application lifecycle event handler.
pub fn use_app_event_handler(
    mut handler: impl FnMut(AppEvent, &Rc<DesktopAppContext>) + 'static,
) -> EventHandlerId {
    use dioxus_core::current_scope_id;

    let runtime = Runtime::current();
    let scope_id = current_scope_id();

    use_hook_with_cleanup(
        move || {
            crate::app().create_app_event_handler(move |event, app| {
                runtime.in_scope(scope_id, || handler(event, app))
            })
        },
        move |handler| handler.remove(),
    )
}

/// Register a cross-platform semantic event handler for the current window.
pub fn use_window_event_handler(
    mut handler: impl FnMut(WindowEvent, &Rc<DesktopAppContext>) + 'static,
) -> EventHandlerId {
    use dioxus_core::current_scope_id;

    // Capture the current runtime and scope ID.
    let runtime = Runtime::current();
    let scope_id = current_scope_id();

    use_hook_with_cleanup(
        move || {
            window().create_window_event_handler(move |event, app| {
                runtime.in_scope(scope_id, || handler(event, app))
            })
        },
        move |handler| handler.remove(),
    )
}

/// Register an event handler for raw desktop Winit events.
///
/// Prefer [`use_app_event_handler`] or [`use_window_event_handler`] when a
/// backend-neutral event is sufficient.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn use_winit_event_handler(
    mut handler: impl for<'a> FnMut(WinitEvent<'a>, &Rc<DesktopAppContext>) + 'static,
) -> EventHandlerId {
    use dioxus_core::current_scope_id;

    let runtime = Runtime::current();
    let scope_id = current_scope_id();

    use_hook_with_cleanup(
        move || {
            window().create_winit_event_handler(move |event, app| {
                runtime.in_scope(scope_id, || handler(event, app))
            })
        },
        move |handler| handler.remove(),
    )
}

/// Register an event handler for raw mobile Tao events.
///
/// Prefer [`use_app_event_handler`] or [`use_window_event_handler`] when a
/// backend-neutral event is sufficient.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn use_tao_event_handler(
    mut handler: impl FnMut(&Event<UserWindowEvent>, &EventLoopWindowTarget<UserWindowEvent>) + 'static,
) -> EventHandlerId {
    use dioxus_core::current_scope_id;

    let runtime = Runtime::current();
    let scope_id = current_scope_id();

    use_hook_with_cleanup(
        move || {
            window().create_tao_event_handler(move |event, target| {
                runtime.in_scope(scope_id, || handler(event, target))
            })
        },
        move |handler| handler.remove(),
    )
}

/// Register an event handler that runs when a muda event is processed.
#[cfg_attr(
    docsrs,
    doc(cfg(any(target_os = "windows", target_os = "linux", target_os = "macos")))
)]
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub fn use_muda_event_handler(
    mut handler: impl FnMut(&muda::MenuEvent) + 'static,
) -> EventHandlerId {
    use dioxus_core::current_scope_id;

    let runtime = Runtime::current();
    let scope_id = current_scope_id();
    use_hook_with_cleanup(
        move || {
            crate::app()
                .event_handlers
                .add_menu(move |event| runtime.in_scope(scope_id, || handler(event)))
        },
        move |handler| handler.remove(),
    )
}

/// Register an event handler that runs when a tray icon menu event is processed.
#[cfg_attr(
    docsrs,
    doc(cfg(any(target_os = "windows", target_os = "linux", target_os = "macos")))
)]
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub fn use_tray_menu_event_handler(
    mut handler: impl FnMut(&tray_icon::menu::MenuEvent) + 'static,
) -> EventHandlerId {
    use dioxus_core::current_scope_id;

    let runtime = Runtime::current();
    let scope_id = current_scope_id();
    use_hook_with_cleanup(
        move || {
            crate::app()
                .event_handlers
                .add_tray_menu(move |event| runtime.in_scope(scope_id, || handler(event)))
        },
        move |handler| handler.remove(),
    )
}

/// Register an event handler that runs when a tray icon event is processed.
/// This is only for tray icon and not it's menus.
/// If you want to register tray icon menus handler use `use_tray_menu_event_handler` instead.
#[cfg_attr(
    docsrs,
    doc(cfg(any(target_os = "windows", target_os = "linux", target_os = "macos")))
)]
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub fn use_tray_icon_event_handler(
    mut handler: impl FnMut(&tray_icon::TrayIconEvent) + 'static,
) -> EventHandlerId {
    use dioxus_core::current_scope_id;

    let runtime = Runtime::current();
    let scope_id = current_scope_id();
    use_hook_with_cleanup(
        move || {
            crate::app()
                .event_handlers
                .add_tray_icon(move |event| runtime.in_scope(scope_id, || handler(event)))
        },
        move |handler| handler.remove(),
    )
}

/// Provide a callback to handle asset loading yourself.
///
/// The callback takes a path as requested by the web view, and it should return `Some(response)`
/// if you want to load the asset, and `None` if you want to fallback on the default behavior.
pub fn use_asset_handler(
    name: &str,
    mut handler: impl FnMut(AssetRequest, RequestAsyncResponder) + 'static,
) {
    // wrap the user's handler in something that keeps it up to date
    let cb = use_callback(move |(asset, responder)| handler(asset, responder));

    use_hook_with_cleanup(
        || {
            crate::window()
                .asset_handlers
                .register_handler(name.to_string(), cb);

            Rc::new(name.to_string())
        },
        move |name| {
            _ = crate::window().asset_handlers.remove_handler(name.as_ref());
        },
    );
}

/// Get a closure that executes any JavaScript in the WebView context.
pub fn use_global_shortcut(
    accelerator: impl IntoAccelerator,
    handler: impl FnMut(HotKeyState) + 'static,
) -> Result<ShortcutHandle, ShortcutRegistryError> {
    // wrap the user's handler in something that keeps it up to date
    let cb = use_callback(handler);

    use_hook_with_cleanup(
        #[allow(clippy::redundant_closure)]
        move || window().create_shortcut(accelerator.accelerator(), move |state| cb(state)),
        |handle| {
            if let Ok(handle) = handle {
                handle.remove();
            }
        },
    )
}
