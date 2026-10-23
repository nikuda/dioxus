use dioxus_core::{LaunchConfig, VirtualDom};
use std::path::PathBuf;
use std::{borrow::Cow, sync::Arc};
use wry::http::{Request as HttpRequest, Response as HttpResponse};
use wry::{RequestAsyncResponder, WebViewId};

use crate::menubar::{DioxusMenu, default_menu_bar};
use crate::windowing::HostWindow;
use crate::{AppEvent, desktop_state::DesktopAppContext, event_handlers::AppEventCallback};
use std::rc::Rc;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::WindowAttributes;
#[cfg(any(target_os = "android", target_os = "ios"))]
use crate::WindowBuilder as TaoWindowBuilder;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::{WinitEvent, event_handlers::WinitEventCallback};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use winit::{event_loop::EventLoopBuilder, icon::Icon, window::WindowLevel};

#[cfg(any(target_os = "android", target_os = "ios"))]
use crate::ipc::UserWindowEvent;
#[cfg(any(target_os = "android", target_os = "ios"))]
use tao::{
    event_loop::{EventLoop, EventLoopWindowTarget},
    window::Icon,
};

#[cfg(any(target_os = "android", target_os = "ios"))]
use crate::event_handlers::TaoEventCallback;

/// A function taking a URL and returning whether the webview should navigate to it or open it in
/// the browser. If missing in the config, all URLs will be allowed.
type NavigationHandler = Box<dyn Fn(&str) -> bool + 'static>;

/// The closing behaviour of specific application window.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum WindowCloseBehaviour {
    /// Window will hide instead of closing
    WindowHides,

    /// Window will close
    WindowCloses,
}

/// The state of the menu builder. We need to keep track of if the state is default
/// so we only swap out the default menu bar when decorations are disabled
pub(crate) enum MenuBuilderState {
    Unset,
    Set(Option<DioxusMenu>),
}

impl From<MenuBuilderState> for Option<DioxusMenu> {
    fn from(val: MenuBuilderState) -> Self {
        match val {
            MenuBuilderState::Unset => Some(default_menu_bar()),
            MenuBuilderState::Set(menu) => menu,
        }
    }
}

pub(crate) type WryProtocol = (
    String,
    Box<dyn Fn(WebViewId, HttpRequest<Vec<u8>>) -> HttpResponse<Cow<'static, [u8]>> + 'static>,
);

pub(crate) type AsyncWryProtocol = (
    String,
    Box<dyn Fn(WebViewId, HttpRequest<Vec<u8>>, RequestAsyncResponder) + 'static>,
);

/// The configuration for a single desktop window.
///
/// Pass a `WindowConfig` to the [`Window`](crate::Window) component's `config` prop to customize a
/// window it opens. The [app-level `Config`](Config) embeds one of these to describe the window
/// created for you by [`launch`](crate::launch::launch).
pub struct WindowConfig {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub(crate) window: WindowAttributes,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) window: TaoWindowBuilder,
    pub(crate) as_child_window: bool,
    pub(crate) menu: MenuBuilderState,
    pub(crate) protocols: Vec<WryProtocol>,
    pub(crate) asynchronous_protocols: Vec<AsyncWryProtocol>,
    pub(crate) pre_rendered: Option<String>,
    pub(crate) disable_context_menu: bool,
    pub(crate) resource_dir: Option<PathBuf>,
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) custom_head: Option<String>,
    pub(crate) custom_index: Option<String>,
    pub(crate) root_name: String,
    pub(crate) background_color: Option<(u8, u8, u8, u8)>,
    pub(crate) window_close_behavior: WindowCloseBehaviour,
    pub(crate) disable_file_drop_handler: bool,
    pub(crate) additional_windows_args: Option<String>,
    pub(crate) navigation_handler: Option<NavigationHandler>,

    #[allow(clippy::type_complexity)]
    pub(crate) on_window: Option<Box<dyn FnMut(Arc<HostWindow>, &mut VirtualDom) + 'static>>,
}

impl WindowConfig {
    /// Initializes a new `WindowConfig` with default values.
    #[inline]
    pub fn new() -> Self {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let mut window: WindowAttributes = WindowAttributes::default()
            .with_title(dioxus_cli_config::app_title().unwrap_or_else(|| "Dioxus App".to_string()));

        #[cfg(any(target_os = "android", target_os = "ios"))]
        let mut window: TaoWindowBuilder = TaoWindowBuilder::new()
            .with_title(dioxus_cli_config::app_title().unwrap_or_else(|| "Dioxus App".to_string()));

        // During development we want the window to be on top so we can see it while we work
        let always_on_top = dioxus_cli_config::always_on_top().unwrap_or(true);

        if cfg!(debug_assertions) {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                window = window.with_window_level(if always_on_top {
                    WindowLevel::AlwaysOnTop
                } else {
                    WindowLevel::Normal
                });
            }

            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                window = window.with_always_on_top(always_on_top);
            }
        }

        Self {
            window,
            as_child_window: false,
            menu: MenuBuilderState::Unset,
            protocols: Vec::new(),
            asynchronous_protocols: Vec::new(),
            pre_rendered: None,
            disable_context_menu: !cfg!(debug_assertions),
            resource_dir: None,
            data_dir: None,
            custom_head: None,
            custom_index: None,
            root_name: "main".to_string(),
            background_color: None,
            window_close_behavior: WindowCloseBehaviour::WindowCloses,
            disable_file_drop_handler: false,
            on_window: None,
            additional_windows_args: None,
            navigation_handler: None,
        }
    }

    /// set the directory from which assets will be searched in release mode
    pub fn with_resource_directory(mut self, path: impl Into<PathBuf>) -> Self {
        self.resource_dir = Some(path.into());
        self
    }

    /// Set the directory where WebView2 stores its user data (cookies, cache, IndexedDB, etc.).
    ///
    /// If not set, Dioxus automatically chooses a sensible default:
    /// - **Windows:** `%LOCALAPPDATA%/<exe_name>` — this avoids the WebView2 default of
    ///   placing data next to the executable, which fails in read-only locations like
    ///   `Program Files` or on certain drive types (e.g. ReFS dev drives).
    /// - **macOS/Linux:** managed by WebKit automatically.
    ///
    /// You typically only need this if you want multiple apps to share a WebView2 profile
    /// or need the data in a specific location for compliance/IT policy reasons.
    pub fn with_data_directory(mut self, path: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(path.into());
        self
    }

    /// Set whether or not the right-click context menu should be disabled.
    pub fn with_disable_context_menu(mut self, disable: bool) -> Self {
        self.disable_context_menu = disable;
        self
    }

    /// Set whether or not the file drop handler should be disabled.
    /// On Windows the drop handler must be disabled for HTML drag and drop APIs to work.
    pub fn with_disable_drag_drop_handler(mut self, disable: bool) -> Self {
        self.disable_file_drop_handler = disable;
        self
    }

    /// Set the pre-rendered HTML content
    pub fn with_prerendered(mut self, content: String) -> Self {
        self.pre_rendered = Some(content);
        self
    }

    /// Set the configuration for the window.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn with_window(mut self, window: WindowAttributes) -> Self {
        self.window = window;
        // If the decorations are off for the window, remove the menu as well
        let decorations = self.window.decorations;

        if !decorations && matches!(self.menu, MenuBuilderState::Unset) {
            self.menu = MenuBuilderState::Set(None);
        }
        self
    }

    /// Set the configuration for the window.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub fn with_window(mut self, window: TaoWindowBuilder) -> Self {
        self.window = window;
        // If the decorations are off for the window, remove the menu as well
        let decorations = self.window.window.decorations;

        if !decorations && matches!(self.menu, MenuBuilderState::Unset) {
            self.menu = MenuBuilderState::Set(None);
        }
        self
    }

    /// Set the window as child
    pub fn with_as_child_window(mut self) -> Self {
        self.as_child_window = true;
        self
    }

    /// Sets the behaviour of this window when it is closed.
    pub fn with_close_behaviour(mut self, behaviour: WindowCloseBehaviour) -> Self {
        self.window_close_behavior = behaviour;
        self
    }

    /// Set a custom protocol
    pub fn with_custom_protocol<F>(mut self, name: impl ToString, handler: F) -> Self
    where
        F: Fn(WebViewId, HttpRequest<Vec<u8>>) -> HttpResponse<Cow<'static, [u8]>> + 'static,
    {
        self.protocols.push((name.to_string(), Box::new(handler)));
        self
    }

    /// Set an asynchronous custom protocol
    ///
    /// **Example Usage**
    /// ```rust
    /// # use wry::http::response::Response as HTTPResponse;
    /// # use std::borrow::Cow;
    /// # use dioxus_desktop::WindowConfig;
    /// #
    /// # fn main() {
    /// let cfg = WindowConfig::new()
    ///     .with_asynchronous_custom_protocol("asset", |_webview_id, request, responder| {
    ///         tokio::spawn(async move {
    ///             responder.respond(
    ///                 HTTPResponse::builder()
    ///                     .status(404)
    ///                     .body(Cow::Borrowed("404 - Not Found".as_bytes()))
    ///                     .unwrap()
    ///             );
    ///         });
    ///     });
    /// # }
    /// ```
    /// note a key difference between Dioxus and Wry, the protocol name doesn't explicitly need to be a
    /// [`String`], but needs to implement [`ToString`].
    ///
    /// See [`wry`](wry::WebViewBuilder::with_asynchronous_custom_protocol) for more details on implementation
    pub fn with_asynchronous_custom_protocol<F>(mut self, name: impl ToString, handler: F) -> Self
    where
        F: Fn(WebViewId, HttpRequest<Vec<u8>>, RequestAsyncResponder) + 'static,
    {
        self.asynchronous_protocols
            .push((name.to_string(), Box::new(handler)));
        self
    }

    /// Set a custom icon for this application
    pub fn with_icon(mut self, icon: Icon) -> Self {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            self.window.window_icon = Some(icon);
        }
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.window.window.window_icon = Some(icon);
        }
        self
    }

    /// Inject additional content into the document's HEAD.
    ///
    /// This is useful for loading CSS libraries, JS libraries, etc.
    pub fn with_custom_head(mut self, head: String) -> Self {
        self.custom_head = Some(head);
        self
    }

    /// Use a custom index.html instead of the default Dioxus one.
    ///
    /// Make sure your index.html is valid HTML.
    ///
    /// Dioxus injects some loader code into the closing body tag. Your document
    /// must include a body element!
    pub fn with_custom_index(mut self, index: String) -> Self {
        self.custom_index = Some(index);
        self
    }

    /// Set the name of the element that Dioxus will use as the root.
    ///
    /// This is akin to calling React.render() on the element with the specified name.
    pub fn with_root_name(mut self, name: impl Into<String>) -> Self {
        self.root_name = name.into();
        self
    }

    /// Sets the background color of the WebView.
    /// This will be set before the HTML is rendered and can be used to prevent flashing when the page loads.
    /// Accepts a color in RGBA format
    pub fn with_background_color(mut self, color: (u8, u8, u8, u8)) -> Self {
        self.background_color = Some(color);
        self
    }

    /// Sets the menu the window will use. This will override the default menu bar.
    ///
    /// > Note: Menu will be hidden if window decorations are disabled on the
    /// > value passed into [`with_window`](WindowConfig::with_window).
    #[allow(unused)]
    pub fn with_menu(mut self, menu: impl Into<Option<DioxusMenu>>) -> Self {
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            if self.window.decorations {
                self.menu = MenuBuilderState::Set(menu.into())
            }
        }
        self
    }

    /// Allows modifying the window and virtual dom right after they are built, but before the webview is created.
    ///
    /// This is important for z-ordering textures in child windows. This callback runs on every
    /// window creation, so it must be safe to invoke more than once.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn with_on_window(
        mut self,
        f: impl FnMut(Arc<dyn winit::window::Window>, &mut VirtualDom) + 'static,
    ) -> Self {
        self.on_window = Some(Box::new(f));
        self
    }

    /// Allows modifying the window and virtual dom right after they are built, but before the webview is created.
    ///
    /// This is important for z-ordering textures in child windows. This callback runs on every
    /// window creation, so it must be safe to invoke more than once.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub fn with_on_window(
        mut self,
        f: impl FnMut(Arc<tao::window::Window>, &mut VirtualDom) + 'static,
    ) -> Self {
        self.on_window = Some(Box::new(f));
        self
    }

    /// Add additional windows only launch arguments for webview2
    pub fn with_windows_browser_args(mut self, additional_args: impl ToString) -> Self {
        self.additional_windows_args = Some(additional_args.to_string());
        self
    }

    /// Set a custom navigation handler for non-dioxus URLs.
    /// Return true to allow navigation inside the webview, false to block.
    pub fn with_navigation_handler(mut self, f: impl Fn(&str) -> bool + 'static) -> Self {
        self.navigation_handler = Some(Box::new(f));
        self
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// The configuration for the desktop application.
///
/// This holds app-wide settings shared by every window plus the [`WindowConfig`] used for the
/// window that [`launch`](crate::launch::launch) opens for you. The window-level builder methods
/// (`with_window`, `with_menu`, `with_custom_head`, …) configure that default window; use
/// [`WindowConfig`] directly when opening additional windows with the [`Window`](crate::Window)
/// component.
pub struct Config {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub(crate) event_loop_builder: Option<EventLoopBuilder>,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) event_loop: Option<EventLoop<UserWindowEvent>>,
    pub(crate) exit_on_last_window_close: bool,
    pub(crate) app_event_handler: Option<AppEventCallback>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub(crate) winit_event_handler: Option<WinitEventCallback>,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) tao_event_handler: Option<TaoEventCallback>,
    pub(crate) disable_dma_buf_on_wayland: bool,
    pub(crate) tray_icon_show_window_on_click: bool,
    pub(crate) headless_root: bool,

    /// Configuration for the window opened by `launch`.
    pub(crate) window: WindowConfig,
}

impl LaunchConfig for Config {}

impl Config {
    /// Initializes a new `Config` with default values.
    #[inline]
    pub fn new() -> Self {
        Self {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            event_loop_builder: None,
            #[cfg(any(target_os = "android", target_os = "ios"))]
            event_loop: None,
            exit_on_last_window_close: true,
            app_event_handler: None,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            winit_event_handler: None,
            #[cfg(any(target_os = "android", target_os = "ios"))]
            tao_event_handler: None,
            disable_dma_buf_on_wayland: true,
            tray_icon_show_window_on_click: true,
            headless_root: false,
            window: WindowConfig::new(),
        }
    }

    /// Set whether [`launch`](crate::launch::launch) should run the root component without
    /// wrapping it in a default [`Window`](crate::Window).
    ///
    /// Defaults to `false`. Set this to `true` when the root component should own all native
    /// windows explicitly by rendering [`Window`](crate::Window) components itself.
    pub fn with_headless_root(mut self, headless: bool) -> Self {
        self.headless_root = headless;
        self
    }

    /// Set the configuration for the window opened by `launch`.
    pub fn with_window_config(mut self, window: WindowConfig) -> Self {
        self.window = window;
        self
    }

    /// Set the Winit event-loop builder used by the desktop host.
    ///
    /// Dioxus still builds and owns the event loop. On Linux it overrides the
    /// display backend to X11 so Winit and the current WebKitGTK host cannot
    /// select incompatible backends.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn with_event_loop_builder(mut self, event_loop_builder: EventLoopBuilder) -> Self {
        self.event_loop_builder = Some(event_loop_builder);
        self
    }

    /// Set the event loop to be used.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub fn with_event_loop(mut self, event_loop: EventLoop<UserWindowEvent>) -> Self {
        self.event_loop = Some(event_loop);
        self
    }

    /// When the last window is closed, the application will exit.
    ///
    /// This is the default behaviour.
    ///
    /// If the last window is hidden, the application will not exit.
    pub fn with_exits_when_last_window_closes(mut self, exit: bool) -> Self {
        self.exit_on_last_window_close = exit;
        self
    }

    /// Sets a callback for cross-platform application lifecycle events.
    pub fn with_app_event_handler(
        mut self,
        f: impl FnMut(AppEvent, &Rc<DesktopAppContext>) + 'static,
    ) -> Self {
        self.app_event_handler = Some(Box::new(f));
        self
    }

    /// Sets a callback for raw desktop Winit events.
    ///
    /// Prefer [`Self::with_app_event_handler`] when a backend-neutral event is
    /// sufficient.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn with_winit_event_handler(
        mut self,
        f: impl for<'a> FnMut(WinitEvent<'a>, &Rc<DesktopAppContext>) + 'static,
    ) -> Self {
        self.winit_event_handler = Some(Box::new(f));
        self
    }

    /// Sets a callback at each raw mobile Tao event-loop boundary.
    ///
    /// Prefer [`Self::with_app_event_handler`] when a backend-neutral event is
    /// sufficient.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub fn with_tao_event_handler(
        mut self,
        f: impl FnMut(&tao::event::Event<'_, UserWindowEvent>, &EventLoopWindowTarget<UserWindowEvent>)
        + 'static,
    ) -> Self {
        self.tao_event_handler = Some(Box::new(f));
        self
    }

    /// Set whether or not DMA-BUF usage should be disabled on Wayland.
    ///
    /// Defaults to true to avoid issues on some systems. Linux applications
    /// must call `Config::prepare_linux_host` before they start any application
    /// threads. A parent process may set `WEBKIT_DISABLE_DMABUF_RENDERER=1`,
    /// but Dioxus must still prepare and validate the GTK backend.
    ///
    /// If you want to enable DMA-BUF usage, set this to false before calling
    /// `Config::prepare_linux_host`.
    /// See <https://github.com/DioxusLabs/dioxus/issues/4528#issuecomment-3476430611>
    pub fn with_disable_dma_buf_on_wayland(mut self, disable: bool) -> Self {
        self.disable_dma_buf_on_wayland = disable;
        self
    }

    /// Prepare Dioxus Desktop's process-wide Linux host settings.
    ///
    /// This selects GTK's X11 backend and, when configured and required,
    /// disables WebKit's DMA-BUF renderer. It then initializes GTK and
    /// validates the selected backend. Call it after the final
    /// [`Self::with_disable_dma_buf_on_wayland`] choice and before starting
    /// application worker threads. Dioxus validates the prepared host state
    /// again before constructing its Winit event loop.
    ///
    /// # Safety
    ///
    /// The current thread must be the process main thread. No other thread may
    /// read or write the process environment while this method runs. GTK,
    /// Winit, and WebKit must not have been initialized.
    #[cfg(target_os = "linux")]
    pub unsafe fn prepare_linux_host(self) -> Result<Self, crate::LinuxHostError> {
        // SAFETY: this public method exposes the same caller obligations.
        unsafe { crate::linux_host::prepare(self.disable_dma_buf_on_wayland)? };
        Ok(self)
    }

    /// Set whether the main window is shown and focused when the tray icon is left-clicked.
    ///
    /// Defaults to `true` (preserves current behavior). Set to `false` for tray/menu bar apps
    /// that only want to show the context menu when the tray icon is clicked, without
    /// activating the main window.
    pub fn with_tray_icon_show_window_on_click(mut self, show: bool) -> Self {
        self.tray_icon_show_window_on_click = show;
        self
    }

    /// set the directory from which assets will be searched in release mode
    pub fn with_resource_directory(mut self, path: impl Into<PathBuf>) -> Self {
        self.window = self.window.with_resource_directory(path);
        self
    }

    /// Set the directory where WebView2 stores its user data (cookies, cache, IndexedDB, etc.).
    ///
    /// See [`WindowConfig::with_data_directory`] for details.
    pub fn with_data_directory(mut self, path: impl Into<PathBuf>) -> Self {
        self.window = self.window.with_data_directory(path);
        self
    }

    /// Set whether or not the right-click context menu should be disabled.
    pub fn with_disable_context_menu(mut self, disable: bool) -> Self {
        self.window = self.window.with_disable_context_menu(disable);
        self
    }

    /// Set whether or not the file drop handler should be disabled.
    /// On Windows the drop handler must be disabled for HTML drag and drop APIs to work.
    pub fn with_disable_drag_drop_handler(mut self, disable: bool) -> Self {
        self.window = self.window.with_disable_drag_drop_handler(disable);
        self
    }

    /// Set the pre-rendered HTML content
    pub fn with_prerendered(mut self, content: String) -> Self {
        self.window = self.window.with_prerendered(content);
        self
    }

    /// Set the configuration for the window opened by `launch`.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn with_window(mut self, window: WindowAttributes) -> Self {
        self.window = self.window.with_window(window);
        self
    }

    /// Set the configuration for the window.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub fn with_window(mut self, window: TaoWindowBuilder) -> Self {
        self.window = self.window.with_window(window);
        self
    }

    /// Set the window opened by `launch` as a child window.
    pub fn with_as_child_window(mut self) -> Self {
        self.window = self.window.with_as_child_window();
        self
    }

    /// Sets the close behaviour of the window opened by `launch`.
    pub fn with_close_behaviour(mut self, behaviour: WindowCloseBehaviour) -> Self {
        self.window = self.window.with_close_behaviour(behaviour);
        self
    }

    /// Set a custom protocol
    pub fn with_custom_protocol<F>(mut self, name: impl ToString, handler: F) -> Self
    where
        F: Fn(WebViewId, HttpRequest<Vec<u8>>) -> HttpResponse<Cow<'static, [u8]>> + 'static,
    {
        self.window = self.window.with_custom_protocol(name, handler);
        self
    }

    /// Set an asynchronous custom protocol
    ///
    /// See [`WindowConfig::with_asynchronous_custom_protocol`] for details.
    pub fn with_asynchronous_custom_protocol<F>(mut self, name: impl ToString, handler: F) -> Self
    where
        F: Fn(WebViewId, HttpRequest<Vec<u8>>, RequestAsyncResponder) + 'static,
    {
        self.window = self.window.with_asynchronous_custom_protocol(name, handler);
        self
    }

    /// Set a custom icon for this application
    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.window = self.window.with_icon(icon);
        self
    }

    /// Inject additional content into the document's HEAD.
    ///
    /// This is useful for loading CSS libraries, JS libraries, etc.
    pub fn with_custom_head(mut self, head: String) -> Self {
        self.window = self.window.with_custom_head(head);
        self
    }

    /// Use a custom index.html instead of the default Dioxus one.
    pub fn with_custom_index(mut self, index: String) -> Self {
        self.window = self.window.with_custom_index(index);
        self
    }

    /// Set the name of the element that Dioxus will use as the root.
    pub fn with_root_name(mut self, name: impl Into<String>) -> Self {
        self.window = self.window.with_root_name(name);
        self
    }

    /// Sets the background color of the WebView.
    pub fn with_background_color(mut self, color: (u8, u8, u8, u8)) -> Self {
        self.window = self.window.with_background_color(color);
        self
    }

    /// Sets the menu the window opened by `launch` will use.
    #[allow(unused)]
    pub fn with_menu(mut self, menu: impl Into<Option<DioxusMenu>>) -> Self {
        self.window = self.window.with_menu(menu);
        self
    }

    /// Allows modifying the window and virtual dom right after they are built, but before the webview is created.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn with_on_window(
        mut self,
        f: impl FnMut(Arc<dyn winit::window::Window>, &mut VirtualDom) + 'static,
    ) -> Self {
        self.window = self.window.with_on_window(f);
        self
    }

    /// Allows modifying the window and virtual dom right after they are built, but before the webview is created.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub fn with_on_window(
        mut self,
        f: impl FnMut(Arc<tao::window::Window>, &mut VirtualDom) + 'static,
    ) -> Self {
        self.window = self.window.with_on_window(f);
        self
    }

    /// Add additional windows only launch arguments for webview2
    pub fn with_windows_browser_args(mut self, additional_args: impl ToString) -> Self {
        self.window = self.window.with_windows_browser_args(additional_args);
        self
    }

    /// Set a custom navigation handler for non-dioxus URLs.
    pub fn with_navigation_handler(mut self, f: impl Fn(&str) -> bool + 'static) -> Self {
        self.window = self.window.with_navigation_handler(f);
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

// dirty trick, avoid introducing `image` at runtime
// TODO: use serde when `Icon` impl serde
//
// This function should only be enabled when generating new icons.
//
// #[test]
// #[ignore]
// fn prepare_default_icon() {
//     use image::io::Reader as ImageReader;
//     use image::ImageFormat;
//     use std::fs::File;
//     use std::io::Cursor;
//     use std::io::Write;
//     use std::path::PathBuf;
//     let png: &[u8] = include_bytes!("default_icon.png");
//     let mut reader = ImageReader::new(Cursor::new(png));
//     reader.set_format(ImageFormat::Png);
//     let icon = reader.decode().unwrap();
//     let bin = PathBuf::from(file!())
//         .parent()
//         .unwrap()
//         .join("default_icon.bin");
//     println!("{:?}", bin);
//     let mut file = File::create(bin).unwrap();
//     file.write_all(icon.as_bytes()).unwrap();
//     println!("({}, {})", icon.width(), icon.height())
// }
