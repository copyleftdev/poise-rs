//! Versioned, atomically published backend membership.
//!
//! Discovery is deliberately separate from transport and selection. A
//! [`Directory`] applies membership changes and creates immutable [`Snapshot`]s.
//! A [`SnapshotPublisher`] makes a newer snapshot visible to every
//! [`SnapshotReader`] in one atomic operation.
//! Readers can also create independent [`SnapshotStream`] subscriptions. A
//! subscription is runtime-neutral, yields coherent revisions, coalesces bursts
//! to the newest state, and wakes when the publisher commits or closes.
//!
//! Removal is two-phase: [`Change::Remove`] marks a member as
//! [`Membership::Draining`], immediately excluding it from the policies in
//! `poise-core`; [`Directory::finish_drain`] physically removes it after a
//! dispatch layer has released outstanding work.
//!
//! # Example
//!
//! ```
//! use poise_core::{Backend, Policy, policy::RoundRobin};
//! use poise_discovery::{Change, Directory, Membership, snapshot_channel};
//!
//! let mut directory = Directory::new();
//! directory
//!     .apply(Change::Upsert("a", Backend::new("http://a")))
//!     .unwrap();
//! directory
//!     .apply(Change::Upsert("b", Backend::new("http://b")))
//!     .unwrap();
//!
//! let (mut publisher, reader) = snapshot_channel(directory.snapshot());
//! directory.apply(Change::Remove("a")).unwrap();
//! directory.publish(&mut publisher).unwrap();
//!
//! let snapshot = reader.load();
//! assert_eq!(snapshot[0].membership(), Membership::Draining);
//! let selected = RoundRobin::new().pick(&snapshot, &()).unwrap();
//! assert_eq!(snapshot[selected.index()].key(), &"b");
//! ```

#![forbid(unsafe_code)]

mod directory;
mod member;
mod revision;
mod snapshot;

pub use directory::{Applied, Change, Directory, Effect, RevisionExhausted};
pub use member::{Discovered, Membership};
pub use revision::Revision;
pub use snapshot::{
    PublishError, Snapshot, SnapshotPublisher, SnapshotReader, SnapshotStream, snapshot_channel,
};
