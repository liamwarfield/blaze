//! `to_action_input_set`: the `NestedSet<ActionInput>` of a spawn is the action's own
//! `NestedSet<Artifact>` in Java (an upcast, the same object), so a sub-set shared by two actions
//! is one node in both spawns' inputs. `CompactSpawnLogContext.logInputSet` keys shared sets by
//! that node and logs each once (the oracle's compact log of brotli: 100 `input_set` entries,
//! which Blaze's log reproduces only when the conversion keeps the sharing).

#![allow(clippy::mutable_key_type)]

mod analysis_actions_common;

use analysis_actions_common::TestDirs;
use blaze_collect::NestedSetBuilder;
use blaze_exec::analysis::actions::to_action_input_set;

#[test]
fn a_shared_sub_set_is_one_node_in_every_converted_set() {
    let dirs = TestDirs::new();
    let mut shared = NestedSetBuilder::stable_order();
    shared.add(dirs.source_artifact("pkg/a.h"));
    shared.add(dirs.source_artifact("pkg/b.h"));
    let shared = shared.build();
    let mut first = NestedSetBuilder::stable_order();
    first.add(dirs.source_artifact("pkg/one.cc"));
    first.add_transitive(&shared);
    let first = first.build();
    let mut second = NestedSetBuilder::stable_order();
    second.add(dirs.source_artifact("pkg/two.cc"));
    second.add_transitive(&shared);
    let second = second.build();

    // Two spawns of two actions, converted separately.
    let first_inputs = to_action_input_set(&first);
    let second_inputs = to_action_input_set(&second);
    let first_children = first_inputs.get_non_leaves();
    let second_children = second_inputs.get_non_leaves();
    assert_eq!(first_children.len(), 1);
    assert_eq!(second_children.len(), 1);
    assert!(first_children[0].to_node() == second_children[0].to_node());
    assert!(first_children[0].to_node() == to_action_input_set(&shared).to_node());
    // The whole set converted again (a second spawn of the same action) is the same node too.
    assert!(to_action_input_set(&first).to_node() == first_inputs.to_node());
    // Different sets stay different nodes.
    assert!(first_inputs.to_node() != second_inputs.to_node());
}
