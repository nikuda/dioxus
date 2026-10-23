//! Target-specific native window vocabulary.
//!
//! Desktop targets use Winit. Android and iOS retain Tao until their Wry host
//! lifecycle is migrated independently.

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) type HostWindow = dyn winit::window::Window;

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) type HostWindow = tao::window::Window;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn surface_size(window: &HostWindow) -> winit::dpi::PhysicalSize<u32> {
    window.surface_size()
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) fn surface_size(window: &HostWindow) -> tao::dpi::PhysicalSize<u32> {
    window.inner_size()
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) use winit::{
    dpi::{PhysicalSize, Position, Size},
    window::WindowId,
};

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) use tao::{
    dpi::{PhysicalSize, Position, Size},
    window::WindowId,
};
