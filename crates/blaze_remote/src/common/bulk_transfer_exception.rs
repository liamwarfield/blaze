//! Port of `lib/remote/common/BulkTransferException.java`.

use std::fmt;
use std::io;
use std::sync::Arc;

use blaze_exec::actions::ActionInputRef;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::cache_not_found_exception::CacheNotFoundException;
use super::errors::{SharedIoError, downcast_ref};
use crate::util::digest_util::DigestUtil;

/// Exception which represents a collection of IOExceptions for the purpose of distinguishing
/// remote communication exceptions from those which occur on filesystems locally. This exception
/// serves as a trace point for the actual transfer, so that the intended operation can be
/// observed in a stack, with all constituent exceptions available for observation.
// Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:36 BulkTransferException
#[derive(Debug, Clone)]
pub struct BulkTransferException {
    /// `getSuppressed()`.
    suppressed: Vec<Arc<io::Error>>,
    // No empty BulkTransferException is ever thrown.
    all_cache_not_found_exception: bool,
}

impl Default for BulkTransferException {
    fn default() -> Self {
        BulkTransferException::new()
    }
}

/// `ImportantOutputHandler.LostArtifacts`: the lost inputs keyed by digest
/// (`ImmutableSetMultimap<String, ActionInput>`), with `EMPTY` and `throwIfNotEmpty()`, which
/// throws the `LostInputsExecException` that the action executor turns into the action's lost
/// inputs (`ActionRewindStrategy.checkRewindingEnabled` without rewinding).
///
/// PORT: the type lives in `blaze_exec` (`lib/actions`), where Java has it; this module
/// re-exports it under the name its users in `lib/remote` import.
// Java: com/google/devtools/build/lib/actions/ImportantOutputHandler.java:141 ImportantOutputHandler.LostArtifacts
pub use blaze_exec::actions::LostArtifacts;

impl BulkTransferException {
    /// `new BulkTransferException()`.
    // Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:40 BulkTransferException.<init>
    pub fn new() -> BulkTransferException {
        BulkTransferException {
            suppressed: Vec::new(),
            all_cache_not_found_exception: true,
        }
    }

    /// `new BulkTransferException(e)`.
    // Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:42 BulkTransferException.<init>
    pub fn from_io(e: io::Error) -> BulkTransferException {
        let mut b = BulkTransferException::new();
        b.add(e);
        b
    }

    /// Adds an IOException to the suppressed list.
    ///
    /// If the IOException is already a BulkTransferException, the contained IOExceptions are
    /// added instead.
    // Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:55 BulkTransferException.add
    pub fn add(&mut self, e: io::Error) {
        if let Some(bulk) = downcast_ref::<BulkTransferException>(&e) {
            for t in bulk.suppressed.clone() {
                self.add_shared(t);
            }
            return;
        }
        self.add_shared(Arc::new(e));
    }

    fn add_shared(&mut self, e: Arc<io::Error>) {
        self.all_cache_not_found_exception &= downcast_ref::<CacheNotFoundException>(&e).is_some();
        self.suppressed.push(e);
    }

    /// `getSuppressed()`.
    pub fn get_suppressed(&self) -> &[Arc<io::Error>] {
        &self.suppressed
    }

    /// `getSuppressed().length == 0`.
    pub fn is_empty(&self) -> bool {
        self.suppressed.is_empty()
    }

    /// `allCausedByCacheNotFoundException()`.
    // Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:70 BulkTransferException.allCausedByCacheNotFoundException
    pub fn all_caused_by_cache_not_found_exception(&self) -> bool {
        self.all_cache_not_found_exception
    }

    /// `BulkTransferException.allCausedByCacheNotFoundException(Throwable)`.
    // Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:74 BulkTransferException.allCausedByCacheNotFoundException
    pub fn all_caused_by_cache_not_found_exception_of(e: &io::Error) -> bool {
        downcast_ref::<BulkTransferException>(e)
            .is_some_and(|b| b.all_caused_by_cache_not_found_exception())
    }

    /// Returns a [`LostArtifacts`] instance that is non-empty if and only if all suppressed
    /// exceptions are caused by cache misses.
    // Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:83 BulkTransferException.getLostArtifacts
    pub fn get_lost_artifacts(
        &self,
        action_input_resolver: &dyn Fn(&PathFragment) -> Option<ActionInputRef>,
    ) -> LostArtifacts {
        if !self.all_caused_by_cache_not_found_exception() {
            return LostArtifacts::empty();
        }

        let mut by_digest: IndexMap<String, IndexSet<ActionInputRef>> = IndexMap::new();
        for suppressed in &self.suppressed {
            let e = downcast_ref::<CacheNotFoundException>(suppressed)
                .expect("allCausedByCacheNotFoundException");
            let missing_digest = e.get_missing_digest();
            let Some(exec_path) = e.get_exec_path() else {
                // This can happen if the lost artifact is not an input of the action, but a
                // special output such as stdout/stderr. This can't be solved by the rewinding
                // that LostArtifacts would trigger, but is rather a failure of the current action
                // execution.
                if e.get_filename().is_none() {
                    panic!(
                        "IllegalArgumentException: CacheNotFoundException that may represent a lost artifact should have been annotated with a filename: {e}"
                    );
                }
                return LostArtifacts::empty();
            };
            let Some(action_input) = action_input_resolver(exec_path) else {
                // This can happen if the lost artifact is not known to the resolver, e.g. an
                // output that failed to be retrieved from the remote cache after a cache hit
                // while resolving inputs. Rewinding can't recover an artifact it can't identify.
                return LostArtifacts::empty();
            };
            by_digest
                .entry(DigestUtil::to_string(missing_digest))
                .or_default()
                .insert(action_input);
        }
        LostArtifacts { by_digest }
    }

    /// `getMessage()`.
    // Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:122 BulkTransferException.getMessage
    pub fn get_message(&self) -> String {
        // Only report unique messages to avoid flooding the user, e.g. in case a remote cache
        // server is unavailable and causing several identical messages. Also sort the messages,
        // for more deterministic result. All of this allows more efficient event deduplication
        // when reporting the returned aggregated message.
        let mut unique_sorted_messages: Vec<String> =
            self.suppressed.iter().map(|e| e.to_string()).collect();
        // `sorted()`: `String.compareTo`, the order of UTF-16 code units.
        unique_sorted_messages.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        unique_sorted_messages.dedup();
        match unique_sorted_messages.len() {
            0 => "Unknown error during bulk transfer".to_string(),
            1 => unique_sorted_messages.pop().unwrap(),
            _ => format!(
                "Multiple errors during bulk transfer:\n{}",
                unique_sorted_messages.join("\n")
            ),
        }
    }
}

impl fmt::Display for BulkTransferException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for BulkTransferException {}

impl From<BulkTransferException> for io::Error {
    fn from(e: BulkTransferException) -> io::Error {
        io::Error::other(e)
    }
}

/// Rebuilds an owned `io::Error` from a suppressed one.
pub fn suppressed_to_io(e: &Arc<io::Error>) -> io::Error {
    io::Error::new(e.kind(), SharedIoError(e.clone()))
}
