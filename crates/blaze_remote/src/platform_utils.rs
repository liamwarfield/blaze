//! Port of `lib/analysis/platform/PlatformUtils.java`: utilities for accessing platform
//! properties.
//!
//! PORT: `blaze_exec::exec::spawn_log_context` holds a private copy of
//! `getPlatformProto(spawn, remoteOptions)` for the execution log (it cannot depend on this
//! crate); the two must stay in step.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use blaze_exec::actions::{ExecException, Spawn, UserExecException};
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail, spawn as spawn_code};
use blaze_util::string_encoding::{internal_to_unicode, unicode_to_internal};
use indexmap::IndexMap;

use crate::reapi::{Platform, platform};
use crate::text_format::{self, ParseException};

/// `String.compareTo`: lexicographic order of UTF-16 code units. For proto strings (which hold
/// exactly the chars of the Java string) this is Java's order.
fn java_string_compare(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.cmp(b);
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Sorts the properties by name (a stable sort, `Ordering.sortedCopy`).
// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:43 PlatformUtils.sortPlatformProperties
fn sort_platform_properties(builder: &mut Platform) {
    let mut properties: Vec<platform::Property> = std::mem::take(&mut builder.properties);
    properties.sort_by(|a, b| java_string_compare(&a.name, &b.name));
    builder.properties = properties;
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:52 PlatformUtils.buildPlatformProto
pub fn build_platform_proto(execution_properties: &IndexMap<String, String>) -> Option<Platform> {
    if execution_properties.is_empty() {
        return None;
    }
    let mut builder = Platform::default();
    for (key, value) in execution_properties {
        let property = platform::Property {
            name: internal_to_unicode(key).into_owned(),
            value: internal_to_unicode(value).into_owned(),
        };
        builder.properties.push(property);
    }

    sort_platform_properties(&mut builder);
    Some(builder)
}

/// `getPlatformProto(spawn, remoteOptions)`.
///
/// PORT: `throws UserExecException` is `ExecException::User`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:71 PlatformUtils.getPlatformProto
pub fn get_platform_proto(
    spawn: &dyn Spawn,
    remote_options: Option<&RemoteOptions>,
) -> Result<Option<Platform>, ExecException> {
    get_platform_proto_with_additional_properties(spawn, remote_options, &IndexMap::new())
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:76 PlatformUtils.shouldProducePlatformProto
fn should_produce_platform_proto(
    spawn: &dyn Spawn,
    default_exec_properties: &BTreeMap<String, String>,
    additional_properties: &IndexMap<String, String>,
) -> bool {
    let execution_platform = spawn.get_execution_platform();
    if let Some(execution_platform) = execution_platform {
        if !execution_platform.exec_properties.is_empty() {
            return true;
        }
        if !execution_platform.remote_execution_properties.is_empty() {
            return true;
        }
    }
    if !spawn.get_combined_exec_properties().is_empty() {
        return true;
    }
    if !default_exec_properties.is_empty() {
        return true;
    }
    if !additional_properties.is_empty() {
        return true;
    }
    false
}

/// `Platform.Builder platformBuilder = Platform.newBuilder();
/// TextFormat.getParser().merge(text, platformBuilder)`; the `Err` is the `ParseException`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:129 PlatformUtils.getPlatformProto
fn text_format_merge_platform(text: &str) -> Result<Platform, ParseException> {
    let mut platform_builder = Platform::default();
    text_format::merge_platform(text, &mut platform_builder)?;
    Ok(platform_builder)
}

/// A `java.util.HashMap<String, String>`, as far as its iteration order goes.
///
/// PORT: java-hash-order. `getPlatformProto` iterates its `properties` map before a stable sort
/// by the Unicode name: two internal names that are both invalid UTF-8 become the same name
/// (U+FFFD) after `internalToUnicode` and keep the map's order. The order of a `HashMap` is
/// deterministic: `String.hashCode()` (`31 * h + c` over the UTF-16 units) spread by
/// `h ^ (h >>> 16)`, a power-of-two table that doubles when the size exceeds 3/4 of the
/// capacity (or pre-sizes for `putAll` / `new HashMap<>(map)`), and per-bin insertion order
/// (which `resize` preserves). A bin that Java would treeify (more than 8 entries in a table of
/// 64 or more slots) is `unported!`.
// Java: java/util/HashMap.java HashMap (JDK 25)
struct JavaHashMap {
    /// `table`; empty until allocated.
    table: Vec<Vec<(u32, String, String)>>,
    size: usize,
    /// `threshold` (before allocation: the initial capacity, or 0 for the default).
    threshold: usize,
}

impl JavaHashMap {
    /// `HashMap.DEFAULT_INITIAL_CAPACITY`.
    const DEFAULT_INITIAL_CAPACITY: usize = 16;
    /// `HashMap.TREEIFY_THRESHOLD`.
    const TREEIFY_THRESHOLD: usize = 8;
    /// `HashMap.MIN_TREEIFY_CAPACITY`.
    const MIN_TREEIFY_CAPACITY: usize = 64;

    // Java: java/util/HashMap.java HashMap.<init>()
    fn new() -> JavaHashMap {
        JavaHashMap {
            table: Vec::new(),
            size: 0,
            threshold: 0,
        }
    }

    // Java: java/util/HashMap.java HashMap.<init>(Map)
    fn from_entries(entries: &[(String, String)]) -> JavaHashMap {
        let mut map = JavaHashMap::new();
        map.put_map_entries(entries);
        map
    }

    // Java: java/util/HashMap.java HashMap.hash
    fn hash(key: &str) -> u32 {
        // Java: java/lang/String.java String.hashCode
        let mut h: i32 = 0;
        for unit in key.encode_utf16() {
            h = h.wrapping_mul(31).wrapping_add(i32::from(unit));
        }
        let h = h as u32;
        h ^ (h >> 16)
    }

    // Java: java/util/HashMap.java HashMap.tableSizeFor
    fn table_size_for(cap: usize) -> usize {
        cap.max(1).next_power_of_two()
    }

    // Java: java/util/HashMap.java HashMap.resize
    fn resize(&mut self) {
        let old_cap = self.table.len();
        let old_thr = self.threshold;
        let (new_cap, mut new_thr) = if old_cap > 0 {
            let new_cap = old_cap << 1;
            let new_thr = if old_cap >= Self::DEFAULT_INITIAL_CAPACITY {
                old_thr << 1
            } else {
                0
            };
            (new_cap, new_thr)
        } else if old_thr > 0 {
            // initial capacity was placed in threshold
            (old_thr, 0)
        } else {
            // zero initial threshold signifies using defaults
            (
                Self::DEFAULT_INITIAL_CAPACITY,
                Self::DEFAULT_INITIAL_CAPACITY * 3 / 4,
            )
        };
        if new_thr == 0 {
            // `(int) (newCap * 0.75f)`
            new_thr = new_cap * 3 / 4;
        }
        self.threshold = new_thr;
        let mut new_table: Vec<Vec<(u32, String, String)>> = Vec::with_capacity(new_cap);
        new_table.resize_with(new_cap, Vec::new);
        // Each old bin is split into two new bins; the nodes keep their relative order.
        for bin in self.table.drain(..) {
            for node in bin {
                let index = (node.0 as usize) & (new_cap - 1);
                new_table[index].push(node);
            }
        }
        self.table = new_table;
    }

    /// `put(key, value)`.
    // Java: java/util/HashMap.java HashMap.putVal
    fn put(&mut self, key: String, value: String) {
        if self.table.is_empty() {
            self.resize();
        }
        let hash = JavaHashMap::hash(&key);
        let index = (hash as usize) & (self.table.len() - 1);
        if let Some(node) = self.table[index]
            .iter_mut()
            .find(|node| node.0 == hash && node.1 == key)
        {
            // An existing mapping keeps its place.
            node.2 = value;
            return;
        }
        let bin_count = self.table[index].len();
        self.table[index].push((hash, key, value));
        // `if (binCount >= TREEIFY_THRESHOLD - 1) treeifyBin(tab, hash)`, where Java's `binCount`
        // is one less than the number of nodes that were in the bin.
        if bin_count >= Self::TREEIFY_THRESHOLD {
            if self.table.len() < Self::MIN_TREEIFY_CAPACITY {
                // `treeifyBin` resizes a small table instead of treeifying the bin.
                self.resize();
            } else {
                blaze_util::unported!("java.util.HashMap.treeifyBin (platform properties)");
            }
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize();
        }
    }

    /// `putAll(map)`, `map` given in its own iteration order.
    // Java: java/util/HashMap.java HashMap.putMapEntries
    fn put_map_entries(&mut self, entries: &[(String, String)]) {
        let s = entries.len();
        if s > 0 {
            if self.table.is_empty() {
                // pre-size: `Math.ceil(s / (double) loadFactor)`
                let t = (s as f64 / 0.75).ceil() as usize;
                if t > self.threshold {
                    self.threshold = JavaHashMap::table_size_for(t);
                }
            } else {
                while s > self.threshold {
                    self.resize();
                }
            }
            for (key, value) in entries {
                self.put(key.clone(), value.clone());
            }
        }
    }

    /// `entrySet()`, in Java's iteration order.
    // Java: java/util/HashMap.java HashMap.HashIterator
    fn into_entries(self) -> Vec<(String, String)> {
        self.table
            .into_iter()
            .flatten()
            .map(|(_, key, value)| (key, value))
            .collect()
    }
}

fn entries_of<'a>(
    map: impl IntoIterator<Item = (&'a String, &'a String)>,
) -> Vec<(String, String)> {
    map.into_iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// `getPlatformProto(spawn, remoteOptions, additionalProperties)`.
///
/// `properties` holds the entries of Java's `properties` map in that map's iteration order: a
/// `HashMap` (see [`JavaHashMap`]), the spawn's `ImmutableMap`, the `SortedMap` of default exec
/// properties, or the caller's additional properties.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:102 PlatformUtils.getPlatformProto
pub fn get_platform_proto_with_additional_properties(
    spawn: &dyn Spawn,
    remote_options: Option<&RemoteOptions>,
    additional_properties: &IndexMap<String, String>,
) -> Result<Option<Platform>, ExecException> {
    let default_exec_properties: BTreeMap<String, String> = match remote_options {
        Some(remote_options) => remote_options.get_remote_default_exec_properties(),
        None => BTreeMap::new(),
    };

    if !should_produce_platform_proto(spawn, &default_exec_properties, additional_properties) {
        // Execution platform is null or functionally empty
        return Ok(None);
    }

    let mut properties: Vec<(String, String)> = Vec::new();
    let combined_exec_properties = spawn.get_combined_exec_properties();
    if !combined_exec_properties.is_empty() {
        // Apply default exec properties if the execution platform does not already set
        // exec_properties
        if spawn
            .get_execution_platform()
            .is_none_or(|p| p.exec_properties.is_empty())
        {
            let mut map = JavaHashMap::new();
            map.put_map_entries(&entries_of(&default_exec_properties));
            map.put_map_entries(&entries_of(&combined_exec_properties));
            properties = map.into_entries();
        } else {
            properties = entries_of(&combined_exec_properties);
        }
    } else if let Some(execution_platform) = spawn.get_execution_platform() {
        let remote_execution_properties = &execution_platform.remote_execution_properties;
        if !remote_execution_properties.is_empty() {
            // Try and get the platform info from the execution properties.
            let platform_builder = match text_format_merge_platform(&internal_to_unicode(
                &execution_platform.remote_execution_properties,
            )) {
                Ok(platform_builder) => platform_builder,
                Err(e) => {
                    let message = format!(
                        "Failed to parse remote_execution_properties from platform {}",
                        execution_platform.label
                    );
                    return Err(UserExecException::with_cause(
                        &e,
                        create_failure_detail(
                            message,
                            spawn_code::Code::InvalidRemoteExecutionProperties,
                        ),
                    )
                    .into());
                }
            };

            let mut map = JavaHashMap::new();
            for property in &platform_builder.properties {
                map.put(
                    unicode_to_internal(&property.name).into_owned(),
                    unicode_to_internal(&property.value).into_owned(),
                );
            }
            properties = map.into_entries();
        } else {
            let mut map = JavaHashMap::new();
            map.put_map_entries(&entries_of(&execution_platform.exec_properties));
            properties = map.into_entries();
        }
    }

    if properties.is_empty() {
        properties = entries_of(&default_exec_properties);
    }

    if !additional_properties.is_empty() {
        if properties.is_empty() {
            properties = entries_of(additional_properties);
        } else {
            // Merge the two maps.
            let mut map = JavaHashMap::from_entries(&properties);
            map.put_map_entries(&entries_of(additional_properties));
            properties = map.into_entries();
        }
    }

    let mut platform_builder = Platform::default();
    for (key, value) in &properties {
        platform_builder.properties.push(platform::Property {
            name: internal_to_unicode(key).into_owned(),
            value: internal_to_unicode(value).into_owned(),
        });
    }
    sort_platform_properties(&mut platform_builder);
    Ok(Some(platform_builder))
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:179 PlatformUtils.createFailureDetail
fn create_failure_detail(message: String, detailed_code: spawn_code::Code) -> FailureDetail {
    FailureDetail {
        message,
        category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
            code: detailed_code as i32,
            ..Default::default()
        })),
    }
}
