//! Restart long-lived background tasks after panic or unexpected return.

use crate::live::Shared;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Keep a configured loop alive; a failed instance ends before its replacement starts.
pub fn spawn<F, Fut>(name: &'static str, shared: &Arc<Shared>, body: F) -> JoinHandle<()>
where
    F: Fn(Arc<Shared>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let shared = shared.clone();
    spawn_with(name, move || body(shared.clone()), Duration::from_secs(5))
}

fn spawn_with<F, Fut>(name: &'static str, body: F, first_delay: Duration) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let body = Arc::new(body);
    tokio::spawn(async move {
        let mut delay = first_delay;
        loop {
            let started = std::time::Instant::now();
            let factory = body.clone();
            // The factory runs inside the child too: initialization can panic before first poll.
            let mut child = Child(tokio::spawn(async move { factory().await }));
            let result = (&mut child.0).await;
            let lived = started.elapsed();
            match result {
                Ok(()) => tracing::error!("background loop {name} returned; restarting in {}s", delay.as_secs()),
                Err(e) => tracing::error!("background loop {name} failed: {e}; restarting in {}s", delay.as_secs()),
            }
            tokio::time::sleep(delay).await;
            delay = next_delay(delay, lived, first_delay);
        }
    })
}

// Dropping a supervisor must cancel its async child, rather than detach a second writer.
// Blocking jobs already submitted continue until completion; never restart on a mere timeout.
struct Child(JoinHandle<()>);
impl Drop for Child {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn next_delay(delay: Duration, lived: Duration, first: Duration) -> Duration {
    if lived >= Duration::from_secs(600) { first } else { (delay * 2).min(Duration::from_secs(300)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn rapid_failures_back_off_and_a_healthy_run_resets_the_delay() {
        let first = Duration::from_secs(5);
        assert_eq!(next_delay(first, Duration::ZERO, first), Duration::from_secs(10));
        assert_eq!(next_delay(Duration::from_secs(300), Duration::ZERO, first), Duration::from_secs(300));
        assert_eq!(next_delay(Duration::from_secs(300), Duration::from_secs(600), first), first);
    }

    #[tokio::test]
    async fn cancelling_the_supervisor_drops_its_child() {
        struct NotifyDrop(tokio::sync::mpsc::Sender<()>);
        impl Drop for NotifyDrop {
            fn drop(&mut self) {
                self.0.try_send(()).unwrap();
            }
        }
        let (started, mut ready) = tokio::sync::mpsc::channel(1);
        let (stopped, mut ended) = tokio::sync::mpsc::channel(1);
        let handle = spawn_with(
            "cancellation drill",
            move || {
                let (started, stopped) = (started.clone(), stopped.clone());
                async move {
                    let _guard = NotifyDrop(stopped);
                    started.send(()).await.unwrap();
                    std::future::pending::<()>().await;
                }
            },
            Duration::ZERO,
        );
        ready.recv().await.unwrap();
        handle.abort();
        assert!(matches!(tokio::time::timeout(Duration::from_secs(1), ended.recv()).await, Ok(Some(()))));
    }

    #[tokio::test]
    async fn panic_and_unexpected_return_both_resume_useful_work() {
        for panic_first in [true, false] {
            let count = Arc::new(AtomicUsize::new(0));
            let attempts = count.clone();
            let (send, mut recv) = tokio::sync::mpsc::channel(1);
            let handle = spawn_with(
                "fault drill",
                move || {
                    let n = attempts.fetch_add(1, Ordering::SeqCst);
                    let send = send.clone();
                    async move {
                        if n == 0 {
                            assert!(!panic_first, "injected task panic");
                            return;
                        }
                        send.send(()).await.unwrap();
                        std::future::pending::<()>().await;
                    }
                },
                Duration::ZERO,
            );
            let recovered = tokio::time::timeout(Duration::from_secs(1), recv.recv()).await;
            handle.abort();
            assert!(matches!(recovered, Ok(Some(()))), "failed loop never resumed useful work: {recovered:?}");
            assert_eq!(count.load(Ordering::SeqCst), 2, "exactly one replacement");
        }
    }
}
