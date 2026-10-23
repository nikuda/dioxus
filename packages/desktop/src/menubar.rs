use crate::windowing::HostWindow;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub type DioxusMenu = muda::Menu;
#[cfg(any(target_os = "ios", target_os = "android"))]
pub type DioxusMenu = ();

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub type DioxusMenuIcon = muda::Icon;
#[cfg(any(target_os = "ios", target_os = "android"))]
pub type DioxusMenuIcon = ();

/// Initializes the menu bar for the window.
#[allow(unused)]
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn init_menu_bar(menu: &DioxusMenu, window: &HostWindow, webview: &wry::WebView) {
    desktop_platforms::init_menu_bar(menu, window, webview);
}

#[cfg(any(target_os = "ios", target_os = "android"))]
/// Initializes the mobile menu integration for the window.
pub fn init_menu_bar(menu: &DioxusMenu, window: &HostWindow) {
    let _ = (menu, window);
}

/// Creates a standard menu bar depending on the users platform. It may be used as a starting point
/// to further customize the menu bar and pass it to [`WindowConfig`](crate::WindowConfig).
/// > Note: The default menu bar enables macOS shortcuts like cut/copy/paste.
/// > The menu bar differs per platform because of constraints introduced
/// > by [`MenuItem`](muda::MenuItem).
#[allow(unused)]
pub fn default_menu_bar() -> DioxusMenu {
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        desktop_platforms::default_menu_bar()
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod desktop_platforms {
    use super::*;
    use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

    #[allow(unused)]
    pub fn init_menu_bar(menu: &Menu, window: &HostWindow, webview: &wry::WebView) {
        #[cfg(target_os = "windows")]
        unsafe {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let Ok(handle) = window.window_handle() else {
                return;
            };
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return;
            };
            _ = menu.init_for_hwnd(handle.hwnd.get());
        }

        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::*;
            use wry::WebViewExtUnix;

            let gtk_webview = webview.webview();
            let Some(container) = gtk_webview.parent().and_downcast::<gtk::Box>() else {
                tracing::error!("Wry's WebKit widget is not parented by a GTK box");
                return;
            };
            let Some(window) = gtk_webview.toplevel().and_downcast::<gtk::Window>() else {
                tracing::error!("Wry's WebKit widget has no GTK toplevel");
                return;
            };
            if let Err(error) = menu.init_for_gtk_window(&window, Some(&container)) {
                tracing::error!(?error, "failed to initialize the GTK menu bar");
            }
        }

        #[cfg(target_os = "macos")]
        {
            menu.init_for_nsapp();
        }

        let _ = (window, webview);
    }

    pub fn default_menu_bar() -> Menu {
        let menu = Menu::new();
        // since it is uncommon on windows to have an "application menu"
        // we add a "window" menu to be more consistent across platforms with the standard menu
        let window_menu = Submenu::new("Window", true);
        window_menu
            .append_items(&[
                &PredefinedMenuItem::fullscreen(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::maximize(None),
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::close_window(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ])
            .unwrap();

        let edit_menu = Submenu::new("Edit", true);
        edit_menu
            .append_items(&[
                &PredefinedMenuItem::undo(None),
                &PredefinedMenuItem::redo(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::cut(None),
                &PredefinedMenuItem::copy(None),
                &PredefinedMenuItem::paste(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::select_all(None),
            ])
            .unwrap();

        menu.append_items(&[&window_menu, &edit_menu]).unwrap();

        if cfg!(debug_assertions) {
            let help_menu = Submenu::new("Help", true);

            help_menu
                .append_items(&[&MenuItem::with_id(
                    "dioxus-toggle-dev-tools",
                    "Toggle Developer Tools",
                    true,
                    None,
                )])
                .unwrap();

            // By default we float the window on top in dev mode, but let the user disable it
            help_menu
                .append_items(&[&MenuItem::with_id(
                    "dioxus-float-top",
                    "Float on Top (dev mode only)",
                    true,
                    None,
                )])
                .unwrap();

            _ = menu.append_items(&[&help_menu]);

            #[cfg(target_os = "macos")]
            {
                help_menu.set_as_help_menu_for_nsapp();
            }
        }

        #[cfg(target_os = "macos")]
        {
            window_menu.set_as_windows_menu_for_nsapp();
        }

        menu
    }
}
