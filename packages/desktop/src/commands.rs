use crate::ipc::UserWindowEvent;

/// Queues application work and wakes the native event-loop owner.
///
/// Winit wake-ups can be coalesced, so the desktop owner drains the channel on
/// every wake. The command is always queued before the wake is requested.
#[derive(Clone)]
pub(crate) struct DesktopEventLoopProxy {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    sender: std::sync::mpsc::Sender<UserWindowEvent>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    waker: std::sync::Arc<dyn WakeEventLoop>,

    #[cfg(any(target_os = "android", target_os = "ios"))]
    proxy: tao::event_loop::EventLoopProxy<UserWindowEvent>,
}

#[derive(Debug)]
pub(crate) struct EventLoopClosed;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
trait WakeEventLoop: Send + Sync {
    fn wake_up(&self);
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl WakeEventLoop for winit::event_loop::EventLoopProxy {
    fn wake_up(&self) {
        self.wake_up();
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn channel(
    proxy: winit::event_loop::EventLoopProxy,
) -> (
    DesktopEventLoopProxy,
    std::sync::mpsc::Receiver<UserWindowEvent>,
) {
    channel_with_waker(std::sync::Arc::new(proxy))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn channel_with_waker(
    waker: std::sync::Arc<dyn WakeEventLoop>,
) -> (
    DesktopEventLoopProxy,
    std::sync::mpsc::Receiver<UserWindowEvent>,
) {
    let (sender, receiver) = std::sync::mpsc::channel();
    (DesktopEventLoopProxy { sender, waker }, receiver)
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) fn from_tao(
    proxy: tao::event_loop::EventLoopProxy<UserWindowEvent>,
) -> DesktopEventLoopProxy {
    DesktopEventLoopProxy { proxy }
}

impl DesktopEventLoopProxy {
    pub(crate) fn send_event(&self, event: UserWindowEvent) -> Result<(), EventLoopClosed> {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            self.sender.send(event).map_err(|_| EventLoopClosed)?;
            self.waker.wake_up();
            Ok(())
        }

        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.proxy.send_event(event).map_err(|_| EventLoopClosed)
        }
    }
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    struct ObservingWaker {
        receiver: Mutex<Option<std::sync::mpsc::Receiver<UserWindowEvent>>>,
        observed_queued_command: AtomicUsize,
    }

    impl WakeEventLoop for ObservingWaker {
        fn wake_up(&self) {
            let receiver = self.receiver.lock().unwrap();
            let queued = receiver
                .as_ref()
                .expect("test receiver must be installed before sending")
                .try_recv();
            if matches!(queued, Ok(UserWindowEvent::NewWindow)) {
                self.observed_queued_command.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    #[test]
    fn queues_command_before_waking_event_loop() {
        let waker = Arc::new(ObservingWaker {
            receiver: Mutex::new(None),
            observed_queued_command: AtomicUsize::new(0),
        });
        let (proxy, receiver) = channel_with_waker(waker.clone());
        *waker.receiver.lock().unwrap() = Some(receiver);

        proxy
            .send_event(UserWindowEvent::NewWindow)
            .expect("open command should be accepted");

        assert_eq!(waker.observed_queued_command.load(Ordering::SeqCst), 1);
    }

    struct CountingWaker(AtomicUsize);

    impl WakeEventLoop for CountingWaker {
        fn wake_up(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn preserves_all_commands_when_wakes_are_coalesced() {
        let waker = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let (proxy, receiver) = channel_with_waker(waker.clone());

        proxy.send_event(UserWindowEvent::Poll).unwrap();
        proxy.send_event(UserWindowEvent::Shutdown).unwrap();

        let commands: Vec<_> = receiver.try_iter().collect();
        assert!(matches!(
            commands.as_slice(),
            [UserWindowEvent::Poll, UserWindowEvent::Shutdown]
        ));
        assert_eq!(waker.0.load(Ordering::SeqCst), 2);
    }
}
