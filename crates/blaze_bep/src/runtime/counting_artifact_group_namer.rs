//! Port of `CountingArtifactGroupNamer`.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_protos::build_event_stream::build_event_id::NamedSetOfFilesId;
use parking_lot::{Condvar, Mutex};

use crate::buildeventstream::build_event_context::{ArtifactGroupNamer, ArtifactGroupNode};

/// Conversion of paths to URIs: names nested sets by a running counter.
// Java: com/google/devtools/build/lib/runtime/CountingArtifactGroupNamer.java:28 CountingArtifactGroupNamer
#[derive(Default)]
pub struct CountingArtifactGroupNamer {
    node_names: Mutex<HashMap<ArtifactGroupNode, Arc<Latch>>>,
}

#[derive(Debug)]
struct Latch {
    name: i32,
    written: Mutex<bool>,
    cond: Condvar,
}

impl CountingArtifactGroupNamer {
    pub fn new() -> Self {
        Self::default()
    }

    /// If the set is not yet named, name it and return the name, which the caller closes once
    /// the named set is written; otherwise wait until the existing name is written and return
    /// `None`.
    // Java: com/google/devtools/build/lib/runtime/CountingArtifactGroupNamer.java:47 CountingArtifactGroupNamer.maybeName
    pub fn maybe_name(&self, id: ArtifactGroupNode) -> Option<LatchedGroupName> {
        let existing_group_name;
        let new_group_name;
        {
            let mut names = self.node_names.lock();
            let latch = Arc::new(Latch {
                name: names.len() as i32,
                written: Mutex::new(false),
                cond: Condvar::new(),
            });
            match names.get(&id) {
                Some(existing) => {
                    existing_group_name = Some(existing.clone());
                    new_group_name = None;
                }
                None => {
                    names.insert(id, latch.clone());
                    existing_group_name = None;
                    new_group_name = Some(latch);
                }
            }
        }
        if let Some(existing) = existing_group_name {
            // waitUntilWritten
            let mut written = existing.written.lock();
            while !*written {
                existing.cond.wait(&mut written);
            }
            return None;
        }
        new_group_name.map(|latch| LatchedGroupName { latch })
    }
}

impl ArtifactGroupNamer for CountingArtifactGroupNamer {
    // Java: com/google/devtools/build/lib/runtime/CountingArtifactGroupNamer.java:35 CountingArtifactGroupNamer.apply
    fn apply(&self, id: &ArtifactGroupNode) -> Option<NamedSetOfFilesId> {
        let names = self.node_names.lock();
        let name = names.get(id)?;
        Some(NamedSetOfFilesId {
            id: name.name.to_string(),
        })
    }
}

/// A name for a nested set, with a latch released when the named set has been written.
// Java: com/google/devtools/build/lib/runtime/CountingArtifactGroupNamer.java:69 CountingArtifactGroupNamer.LatchedGroupName
pub struct LatchedGroupName {
    latch: Arc<Latch>,
}

impl LatchedGroupName {
    // Java: com/google/devtools/build/lib/runtime/CountingArtifactGroupNamer.java:83 LatchedGroupName.getName
    pub fn get_name(&self) -> String {
        self.latch.name.to_string()
    }
}

impl Drop for LatchedGroupName {
    // Java: com/google/devtools/build/lib/runtime/CountingArtifactGroupNamer.java:79 LatchedGroupName.close
    fn drop(&mut self) {
        *self.latch.written.lock() = true;
        self.latch.cond.notify_all();
    }
}
