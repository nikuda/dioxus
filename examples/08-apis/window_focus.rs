//! Listen for window focus events using the backend-neutral window event hook.
//!
//! This example shows how to use the Dioxus window event hook to listen for focus changes.
//!
//! This lets you do things like backgrounding tasks, pausing animations, or changing the UI when the window is focused or not.

use dioxus::desktop::{WindowEvent, use_window_event_handler};
use dioxus::prelude::*;

fn main() {
    dioxus::launch(app);
}

fn app() -> Element {
    let mut focused = use_signal(|| true);

    use_window_event_handler(move |event, _| {
        if let WindowEvent::Focused(new_focused) = event {
            focused.set(new_focused)
        }
    });

    rsx! {
        div { width: "100%", height: "100%", display: "flex", flex_direction: "column", align_items: "center",
            if focused() {
                "This window is focused!"
            } else {
                "This window is not focused!"
            }
        }
    }
}
