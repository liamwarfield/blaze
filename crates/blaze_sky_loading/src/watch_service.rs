//! The `java.nio.file.WatchService` that `WatchServiceDiffAwareness` uses, on Linux: the JDK's
//! `sun.nio.fs.LinuxWatchService` with `sun.nio.fs.AbstractWatchKey` (inotify).
//!
//! PORT: the JDK is not part of the pinned Bazel source; this follows its behaviour as Bazel
//! relies on it:
//!
//! - `register(dir, ENTRY_CREATE, ENTRY_MODIFY, ENTRY_DELETE)` adds an inotify watch with
//!   `IN_CREATE | IN_MOVED_TO | IN_DELETE | IN_MOVED_FROM | IN_MODIFY | IN_ATTRIB`; registering
//!   a directory that is already watched (same watch descriptor) returns its key.
//! - A background thread reads the inotify descriptor and signals the keys: `IN_Q_OVERFLOW` is an
//!   `OVERFLOW` event on every key, `IN_IGNORED` invalidates the key, an event without a name is
//!   dropped, and the mask maps to `ENTRY_MODIFY` (`IN_MODIFY`, `IN_ATTRIB`), `ENTRY_CREATE`
//!   (`IN_CREATE`, `IN_MOVED_TO`) or `ENTRY_DELETE` (`IN_DELETE`, `IN_MOVED_FROM`).
//! - A key keeps its events in order; a repeated event (same kind and name as the last one) and
//!   a modification of a name whose last event was a modification only increment a count; once
//!   a key holds `jdk.nio.file.WatchService.maxEventsPerPoll` events (10000, which Bazel's client
//!   sets: `src/main/cpp/blaze.cc:444`) its pending events are replaced by one `OVERFLOW`.
//! - `poll()` returns the next signalled key without blocking; a signalled key is not queued
//!   again until `reset()`, which queues it again if events arrived meanwhile.

use std::collections::{HashMap, VecDeque};
use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use parking_lot::Mutex;

/// `jdk.nio.file.WatchService.maxEventsPerPoll` as Bazel's client sets it.
// Java: main/cpp/blaze.cc:444 GetServerExeArgs
pub const MAX_EVENTS_PER_POLL: usize = 10000;

/// `StandardWatchEventKinds`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEventKind {
    Create,
    Delete,
    Modify,
    Overflow,
}

/// A `WatchEvent`: its kind, its context (the name relative to the watched directory; `None`
/// for `OVERFLOW`) and its count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchEvent {
    pub kind: WatchEventKind,
    pub context: Option<OsString>,
    pub count: u32,
}

/// A `WatchKey`: one registered directory.
pub type WatchKey = u64;

#[derive(PartialEq, Eq)]
enum KeyStateKind {
    Ready,
    Signalled,
}

struct KeyState {
    wd: i32,
    valid: bool,
    state: KeyStateKind,
    events: Vec<WatchEvent>,
    /// Index in `events` of the last modification of a name, while it is the name's last event.
    last_modify_events: HashMap<OsString, usize>,
}

impl KeyState {
    /// `AbstractWatchKey.signalEvent`. Returns whether the key must be queued.
    fn signal_event(&mut self, mut kind: WatchEventKind, mut context: Option<OsString>) -> bool {
        let mut is_modify = kind == WatchEventKind::Modify;
        let size = self.events.len();
        if size > 0 {
            // If the previous event is an OVERFLOW event or this is a repeated event then we
            // simply increment the counter.
            let prev = &mut self.events[size - 1];
            if prev.kind == WatchEventKind::Overflow
                || (kind == prev.kind && context == prev.context)
            {
                prev.count += 1;
                return false;
            }
            // If this is a modify event and the last entry for the context is a modify event
            // then we simply increment the count.
            if !self.last_modify_events.is_empty() {
                let ctx = context.clone().unwrap_or_default();
                if is_modify {
                    if let Some(&i) = self.last_modify_events.get(&ctx) {
                        self.events[i].count += 1;
                        return false;
                    }
                } else {
                    // Not a modify event so remove from the map as the last event will no
                    // longer be a modify event.
                    self.last_modify_events.remove(&ctx);
                }
            }
            // If the list has reached the limit then drop pending events and queue an OVERFLOW
            // event.
            if size >= MAX_EVENTS_PER_POLL {
                kind = WatchEventKind::Overflow;
                is_modify = false;
                context = None;
            }
        }
        // Non-repeated event.
        if is_modify {
            self.last_modify_events
                .insert(context.clone().unwrap_or_default(), self.events.len());
        } else if kind == WatchEventKind::Overflow {
            // Drop all pending events.
            self.events.clear();
            self.last_modify_events.clear();
        }
        self.events.push(WatchEvent {
            kind,
            context,
            count: 1,
        });
        self.signal()
    }

    /// `AbstractWatchKey.signal`: queue the key if it is ready.
    fn signal(&mut self) -> bool {
        if self.state == KeyStateKind::Ready {
            self.state = KeyStateKind::Signalled;
            true
        } else {
            false
        }
    }
}

#[derive(Default)]
struct State {
    keys: HashMap<WatchKey, KeyState>,
    wd_to_key: HashMap<i32, WatchKey>,
    ready: VecDeque<WatchKey>,
    next_key: WatchKey,
}

struct Shared {
    inotify_fd: RawFd,
    wake_fds: [RawFd; 2],
    closed: AtomicBool,
    state: Mutex<State>,
}

/// A watch service backed by inotify.
pub struct WatchService {
    shared: Arc<Shared>,
    poller: Option<JoinHandle<()>>,
}

fn last_os_error() -> io::Error {
    io::Error::last_os_error()
}

impl WatchService {
    /// `FileSystem.newWatchService()`.
    pub fn new() -> io::Result<WatchService> {
        // SAFETY: plain system calls; the descriptors are owned by `Shared` and closed in
        // `close`.
        let inotify_fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        if inotify_fd < 0 {
            return Err(last_os_error());
        }
        let mut wake_fds = [0; 2];
        // SAFETY: as above.
        if unsafe { libc::pipe2(wake_fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } < 0 {
            let e = last_os_error();
            // SAFETY: closing the descriptor created above.
            unsafe { libc::close(inotify_fd) };
            return Err(e);
        }
        let shared = Arc::new(Shared {
            inotify_fd,
            wake_fds,
            closed: AtomicBool::new(false),
            state: Mutex::new(State::default()),
        });
        let poller_shared = shared.clone();
        let poller = std::thread::Builder::new()
            .name("FileSystemWatchService".to_string())
            .spawn(move || poll_loop(&poller_shared))?;
        Ok(WatchService {
            shared,
            poller: Some(poller),
        })
    }

    /// `dir.register(watchService, ENTRY_CREATE, ENTRY_MODIFY, ENTRY_DELETE)`.
    pub fn register(&self, dir: &std::path::Path) -> io::Result<WatchKey> {
        let metadata = std::fs::symlink_metadata(dir)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("{}: Not a directory", dir.display()),
            ));
        }
        let c_path = CString::new(dir.as_os_str().as_bytes())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mask = libc::IN_CREATE
            | libc::IN_MOVED_TO
            | libc::IN_DELETE
            | libc::IN_MOVED_FROM
            | libc::IN_MODIFY
            | libc::IN_ATTRIB;
        let mut state = self.shared.state.lock();
        if self.shared.closed.load(Ordering::SeqCst) {
            return Err(io::Error::other("ClosedWatchServiceException"));
        }
        // SAFETY: a valid descriptor and a NUL-terminated path.
        let wd = unsafe { libc::inotify_add_watch(self.shared.inotify_fd, c_path.as_ptr(), mask) };
        if wd < 0 {
            return Err(last_os_error());
        }
        if let Some(&key) = state.wd_to_key.get(&wd) {
            return Ok(key);
        }
        let key = state.next_key;
        state.next_key += 1;
        state.keys.insert(
            key,
            KeyState {
                wd,
                valid: true,
                state: KeyStateKind::Ready,
                events: Vec::new(),
                last_modify_events: HashMap::new(),
            },
        );
        state.wd_to_key.insert(wd, key);
        Ok(key)
    }

    /// `WatchService.poll()`: the next signalled key, or `None`.
    pub fn poll(&self) -> io::Result<Option<WatchKey>> {
        if self.shared.closed.load(Ordering::SeqCst) {
            return Err(io::Error::other("ClosedWatchServiceException"));
        }
        Ok(self.shared.state.lock().ready.pop_front())
    }

    /// `WatchKey.pollEvents()`.
    pub fn poll_events(&self, key: WatchKey) -> Vec<WatchEvent> {
        let mut state = self.shared.state.lock();
        match state.keys.get_mut(&key) {
            Some(k) => {
                k.last_modify_events.clear();
                std::mem::take(&mut k.events)
            }
            None => Vec::new(),
        }
    }

    /// `WatchKey.reset()`: whether the key is still valid.
    pub fn reset(&self, key: WatchKey) -> bool {
        let mut state = self.shared.state.lock();
        let Some(k) = state.keys.get_mut(&key) else {
            return false;
        };
        if !k.valid {
            return false;
        }
        if k.state == KeyStateKind::Signalled {
            if k.events.is_empty() {
                k.state = KeyStateKind::Ready;
            } else {
                // Pending events: the key stays signalled and is queued again.
                state.ready.push_back(key);
            }
        }
        true
    }

    /// `WatchKey.cancel()`.
    pub fn cancel(&self, key: WatchKey) {
        let mut state = self.shared.state.lock();
        if let Some(k) = state.keys.get_mut(&key)
            && k.valid
        {
            k.valid = false;
            let wd = k.wd;
            state.wd_to_key.remove(&wd);
            // SAFETY: removing a watch of our descriptor.
            unsafe { libc::inotify_rm_watch(self.shared.inotify_fd, wd) };
        }
    }

    /// `WatchService.close()`.
    pub fn close(&mut self) {
        if self.shared.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // SAFETY: wakes the poller thread, which then exits.
        unsafe { libc::write(self.shared.wake_fds[1], [1u8].as_ptr().cast(), 1) };
        if let Some(poller) = self.poller.take() {
            let _ = poller.join();
        }
        let mut state = self.shared.state.lock();
        for k in state.keys.values_mut() {
            k.valid = false;
        }
        state.ready.clear();
        // SAFETY: the poller has exited; nothing uses the descriptors any more.
        unsafe {
            libc::close(self.shared.inotify_fd);
            libc::close(self.shared.wake_fds[0]);
            libc::close(self.shared.wake_fds[1]);
        }
    }
}

impl Drop for WatchService {
    fn drop(&mut self) {
        self.close();
    }
}

/// The JDK's poller thread: reads inotify events and signals the keys.
fn poll_loop(shared: &Shared) {
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let mut fds = [
            libc::pollfd {
                fd: shared.inotify_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: shared.wake_fds[0],
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: two valid pollfd entries.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if n < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        if shared.closed.load(Ordering::SeqCst) || fds[1].revents != 0 {
            return;
        }
        if fds[0].revents & libc::POLLIN == 0 {
            continue;
        }
        loop {
            // SAFETY: reading into our buffer.
            let read =
                unsafe { libc::read(shared.inotify_fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if read <= 0 {
                break;
            }
            process_events(shared, &buffer[..read as usize]);
        }
    }
}

/// `LinuxWatchService.Poller.processEvent` for each event of a read.
fn process_events(shared: &Shared, bytes: &[u8]) {
    let header = std::mem::size_of::<libc::inotify_event>();
    let mut state = shared.state.lock();
    let mut offset = 0;
    while offset + header <= bytes.len() {
        // SAFETY: the kernel writes whole events; the header is read unaligned.
        let event: libc::inotify_event =
            unsafe { std::ptr::read_unaligned(bytes[offset..].as_ptr().cast()) };
        let name_start = offset + header;
        let name_end = name_start + event.len as usize;
        let name = if event.len > 0 {
            let raw = &bytes[name_start..name_end.min(bytes.len())];
            let len = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
            Some(OsString::from_vec(raw[..len].to_vec()))
        } else {
            None
        };
        offset = name_end;

        let mask = event.mask;
        if mask & libc::IN_Q_OVERFLOW != 0 {
            let keys: Vec<WatchKey> = state.keys.keys().copied().collect();
            for key in keys {
                let queue = state
                    .keys
                    .get_mut(&key)
                    .filter(|k| k.valid)
                    .is_some_and(|k| k.signal_event(WatchEventKind::Overflow, None));
                if queue {
                    state.ready.push_back(key);
                }
            }
            continue;
        }
        let Some(&key) = state.wd_to_key.get(&event.wd) else {
            continue;
        };
        if mask & libc::IN_IGNORED != 0 {
            state.wd_to_key.remove(&event.wd);
            let queue = state.keys.get_mut(&key).is_some_and(|k| {
                k.valid = false;
                k.signal()
            });
            if queue {
                state.ready.push_back(key);
            }
            continue;
        }
        let Some(name) = name else {
            continue;
        };
        let kind = if mask & (libc::IN_MODIFY | libc::IN_ATTRIB) != 0 {
            WatchEventKind::Modify
        } else if mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0 {
            WatchEventKind::Create
        } else if mask & (libc::IN_DELETE | libc::IN_MOVED_FROM) != 0 {
            WatchEventKind::Delete
        } else {
            continue;
        };
        let queue = state
            .keys
            .get_mut(&key)
            .is_some_and(|k| k.signal_event(kind, Some(name)));
        if queue {
            state.ready.push_back(key);
        }
    }
}

/// The name of an event as an `OsStr` (for joining with the key's directory).
pub fn context_as_os_str(event: &WatchEvent) -> Option<&OsStr> {
    event.context.as_deref()
}
