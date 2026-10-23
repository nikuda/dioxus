Dioxus Desktop Renderer

Render the Dioxus VirtualDom using the platform's native WebView implementation.

# Desktop

One of Dioxus' flagship features is the ability to quickly build a native desktop app that looks and feels the same across platforms. Apps built with Dioxus are typically <5mb in size and use existing system resources, so they won't hog extreme amounts of RAM or memory.

Dioxus Desktop uses [Winit](https://github.com/rust-windowing/winit) for its
desktop window and event-loop host and [Wry](https://github.com/tauri-apps/wry)
for the system WebView. Android and iOS temporarily retain their existing
[Tao](https://github.com/tauri-apps/tao) host. Dioxus provides
backend-neutral application and window events for shared code, with explicit
Winit and Tao hooks when direct host access is needed.

## Getting Set up

Getting Set up with Dioxus-Desktop is quite easy. Make sure you have Rust and Cargo installed, and then create a new project:

```shell
$ cargo new --bin demo
$ cd app
```

Add Dioxus and the `desktop` renderer feature:

```shell
$ cargo add dioxus
$ cargo add dioxus-desktop
```

Edit your `main.rs`:

```rust, ignore
// main.rs
use dioxus::prelude::*;
use dioxus_desktop::Config;

fn main() {
    let config = Config::new();
    #[cfg(target_os = "linux")]
    // SAFETY: this is the process entry point, before application threads or
    // GTK/Winit/WebKit initialization.
    let config = unsafe {
        config
            .prepare_linux_host()
            .expect("prepare the Dioxus Desktop Linux host")
    };

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(app);
}

fn app() -> Element {
    rsx! {
        div {
            "hello world!"
        }
    }
}
```

To configure the webview, menubar, and other important desktop-specific features, checkout out some of the launch configuration in the [API reference](https://docs.rs/dioxus-desktop/).

## Future Steps

Make sure to read the [Dioxus Guide](https://dioxuslabs.com/learn/0.7/) if you already haven't!
