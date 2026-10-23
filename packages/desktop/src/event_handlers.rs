use crate::{app, desktop_state::DesktopAppContext, windowing::WindowId};
use slab::Slab;
use std::{cell::RefCell, rc::Rc};

#[cfg(any(target_os = "android", target_os = "ios"))]
use crate::ipc::UserWindowEvent;
#[cfg(any(target_os = "android", target_os = "ios"))]
use tao::{event::Event, event_loop::EventLoopWindowTarget};

pub(crate) type AppEventCallback = Box<dyn FnMut(AppEvent, &Rc<DesktopAppContext>) + 'static>;
type WindowEventCallback = Box<dyn FnMut(WindowEvent, &Rc<DesktopAppContext>) + 'static>;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) type WinitEventCallback =
    Box<dyn for<'a> FnMut(WinitEvent<'a>, &Rc<DesktopAppContext>) + 'static>;

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) type TaoEventCallback = Box<
    dyn 'static
        + for<'a> FnMut(
            &tao::event::Event<'a, UserWindowEvent>,
            &EventLoopWindowTarget<UserWindowEvent>,
        ),
>;

/// A backend-neutral application lifecycle event from a Dioxus Desktop host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AppEvent {
    /// The application resumed.
    Resumed,
    /// The application was suspended.
    Suspended,
    /// The operating system reported memory pressure.
    MemoryWarning,
    /// Dioxus accepted a request to terminate the application.
    ExitRequested,
    /// The application owner is about to tear down its handlers and windows.
    Exiting,
}

/// A semantic event associated with one Dioxus Desktop window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowEvent {
    /// The user or operating system requested that the window close.
    CloseRequested,
    /// The native window was destroyed.
    Destroyed,
    /// The window's drawable surface changed size in physical pixels.
    Resized {
        /// The new physical width.
        width: u32,
        /// The new physical height.
        height: u32,
    },
    /// The window gained or lost input focus.
    Focused(bool),
}

impl WindowEvent {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub(crate) fn from_winit(event: &winit::event::WindowEvent) -> Option<Self> {
        match event {
            winit::event::WindowEvent::CloseRequested => Some(Self::CloseRequested),
            winit::event::WindowEvent::Destroyed => Some(Self::Destroyed),
            winit::event::WindowEvent::SurfaceResized(size) => Some(Self::Resized {
                width: size.width,
                height: size.height,
            }),
            winit::event::WindowEvent::Focused(focused) => Some(Self::Focused(*focused)),
            _ => None,
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn from_tao(event: &tao::event::WindowEvent<'_>) -> Option<Self> {
        match event {
            tao::event::WindowEvent::CloseRequested => Some(Self::CloseRequested),
            tao::event::WindowEvent::Destroyed => Some(Self::Destroyed),
            tao::event::WindowEvent::Resized(size) => Some(Self::Resized {
                width: size.width,
                height: size.height,
            }),
            tao::event::WindowEvent::Focused(focused) => Some(Self::Focused(*focused)),
            _ => None,
        }
    }
}

/// A raw callback boundary from the desktop Winit application owner.
///
/// Most applications should use [`AppEvent`] and [`WindowEvent`]. This type is
/// the explicit escape hatch for Winit-specific lifecycle, device, or input
/// handling. It deliberately omits Winit's borrowed `ActiveEventLoop`;
/// application operations remain available through [`DesktopAppContext`].
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum WinitEvent<'a> {
    /// A new native event batch has started.
    NewEvents(winit::event::StartCause),
    /// Winit reported that the application resumed.
    Resumed,
    /// Native surfaces may now be created.
    CanCreateSurfaces,
    /// The event-loop proxy was awakened.
    ProxyWakeUp,
    /// A raw event associated with one Winit window.
    WindowEvent {
        /// The Winit window identifier.
        window_id: winit::window::WindowId,
        /// The raw Winit window event.
        event: &'a winit::event::WindowEvent,
    },
    /// A raw Winit device event.
    DeviceEvent {
        /// The input device identifier, when available.
        device_id: Option<winit::event::DeviceId>,
        /// The raw Winit device event.
        event: &'a winit::event::DeviceEvent,
    },
    /// The application owner is about to wait for more events.
    AboutToWait,
    /// Winit reported that the application was suspended.
    Suspended,
    /// Native surfaces must be destroyed.
    DestroySurfaces,
    /// Winit reported memory pressure.
    MemoryWarning,
}

/// The unique identifier of an event handler registered with Dioxus Desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventHandlerId(pub(crate) usize);

impl EventHandlerId {
    /// Unregister this event handler.
    pub fn remove(&self) {
        app().event_handlers.remove(*self)
    }
}

enum EventHandlerInner {
    App(AppEventCallback),
    Window {
        window_id: WindowId,
        handler: WindowEventCallback,
    },
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    Winit {
        window_id: WindowId,
        handler: WinitEventCallback,
    },
    #[cfg(any(target_os = "android", target_os = "ios"))]
    Tao {
        window_id: WindowId,
        handler: TaoEventCallback,
    },
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    Menu(Box<dyn FnMut(&muda::MenuEvent)>),
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    TrayMenu(Box<dyn FnMut(&tray_icon::menu::MenuEvent)>),
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    TrayIcon(Box<dyn FnMut(&tray_icon::TrayIconEvent)>),
}

#[derive(Default)]
pub(crate) struct EventHandlers {
    handlers: RefCell<Slab<EventHandlerInner>>,
}

impl EventHandlers {
    fn insert(&self, handler: EventHandlerInner) -> EventHandlerId {
        EventHandlerId(self.handlers.borrow_mut().insert(handler))
    }

    pub(crate) fn add_app(
        &self,
        handler: impl FnMut(AppEvent, &Rc<DesktopAppContext>) + 'static,
    ) -> EventHandlerId {
        self.insert(EventHandlerInner::App(Box::new(handler)))
    }

    pub(crate) fn add_window(
        &self,
        window_id: WindowId,
        handler: impl FnMut(WindowEvent, &Rc<DesktopAppContext>) + 'static,
    ) -> EventHandlerId {
        self.insert(EventHandlerInner::Window {
            window_id,
            handler: Box::new(handler),
        })
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub(crate) fn add_winit(
        &self,
        window_id: WindowId,
        handler: impl for<'a> FnMut(WinitEvent<'a>, &Rc<DesktopAppContext>) + 'static,
    ) -> EventHandlerId {
        self.insert(EventHandlerInner::Winit {
            window_id,
            handler: Box::new(handler),
        })
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn add_tao(
        &self,
        window_id: WindowId,
        handler: impl FnMut(&Event<UserWindowEvent>, &EventLoopWindowTarget<UserWindowEvent>) + 'static,
    ) -> EventHandlerId {
        self.insert(EventHandlerInner::Tao {
            window_id,
            handler: Box::new(handler),
        })
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    pub(crate) fn add_menu(
        &self,
        handler: impl FnMut(&muda::MenuEvent) + 'static,
    ) -> EventHandlerId {
        self.insert(EventHandlerInner::Menu(Box::new(handler)))
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    pub(crate) fn add_tray_menu(
        &self,
        handler: impl FnMut(&tray_icon::menu::MenuEvent) + 'static,
    ) -> EventHandlerId {
        self.insert(EventHandlerInner::TrayMenu(Box::new(handler)))
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    pub(crate) fn add_tray_icon(
        &self,
        handler: impl FnMut(&tray_icon::TrayIconEvent) + 'static,
    ) -> EventHandlerId {
        self.insert(EventHandlerInner::TrayIcon(Box::new(handler)))
    }

    pub(crate) fn remove(&self, id: EventHandlerId) {
        self.handlers.borrow_mut().try_remove(id.0);
    }

    pub(crate) fn apply_app(&self, event: AppEvent, app: &Rc<DesktopAppContext>) {
        for (_, handler) in self.handlers.borrow_mut().iter_mut() {
            if let EventHandlerInner::App(handler) = handler {
                handler(event, app);
            }
        }
    }

    pub(crate) fn apply_window(
        &self,
        window_id: WindowId,
        event: WindowEvent,
        app: &Rc<DesktopAppContext>,
    ) {
        for (_, handler) in self.handlers.borrow_mut().iter_mut() {
            if let EventHandlerInner::Window {
                window_id: handler_window_id,
                handler,
            } = handler
                && *handler_window_id == window_id
            {
                handler(event, app);
            }
        }
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub(crate) fn apply_winit(&self, event: WinitEvent<'_>, app: &Rc<DesktopAppContext>) {
        for (_, handler) in self.handlers.borrow_mut().iter_mut() {
            if let EventHandlerInner::Winit { window_id, handler } = handler {
                if let WinitEvent::WindowEvent {
                    window_id: event_window_id,
                    ..
                } = event
                    && *window_id != event_window_id
                {
                    continue;
                }
                handler(event, app);
            }
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn apply_tao(
        &self,
        event: &Event<UserWindowEvent>,
        target: &EventLoopWindowTarget<UserWindowEvent>,
    ) {
        for (_, handler) in self.handlers.borrow_mut().iter_mut() {
            if let EventHandlerInner::Tao { window_id, handler } = handler {
                #[cfg(target_os = "android")]
                let _lock = crate::android_sync_lock::android_runtime_lock();

                if let Event::WindowEvent {
                    window_id: event_window_id,
                    ..
                } = event
                    && *window_id != *event_window_id
                {
                    continue;
                }
                handler(event, target);
            }
        }
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    pub(crate) fn apply_menu(&self, event: &muda::MenuEvent) {
        for (_, handler) in self.handlers.borrow_mut().iter_mut() {
            if let EventHandlerInner::Menu(handler) = handler {
                handler(event);
            }
        }
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    pub(crate) fn apply_tray_menu(&self, event: &tray_icon::menu::MenuEvent) {
        for (_, handler) in self.handlers.borrow_mut().iter_mut() {
            if let EventHandlerInner::TrayMenu(handler) = handler {
                handler(event);
            }
        }
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    pub(crate) fn apply_tray_icon(&self, event: &tray_icon::TrayIconEvent) {
        for (_, handler) in self.handlers.borrow_mut().iter_mut() {
            if let EventHandlerInner::TrayIcon(handler) = handler {
                handler(event);
            }
        }
    }
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod tests {
    use super::*;

    #[test]
    fn maps_winit_window_events_to_backend_neutral_events() {
        assert_eq!(
            WindowEvent::from_winit(&winit::event::WindowEvent::Focused(true)),
            Some(WindowEvent::Focused(true))
        );
        assert_eq!(
            WindowEvent::from_winit(&winit::event::WindowEvent::SurfaceResized(
                winit::dpi::PhysicalSize::new(640, 480),
            )),
            Some(WindowEvent::Resized {
                width: 640,
                height: 480,
            })
        );
    }
}
