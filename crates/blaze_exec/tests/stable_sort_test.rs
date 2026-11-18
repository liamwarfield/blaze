//! Port of `src/test/java/com/google/devtools/build/lib/exec/StableSortTest.java`.

use std::io::{self, Cursor};

use blaze_exec::exec::io::{BinaryInputStreamWrapper, MessageInputStream, MessageOutputStream};
use blaze_exec::exec::stable_sort::stable_sort;
use blaze_protos::JavaMessage;
use blaze_protos::tools::protos::{File, SpawnExec};

struct ListOutput {
    list: Vec<SpawnExec>,
}

impl MessageOutputStream<SpawnExec> for ListOutput {
    fn write(&mut self, m: &SpawnExec) -> io::Result<()> {
        self.list.push(m.clone());
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn test_stable_sort(list: &[&SpawnExec]) -> Vec<SpawnExec> {
    let mut baos = Vec::new();
    for spawn in list {
        spawn.write_delimited_to(&mut baos);
    }

    let mut input: Box<dyn MessageInputStream<SpawnExec>> =
        Box::new(BinaryInputStreamWrapper::<SpawnExec>::new(Box::new(
            Cursor::new(baos),
        )));

    let mut out = ListOutput { list: Vec::new() };

    stable_sort(&mut *input, &mut out).unwrap();
    out.list
}

fn create_spawn_exec_builder(inputs: &[&str], outputs: &[&str]) -> SpawnExec {
    let mut e = SpawnExec::default();
    for output in outputs {
        e.actual_outputs.push(File {
            path: output.to_string(),
            ..Default::default()
        });
        e.listed_outputs.push(output.to_string());
    }
    for s in inputs {
        e.inputs.push(File {
            path: s.to_string(),
            ..Default::default()
        });
    }
    e
}

fn create_spawn_exec(inputs: &[&str], outputs: &[&str]) -> SpawnExec {
    create_spawn_exec_builder(inputs, outputs)
}

fn with_command_arg(mut e: SpawnExec, arg: &str) -> SpawnExec {
    e.command_args.push(arg.to_string());
    e
}

fn assert_in_order(actual: Vec<SpawnExec>, expected: &[&SpawnExec]) {
    let expected: Vec<SpawnExec> = expected.iter().map(|e| (*e).clone()).collect();
    assert_eq!(actual, expected);
}

#[test]
fn stable_sort_empty() {
    let l = test_stable_sort(&[]);
    assert!(l.is_empty());
}

#[test]
fn stable_sort_one() {
    let e1 = create_spawn_exec(&[], &["output"]);
    let l = test_stable_sort(&[&e1]);
    assert_in_order(l, &[&e1]);
}

#[test]
fn stable_sort_two_unlinked_lexicographic() {
    let e1 = create_spawn_exec(&["leaf1"], &["a"]);
    let e2 = create_spawn_exec(&["leaf2"], &["b"]);

    let l = test_stable_sort(&[&e1, &e2]);
    assert_in_order(l, &[&e1, &e2]);
}

#[test]
fn stable_sort_two_unlinked_lexicographic_reverse() {
    let e1 = create_spawn_exec(&["leaf1"], &["b"]);
    let e2 = create_spawn_exec(&["leaf2"], &["a"]);

    let l = test_stable_sort(&[&e1, &e2]);
    assert_in_order(l, &[&e2, &e1]);
}

#[test]
fn stable_sort_two_linked() {
    let e1 = create_spawn_exec(&["leaf1"], &["b"]);
    let e2 = create_spawn_exec(&["b"], &["a"]);

    let l = test_stable_sort(&[&e1, &e2]);
    assert_in_order(l, &[&e1, &e2]);
}

#[test]
fn stable_sort_two_linked_input_order_does_not_matter() {
    let e1 = create_spawn_exec(&["leaf1"], &["b"]);
    let e2 = create_spawn_exec(&["b"], &["a"]);

    let l = test_stable_sort(&[&e2, &e1]);
    assert_in_order(l, &[&e1, &e2]);
}

#[test]
fn stable_sort_two_one_of_multiple_inputs() {
    let e1 = create_spawn_exec(&["leaf1"], &["b1", "b2", "b3"]);
    let e2 = create_spawn_exec(&["b2"], &["a"]);

    let l = test_stable_sort(&[&e2, &e1]);
    assert_in_order(l, &[&e1, &e2]);
}

#[test]
fn stable_sort_two_many_of_multiple_inputs() {
    let e1 = create_spawn_exec(&["leaf1"], &["b1", "b2", "b3"]);
    let e2 = create_spawn_exec(&["b2", "b3"], &["a"]);

    let l = test_stable_sort(&[&e2, &e1]);
    assert_in_order(l, &[&e1, &e2]);
}

#[test]
fn stable_sort_two_irrelevant_inputs() {
    let e1 = create_spawn_exec(&["leaf1"], &["b1", "b2", "b3"]);
    let e2 = create_spawn_exec(&["z", "b2", "1"], &["a"]);

    let l = test_stable_sort(&[&e2, &e1]);
    assert_in_order(l, &[&e1, &e2]);
}

#[test]
fn stable_sort_two_abc() {
    let a = create_spawn_exec(&[""], &["a"]);
    let b = create_spawn_exec(&[""], &["b"]);
    let c = create_spawn_exec(&[""], &["c"]);

    let l = test_stable_sort(&[&a, &b, &c]);
    assert_in_order(l, &[&a, &b, &c]);
}

#[test]
fn stable_sort_two_cba() {
    let a = create_spawn_exec(&["b"], &["a"]);
    let b = create_spawn_exec(&["c"], &["b"]);
    let c = create_spawn_exec(&[""], &["c"]);

    let l = test_stable_sort(&[&a, &b, &c]);
    assert_in_order(l, &[&c, &b, &a]);
}

#[test]
fn stable_sort_two_acb() {
    let a = create_spawn_exec(&[""], &["a"]);
    let b = create_spawn_exec(&["a", "c"], &["b"]);
    let c = create_spawn_exec(&[""], &["c"]);

    let l = test_stable_sort(&[&a, &b, &c]);
    assert_in_order(l, &[&a, &c, &b]);
}

#[test]
fn stable_sort_two_cab() {
    let a = create_spawn_exec(&["c"], &["a"]);
    let b = create_spawn_exec(&["c"], &["b"]);
    let c = create_spawn_exec(&[""], &["c"]);

    let l = test_stable_sort(&[&a, &b, &c]);
    assert_in_order(l, &[&c, &a, &b]);
}

#[test]
fn stable_sort_two_cab2() {
    let a = create_spawn_exec(&["c1"], &["a"]);
    let b = create_spawn_exec(&["c2"], &["b"]);
    let c = create_spawn_exec(&[""], &["c1", "c2"]);

    let l = test_stable_sort(&[&a, &b, &c]);
    assert_in_order(l, &[&c, &a, &b]);
}

#[test]
fn stable_sort_two_cbafed() {
    let a = create_spawn_exec(&["b"], &["a"]);
    let b = create_spawn_exec(&["c"], &["b"]);
    let c = create_spawn_exec(&[""], &["c"]);
    let d = create_spawn_exec(&["e"], &["d"]);
    let e = create_spawn_exec(&["f"], &["e"]);
    let f = create_spawn_exec(&[""], &["f"]);

    let l = test_stable_sort(&[&a, &b, &c, &d, &e, &f]);
    assert_in_order(l, &[&c, &b, &a, &f, &e, &d]);
}

#[test]
fn stable_sort_two_interleaved_paths() {
    let a = create_spawn_exec(&["c"], &["a"]);
    let b = create_spawn_exec(&[""], &["b"]);
    let c = create_spawn_exec(&[""], &["c"]);
    let d = create_spawn_exec(&["a"], &["d"]);
    let e = create_spawn_exec(&["f"], &["e"]);
    let f = create_spawn_exec(&["b"], &["f"]);

    let l = test_stable_sort(&[&a, &b, &c, &d, &e, &f]);
    assert_in_order(l, &[&b, &c, &a, &d, &f, &e]);
}

#[test]
fn stable_sort_two_many_dependencies() {
    let a = create_spawn_exec(&["b", "c", "f"], &["a"]);
    let b = create_spawn_exec(&["d", "e"], &["b"]);
    let c = create_spawn_exec(&["e", "d", "f"], &["c"]);
    let d = create_spawn_exec(&[""], &["d"]);
    let e = create_spawn_exec(&["f"], &["e"]);
    let f = create_spawn_exec(&[""], &["f"]);

    let l = test_stable_sort(&[&a, &b, &c, &d, &e, &f]);
    assert_in_order(l, &[&d, &f, &e, &b, &c, &a]);
}

#[test]
fn stable_sort_no_outputs() {
    let a = create_spawn_exec(&["a"], &[]);
    let b = create_spawn_exec(&["b"], &[]);

    let l = test_stable_sort(&[&a, &b]);
    assert_in_order(l, &[&a, &b]);
}

#[test]
fn stable_sort_no_outputs_reversed() {
    let a = create_spawn_exec(&["a"], &[]);
    let b = create_spawn_exec(&["b"], &[]);

    let l = test_stable_sort(&[&b, &a]);
    assert_in_order(l, &[&a, &b]);
}

#[test]
fn stable_sort_listed_outputs() {
    let a = with_command_arg(create_spawn_exec_builder(&[], &["a"]), "a");
    let b = with_command_arg(create_spawn_exec_builder(&[], &[]), "b");
    let c = with_command_arg(create_spawn_exec_builder(&[], &["c"]), "c");
    let d = with_command_arg(create_spawn_exec_builder(&[], &["d"]), "d");
    let e = with_command_arg(create_spawn_exec_builder(&[], &[]), "e");
    let f = with_command_arg(create_spawn_exec_builder(&[], &[]), "f");

    let l = test_stable_sort(&[&a, &b, &c, &d, &e, &f]);
    assert_in_order(
        l,
        &[
            // sorted elements with actual outputs
            &a, &c, &d, // sorted elements without listed outputs
            &b, &e, &f,
        ],
    );
}

#[test]
fn stable_sort_listed_outputs_reordered() {
    let a = with_command_arg(create_spawn_exec_builder(&[], &["a"]), "a");
    let b = with_command_arg(create_spawn_exec_builder(&[], &[]), "b");
    let c = with_command_arg(create_spawn_exec_builder(&[], &["c"]), "c");
    let d = with_command_arg(create_spawn_exec_builder(&[], &["d"]), "d");
    let e = with_command_arg(create_spawn_exec_builder(&[], &[]), "e");
    let f = with_command_arg(create_spawn_exec_builder(&[], &[]), "f");

    // Reordering the input from the previous test does not change the resulting order
    let l = test_stable_sort(&[&f, &e, &d, &c, &b, &a]);
    assert_in_order(
        l,
        &[
            // sorted elements with actual outputs
            &a, &c, &d, // sorted elements without listed outputs
            &b, &e, &f,
        ],
    );
}

#[test]
fn stable_sort_listed_outputs_dependencies() {
    // Dependencies are respected
    let a = with_command_arg(create_spawn_exec_builder(&["d"], &["a"]), "a");
    let b = with_command_arg(create_spawn_exec_builder(&[], &[]), "b");
    let c = with_command_arg(create_spawn_exec_builder(&["d"], &["c"]), "c");
    let d = with_command_arg(create_spawn_exec_builder(&[], &["d"]), "d");
    let e = with_command_arg(create_spawn_exec_builder(&[], &[]), "e");
    let f = with_command_arg(create_spawn_exec_builder(&[], &[]), "f");

    let l = test_stable_sort(&[&f, &e, &d, &c, &b, &a]);
    assert_in_order(l, &[&d, &a, &c, &b, &e, &f]);
}

#[test]
fn stable_sort_execs_with_duplicate_outputs() {
    let a = create_spawn_exec(&["a"], &["c"]);
    let b = create_spawn_exec(&["b"], &["c"]);
    let c = create_spawn_exec(&["c"], &["d"]);

    let l = test_stable_sort(&[&a, &b, &c]);
    assert_in_order(l, &[&a, &b, &c]);
}
