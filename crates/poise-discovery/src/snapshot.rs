use std::{
    error::Error,
    fmt,
    ops::Deref,
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
};

use arc_swap::ArcSwap;
use futures_core::Stream;

use crate::Revision;

/// An immutable, coherent view of discovered members.
#[derive(Debug)]
pub struct Snapshot<T> {
    revision: Revision,
    members: Arc<[T]>,
}

impl<T> Snapshot<T> {
    /// Creates a snapshot from an owned member vector.
    #[must_use]
    pub fn new(revision: Revision, members: Vec<T>) -> Self {
        Self {
            revision,
            members: members.into(),
        }
    }

    /// Creates an empty initial snapshot.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Revision::INITIAL, Vec::new())
    }

    /// Returns the membership revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the snapshot members.
    #[must_use]
    pub fn members(&self) -> &[T] {
        &self.members
    }

    /// Returns a shared handle to the member allocation.
    #[must_use]
    pub fn members_arc(&self) -> Arc<[T]> {
        Arc::clone(&self.members)
    }

    /// Returns the number of members, including draining members.
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Returns whether the snapshot has no active or draining members.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

impl<T> Default for Snapshot<T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T> Deref for Snapshot<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.members()
    }
}

impl<T> AsRef<[T]> for Snapshot<T> {
    fn as_ref(&self) -> &[T] {
        self.members()
    }
}

/// Creates a single-writer publisher and cloneable atomic readers.
#[must_use]
pub fn snapshot_channel<T>(initial: Snapshot<T>) -> (SnapshotPublisher<T>, SnapshotReader<T>) {
    let shared = Arc::new(Shared {
        current: ArcSwap::from_pointee(initial),
        publisher_alive: AtomicBool::new(true),
        waiters: Mutex::new(Vec::new()),
    });
    (
        SnapshotPublisher {
            shared: Arc::clone(&shared),
        },
        SnapshotReader { shared },
    )
}

struct Waiter {
    waker: Mutex<Option<Waker>>,
}

struct Shared<T> {
    current: ArcSwap<Snapshot<T>>,
    publisher_alive: AtomicBool,
    waiters: Mutex<Vec<Weak<Waiter>>>,
}

impl<T> Shared<T> {
    fn wake_subscribers(&self) {
        let mut wake = Vec::new();
        self.lock_waiters().retain(|weak| {
            let Some(waiter) = weak.upgrade() else {
                return false;
            };
            if let Some(waker) = lock_waker(&waiter).take() {
                wake.push(waker);
            }
            true
        });
        for waker in wake {
            waker.wake();
        }
    }

    fn lock_waiters(&self) -> MutexGuard<'_, Vec<Weak<Waiter>>> {
        self.waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn lock_waker(waiter: &Waiter) -> MutexGuard<'_, Option<Waker>> {
    waiter
        .waker
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The single-writer side of an atomic snapshot cell.
///
/// Publishing requires `&mut self`, making the intended single-writer model
/// explicit. This type is deliberately not `Clone`.
pub struct SnapshotPublisher<T> {
    shared: Arc<Shared<T>>,
}

impl<T> SnapshotPublisher<T> {
    /// Atomically publishes a strictly newer snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`PublishError`] when the attempted revision is not newer than
    /// the currently published revision.
    pub fn publish(&mut self, snapshot: Snapshot<T>) -> Result<Arc<Snapshot<T>>, PublishError> {
        let current = self.shared.current.load_full();
        if snapshot.revision() <= current.revision() {
            return Err(PublishError {
                current: current.revision(),
                attempted: snapshot.revision(),
            });
        }

        let snapshot = Arc::new(snapshot);
        self.shared.current.store(Arc::clone(&snapshot));
        self.shared.wake_subscribers();
        Ok(snapshot)
    }

    /// Returns the currently published revision.
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.shared.current.load().revision()
    }
}

impl<T> Drop for SnapshotPublisher<T> {
    fn drop(&mut self) {
        self.shared.publisher_alive.store(false, Ordering::Release);
        self.shared.wake_subscribers();
    }
}

/// A cloneable reader for atomically published snapshots.
pub struct SnapshotReader<T> {
    shared: Arc<Shared<T>>,
}

impl<T> SnapshotReader<T> {
    /// Loads one coherent snapshot.
    #[must_use]
    pub fn load(&self) -> Arc<Snapshot<T>> {
        self.shared.current.load_full()
    }

    /// Returns the currently published revision.
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.shared.current.load().revision()
    }

    /// Subscribes to the current snapshot and coalesced future publications.
    ///
    /// The first poll yields the current snapshot. If several revisions are
    /// published before the next poll, the stream yields only the newest one.
    /// It ends after the publisher is dropped and the latest snapshot has been
    /// observed.
    #[must_use]
    pub fn subscribe(&self) -> SnapshotStream<T> {
        SnapshotStream::new(self.clone(), None)
    }

    /// Subscribes only to publications newer than the currently visible
    /// revision.
    ///
    /// Like [`subscribe`](Self::subscribe), intermediate revisions may be
    /// coalesced.
    #[must_use]
    pub fn changes(&self) -> SnapshotStream<T> {
        SnapshotStream::new(self.clone(), Some(self.revision()))
    }
}

impl<T> Clone for SnapshotReader<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

/// A runtime-neutral stream of coherent atomic snapshots.
///
/// Each subscriber owns an independent revision cursor and waker. Publications
/// are state, not an event log: a slow subscriber observes the latest revision
/// and may skip intermediate snapshots.
pub struct SnapshotStream<T> {
    reader: SnapshotReader<T>,
    waiter: Arc<Waiter>,
    last_seen: Option<Revision>,
    terminated: bool,
}

impl<T> SnapshotStream<T> {
    fn new(reader: SnapshotReader<T>, last_seen: Option<Revision>) -> Self {
        let waiter = Arc::new(Waiter {
            waker: Mutex::new(None),
        });
        reader.shared.lock_waiters().push(Arc::downgrade(&waiter));
        Self {
            reader,
            waiter,
            last_seen,
            terminated: false,
        }
    }

    /// Returns the latest revision yielded by this subscriber.
    #[must_use]
    pub const fn last_seen(&self) -> Option<Revision> {
        self.last_seen
    }

    /// Returns whether publisher closure has terminated this stream.
    #[must_use]
    pub const fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// Polls for the next coherent snapshot without requiring an extension
    /// trait import.
    pub fn poll_snapshot(&mut self, context: &mut Context<'_>) -> Poll<Option<Arc<Snapshot<T>>>> {
        Stream::poll_next(Pin::new(self), context)
    }
}

impl<T> Unpin for SnapshotStream<T> {}

impl<T> fmt::Debug for SnapshotStream<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotStream")
            .field("last_seen", &self.last_seen)
            .field("terminated", &self.terminated)
            .finish_non_exhaustive()
    }
}

impl<T> Stream for SnapshotStream<T> {
    type Item = Arc<Snapshot<T>>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.terminated {
            return Poll::Ready(None);
        }

        let current = self.reader.load();
        if self
            .last_seen
            .is_none_or(|revision| current.revision() > revision)
        {
            self.last_seen = Some(current.revision());
            return Poll::Ready(Some(current));
        }
        if !self.reader.shared.publisher_alive.load(Ordering::Acquire) {
            self.terminated = true;
            return Poll::Ready(None);
        }

        {
            let mut registered = lock_waker(&self.waiter);
            if registered
                .as_ref()
                .is_none_or(|waker| !waker.will_wake(context.waker()))
            {
                *registered = Some(context.waker().clone());
            }
        }

        // Close the check/register race with publication and publisher drop.
        let current = self.reader.load();
        if self
            .last_seen
            .is_some_and(|revision| current.revision() > revision)
        {
            lock_waker(&self.waiter).take();
            self.last_seen = Some(current.revision());
            return Poll::Ready(Some(current));
        }
        if !self.reader.shared.publisher_alive.load(Ordering::Acquire) {
            lock_waker(&self.waiter).take();
            self.terminated = true;
            return Poll::Ready(None);
        }

        Poll::Pending
    }
}

impl<T> fmt::Debug for SnapshotReader<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotReader")
            .field("revision", &self.revision())
            .finish_non_exhaustive()
    }
}

/// A rejected non-monotonic publication.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PublishError {
    current: Revision,
    attempted: Revision,
}

impl PublishError {
    /// Returns the revision visible when publication was attempted.
    #[must_use]
    pub const fn current(self) -> Revision {
        self.current
    }

    /// Returns the rejected revision.
    #[must_use]
    pub const fn attempted(self) -> Revision {
        self.attempted
    }
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "snapshot revision {} is not newer than published revision {}",
            self.attempted, self.current
        )
    }
}

impl Error for PublishError {}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
        },
        task::Wake,
        thread,
    };

    use super::*;

    #[derive(Default)]
    struct WakeCount(AtomicUsize);

    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, AtomicOrdering::Relaxed);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, AtomicOrdering::Relaxed);
        }
    }

    fn poll_stream<T>(
        stream: &mut SnapshotStream<T>,
        waker: &Waker,
    ) -> Poll<Option<Arc<Snapshot<T>>>> {
        Stream::poll_next(Pin::new(stream), &mut Context::from_waker(waker))
    }

    #[test]
    fn publication_requires_a_strictly_newer_revision() {
        let (mut publisher, reader) =
            snapshot_channel(Snapshot::new(Revision::new(5), vec!["current"]));

        for attempted in [4, 5] {
            let error = publisher
                .publish(Snapshot::new(Revision::new(attempted), vec!["stale"]))
                .unwrap_err();
            assert_eq!(error.current(), Revision::new(5));
            assert_eq!(error.attempted(), Revision::new(attempted));
        }
        assert_eq!(reader.load().members(), ["current"]);

        publisher
            .publish(Snapshot::new(Revision::new(6), vec!["new"]))
            .unwrap();
        assert_eq!(reader.load().members(), ["new"]);
    }

    #[test]
    fn concurrent_readers_never_observe_a_torn_snapshot() {
        const MEMBER_COUNT: usize = 64;
        const LAST_REVISION: u64 = 2_000;

        let (mut publisher, reader) =
            snapshot_channel(Snapshot::new(Revision::INITIAL, vec![0_u64; MEMBER_COUNT]));
        let writer = thread::spawn(move || {
            for revision in 1..=LAST_REVISION {
                publisher
                    .publish(Snapshot::new(
                        Revision::new(revision),
                        vec![revision; MEMBER_COUNT],
                    ))
                    .unwrap();
            }
        });

        let readers: Vec<_> = (0..4)
            .map(|_| {
                let reader = reader.clone();
                thread::spawn(move || {
                    for _ in 0..10_000 {
                        let snapshot = reader.load();
                        let expected = snapshot.revision().get();
                        assert_eq!(snapshot.len(), MEMBER_COUNT);
                        assert!(snapshot.iter().all(|member| *member == expected));
                    }
                })
            })
            .collect();

        writer.join().unwrap();
        for reader in readers {
            reader.join().unwrap();
        }
    }

    #[test]
    fn old_snapshot_handles_remain_valid_after_publication() {
        let (mut publisher, reader) =
            snapshot_channel(Snapshot::new(Revision::INITIAL, vec![1, 2]));
        let old = reader.load();

        publisher
            .publish(Snapshot::new(Revision::new(1), vec![3]))
            .unwrap();

        assert_eq!(old.members(), [1, 2]);
        assert_eq!(reader.load().members(), [3]);
    }

    #[test]
    fn subscription_yields_current_then_wakes_for_a_newer_snapshot() {
        let (mut publisher, reader) =
            snapshot_channel(Snapshot::new(Revision::INITIAL, vec!["initial"]));
        let mut stream = reader.subscribe();
        let wake_count = Arc::new(WakeCount::default());
        let waker = Waker::from(Arc::clone(&wake_count));

        let Poll::Ready(Some(initial)) = poll_stream(&mut stream, &waker) else {
            panic!("subscription did not yield its initial snapshot");
        };
        assert_eq!(initial.members(), ["initial"]);
        assert_eq!(stream.last_seen(), Some(Revision::INITIAL));
        assert!(poll_stream(&mut stream, &waker).is_pending());

        publisher
            .publish(Snapshot::new(Revision::new(1), vec!["new"]))
            .unwrap();
        assert_eq!(wake_count.0.load(AtomicOrdering::Relaxed), 1);
        let Poll::Ready(Some(new)) = poll_stream(&mut stream, &waker) else {
            panic!("subscription did not yield the published snapshot");
        };
        assert_eq!(new.revision(), Revision::new(1));
        assert_eq!(new.members(), ["new"]);
    }

    #[test]
    fn changes_waits_for_a_revision_newer_than_construction() {
        let (mut publisher, reader) = snapshot_channel(Snapshot::new(Revision::new(4), vec![4]));
        let mut changes = reader.changes();
        let waker = Waker::noop();

        assert_eq!(changes.last_seen(), Some(Revision::new(4)));
        assert!(poll_stream(&mut changes, waker).is_pending());
        publisher
            .publish(Snapshot::new(Revision::new(5), vec![5]))
            .unwrap();
        let Poll::Ready(Some(snapshot)) = poll_stream(&mut changes, waker) else {
            panic!("change stream did not yield the newer revision");
        };
        assert_eq!(snapshot.members(), [5]);
    }

    #[test]
    fn slow_subscribers_coalesce_to_the_latest_revision() {
        let (mut publisher, reader) = snapshot_channel(Snapshot::empty());
        let mut changes = reader.changes();
        publisher
            .publish(Snapshot::new(Revision::new(1), vec![1]))
            .unwrap();
        publisher
            .publish(Snapshot::new(Revision::new(2), vec![2]))
            .unwrap();

        let Poll::Ready(Some(snapshot)) = poll_stream(&mut changes, Waker::noop()) else {
            panic!("change stream did not yield the latest snapshot");
        };
        assert_eq!(snapshot.revision(), Revision::new(2));
        assert_eq!(snapshot.members(), [2]);
        assert!(poll_stream(&mut changes, Waker::noop()).is_pending());
    }

    #[test]
    fn publication_wakes_each_independent_subscriber() {
        let (mut publisher, reader) = snapshot_channel(Snapshot::<usize>::empty());
        let mut left = reader.changes();
        let mut right = reader.changes();
        let left_count = Arc::new(WakeCount::default());
        let right_count = Arc::new(WakeCount::default());
        let left_waker = Waker::from(Arc::clone(&left_count));
        let right_waker = Waker::from(Arc::clone(&right_count));
        assert!(poll_stream(&mut left, &left_waker).is_pending());
        assert!(poll_stream(&mut right, &right_waker).is_pending());

        publisher
            .publish(Snapshot::new(Revision::new(1), vec![1]))
            .unwrap();
        assert_eq!(left_count.0.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(right_count.0.load(AtomicOrdering::Relaxed), 1);
        assert!(matches!(
            poll_stream(&mut left, &left_waker),
            Poll::Ready(Some(_))
        ));
        assert!(matches!(
            poll_stream(&mut right, &right_waker),
            Poll::Ready(Some(_))
        ));
    }

    #[test]
    fn publisher_drop_wakes_and_terminates_subscribers_after_latest_state() {
        let (publisher, reader) = snapshot_channel(Snapshot::new(Revision::INITIAL, vec!["last"]));
        let mut stream = reader.subscribe();
        let wake_count = Arc::new(WakeCount::default());
        let waker = Waker::from(Arc::clone(&wake_count));
        assert!(matches!(
            poll_stream(&mut stream, &waker),
            Poll::Ready(Some(_))
        ));
        assert!(poll_stream(&mut stream, &waker).is_pending());

        drop(publisher);
        assert_eq!(wake_count.0.load(AtomicOrdering::Relaxed), 1);
        assert!(matches!(
            poll_stream(&mut stream, &waker),
            Poll::Ready(None)
        ));
        assert!(stream.is_terminated());
        assert!(matches!(
            poll_stream(&mut stream, &waker),
            Poll::Ready(None)
        ));
    }

    #[test]
    fn rejected_publication_does_not_wake_subscribers() {
        let (mut publisher, reader) = snapshot_channel(Snapshot::<usize>::empty());
        let mut changes = reader.changes();
        let wake_count = Arc::new(WakeCount::default());
        let waker = Waker::from(Arc::clone(&wake_count));
        assert!(poll_stream(&mut changes, &waker).is_pending());

        assert!(
            publisher
                .publish(Snapshot::new(Revision::INITIAL, vec![1]))
                .is_err()
        );
        assert_eq!(wake_count.0.load(AtomicOrdering::Relaxed), 0);
        assert!(poll_stream(&mut changes, &waker).is_pending());
    }
}
