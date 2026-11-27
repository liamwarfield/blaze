//! The build arena: the owner of state that used to live "for the process" because Blaze ran
//! one command per process (frozen Starlark heaps that other values point into without owning
//! them, memo tables keyed by the address of a frozen value, per-command counters).
//!
//! PORT: Blaze-only (decision D7, docs/research/08-server-and-incrementality.md 3.3 option a).
//! Java has no counterpart: the JVM's collector frees a heap once nothing references it. In
//! Blaze a frozen value is often held without its heap (`unchecked_frozen_value`, an
//! `OwnedFrozenValue` with an empty owner); such a value is valid as long as some other holder
//! keeps the heap, and that holder is a slot of this arena.
//!
//! The arena is free of Starlark types. Each user crate keeps its own typed state in a slot
//! ([`ArenaSlot`], one slot per Rust type) through [`slot`], which replaces the former static.
//!
//! Lifetime. There is one current arena per process. Whoever owns an evaluator whose values hold
//! frozen Starlark values holds a [`Lease`] for as long as the evaluator and its graph exist (the
//! per-command `SkyframeExecutor`, and every `BzlLoadFunction`, which every evaluator that loads
//! a `.bzl` file owns). The contents are released only
//!
//! 1. when no lease is held, and
//! 2. on an explicit [`release_if_unused`], which only the persistent server makes, once a
//!    command has completely ended (after `afterCommand`, the graph dropped, the command's
//!    threads joined), while it still holds the command lock (blaze_runtime
//!    `BlazeCommandDispatcher::release_build_arena_after_commands`; not after a crash or a
//!    `shutdown`).
//!
//! Per-command state does not depend on a release: counters are reset where Java resets them
//! (for example the package progress at `syncPackageLoading`); only state that points into the
//! heaps (memo tables keyed by frozen values, caches of values that hold frozen values without
//! owning their heap) lives here.
//!
//! Nothing in a slot's heaps may own (through a `FrozenHeapRef` or an owning `OwnedFrozenValue`)
//! a heap that holds it, or the heap owns itself and a release never frees it (the leak of
//! `repository_rule` modules found by the review of 2026-11-25); values cached inside a heap hold
//! its frozen values without owning the heap, as this arena keeps it.
//!
//! Batch mode, the in-process mode and test binaries never call [`release_if_unused`]: there
//! everything still lives until the process ends, as before. Concurrent executors in one process
//! (cargo tests) are safe by rule 1.
//!
//! Release order: the graph is gone first (rule 1 and 2); then the slots in the order of
//! [`ReleaseOrder`]: per-command state, then memo tables, then the heaps.
//!
//! [`slot`] hands out an `Arc` of the slot: a caller that still holds it across a release keeps
//! that slot's contents alive (never a dangling one), so callers take the slot for the duration
//! of one operation only and never store it.
//!
//! Two rules keep a release sound; break either and a server use after free follows:
//!
//! 1. No thread that can outlive the command that started it (a detached thread: a worker's
//!    pipe reader, an `AsyncFinish` worker, a download helper, a background upload that is not
//!    joined before `afterCommand`) may hold a frozen Starlark value, an action, a spawn, or
//!    anything else that points into the arena's heaps without owning them. Such a thread
//!    captures plain data only (paths, digests, requests, process handles). The lease count does
//!    not cover detached threads.
//! 2. Every holder of an unowned frozen handle lives in a leased evaluator's graph, in the
//!    evaluation running now, in a slot of this arena, or on the heap the handle points into.
//!    No process static, module or cache keeps one across commands.
//!
//! Unowned handles record the generation they were made in ([`ArenaStamp`]) and check it where
//! the value is used, so a handle that outlived its arena is a crash with a message, not
//! undefined behaviour.

use std::any::{Any, TypeId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use parking_lot::{Mutex, RwLock};

/// When a slot's contents are dropped at a release, relative to the other slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReleaseOrder {
    /// Per-command state (counters, caches whose values own what they point to): first.
    State,
    /// Memo tables keyed by the address of a frozen value: after the state.
    Memo,
    /// The pinned frozen heaps every address and unchecked value of the other slots points into:
    /// last.
    Heaps,
}

/// A type kept in a slot of the arena: its initial value is `Default::default()`.
pub trait ArenaSlot: Default + Send + Sync + 'static {
    /// When the slot is dropped at a release.
    const RELEASE_ORDER: ReleaseOrder;

    /// How many entries (heaps, modules) the slot holds now, for the server's diagnostics
    /// ([`slot_counts`]); 0 for slots that do not count.
    fn held_count(&self) -> usize {
        0
    }
}

struct Slot {
    type_id: TypeId,
    order: ReleaseOrder,
    value: Arc<dyn Any + Send + Sync>,
    name: &'static str,
    count: fn(&Arc<dyn Any + Send + Sync>) -> usize,
}

fn count_of<T: ArenaSlot>(value: &Arc<dyn Any + Send + Sync>) -> usize {
    downcast::<T>(value).held_count()
}

/// The slots of one arena.
#[derive(Default)]
pub struct BuildArena {
    slots: RwLock<Vec<Slot>>,
}

impl BuildArena {
    pub fn new() -> BuildArena {
        BuildArena::default()
    }

    /// The slot of type `T`, created empty on first use.
    pub fn slot<T: ArenaSlot>(&self) -> Arc<T> {
        let type_id = TypeId::of::<T>();
        if let Some(s) = self.slots.read().iter().find(|s| s.type_id == type_id) {
            return downcast(&s.value);
        }
        let mut slots = self.slots.write();
        if let Some(s) = slots.iter().find(|s| s.type_id == type_id) {
            return downcast(&s.value);
        }
        let value: Arc<T> = Arc::new(T::default());
        slots.push(Slot {
            type_id,
            order: T::RELEASE_ORDER,
            value: value.clone(),
            name: std::any::type_name::<T>(),
            count: count_of::<T>,
        });
        value
    }
}

fn downcast<T: ArenaSlot>(value: &Arc<dyn Any + Send + Sync>) -> Arc<T> {
    value
        .clone()
        .downcast::<T>()
        .unwrap_or_else(|_| panic!("build arena slot keyed by its type"))
}

impl Drop for BuildArena {
    fn drop(&mut self) {
        let mut slots = std::mem::take(&mut *self.slots.write());
        // A stable sort keeps the creation order within one release order.
        slots.sort_by_key(|s| s.order);
        for slot in slots {
            drop(slot);
        }
    }
}

struct Current {
    arena: Arc<BuildArena>,
    leases: usize,
    /// Number of releases so far.
    generation: u64,
}

static CURRENT: LazyLock<Mutex<Current>> = LazyLock::new(|| {
    Mutex::new(Current {
        arena: Arc::new(BuildArena::new()),
        leases: 0,
        generation: 0,
    })
});

/// The slot of type `T` of the current arena.
pub fn slot<T: ArenaSlot>() -> Arc<T> {
    let arena = CURRENT.lock().arena.clone();
    arena.slot::<T>()
}

/// A claim on the current arena: while any lease is held, [`release_if_unused`] releases
/// nothing. Dropping it ends the claim.
#[must_use = "the arena may be released once the lease is dropped"]
pub struct Lease {
    _private: (),
}

/// Takes a lease on the current arena (see the module comment for who holds one).
pub fn lease() -> Lease {
    CURRENT.lock().leases += 1;
    Lease { _private: () }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut current = CURRENT.lock();
        current.leases = current
            .leases
            .checked_sub(1)
            .expect("build arena lease count underflow");
    }
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("build_arena::Lease")
    }
}

/// The entries each counting slot of the current arena holds (`(type name, count)`, slots that
/// count only): Blaze-only diagnostics of what a kept graph keeps alive.
pub fn slot_counts() -> Vec<(&'static str, usize)> {
    let arena = CURRENT.lock().arena.clone();
    let slots = arena.slots.read();
    slots
        .iter()
        .map(|s| (s.name, (s.count)(&s.value)))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// The number of leases held now.
pub fn active_leases() -> usize {
    CURRENT.lock().leases
}

/// The number of releases made so far in this process.
pub fn generation() -> u64 {
    CURRENT.lock().generation
}

/// [`generation`], readable without the lock (written under it at each release).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The arena generation an unowned frozen handle (an `OwnedFrozenValue` with an empty owner, a
/// bare `FrozenValue` kept by this arena's heaps) was made in. [`ArenaStamp::check`] fails
/// loudly when the arena of that generation has been released since, which only a holder that
/// broke the module's rules can observe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArenaStamp(u64);

impl ArenaStamp {
    /// The stamp of the current arena.
    pub fn current() -> ArenaStamp {
        ArenaStamp(GENERATION.load(Ordering::Acquire))
    }

    /// The generation recorded.
    pub fn generation(self) -> u64 {
        self.0
    }

    /// Panics unless the arena this stamp was taken in is still the current one. `what` names
    /// the handle in the message.
    #[track_caller]
    pub fn check(self, what: &str) {
        let now = GENERATION.load(Ordering::Acquire);
        if now != self.0 {
            panic!(
                "IllegalStateException: {what} points into a frozen heap of build arena \
                 generation {}, which was released (the current generation is {now}): a holder \
                 kept it past the end of the command that made it",
                self.0
            );
        }
    }
}

/// Releases the current arena's contents (in [`ReleaseOrder`]) and starts an empty one, if no
/// lease is held; returns whether it did.
///
/// Only the server calls this, once a command has completely ended (see the module comment);
/// nothing else may, because outside a server values that point into the arena's heaps are held
/// without a lease (a test's loaded modules, the single command of a batch process).
pub fn release_if_unused() -> bool {
    let old = {
        let mut current = CURRENT.lock();
        if current.leases != 0 {
            return false;
        }
        current.generation += 1;
        GENERATION.store(current.generation, Ordering::Release);
        std::mem::replace(&mut current.arena, Arc::new(BuildArena::new()))
    };
    // Dropped outside the lock: a slot's drop may take a slot of the new arena.
    drop(old);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // The tests share the process's current arena; they run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// Records the order in which slots were dropped.
    static DROPS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

    #[derive(Default)]
    struct Heaps(AtomicUsize);
    impl ArenaSlot for Heaps {
        const RELEASE_ORDER: ReleaseOrder = ReleaseOrder::Heaps;
    }
    impl Drop for Heaps {
        fn drop(&mut self) {
            DROPS.lock().push("heaps");
        }
    }

    #[derive(Default)]
    struct Memo(AtomicUsize);
    impl ArenaSlot for Memo {
        const RELEASE_ORDER: ReleaseOrder = ReleaseOrder::Memo;
    }
    impl Drop for Memo {
        fn drop(&mut self) {
            DROPS.lock().push("memo");
        }
    }

    #[derive(Default)]
    struct State(AtomicUsize);
    impl ArenaSlot for State {
        const RELEASE_ORDER: ReleaseOrder = ReleaseOrder::State;
    }
    impl Drop for State {
        fn drop(&mut self) {
            DROPS.lock().push("state");
        }
    }

    fn reset() {
        assert_eq!(active_leases(), 0);
        assert!(release_if_unused());
        DROPS.lock().clear();
    }

    #[test]
    fn slot_is_shared_until_release() {
        let _serial = SERIAL.lock();
        reset();
        slot::<State>().0.fetch_add(1, Ordering::SeqCst);
        assert_eq!(slot::<State>().0.load(Ordering::SeqCst), 1);
        let before = generation();
        assert!(release_if_unused());
        assert_eq!(generation(), before + 1);
        assert_eq!(slot::<State>().0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn release_only_at_zero_leases() {
        let _serial = SERIAL.lock();
        reset();
        slot::<Memo>().0.store(7, Ordering::SeqCst);
        let a = lease();
        let b = lease();
        assert_eq!(active_leases(), 2);
        assert!(!release_if_unused());
        drop(a);
        assert!(!release_if_unused());
        assert_eq!(slot::<Memo>().0.load(Ordering::SeqCst), 7);
        drop(b);
        assert_eq!(active_leases(), 0);
        assert!(release_if_unused());
        assert_eq!(slot::<Memo>().0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn drop_order_state_memo_heaps() {
        let _serial = SERIAL.lock();
        reset();
        // Created in the reverse of the release order.
        slot::<Heaps>();
        slot::<Memo>();
        slot::<State>();
        assert!(release_if_unused());
        assert_eq!(*DROPS.lock(), vec!["state", "memo", "heaps"]);
    }

    #[test]
    fn held_slot_outlives_release() {
        let _serial = SERIAL.lock();
        reset();
        let held = slot::<Heaps>();
        held.0.store(3, Ordering::SeqCst);
        assert!(release_if_unused());
        // The holder keeps the old slot; the new arena has a fresh one.
        assert!(DROPS.lock().is_empty());
        assert_eq!(held.0.load(Ordering::SeqCst), 3);
        assert_eq!(slot::<Heaps>().0.load(Ordering::SeqCst), 0);
        drop(held);
        assert_eq!(*DROPS.lock(), vec!["heaps"]);
    }

    #[test]
    fn concurrent_leases() {
        let _serial = SERIAL.lock();
        reset();
        let barrier = std::sync::Barrier::new(9);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let lease = lease();
                    for _ in 0..1000 {
                        slot::<Memo>().0.fetch_add(1, Ordering::SeqCst);
                    }
                    barrier.wait();
                    // Every thread holds its lease here: nothing is released.
                    barrier.wait();
                    drop(lease);
                });
            }
            barrier.wait();
            assert_eq!(active_leases(), 8);
            assert!(!release_if_unused());
            assert_eq!(slot::<Memo>().0.load(Ordering::SeqCst), 8000);
            barrier.wait();
        });
        assert_eq!(active_leases(), 0);
        assert!(release_if_unused());
        assert_eq!(slot::<Memo>().0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stamp_fails_loudly_after_release() {
        let _serial = SERIAL.lock();
        reset();
        let stamp = ArenaStamp::current();
        stamp.check("handle");
        assert!(release_if_unused());
        let result = std::panic::catch_unwind(|| stamp.check("RepoRule.impl_"));
        let message = result.unwrap_err();
        let message = message
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(
            message.contains("RepoRule.impl_ points into a frozen heap of build arena generation"),
            "{message}"
        );
        ArenaStamp::current().check("handle");
    }
}
