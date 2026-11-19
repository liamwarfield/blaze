//! Port of `lib/runtime/CommandNameCacheImpl.java`.

use std::collections::{HashMap, HashSet, VecDeque};

use blaze_options::invocation_policy::CommandNameCache;
use parking_lot::Mutex;

use crate::command::CommandAnnotation;

/// Maps a command to the names of all commands it inherits from, including itself.
///
/// PORT: Java keeps a live view of the runtime's command map; the runtime sets this cache after
/// its last command is added, so a snapshot of the map is the same.
// Java: com/google/devtools/build/lib/runtime/CommandNameCacheImpl.java:27 CommandNameCacheImpl
pub(crate) struct CommandNameCacheImpl {
    command_map: HashMap<String, &'static CommandAnnotation>,
    cache: Mutex<HashMap<String, Vec<String>>>,
}

impl CommandNameCacheImpl {
    // Java: com/google/devtools/build/lib/runtime/CommandNameCacheImpl.java:31 CommandNameCacheImpl.<init>
    pub(crate) fn new(
        command_map: impl IntoIterator<Item = (String, &'static CommandAnnotation)>,
    ) -> CommandNameCacheImpl {
        CommandNameCacheImpl {
            command_map: command_map.into_iter().collect(),
            cache: Mutex::new(HashMap::new()),
        }
    }
}

impl CommandNameCache for CommandNameCacheImpl {
    // Java: com/google/devtools/build/lib/runtime/CommandNameCacheImpl.java:40 CommandNameCacheImpl.get
    fn get(&self, command_name: &str) -> Vec<String> {
        if let Some(cached_result) = self.cache.lock().get(command_name) {
            return cached_result.clone();
        }
        // ImmutableSet.Builder: insertion order, duplicates dropped.
        let mut builder: Vec<String> = Vec::new();

        let command = *self
            .command_map
            .get(command_name)
            .unwrap_or_else(|| panic!("NullPointerException: {command_name}"));
        let mut visited: HashSet<*const CommandAnnotation> = HashSet::new();
        visited.insert(command as *const _);
        let mut queue: VecDeque<&'static CommandAnnotation> = VecDeque::new();
        queue.push_back(command);
        while let Some(cur) = queue.pop_front() {
            if !builder.iter().any(|n| n == cur.name()) {
                builder.push(cur.name().to_string());
            }
            for parent in cur.inherits_options_from {
                if visited.insert(*parent as *const _) {
                    queue.push_back(parent);
                }
            }
        }
        self.cache
            .lock()
            .insert(command_name.to_string(), builder.clone());
        builder
    }
}
