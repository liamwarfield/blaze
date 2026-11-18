//! Port of `lib/exec/StableSort.java`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io;

use blaze_protos::tools::protos::{Digest, File, SpawnExec};
use indexmap::IndexSet;

use super::io::{MessageInputStream, MessageOutputStream};
use super::spawn_log_context::java_string_compare;

/// Reads `SpawnExec` protos from a [`MessageInputStream`], sorts them, and writes them to a
/// [`MessageOutputStream`].
///
/// The sort order has the following properties:
///
/// 1. If an output of spawn A is an input to spawn B, A sorts before B.
/// 2. When not constrained by the above, spawns sort in lexicographic order of their primary
///    output path.
///
/// Assumes that there are no cyclic dependencies.
// Java: com/google/devtools/build/lib/exec/StableSort.java:55 StableSort.stableSort
pub fn stable_sort(
    input: &mut dyn MessageInputStream<SpawnExec>,
    out: &mut dyn MessageOutputStream<SpawnExec>,
) -> io::Result<()> {
    let mut inputs: Vec<SpawnExec> = Vec::new();

    while let Some(ex) = input.read()? {
        inputs.push(ex);
    }

    // A map from each output to every spawn that produces it.
    // The same output may be produced by multiple spawns in the case of multiple test attempts.
    let mut output_producer: HashMap<&str, Vec<usize>> = HashMap::with_capacity(inputs.len());

    for (i, ex) in inputs.iter().enumerate() {
        for output in &ex.actual_outputs {
            output_producer
                .entry(output.path.as_str())
                .or_default()
                .push(i);
        }
    }

    // A blocks B if A produces an output consumed by B.
    // Use reference equality to avoid expensive comparisons.
    let mut blocked_by = IdentitySetMultimap::default();
    let mut blocking = IdentitySetMultimap::default();

    // The queue contains all spawns whose blockers have already been emitted.
    // Sort by comparing the path of the first output. We don't want the sorting to rely on file
    // hashes because we want the same action graph to be sorted in the same way regardless of
    // file contents.
    let keys: Vec<String> = inputs.iter().map(sort_key).collect();
    let mut queue =
        PriorityQueue::new(|a: &usize, b: &usize| java_string_compare(&keys[*a], &keys[*b]));

    for (i, ex) in inputs.iter().enumerate() {
        let mut blocked = false;
        for s in &ex.inputs {
            if let Some(blockers) = output_producer.get(s.path.as_str()) {
                for &blocker in blockers {
                    blocked_by.put(i, blocker);
                    blocking.put(blocker, i);
                    blocked = true;
                }
            }
        }
        if !blocked {
            queue.add(i);
        }
    }

    while let Some(curr) = queue.poll() {
        out.write(&inputs[curr])?;

        let blocked_list: Vec<usize> = blocking.get(curr).collect();
        for blocked in blocked_list {
            blocked_by.remove(blocked, curr);
            if !blocked_by.contains_key(blocked) {
                queue.add(blocked);
            }
        }
    }
    Ok(())
}

/// The comparator key of a spawn: `"1_" + firstListedOutput`, or `"2_"` and the text format of
/// a proto with only its stable information.
// Java: com/google/devtools/build/lib/exec/StableSort.java:88 StableSort.stableSort
fn sort_key(o: &SpawnExec) -> String {
    if let Some(first) = o.listed_outputs.first() {
        return format!("1_{first}");
    }

    // Get a proto with only stable information from this proto
    let stripped = SpawnExec {
        command_args: o.command_args.clone(),
        environment_variables: o.environment_variables.clone(),
        platform: Some(o.platform.clone().unwrap_or_default()),
        inputs: o.inputs.clone(),
        mnemonic: o.mnemonic.clone(),
        ..Default::default()
    };
    // `"2_" + stripped.build()`: `Message.toString()` is `TextFormat.printer().printToString`.
    format!("2_{}", text_format_spawn_exec(&stripped))
}

// A SetMultimap that uses reference equality for keys and values.
// Implements only the subset of the SetMultimap API needed by stableSort().
// PORT: java-identity-hash-order. Java iterates an identity hash set, whose order follows
// `System.identityHashCode` and is not reproducible across runs; insertion order is used here.
// The order only matters among spawns whose sort keys are equal.
// Java: com/google/devtools/build/lib/exec/StableSort.java:137 StableSort.IdentitySetMultimap
#[derive(Default)]
struct IdentitySetMultimap {
    map: HashMap<usize, IndexSet<usize>>,
}

impl IdentitySetMultimap {
    fn contains_key(&self, key: usize) -> bool {
        self.map.contains_key(&key)
    }

    fn get(&self, key: usize) -> impl Iterator<Item = usize> + '_ {
        self.map
            .get(&key)
            .into_iter()
            .flat_map(|s| s.iter().copied())
    }

    fn put(&mut self, key: usize, value: usize) {
        self.map.entry(key).or_default().insert(value);
    }

    fn remove(&mut self, key: usize, value: usize) {
        if let Some(value_set) = self.map.get_mut(&key) {
            value_set.shift_remove(&value);
            if value_set.is_empty() {
                self.map.remove(&key);
            }
        }
    }
}

/// `java.util.PriorityQueue` (a binary min-heap with Java's sift algorithms), so that elements
/// with equal keys leave the queue in Java's order.
struct PriorityQueue<T, C: Fn(&T, &T) -> Ordering> {
    queue: Vec<T>,
    comparator: C,
}

impl<T: Copy, C: Fn(&T, &T) -> Ordering> PriorityQueue<T, C> {
    fn new(comparator: C) -> Self {
        PriorityQueue {
            queue: Vec::new(),
            comparator,
        }
    }

    /// `offer(e)`.
    // Java: java/util/PriorityQueue.java offer
    fn add(&mut self, e: T) {
        let k = self.queue.len();
        self.queue.push(e);
        self.sift_up(k, e);
    }

    /// `poll()`.
    // Java: java/util/PriorityQueue.java poll
    fn poll(&mut self) -> Option<T> {
        if self.queue.is_empty() {
            return None;
        }
        let result = self.queue[0];
        let n = self.queue.len() - 1;
        let x = self.queue[n];
        self.queue.truncate(n);
        if n > 0 {
            self.sift_down(0, x, n);
        }
        Some(result)
    }

    // Java: java/util/PriorityQueue.java siftUpUsingComparator
    fn sift_up(&mut self, mut k: usize, x: T) {
        while k > 0 {
            let parent = (k - 1) >> 1;
            let e = self.queue[parent];
            if (self.comparator)(&x, &e) != Ordering::Less {
                break;
            }
            self.queue[k] = e;
            k = parent;
        }
        self.queue[k] = x;
    }

    // Java: java/util/PriorityQueue.java siftDownUsingComparator
    fn sift_down(&mut self, mut k: usize, x: T, n: usize) {
        let half = n >> 1;
        while k < half {
            let mut child = (k << 1) + 1;
            let mut c = self.queue[child];
            let right = child + 1;
            if right < n && (self.comparator)(&c, &self.queue[right]) == Ordering::Greater {
                child = right;
                c = self.queue[child];
            }
            if (self.comparator)(&x, &c) != Ordering::Greater {
                break;
            }
            self.queue[k] = c;
            k = child;
        }
        self.queue[k] = x;
    }
}

// ---- TextFormat (protobuf-java `TextFormat.printer().printToString`) for the fields that
// StableSort's stripped SpawnExec can hold. Fields print in field-number order; proto3 scalars
// only when non-default; message fields when present.

/// `TextFormat.escapeBytes(ByteString.copyFromUtf8(text))`.
// Java: com/google/protobuf/TextFormatEscaper.java escapeBytes
fn escape_text(text: &str, out: &mut String) {
    for &b in text.as_bytes() {
        match b {
            0x07 => out.push_str("\\a"),
            0x08 => out.push_str("\\b"),
            0x0c => out.push_str("\\f"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x0b => out.push_str("\\v"),
            b'\\' => out.push_str("\\\\"),
            b'\'' => out.push_str("\\'"),
            b'"' => out.push_str("\\\""),
            0x20..=0x7e => out.push(b as char),
            _ => {
                out.push('\\');
                out.push((b'0' + ((b >> 6) & 3)) as char);
                out.push((b'0' + ((b >> 3) & 7)) as char);
                out.push((b'0' + (b & 7)) as char);
            }
        }
    }
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn print_string_field(out: &mut String, level: usize, name: &str, value: &str) {
    indent(out, level);
    out.push_str(name);
    out.push_str(": \"");
    escape_text(value, out);
    out.push_str("\"\n");
}

fn print_scalar_field(out: &mut String, level: usize, name: &str, value: &str) {
    indent(out, level);
    out.push_str(name);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

fn begin_message_field(out: &mut String, level: usize, name: &str) {
    indent(out, level);
    out.push_str(name);
    out.push_str(" {\n");
}

fn end_message_field(out: &mut String, level: usize) {
    indent(out, level);
    out.push_str("}\n");
}

fn print_digest(d: &Digest, out: &mut String, level: usize) {
    if !d.hash.is_empty() {
        print_string_field(out, level, "hash", &d.hash);
    }
    if d.size_bytes != 0 {
        print_scalar_field(out, level, "size_bytes", &d.size_bytes.to_string());
    }
    if !d.hash_function_name.is_empty() {
        print_string_field(out, level, "hash_function_name", &d.hash_function_name);
    }
}

fn print_file(f: &File, out: &mut String, level: usize) {
    if !f.path.is_empty() {
        print_string_field(out, level, "path", &f.path);
    }
    if let Some(digest) = &f.digest {
        begin_message_field(out, level, "digest");
        print_digest(digest, out, level + 1);
        end_message_field(out, level);
    }
    if f.is_tool {
        print_scalar_field(out, level, "is_tool", "true");
    }
    if !f.symlink_target_path.is_empty() {
        print_string_field(out, level, "symlink_target_path", &f.symlink_target_path);
    }
}

/// `TextFormat.printer().printToString(spawnExec)` for a SpawnExec holding only command_args,
/// environment_variables, platform, inputs and mnemonic.
// Java: com/google/protobuf/TextFormat.java Printer.printToString
fn text_format_spawn_exec(ex: &SpawnExec) -> String {
    let mut out = String::new();
    for arg in &ex.command_args {
        print_string_field(&mut out, 0, "command_args", arg);
    }
    for env in &ex.environment_variables {
        begin_message_field(&mut out, 0, "environment_variables");
        if !env.name.is_empty() {
            print_string_field(&mut out, 1, "name", &env.name);
        }
        if !env.value.is_empty() {
            print_string_field(&mut out, 1, "value", &env.value);
        }
        end_message_field(&mut out, 0);
    }
    if let Some(platform) = &ex.platform {
        begin_message_field(&mut out, 0, "platform");
        for property in &platform.properties {
            begin_message_field(&mut out, 1, "properties");
            if !property.name.is_empty() {
                print_string_field(&mut out, 2, "name", &property.name);
            }
            if !property.value.is_empty() {
                print_string_field(&mut out, 2, "value", &property.value);
            }
            end_message_field(&mut out, 1);
        }
        end_message_field(&mut out, 0);
    }
    for input in &ex.inputs {
        begin_message_field(&mut out, 0, "inputs");
        print_file(input, &mut out, 1);
        end_message_field(&mut out, 0);
    }
    if !ex.mnemonic.is_empty() {
        print_string_field(&mut out, 0, "mnemonic", &ex.mnemonic);
    }
    out
}
