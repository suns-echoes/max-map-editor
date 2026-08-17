//! Minimal executor for wgpu's setup futures (`request_adapter`,
//! `request_device`), which complete without external I/O drivers on native
//! backends. Hand-rolled to keep the dependency tree minimal (no `pollster`).

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;

    /// A future that is `Pending` on the first poll and hands its waker to
    /// another thread — the exact shape of wgpu's setup futures, whose
    /// callbacks fire on driver threads.
    struct YieldOnce(bool);

    impl Future for YieldOnce {
        type Output = u32;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
            if self.0 {
                Poll::Ready(42)
            } else {
                self.0 = true;
                let waker = cx.waker().clone();
                thread::spawn(move || waker.wake());
                Poll::Pending
            }
        }
    }

    /// `block_on` must park on `Pending` and resume when the waker fires from
    /// another thread. The park/unpark token makes this race-free: even a
    /// wake delivered before the park unblocks it immediately.
    #[test]
    fn parks_until_a_cross_thread_wake() {
        assert_eq!(block_on(YieldOnce(false)), 42);
    }
}
