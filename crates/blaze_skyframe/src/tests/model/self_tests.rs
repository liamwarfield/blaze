//! Self-tests of the model: the simulated incremental evaluation against the from-scratch
//! semantics for many seeds, minimality of `ran`, and no-op edits.

use super::generate::{GenConfig, generate_with};
use super::graph::{ModelGraph, ModelRun};
use super::scratch::{eval_from_scratch, keep_going_outcomes};
use super::spec::{Edit, ErrClass, EvalRequest, GraphSpec, NodeId, RootResult, Scenario};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

/// Seeds per randomized self-test.
const SEEDS: u64 = 300;

/// Bookkeeping for the minimality check: a node may run only if it was absent when the
/// evaluation started, never finished before, or something in its from-scratch closure changed
/// since it last finished (a leaf injected, a program set, a node invalidated, transient errors
/// invalidated while the closure has a transient thrower).
#[derive(Default)]
struct Minimality {
    touches: BTreeMap<NodeId, u64>,
    transient_touches: u64,
    finished: BTreeMap<NodeId, u64>,
}

impl Minimality {
    fn note_edit(&mut self, e: &Edit) {
        match e {
            Edit::SetLeaf { node, .. }
            | Edit::SetProgram { node, .. }
            | Edit::Invalidate { node } => *self.touches.entry(*node).or_default() += 1,
            Edit::InvalidateTransientErrors => self.transient_touches += 1,
            Edit::Delete { .. } | Edit::DeleteDirty { .. } => {}
        }
    }

    fn sig(&self, spec: &GraphSpec, n: NodeId) -> u64 {
        let mut h = DefaultHasher::new();
        let mut transient = false;
        for m in keep_going_outcomes(spec, &[n]).keys() {
            m.hash(&mut h);
            format!("{:?}", spec.node(*m)).hash(&mut h);
            self.touches.get(m).hash(&mut h);
            transient |= spec
                .program(*m)
                .is_some_and(|p| p.throws.as_ref().is_some_and(|t| t.transient));
        }
        if transient {
            self.transient_touches.hash(&mut h);
        }
        h.finish()
    }

    fn check(&self, spec: &GraphSpec, run: &ModelRun) -> Result<(), String> {
        for n in &run.ran {
            if run.present_at_start.contains(n) && self.finished.get(n) == Some(&self.sig(spec, *n))
            {
                return Err(format!("n{n} ran although nothing it depends on changed"));
            }
        }
        Ok(())
    }

    fn record(&mut self, spec: &GraphSpec, run: &ModelRun) {
        for n in run.ran.iter().chain(&run.marked_clean) {
            let s = self.sig(spec, *n);
            self.finished.insert(*n, s);
        }
    }
}

#[derive(Default, Debug)]
struct Coverage {
    evaluations: u64,
    exact: u64,
    aborted: u64,
    interrupted: u64,
    with_ran: u64,
    with_marked_clean: u64,
    with_change_pruned: u64,
    cycle_results: u64,
    exception_results: u64,
    transient_results: u64,
    no_keep_going_exact: u64,
}

fn check_run(
    spec: &GraphSpec,
    req: &EvalRequest,
    run: &ModelRun,
    first: bool,
    cov: &mut Coverage,
) -> Result<(), String> {
    cov.evaluations += 1;
    cov.exact += run.exact as u64;
    cov.aborted += run.aborted as u64;
    cov.interrupted += run.interrupted as u64;
    cov.with_ran += !run.ran.is_empty() as u64;
    cov.with_marked_clean += !run.marked_clean.is_empty() as u64;
    cov.with_change_pruned += !run.change_pruned.is_empty() as u64;
    if !run.ran.is_subset(&run.ran_upper) {
        return Err(format!(
            "ran {:?} not within upper {:?}",
            run.ran, run.ran_upper
        ));
    }
    if !run.change_pruned.is_subset(&run.ran) {
        return Err("change pruned without running".into());
    }
    if !run.marked_clean.is_disjoint(&run.ran) {
        return Err("marked clean and ran".into());
    }
    let Some(results) = &run.results else {
        return Ok(());
    };
    if run.exact && !req.keep_going {
        cov.no_keep_going_exact += 1;
    }
    for (_, r) in results {
        match r {
            RootResult::Error(ErrClass::Cycle) => cov.cycle_results += 1,
            RootResult::Error(ErrClass::Exception { transient, .. }) => {
                cov.exception_results += 1;
                cov.transient_results += *transient as u64;
            }
            _ => {}
        }
    }
    let expected = eval_from_scratch(spec, &req.roots, req.keep_going);
    expected
        .check(results, run.has_error.expect("has_error with results"))
        .map_err(|e| format!("incremental != from scratch: {e}\nexpected {expected:?}"))?;
    if run.exact && req.keep_going {
        if !run.warnings.is_subset(&expected.warnings) {
            return Err(format!(
                "warnings {:?} not reported by a clean build",
                run.warnings
            ));
        }
        if !run.stored_events.is_subset(&expected.stored_events) {
            return Err(format!(
                "events {:?} not reported by a clean build",
                run.stored_events
            ));
        }
    }
    if first && run.exact {
        // A fresh graph with every leaf injected: every function node the roots reach runs once
        // and every completed run reports its events.
        let functions: BTreeSet<NodeId> = expected
            .closure
            .iter()
            .copied()
            .filter(|n| spec.program(*n).is_some())
            .collect();
        if run.ran != functions {
            return Err(format!(
                "first evaluation ran {:?}, closure {functions:?}",
                run.ran
            ));
        }
        if run.warnings != expected.warnings || run.stored_events != expected.stored_events {
            return Err("first evaluation events differ from a clean build".into());
        }
    }
    Ok(())
}

fn replay(sc: &Scenario, require_exact: bool, cov: &mut Coverage) -> Result<(), String> {
    let mut spec = sc.spec.clone();
    let mut model = ModelGraph::new();
    let mut mini = Minimality::default();
    for (i, (edits, req)) in sc.steps.iter().enumerate() {
        for e in edits {
            model.apply_edit(&mut spec, e);
            mini.note_edit(e);
        }
        let run = if req.interrupt_after_runs.is_some() {
            model.evaluate_interrupted(&spec, req)
        } else {
            model.evaluate(&spec, req)
        };
        let fail = |what: String| {
            format!(
                "seed {} step {i}: {what}\n{spec:?}\nedits {edits:?}\n{req:?}\nran {:?} clean {:?} \
                 pruned {:?} results {:?}",
                sc.seed, run.ran, run.marked_clean, run.change_pruned, run.results
            )
        };
        if require_exact && !run.exact {
            return Err(fail("not exact".into()));
        }
        check_run(&spec, req, &run, i == 0, cov).map_err(fail)?;
        if run.exact {
            mini.check(&spec, &run).map_err(fail)?;
            mini.record(&spec, &run);
        }
    }
    Ok(())
}

#[test]
fn model_matches_from_scratch_exact_mode() {
    let mut cov = Coverage::default();
    for seed in 0..SEEDS {
        let sc = generate_with(seed, &GenConfig::default());
        if let Err(e) = replay(&sc, true, &mut cov) {
            panic!("{e}");
        }
    }
    // The generator exercises what the model is for.
    assert!(cov.with_marked_clean > 20, "{cov:?}");
    assert!(cov.with_change_pruned > 20, "{cov:?}");
    assert!(cov.cycle_results > 20, "{cov:?}");
    assert!(cov.exception_results > 20, "{cov:?}");
    assert!(cov.transient_results > 5, "{cov:?}");
    assert!(cov.no_keep_going_exact > 20, "{cov:?}");
    assert_eq!(cov.exact, cov.evaluations);
}

#[test]
fn model_matches_from_scratch_wide_mode() {
    let mut cov = Coverage::default();
    for seed in 0..SEEDS {
        let sc = generate_with(seed, &GenConfig::wide());
        if let Err(e) = replay(&sc, false, &mut cov) {
            panic!("{e}");
        }
    }
    assert!(cov.aborted > 20, "{cov:?}");
    assert!(cov.interrupted > 20, "{cov:?}");
}

#[test]
fn model_matches_from_scratch_guarded_and_larger() {
    let mut cov = Coverage::default();
    let cfg = GenConfig {
        guarded_requests: true,
        max_nodes: 14,
        steps: 10,
        ..GenConfig::default()
    };
    for seed in 0..SEEDS / 3 {
        let sc = generate_with(seed, &cfg);
        sc.spec.validate().unwrap();
        if let Err(e) = replay(&sc, true, &mut cov) {
            panic!("{e}");
        }
    }
}

#[test]
fn generator_is_deterministic() {
    for seed in 0..20 {
        let a = generate_with(seed, &GenConfig::wide());
        let b = generate_with(seed, &GenConfig::wide());
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
}

/// With every node comparable: injecting every leaf's current value runs nothing and dirties
/// nothing; setting a function node's program to itself (CHANGE) reruns only that node when its
/// value carries no error (it is change pruned, its parents are marked clean).
#[test]
fn no_op_edits_run_nothing() {
    let cfg = GenConfig {
        comparable_percent: 100,
        ..GenConfig::default()
    };
    let mut checked = 0;
    let mut pruned_checked = 0;
    for seed in 0..SEEDS {
        let sc = generate_with(seed, &cfg);
        let mut spec = sc.spec.clone();
        let mut model = ModelGraph::new();
        for (edits, req) in &sc.steps {
            for e in edits {
                model.apply_edit(&mut spec, e);
            }
            let run = model.evaluate(&spec, req);
            assert!(run.exact);
            if !model.in_flight().is_empty() {
                continue;
            }
            // Same values injected again.
            let mut m2 = model.clone();
            let mut s2 = spec.clone();
            let leaves: Vec<(NodeId, i64)> = s2.leaves().collect();
            for (node, value) in leaves {
                m2.apply_edit(&mut s2, &Edit::SetLeaf { node, value });
            }
            let r2 = m2.evaluate(&s2, req);
            assert!(
                r2.ran.is_empty() && r2.marked_clean.is_empty() && r2.dirtied.is_empty(),
                "seed {seed}: {r2:?}"
            );
            checked += 1;
            // The same program set again on a function node the evaluation reached whose value
            // carries no error.
            let candidates: Vec<NodeId> = run
                .ran
                .iter()
                .copied()
                .filter(|n| {
                    spec.program(*n).is_some()
                        && model
                            .node(super::graph::MKey::Node(*n))
                            .is_some_and(|node| match &node.state {
                                super::graph::MState::Done { value, .. } => value.error.is_none(),
                                _ => false,
                            })
                })
                .collect();
            if let Some(f) = candidates.first() {
                let mut m3 = model.clone();
                let mut s3 = spec.clone();
                let program = s3.program(*f).unwrap().clone();
                m3.apply_edit(&mut s3, &Edit::SetProgram { node: *f, program });
                let r3 = m3.evaluate(&s3, req);
                assert!(
                    r3.ran.is_subset(&BTreeSet::from([*f])),
                    "seed {seed}: {r3:?}"
                );
                assert_eq!(r3.change_pruned, r3.ran, "seed {seed}: {r3:?}");
                pruned_checked += !r3.ran.is_empty() as u64;
            }
        }
    }
    assert!(
        checked > 100 && pruned_checked > 50,
        "{checked} {pruned_checked}"
    );
}
