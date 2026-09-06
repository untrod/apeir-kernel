use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<CancellationState>,
}

#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notification: Notify,
}

impl CancellationToken {
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notification.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.inner.notification.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CancellationToken;
    use std::time::Duration;

    #[tokio::test]
    async fn cancellation_notification_is_not_lost_during_registration() {
        for _ in 0..1_000 {
            let token = CancellationToken::default();
            let waiter = {
                let token = token.clone();
                tokio::spawn(async move { token.cancelled().await })
            };
            tokio::task::yield_now().await;
            token.cancel();
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .expect("cancellation waiter must not miss notification")
                .expect("cancellation waiter must not panic");
        }
    }
}
