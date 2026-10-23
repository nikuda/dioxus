use crate::{
    AppEvent, EventHandlerId, WindowConfig,
    assets::AssetHandlerRegistry,
    commands::DesktopEventLoopProxy,
    edits::EditWebsocket,
    event_handlers::EventHandlers,
    file_upload::NativeFileHover,
    ipc::UserWindowEvent,
    query::QueryEngine,
    shortcut::ShortcutRegistry,
    webview::PendingWebview,
    windowing::{HostWindow, WindowId},
};
use dioxus_core::{RenderTargetId, Runtime};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};
use wry::WebView;

#[cfg(any(target_os = "android", target_os = "ios"))]
use tao::event_loop::EventLoopWindowTarget;

/// App-wide desktop state shared by every desktop window.
pub struct DesktopAppContext {
    pub(crate) proxy: DesktopEventLoopProxy,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) target: EventLoopWindowTarget<UserWindowEvent>,
    pub(crate) event_handlers: EventHandlers,
    pub(crate) shortcut_manager: ShortcutRegistry,
    pub(crate) websocket: EditWebsocket,
    pending_webviews: RefCell<Vec<PendingWebview>>,
}

struct ComponentWindowCallbacks {
    on_close_requested: Box<dyn FnMut()>,
    on_destroyed: Box<dyn FnMut()>,
}

/// Result of asking the component tree to close a component-owned window.
pub(crate) enum WindowCloseRequestResult {
    /// A mounted [`Window`](crate::Window) component will remove the portal before native teardown.
    DeferredToComponent,

    /// No mounted component owns this native window, so the app can close it immediately.
    CloseImmediately,
}

impl DesktopAppContext {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub(crate) fn new(proxy: DesktopEventLoopProxy) -> Self {
        Self {
            proxy,
            event_handlers: EventHandlers::default(),
            shortcut_manager: ShortcutRegistry::new(),
            websocket: EditWebsocket::start(),
            pending_webviews: RefCell::new(Vec::new()),
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn new(
        proxy: DesktopEventLoopProxy,
        target: EventLoopWindowTarget<UserWindowEvent>,
    ) -> Self {
        Self {
            proxy,
            target,
            event_handlers: EventHandlers::default(),
            shortcut_manager: ShortcutRegistry::new(),
            websocket: EditWebsocket::start(),
            pending_webviews: RefCell::new(Vec::new()),
        }
    }

    /// Queue a new desktop window from any Dioxus scope.
    pub fn new_window(self: &Rc<Self>, cfg: WindowConfig) -> crate::PendingDesktopWindow {
        let target_id = Runtime::current().create_render_target();
        let (window, context) = PendingWebview::new(target_id, cfg);

        self.queue_pending_webview(window);
        self.proxy.send_event(UserWindowEvent::NewWindow).unwrap();

        context
    }

    /// Shut down the desktop application.
    pub fn shutdown(&self) {
        _ = self.proxy.send_event(UserWindowEvent::Shutdown);
    }

    pub(crate) fn queue_pending_webview(&self, window: PendingWebview) {
        self.pending_webviews.borrow_mut().push(window);
    }

    pub(crate) fn drain_pending_webviews(&self) -> Vec<PendingWebview> {
        self.pending_webviews.borrow_mut().drain(..).collect()
    }

    /// Create an application lifecycle event handler.
    ///
    /// The returned id can be used with [`Self::remove_event_handler`].
    pub fn create_app_event_handler(
        &self,
        handler: impl FnMut(AppEvent, &Rc<DesktopAppContext>) + 'static,
    ) -> EventHandlerId {
        self.event_handlers.add_app(handler)
    }

    /// Remove an event handler created through this application context.
    pub fn remove_event_handler(&self, id: EventHandlerId) {
        self.event_handlers.remove(id)
    }
}

/// Native-window state exposed through [`DesktopContext`](crate::DesktopContext).
///
/// Dereferences to the underlying native window, so window-manipulation methods such as
/// `set_minimized`, `set_resizable`, or `request_redraw` can be called directly on a
/// [`DesktopContext`](crate::DesktopContext).
pub struct DesktopWindowContext {
    /// The underlying webview handle.
    pub webview: WebView,

    #[cfg(target_os = "linux")]
    _gtk_teardown: GtkTeardown,

    /// The Winit window handle used by supported desktop hosts.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub window: Arc<dyn winit::window::Window>,

    /// The Tao window retained by the current mobile host.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub window: Arc<tao::window::Window>,
    pub(crate) target_id: RenderTargetId,
    pub(crate) asset_handlers: AssetHandlerRegistry,
    pub(crate) file_hover: NativeFileHover,
    pub(crate) query: QueryEngine,
    pub(crate) close_behaviour: Cell<crate::WindowCloseBehaviour>,
    component_window_callbacks: RefCell<Option<ComponentWindowCallbacks>>,

    #[cfg(target_os = "ios")]
    pub(crate) views: RefCell<Vec<objc2::rc::Retained<objc2_ui_kit::UIView>>>,
}

impl DesktopWindowContext {
    pub(crate) fn new(
        webview: WebView,
        window: Arc<HostWindow>,
        target_id: RenderTargetId,
        asset_handlers: AssetHandlerRegistry,
        file_hover: NativeFileHover,
        close_behaviour: crate::WindowCloseBehaviour,
    ) -> Self {
        Self {
            webview,
            #[cfg(target_os = "linux")]
            _gtk_teardown: GtkTeardown,
            window,
            target_id,
            asset_handlers,
            file_hover,
            query: QueryEngine::default(),
            close_behaviour: Cell::new(close_behaviour),
            component_window_callbacks: RefCell::new(None),
            #[cfg(target_os = "ios")]
            views: RefCell::new(Vec::new()),
        }
    }

    /// Get the native window ID.
    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// Set the native window title.
    pub fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    pub(crate) fn register_component_window_callbacks(
        &self,
        on_close_requested: impl FnMut() + 'static,
        on_destroyed: impl FnMut() + 'static,
    ) {
        self.component_window_callbacks
            .borrow_mut()
            .replace(ComponentWindowCallbacks {
                on_close_requested: Box::new(on_close_requested),
                on_destroyed: Box::new(on_destroyed),
            });
    }

    pub(crate) fn clear_component_window_callbacks(&self) {
        self.component_window_callbacks.borrow_mut().take();
    }

    pub(crate) fn request_window_close(&self) -> WindowCloseRequestResult {
        if let Some(callbacks) = self.component_window_callbacks.borrow_mut().as_mut() {
            (callbacks.on_close_requested)();
            WindowCloseRequestResult::DeferredToComponent
        } else {
            WindowCloseRequestResult::CloseImmediately
        }
    }

    pub(crate) fn notify_window_destroyed(&self) {
        if let Some(callbacks) = self.component_window_callbacks.borrow_mut().as_mut() {
            (callbacks.on_destroyed)();
        }
    }
}

/// Drains deferred GTK widget destruction after Wry's WebView is dropped and
/// before Winit destroys the X11 host window.
#[cfg(target_os = "linux")]
struct GtkTeardown;

#[cfg(target_os = "linux")]
impl Drop for GtkTeardown {
    fn drop(&mut self) {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
    }
}

/// Expose the underlying native window so its windowing methods can be called directly on a
/// [`DesktopContext`](crate::DesktopContext).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl std::ops::Deref for DesktopWindowContext {
    type Target = dyn winit::window::Window;

    fn deref(&self) -> &Self::Target {
        self.window.as_ref()
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
impl std::ops::Deref for DesktopWindowContext {
    type Target = tao::window::Window;

    fn deref(&self) -> &Self::Target {
        self.window.as_ref()
    }
}
