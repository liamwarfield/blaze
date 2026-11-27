//! Port of `lib/remote/ReaderPreferringReadWriteLock.java`.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_shell::InterruptedException;
use parking_lot::{Condvar, Mutex, MutexGuard};

// Java: com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLock.java:45 ReaderPreferringReadWriteLock.WRITER
const WRITER: i32 = -1;
// Java: com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLock.java:46 ReaderPreferringReadWriteLock.WRITER_WAITING
const WRITER_WAITING: i32 = 1 << 30;
// Java: com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLock.java:47 ReaderPreferringReadWriteLock.READER_COUNT_MASK
const READER_COUNT_MASK: i32 = WRITER_WAITING - 1;

/// A non-reentrant read-write lock that admits new readers even while a writer is waiting.
///
/// A reader only ever waits for a writer that holds the lock, never for other readers or for a
/// waiting writer. A writer waits until the lock is free and may thus be starved by a steady
/// stream of readers. Writers exclude each other.
///
/// The monitor is only entered by writers and by the last reader releasing the lock while a
/// writer is waiting for it.
///
/// PORT: Java's object monitor (`synchronized`, `wait`, `notifyAll`) is a mutex and a condition
/// variable; `IllegalMonitorStateException` is a panic with that name. A waiting thread is woken
/// by an interrupt through an interrupt listener (`Object.wait` throws `InterruptedException`).
// Java: com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLock.java:32 ReaderPreferringReadWriteLock
pub struct ReaderPreferringReadWriteLock {
    // WRITER means that the writer holds the lock. Otherwise, the low bits count the readers
    // holding it and WRITER_WAITING is set while a writer waits for them to release it. The lock
    // is free at zero.
    holds: AtomicI32,
    monitor: Mutex<()>,
    notified: Condvar,
}

impl Default for ReaderPreferringReadWriteLock {
    fn default() -> Self {
        ReaderPreferringReadWriteLock {
            holds: AtomicI32::new(0),
            monitor: Mutex::new(()),
            notified: Condvar::new(),
        }
    }
}

impl ReaderPreferringReadWriteLock {
    pub fn new() -> Arc<ReaderPreferringReadWriteLock> {
        Arc::new(ReaderPreferringReadWriteLock::default())
    }

    fn compare_and_set(&self, expected: i32, new: i32) -> bool {
        self.holds
            .compare_exchange(expected, new, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// `Object.wait()` on the monitor held through `guard`: fails with `InterruptedException`
    /// (clearing the interrupt) if the thread is or gets interrupted.
    fn wait(self: &Arc<Self>, guard: &mut MutexGuard<'_, ()>) -> Result<(), InterruptedException> {
        let me = self.clone();
        let waiter = std::thread::current().id();
        let _registration =
            blaze_shell::ThreadInterrupt::current().add_interrupt_listener(Arc::new(move || {
                // Run at once on this thread when it is interrupted already, while it holds the
                // monitor: the check below sees the interrupt. From another thread, taking the
                // monitor orders the notification after this thread starts waiting.
                if std::thread::current().id() == waiter {
                    return;
                }
                let _monitor = me.monitor.lock();
                me.notified.notify_all();
            }));
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(InterruptedException);
        }
        self.notified.wait(guard);
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(InterruptedException);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLock.java:54 ReaderPreferringReadWriteLock.lockReadInterruptibly
    pub fn lock_read_interruptibly(self: &Arc<Self>) -> Result<(), InterruptedException> {
        loop {
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(InterruptedException);
            }
            let current_holds = self.holds.load(Ordering::SeqCst);
            if current_holds != WRITER {
                // Readers are admitted even if a writer is waiting.
                if self.compare_and_set(current_holds, current_holds + 1) {
                    return Ok(());
                }
            } else {
                let mut guard = self.monitor.lock();
                // Rechecked under the monitor so that the notification in unlockWrite can't be
                // missed.
                while self.holds.load(Ordering::SeqCst) == WRITER {
                    self.wait(&mut guard)?;
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLock.java:76 ReaderPreferringReadWriteLock.unlockRead
    pub fn unlock_read(&self) {
        let mut current_holds;
        loop {
            current_holds = self.holds.load(Ordering::SeqCst);
            if current_holds == WRITER || (current_holds & READER_COUNT_MASK) == 0 {
                panic!("IllegalMonitorStateException: holds: {current_holds}");
            }
            // Clear the record of a waiting writer when the last reader releases the lock, even
            // if all waiting writers have been interrupted. Notified writers recheck the state
            // and record themselves again if needed.
            let new_holds = if current_holds == WRITER_WAITING + 1 {
                0
            } else {
                current_holds - 1
            };
            if self.compare_and_set(current_holds, new_holds) {
                break;
            }
        }
        if current_holds == WRITER_WAITING + 1 {
            // This was the last reader and a writer may still be waiting for it.
            let _monitor = self.monitor.lock();
            self.notified.notify_all();
        }
    }

    // Java: com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLock.java:97 ReaderPreferringReadWriteLock.lockWriteInterruptibly
    pub fn lock_write_interruptibly(self: &Arc<Self>) -> Result<(), InterruptedException> {
        let mut guard = self.monitor.lock();
        loop {
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(InterruptedException);
            }
            let current_holds = self.holds.load(Ordering::SeqCst);
            if current_holds == 0 {
                // Barging ahead of another waiting writer is safe since it will be woken up and
                // recheck the state after the next unlock.
                if self.compare_and_set(current_holds, WRITER) {
                    return Ok(());
                }
            } else if current_holds == WRITER || (current_holds & WRITER_WAITING) != 0 {
                self.wait(&mut guard)?;
            } else {
                // Readers hold the lock. Record the waiting writer so that the last reader
                // notifies it.
                let _unused = self.compare_and_set(current_holds, current_holds | WRITER_WAITING);
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLock.java:120 ReaderPreferringReadWriteLock.unlockWrite
    pub fn unlock_write(&self) {
        if !self.compare_and_set(WRITER, 0) {
            panic!(
                "IllegalMonitorStateException: holds: {}",
                self.holds.load(Ordering::SeqCst)
            );
        }
        let _monitor = self.monitor.lock();
        self.notified.notify_all();
    }

    /// The monitor, for tests that hold it (`synchronized (lock)`).
    #[cfg(test)]
    fn monitor(&self) -> MutexGuard<'_, ()> {
        self.monitor.lock()
    }
}

impl fmt::Display for ReaderPreferringReadWriteLock {
    // Java: com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLock.java:130 ReaderPreferringReadWriteLock.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let current_holds = self.holds.load(Ordering::SeqCst);
        let writer = current_holds == WRITER;
        write!(
            f,
            "ReaderPreferringReadWriteLock[readers={}, writer={}, writerWaiting={}]",
            if writer {
                0
            } else {
                current_holds & READER_COUNT_MASK
            },
            writer,
            !writer && (current_holds & WRITER_WAITING) != 0
        )
    }
}

impl fmt::Debug for ReaderPreferringReadWriteLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// `ReaderPreferringReadWriteLockTest`.
///
/// PORT: Java waits for a thread to reach `Thread.State.WAITING` / `BLOCKED`; Rust threads do
/// not expose their state, so the tests wait until the lock records the waiting writer
/// (`writerWaiting=true`) where that is what the state means, and otherwise give the other
/// thread time to block before checking that it has not acquired the lock.
// Java: src/test/java/com/google/devtools/build/lib/remote/ReaderPreferringReadWriteLockTest.java
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    #[derive(Clone, Copy, Debug)]
    enum Group {
        Read,
        Write,
    }

    impl Group {
        fn lock(
            self,
            lock: &Arc<ReaderPreferringReadWriteLock>,
        ) -> Result<(), InterruptedException> {
            match self {
                Group::Read => lock.lock_read_interruptibly(),
                Group::Write => lock.lock_write_interruptibly(),
            }
        }

        fn unlock(self, lock: &ReaderPreferringReadWriteLock) {
            match self {
                Group::Read => lock.unlock_read(),
                Group::Write => lock.unlock_write(),
            }
        }

        fn other_group(self) -> Group {
            match self {
                Group::Read => Group::Write,
                Group::Write => Group::Read,
            }
        }
    }

    const GROUPS: [Group; 2] = [Group::Read, Group::Write];

    /// Gives a thread time to block on the lock.
    fn let_block() {
        std::thread::sleep(Duration::from_millis(50));
    }

    fn wait_until_writer_waiting(lock: &ReaderPreferringReadWriteLock) {
        while !lock.to_string().contains("writerWaiting=true") {
            std::thread::yield_now();
        }
    }

    fn panics_with_illegal_monitor_state(f: impl FnOnce()) -> bool {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(()) => false,
            Err(p) => p
                .downcast_ref::<String>()
                .is_some_and(|m| m.starts_with("IllegalMonitorStateException")),
        }
    }

    #[test]
    fn lock_read_multiple_readers_succeeds() {
        let lock = ReaderPreferringReadWriteLock::new();
        for _ in 0..10 {
            lock.lock_read_interruptibly().unwrap();
        }
        for _ in 0..10 {
            lock.unlock_read();
        }
        // The lock has been fully released: a writer can acquire it.
        lock.lock_write_interruptibly().unwrap();
        lock.unlock_write();
    }

    #[test]
    fn unlock_lock_not_held_throws() {
        for group in GROUPS {
            let lock = ReaderPreferringReadWriteLock::new();
            assert!(panics_with_illegal_monitor_state(|| group.unlock(&lock)));
        }
    }

    #[test]
    fn unlock_lock_held_by_other_group_throws() {
        for group in GROUPS {
            let lock = ReaderPreferringReadWriteLock::new();
            group.lock(&lock).unwrap();
            let l = lock.clone();
            assert!(panics_with_illegal_monitor_state(move || group
                .other_group()
                .unlock(&l)));
            group.unlock(&lock);
        }
    }

    #[test]
    fn lock_lock_held_by_other_group_blocks_until_released() {
        for group in GROUPS {
            let other_group = group.other_group();
            let lock = ReaderPreferringReadWriteLock::new();
            group.lock(&lock).unwrap();
            let acquisitions = Arc::new(AtomicUsize::new(0));
            let thread = {
                let lock = lock.clone();
                let acquisitions = acquisitions.clone();
                std::thread::spawn(move || {
                    other_group.lock(&lock).unwrap();
                    acquisitions.fetch_add(1, Ordering::SeqCst);
                    other_group.unlock(&lock);
                })
            };
            let_block();
            assert_eq!(acquisitions.load(Ordering::SeqCst), 0);
            group.unlock(&lock);
            thread.join().unwrap();
            assert_eq!(acquisitions.load(Ordering::SeqCst), 1);
            // group can acquire again after otherGroup has released the lock.
            group.lock(&lock).unwrap();
            group.unlock(&lock);
        }
    }

    #[test]
    fn lock_write_lock_held_by_writer_blocks_until_released() {
        let lock = ReaderPreferringReadWriteLock::new();
        lock.lock_write_interruptibly().unwrap();
        let acquisitions = Arc::new(AtomicUsize::new(0));
        let other_writer = {
            let lock = lock.clone();
            let acquisitions = acquisitions.clone();
            std::thread::spawn(move || {
                lock.lock_write_interruptibly().unwrap();
                acquisitions.fetch_add(1, Ordering::SeqCst);
                lock.unlock_write();
            })
        };
        let_block();
        assert_eq!(acquisitions.load(Ordering::SeqCst), 0);
        lock.unlock_write();
        other_writer.join().unwrap();
        assert_eq!(acquisitions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn lock_read_writer_waiting_is_admitted() {
        let lock = ReaderPreferringReadWriteLock::new();
        lock.lock_read_interruptibly().unwrap();
        let acquisitions = Arc::new(AtomicUsize::new(0));
        let writer = {
            let lock = lock.clone();
            let acquisitions = acquisitions.clone();
            std::thread::spawn(move || {
                lock.lock_write_interruptibly().unwrap();
                acquisitions.fetch_add(1, Ordering::SeqCst);
                lock.unlock_write();
            })
        };
        wait_until_writer_waiting(&lock);
        // A new reader is admitted while the writer waits.
        lock.lock_read_interruptibly().unwrap();
        assert_eq!(acquisitions.load(Ordering::SeqCst), 0);
        // The writer only acquires the lock once all readers have released it.
        lock.unlock_read();
        assert_eq!(acquisitions.load(Ordering::SeqCst), 0);
        lock.unlock_read();
        writer.join().unwrap();
        assert_eq!(acquisitions.load(Ordering::SeqCst), 1);
        // Both groups can acquire the lock without blocking after the writer has released it.
        lock.lock_read_interruptibly().unwrap();
        lock.unlock_read();
        lock.lock_write_interruptibly().unwrap();
        lock.unlock_write();
    }

    #[test]
    fn unlock_read_new_reader_before_notification_writer_is_not_stranded() {
        let lock = ReaderPreferringReadWriteLock::new();
        lock.lock_read_interruptibly().unwrap();
        let acquisitions = Arc::new(AtomicUsize::new(0));
        let writer = {
            let lock = lock.clone();
            let acquisitions = acquisitions.clone();
            std::thread::spawn(move || {
                lock.lock_write_interruptibly().unwrap();
                acquisitions.fetch_add(1, Ordering::SeqCst);
                lock.unlock_write();
            })
        };
        wait_until_writer_waiting(&lock);

        let releasing_reader;
        {
            let _monitor = lock.monitor();
            releasing_reader = {
                let lock = lock.clone();
                std::thread::spawn(move || lock.unlock_read())
            };
            // Pause the last reader after it releases the lock but before it notifies the writer.
            while lock.to_string()
                != "ReaderPreferringReadWriteLock[readers=0, writer=false, writerWaiting=false]"
            {
                std::thread::yield_now();
            }
            lock.lock_read_interruptibly().unwrap();
        }
        releasing_reader.join().unwrap();
        // The notified writer must record itself as waiting again while this new reader holds
        // the lock.
        wait_until_writer_waiting(&lock);
        assert_eq!(acquisitions.load(Ordering::SeqCst), 0);
        lock.unlock_read();
        writer.join().unwrap();
        assert_eq!(acquisitions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn lock_interrupted_while_blocked_throws_and_keeps_lock_usable() {
        for group in GROUPS {
            let other_group = group.other_group();
            let lock = ReaderPreferringReadWriteLock::new();
            group.lock(&lock).unwrap();

            let (tx, rx) = std::sync::mpsc::channel();
            let thread = {
                let lock = lock.clone();
                std::thread::spawn(move || {
                    tx.send(blaze_shell::ThreadInterrupt::current()).unwrap();
                    other_group.lock(&lock).is_err()
                })
            };
            let interrupt = rx.recv().unwrap();
            let_block();
            interrupt.interrupt();
            assert!(thread.join().unwrap(), "{group:?}: interrupted");

            // The interrupted acquisition must not have taken or corrupted the lock: group still
            // holds it and after releasing it, both groups can acquire it.
            group.unlock(&lock);
            other_group.lock(&lock).unwrap();
            other_group.unlock(&lock);
            group.lock(&lock).unwrap();
            group.unlock(&lock);
        }
    }

    #[test]
    fn lock_read_interrupted_writer_does_not_enter_monitor() {
        let lock = ReaderPreferringReadWriteLock::new();
        lock.lock_read_interruptibly().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let writer = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                tx.send(blaze_shell::ThreadInterrupt::current()).unwrap();
                lock.lock_write_interruptibly().is_err()
            })
        };
        let interrupt = rx.recv().unwrap();
        wait_until_writer_waiting(&lock);
        interrupt.interrupt();
        assert!(writer.join().unwrap());
        lock.unlock_read();

        // Cancelling the only writer must leave subsequent readers independent of the monitor.
        let _monitor = lock.monitor();
        let reader = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                for _ in 0..3 {
                    lock.lock_read_interruptibly().unwrap();
                    lock.unlock_read();
                }
            })
        };
        reader.join().unwrap();
    }

    #[test]
    fn lock_interrupted_before_acquisition_throws_and_keeps_lock_usable() {
        for group in GROUPS {
            let lock = ReaderPreferringReadWriteLock::new();
            blaze_shell::ThreadInterrupt::current().interrupt();
            assert!(group.lock(&lock).is_err());
            // Avoid leaking the interrupt into other tests if the assertion fails.
            blaze_shell::thread_interrupt::interrupted();

            // The interrupted acquisition must not have taken or corrupted the lock.
            group.other_group().lock(&lock).unwrap();
            group.other_group().unlock(&lock);
        }
    }

    #[test]
    fn lock_and_unlock_concurrent_threads_readers_and_writers_never_overlap() {
        let lock = ReaderPreferringReadWriteLock::new();
        let writers = Arc::new(AtomicI32::new(0));
        let readers = Arc::new(AtomicI32::new(0));
        let violations = Arc::new(AtomicI32::new(0));
        let threads: Vec<_> = (0..16u64)
            .map(|seed| {
                let (lock, writers, readers, violations) = (
                    lock.clone(),
                    writers.clone(),
                    readers.clone(),
                    violations.clone(),
                );
                std::thread::spawn(move || {
                    // A small deterministic generator per thread (Java: new Random(seed)).
                    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    for _ in 0..20_000 {
                        state = state
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407);
                        if (state >> 33) % 4 == 0 {
                            lock.lock_write_interruptibly().unwrap();
                            // The writer is exclusive: no other writer and no reader may be
                            // active.
                            if writers.fetch_add(1, Ordering::SeqCst) + 1 != 1
                                || readers.load(Ordering::SeqCst) != 0
                            {
                                violations.fetch_add(1, Ordering::SeqCst);
                            }
                            writers.fetch_sub(1, Ordering::SeqCst);
                            lock.unlock_write();
                        } else {
                            lock.lock_read_interruptibly().unwrap();
                            readers.fetch_add(1, Ordering::SeqCst);
                            if writers.load(Ordering::SeqCst) != 0 {
                                violations.fetch_add(1, Ordering::SeqCst);
                            }
                            readers.fetch_sub(1, Ordering::SeqCst);
                            lock.unlock_read();
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(violations.load(Ordering::SeqCst), 0);
        assert_eq!(readers.load(Ordering::SeqCst), 0);
        assert_eq!(writers.load(Ordering::SeqCst), 0);
    }
}
