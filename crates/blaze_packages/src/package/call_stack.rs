//! Port of `packages/CallStack.java`: compact, interned call stacks of rule and macro
//! instantiations.

use std::sync::Arc;

use blaze_starlark::Location;
use blaze_starlark::error::StackEntry;

use crate::target::Rule;

/// One frame of a compacted call stack, linked to the next (inner) frame (Java
/// `CallStack.Node`).
// Java: com/google/devtools/build/lib/packages/CallStack.java:53 CallStack.Node
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallStackNode {
    name: String,
    file: String,
    line: i32,
    col: i32,
    next: Option<Arc<CallStackNode>>,
}

impl CallStackNode {
    // Java: com/google/devtools/build/lib/packages/CallStack.java:76 CallStack.Node.<init>
    pub fn new(
        name: String,
        file: String,
        line: i32,
        col: i32,
        next: Option<Arc<CallStackNode>>,
    ) -> Self {
        CallStackNode {
            name,
            file,
            line,
            col,
            next,
        }
    }

    // Java: com/google/devtools/build/lib/packages/CallStack.java:84 CallStack.Node.toLocation
    pub fn to_location(&self) -> Location {
        Location {
            file: self.file.clone(),
            line: self.line as usize,
            column: self.col as usize,
        }
    }

    // Java: com/google/devtools/build/lib/packages/CallStack.java:88 CallStack.Node.toCallStackEntry
    pub fn to_call_stack_entry(&self) -> StackEntry {
        StackEntry {
            name: self.name.clone(),
            // PORT: blaze_starlark represents Java's `Location.BUILTIN` frame location as None.
            location: if self.to_location() == Location::builtin() {
                None
            } else {
                Some(self.to_location())
            },
            source_line: None,
        }
    }

    // Java: com/google/devtools/build/lib/packages/CallStack.java:92 CallStack.Node.functionName
    pub fn function_name(&self) -> &str {
        &self.name
    }

    // Java: com/google/devtools/build/lib/packages/CallStack.java:97 CallStack.Node.next
    pub fn next(&self) -> Option<&Arc<CallStackNode>> {
        self.next.as_ref()
    }
}

/// The full call stack of a rule: its location frame plus its interior call stack.
// Java: com/google/devtools/build/lib/packages/CallStack.java:47 CallStack.getFullCallStack
pub fn get_full_call_stack(rule: &Rule) -> Option<Arc<CallStackNode>> {
    let location = rule.get_location();
    Some(Arc::new(CallStackNode::new(
        TOP_LEVEL.to_owned(),
        location.file.clone(),
        location.line as i32,
        location.column as i32,
        rule.get_interior_call_stack().cloned(),
    )))
}

/// Java `StarlarkThread.TOP_LEVEL`.
// Java: net/starlark/java/eval/StarlarkThread.java StarlarkThread.TOP_LEVEL
pub const TOP_LEVEL: &str = "<toplevel>";

/// Java `new Node(entry.name, entry.location, next)`; a builtin frame has Java's
/// `Location.BUILTIN` (`<builtin>`, 0, 0).
fn node_of_entry(entry: &StackEntry, next: Option<Arc<CallStackNode>>) -> CallStackNode {
    let location = entry.location.clone().unwrap_or_else(Location::builtin);
    CallStackNode::new(
        entry.name.clone(),
        location.file,
        location.line as i32,
        location.column as i32,
        next,
    )
}

/// Compacts `stack[start..]` into a linked list of nodes (`None` for an empty suffix).
// PORT: Java interns nodes in a weak interner (memory only); nodes are shared via Arc.
// Java: com/google/devtools/build/lib/packages/CallStack.java:139 CallStack.compact
pub fn compact(stack: &[StackEntry], start: usize) -> Option<Arc<CallStackNode>> {
    let mut node: Option<Arc<CallStackNode>> = None;
    let mut i = stack.len();
    while i > start {
        i -= 1;
        node = Some(Arc::new(node_of_entry(&stack[i], node)));
    }
    node
}

/// The stack `outer` followed by `inner`.
// Java: com/google/devtools/build/lib/packages/CallStack.java:158 CallStack.concatenate
pub fn concatenate(
    outer: Option<&Arc<CallStackNode>>,
    inner: Option<Arc<CallStackNode>>,
) -> Option<Arc<CallStackNode>> {
    let mut outer_reversed: Vec<&Arc<CallStackNode>> = Vec::new();
    let mut outer = outer;
    while let Some(o) = outer {
        outer_reversed.push(o);
        outer = o.next();
    }
    let mut node = inner;
    // Java iterates the deque built with addFirst: innermost outer frame first.
    for orig_outer_node in outer_reversed.into_iter().rev() {
        node = Some(Arc::new(CallStackNode::new(
            orig_outer_node.name.clone(),
            orig_outer_node.file.clone(),
            orig_outer_node.line,
            orig_outer_node.col,
            node,
        )));
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_from_name_and_location(name: &str, file: &str, line: usize, col: usize) -> StackEntry {
        StackEntry {
            name: name.to_owned(),
            location: Some(Location {
                file: file.to_owned(),
                line,
                column: col,
            }),
            source_line: None,
        }
    }

    /// Asserts the node faithfully represents the expected stack, ignoring `expected_start` of
    /// the expected stack's outer frames.
    fn assert_call_stack_contents(
        compacted: Option<&Arc<CallStackNode>>,
        expected: &[StackEntry],
        expected_start: usize,
    ) {
        let mut reconstituted = Vec::new();
        let mut node = compacted;
        while let Some(n) = node {
            reconstituted.push(n.to_call_stack_entry());
            node = n.next();
        }
        assert_eq!(reconstituted, expected[expected_start..].to_vec());
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/CallStackTest.java:42 CallStackTest.emptyCallStack_null
    #[test]
    fn empty_call_stack_null() {
        assert!(compact(&[], 0).is_none());
        assert!(compact(&[], 1).is_none());
        assert!(compact(&[], 42).is_none());
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/CallStackTest.java:49 CallStackTest.singleFrameCallStack_nullInterior
    #[test]
    fn single_frame_call_stack_null_interior() {
        let stack = vec![entry_from_name_and_location(TOP_LEVEL, "BUILD", 10, 20)];

        let compacted = compact(&stack, 0);
        assert!(compacted.is_some());
        assert!(compacted.unwrap().next().is_none());
        assert!(compact(&stack, 1).is_none());
        assert!(compact(&stack, 42).is_none());
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/CallStackTest.java:61 CallStackTest.simpleCallStack
    #[test]
    fn simple_call_stack() {
        let stack = vec![
            entry_from_name_and_location(TOP_LEVEL, "BUILD", 10, 20),
            entry_from_name_and_location("func", "file.bzl", 20, 30),
        ];

        let compacted0 = compact(&stack, 0);
        let compacted1 = compact(&stack, 1);
        assert_eq!(compacted0.as_ref().unwrap().next(), compacted1.as_ref());
        assert!(compacted1.as_ref().unwrap().next().is_none());
        assert_call_stack_contents(compacted0.as_ref(), &stack, 0);
        assert_call_stack_contents(compacted1.as_ref(), &stack, 1);
        assert!(compact(&stack, 2).is_none());
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/CallStackTest.java:77 CallStackTest.callStackWithLoops
    #[test]
    fn call_stack_with_loops() {
        let loop_entry1 = entry_from_name_and_location("loop1", "file1.bzl", 20, 30);
        let loop_entry2 = entry_from_name_and_location("loop2", "file2.bzl", 30, 40);

        let stack = vec![
            entry_from_name_and_location(TOP_LEVEL, "BUILD", 10, 20),
            loop_entry1.clone(),
            loop_entry2.clone(),
            loop_entry1,
            loop_entry2,
        ];

        assert_call_stack_contents(compact(&stack, 1).as_ref(), &stack, 1);
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/CallStackTest.java:95 CallStackTest.consecutiveCalls
    #[test]
    fn consecutive_calls() {
        let stack1 = vec![
            entry_from_name_and_location(TOP_LEVEL, "BUILD", 1, 2),
            entry_from_name_and_location("f1", "f.bzl", 2, 3),
            entry_from_name_and_location("g1", "g.bzl", 3, 4),
        ];
        let mut stack2 = stack1.clone();
        stack2.push(entry_from_name_and_location("h1", "h.bzl", 4, 5));

        assert_call_stack_contents(compact(&stack1, 1).as_ref(), &stack1, 1);
        assert_call_stack_contents(compact(&stack2, 1).as_ref(), &stack2, 1);
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/CallStackTest.java:110 CallStackTest.sharesCommonTail
    #[test]
    #[ignore = "needs CallStack.Node interning (Java weak interner; nodes are not interned here)"]
    fn shares_common_tail() {
        let stack1 = vec![
            entry_from_name_and_location(TOP_LEVEL, "a/BUILD", 1, 2),
            entry_from_name_and_location("java_library_macro", "java_library_macro.bzl", 2, 3),
            entry_from_name_and_location("java_library", "java_library.bzl", 4, 5),
        ];
        let stack2 = vec![
            entry_from_name_and_location(TOP_LEVEL, "b/BUILD", 6, 7),
            entry_from_name_and_location("java_library_macro", "java_library_macro.bzl", 2, 3),
            entry_from_name_and_location("java_library", "java_library.bzl", 4, 5),
        ];

        let optimized_interior_stack1 = compact(&stack1, 1).unwrap();
        let optimized_full_stack1 = compact(&stack1, 0).unwrap();
        let optimized_full_stack2 = compact(&stack2, 0).unwrap();
        assert!(Arc::ptr_eq(
            optimized_full_stack1.next().unwrap(),
            optimized_full_stack2.next().unwrap()
        ));
        assert!(Arc::ptr_eq(
            optimized_full_stack1.next().unwrap(),
            &optimized_interior_stack1
        ));
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/CallStackTest.java:138 CallStackTest.serialization
    #[test]
    #[ignore = "needs serialization codecs (not ported)"]
    fn serialization() {
        unimplemented!("CallStack.Node codec round trip")
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/CallStackTest.java:197 CallStackTest.concatenate
    #[test]
    fn concatenate_test() {
        let outer_stack = vec![
            entry_from_name_and_location(TOP_LEVEL, "BUILD", 10, 20),
            entry_from_name_and_location("foo", "f.bzl", 1, 2),
            entry_from_name_and_location("bar", "g.bzl", 3, 4),
        ];
        let inner_stack = vec![
            entry_from_name_and_location("baz", "h.bzl", 5, 6),
            entry_from_name_and_location("qux", "g.bzl", 7, 8),
        ];

        assert!(concatenate(None, None).is_none());
        assert_call_stack_contents(
            concatenate(compact(&outer_stack, 1).as_ref(), None).as_ref(),
            &outer_stack,
            1,
        );
        assert_call_stack_contents(
            concatenate(None, compact(&inner_stack, 0)).as_ref(),
            &inner_stack,
            0,
        );
        let mut both = outer_stack.clone();
        both.extend(inner_stack.clone());
        assert_call_stack_contents(
            concatenate(compact(&outer_stack, 1).as_ref(), compact(&inner_stack, 0)).as_ref(),
            &both,
            1,
        );
    }
}
