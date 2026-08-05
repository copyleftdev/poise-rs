use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use poise_discovery::{Revision, Snapshot, SnapshotReader, SnapshotStream};

/// A future that resolves with the next discovery snapshot.
///
/// The future borrows the stream and does not allocate. Dropping it leaves the
/// stream usable; a later call can continue waiting for the same next revision.
#[derive(Debug)]
pub struct NextSnapshot<'a, T> {
    stream: &'a mut SnapshotStream<T>,
}

impl<T> Future for NextSnapshot<'_, T> {
    type Output = Option<Arc<Snapshot<T>>>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().stream.poll_snapshot(context)
    }
}

/// Returns an allocation-free future for the stream's next snapshot.
pub const fn next_snapshot<T>(stream: &mut SnapshotStream<T>) -> NextSnapshot<'_, T> {
    NextSnapshot { stream }
}

/// Waits until a snapshot at or beyond `minimum` is available.
///
/// The current snapshot is returned immediately when it already satisfies the
/// requirement. `None` means the publisher closed before that revision arrived.
pub async fn wait_for_revision<T>(
    reader: &SnapshotReader<T>,
    minimum: Revision,
) -> Option<Arc<Snapshot<T>>> {
    // `subscribe` emits its current revision, closing the load-then-register
    // race that a separate `load` followed by `changes` would introduce.
    let mut changes = reader.subscribe();
    while let Some(snapshot) = next_snapshot(&mut changes).await {
        if snapshot.revision() >= minimum {
            return Some(snapshot);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use poise_discovery::{Revision, Snapshot, snapshot_channel};

    use super::{next_snapshot, wait_for_revision};

    #[tokio::test]
    async fn next_snapshot_awaits_a_future_revision() {
        let (mut publisher, reader) = snapshot_channel(Snapshot::empty());
        let mut changes = reader.changes();

        publisher
            .publish(Snapshot::new(Revision::new(1), vec!["one"]))
            .unwrap();

        let snapshot = next_snapshot(&mut changes).await.unwrap();
        assert_eq!(snapshot.revision(), Revision::new(1));
        assert_eq!(&**snapshot, &["one"]);
    }

    #[tokio::test]
    async fn wait_for_revision_returns_an_already_current_snapshot() {
        let (_publisher, reader) =
            snapshot_channel(Snapshot::new(Revision::new(7), vec!["current"]));

        let snapshot = wait_for_revision(&reader, Revision::new(5)).await.unwrap();
        assert_eq!(snapshot.revision(), Revision::new(7));
    }

    #[tokio::test]
    async fn wait_for_revision_survives_intermediate_revisions() {
        let (mut publisher, reader) = snapshot_channel(Snapshot::empty());
        let waiter = tokio::spawn(async move {
            wait_for_revision(&reader, Revision::new(3))
                .await
                .map(|snapshot| snapshot.revision())
        });

        tokio::task::yield_now().await;
        publisher
            .publish(Snapshot::new(Revision::new(1), vec![1]))
            .unwrap();
        tokio::task::yield_now().await;
        publisher
            .publish(Snapshot::new(Revision::new(2), vec![2]))
            .unwrap();
        tokio::task::yield_now().await;
        publisher
            .publish(Snapshot::new(Revision::new(3), vec![3]))
            .unwrap();

        assert_eq!(waiter.await.unwrap(), Some(Revision::new(3)));
    }

    #[tokio::test]
    async fn wait_for_revision_ends_when_the_publisher_closes() {
        let (publisher, reader) = snapshot_channel(Snapshot::<usize>::empty());
        drop(publisher);

        assert!(wait_for_revision(&reader, Revision::new(1)).await.is_none());
    }
}
