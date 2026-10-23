#![doc = include_str!("readme.md")]
#![doc(html_logo_url = "https://avatars.githubusercontent.com/u/79236386")]
#![doc(html_favicon_url = "https://avatars.githubusercontent.com/u/79236386")]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(unexpected_cfgs)]

mod android_sync_lock;
mod app;
mod assets;
mod commands;
mod config;
mod default_icon;
mod desktop_context;
mod desktop_state;
mod document;
mod edits;
mod element;
mod event_handlers;
mod events;
mod file_upload;
mod hooks;
mod ipc;
#[cfg(target_os = "linux")]
mod linux_host;
mod menubar;
mod mobile;
mod protocol;
mod query;
mod shortcut;
mod waker;
mod webview;
mod window_component;
mod window_lifecycle;
mod windowing;

// mobile shortcut is only supported on mobile platforms
#[cfg(any(target_os = "ios", target_os = "android"))]
mod mobile_shortcut;

/// The main entrypoint for this crate
pub mod launch;

// Mobile retains Tao until its Wry lifecycle has a separate Winit host.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub use tao;
#[cfg(any(target_os = "ios", target_os = "android"))]
pub use tao::dpi::{LogicalPosition, LogicalSize};
#[cfg(any(target_os = "ios", target_os = "android"))]
pub use tao::window::WindowBuilder;

/// The window and event-loop library used by supported desktop hosts.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use winit;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use winit::dpi::{LogicalPosition, LogicalSize};
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use winit::window::WindowAttributes;

pub use wry;
// Reexport muda only if we are on desktop platforms that support menus
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use muda;

// Tray icon
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub mod trayicon;

// Public exports
pub use assets::AssetRequest;
pub use config::{Config, WindowCloseBehaviour, WindowConfig};
pub use default_icon::{default_icon, icon_from_memory, icon_from_path};
pub use desktop_context::{
    DesktopContext, DesktopService, PendingDesktopWindow, WeakDesktopContext, app, window,
};
pub use desktop_state::DesktopAppContext;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use event_handlers::WinitEvent;
pub use event_handlers::{AppEvent, EventHandlerId, WindowEvent};
pub use hooks::*;
#[cfg(target_os = "linux")]
pub use linux_host::LinuxHostError;
pub use shortcut::{HotKeyState, ShortcutHandle, ShortcutRegistryError};
pub use window_component::{Window, WindowProps};
pub use wry::RequestAsyncResponder;
