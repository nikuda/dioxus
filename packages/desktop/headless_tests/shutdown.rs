use dioxus::prelude::*;

#[path = "./utils.rs"]
mod utils;

#[cfg(not(windows))]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(not(windows))]
static EXIT_STATE: AtomicU8 = AtomicU8::new(0);

fn main() {
    #[cfg(not(windows))]
    {
        let config =
            dioxus::desktop::Config::new().with_app_event_handler(move |event, _| match event {
                dioxus::desktop::AppEvent::ExitRequested => {
                    if EXIT_STATE.load(Ordering::SeqCst) == 0 {
                        EXIT_STATE.store(1, Ordering::SeqCst);
                    } else {
                        EXIT_STATE.store(u8::MAX, Ordering::SeqCst);
                    }
                }
                dioxus::desktop::AppEvent::Exiting => {
                    if EXIT_STATE.load(Ordering::SeqCst) == 1 {
                        EXIT_STATE.store(2, Ordering::SeqCst);
                    } else {
                        EXIT_STATE.store(u8::MAX, Ordering::SeqCst);
                    }
                }
                _ => {}
            });

        utils::check_app_exits_with_config(app, config);
    }
}

fn app() -> Element {
    #[cfg(not(windows))]
    use_drop(|| {
        let observed = EXIT_STATE.load(Ordering::SeqCst);
        if observed != 2 {
            eprintln!(
                "the desktop owner did not dispatch AppEvent::ExitRequested then AppEvent::Exiting; final state was {observed}"
            );
            std::process::exit(exitcode::SOFTWARE);
        }
    });

    let desktop_app = dioxus::desktop::use_app();

    use_hook(move || {
        spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            desktop_app.shutdown();
        });
    });

    rsx! { div { "shutdown test" } }
}
