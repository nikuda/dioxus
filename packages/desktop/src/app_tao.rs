use super::App;
use crate::{
    AppEvent, WindowEvent, commands, config::Config, desktop_state::DesktopAppContext,
    event_handlers::TaoEventCallback, ipc::UserWindowEvent, waker::create_dom_waker,
};
use dioxus_core::VirtualDom;
use std::rc::Rc;
use tao::{
    event::Event,
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
};

pub(super) struct State {
    control_flow: ControlFlow,
    event_handler: Option<TaoEventCallback>,
}

impl App {
    pub fn new(mut cfg: Config, virtual_dom: VirtualDom) -> (EventLoop<UserWindowEvent>, Self) {
        let event_loop = cfg
            .event_loop
            .take()
            .unwrap_or_else(|| EventLoopBuilder::<UserWindowEvent>::with_user_event().build());
        let tao_proxy = event_loop.create_proxy();
        let proxy = commands::from_tao(tao_proxy);
        let dom_waker = create_dom_waker(proxy.clone());
        let app_context = Rc::new(DesktopAppContext::new(proxy, event_loop.clone()));
        let host = State {
            control_flow: ControlFlow::Wait,
            event_handler: cfg.tao_event_handler.take(),
        };
        let app = Self::from_host(cfg, virtual_dom, dom_waker, app_context, host);

        (event_loop, app)
    }

    pub(crate) fn request_exit(&mut self) {
        if !matches!(self.host.control_flow, ControlFlow::Exit) {
            self.dispatch_app_event(AppEvent::ExitRequested);
            self.host.control_flow = ControlFlow::Exit;
        }
    }

    pub(crate) fn control_flow(&self) -> ControlFlow {
        self.host.control_flow
    }

    pub fn tick(&mut self, event: &Event<'_, UserWindowEvent>) {
        self.host.control_flow = ControlFlow::Wait;
        self.app_context
            .event_handlers
            .apply_tao(event, &self.app_context.target);
        if let Some(mut handler) = self.host.event_handler.take() {
            let app_context = self.app_context.clone();
            self.dom.in_runtime(|| handler(event, &app_context.target));
            self.host.event_handler = Some(handler);
        }

        match event {
            Event::WindowEvent {
                window_id, event, ..
            } => {
                if let Some(event) = WindowEvent::from_tao(event) {
                    self.dispatch_window_event(*window_id, event);
                }
                match event {
                    tao::event::WindowEvent::Resumed => self.dispatch_app_event(AppEvent::Resumed),
                    tao::event::WindowEvent::Suspended => {
                        self.dispatch_app_event(AppEvent::Suspended)
                    }
                    _ => {}
                }
            }
            Event::LoopDestroyed => self.dispatch_app_event(AppEvent::Exiting),
            Event::UserEvent(UserWindowEvent::Shutdown) => self.request_exit(),
            _ => {}
        }
    }

    pub fn handle_new_window(&mut self) {
        for pending_webview in self.app_context.drain_pending_webviews() {
            let app_webview = pending_webview.create_window(&mut self.dom, &self.app_context);
            let id = app_webview.desktop_context.window.id();
            self.webviews.insert(id, app_webview);
        }
    }

    pub fn handle_start_cause_init(&mut self) {
        self.provide_app_context();
        self.rebuild_dom();
        self.handle_new_window();
    }
}
