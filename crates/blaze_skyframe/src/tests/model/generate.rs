//! Seeded generator of scenarios.

use super::graph::ModelGraph;
use super::spec::{
    Dep, Edit, EvalRequest, GraphSpec, NodeId, NodeKind, NodeSpec, Program, Scenario, Step, Throw,
    ThrowWhen,
};
use std::collections::BTreeSet;

/// SplitMix64: small, deterministic, good enough for test generation.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x5DEE_CE66_D1CE_4E5B)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    pub fn percent(&mut self, p: u32) -> bool {
        self.below(100) < p as u64
    }

    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

/// Generator settings. The defaults give scenarios small enough that 200 seeds run in a few
/// seconds single-threaded.
#[derive(Clone, Debug)]
pub struct GenConfig {
    pub min_nodes: u32,
    pub max_nodes: u32,
    pub steps: usize,
    pub max_edits_per_step: usize,
    /// Programs may request themselves or later nodes, so cycles appear and disappear.
    pub cycles: bool,
    /// Every evaluation is exact (`ModelRun::exact`): no interrupts, and a no-keep-going
    /// evaluation that the model says stops at an error is made keep_going instead.
    pub exact: bool,
    /// Every step after a program's first is an `IfEven` on a dep of the first step, so a run
    /// never requests a new group after a dep it could not read: Java's restarting functions
    /// and Blaze's blocking requests then ask for the same deps in the same order (crate docs,
    /// "PORT: dependencies are evaluated in request order"). Off by default: the difference
    /// only changes which functions run in no-keep-going evaluations that stop at an error,
    /// which the model never treats as exact.
    pub guarded_requests: bool,
    pub comparable_percent: u32,
    /// Chance of an interrupt per evaluation (ignored when `exact`).
    pub interrupt_percent: u32,
}

impl Default for GenConfig {
    fn default() -> Self {
        GenConfig {
            min_nodes: 4,
            max_nodes: 9,
            steps: 6,
            max_edits_per_step: 3,
            cycles: true,
            exact: true,
            guarded_requests: false,
            comparable_percent: 70,
            interrupt_percent: 10,
        }
    }
}

impl GenConfig {
    /// Everything allowed: no-keep-going evaluations that stop at an error and interrupts (after
    /// which the model gives bounds only).
    pub fn wide() -> GenConfig {
        GenConfig {
            exact: false,
            ..GenConfig::default()
        }
    }
}

/// `generate_with(seed, &GenConfig::default())`.
pub fn generate(seed: u64) -> Scenario {
    generate_with(seed, &GenConfig::default())
}

pub fn generate_with(seed: u64, cfg: &GenConfig) -> Scenario {
    let mut rng = Rng::new(seed);
    let n = cfg.min_nodes + rng.below((cfg.max_nodes - cfg.min_nodes + 1) as u64) as u32;
    let mut nodes = vec![];
    for i in 0..n {
        let comparable = rng.percent(cfg.comparable_percent);
        let kind = if i < 2 || rng.percent(25) {
            NodeKind::Leaf(rng.below(4) as i64)
        } else {
            NodeKind::Function(gen_program(&mut rng, cfg, i, n))
        };
        nodes.push(NodeSpec { kind, comparable });
    }
    let spec = GraphSpec { nodes };
    spec.validate().expect("generated spec");

    let mut steps = vec![];
    let mut cur = spec.clone();
    let mut model = ModelGraph::new();
    for step in 0..cfg.steps {
        let edits: Vec<Edit> = if step == 0 {
            cur.leaves()
                .map(|(node, value)| Edit::SetLeaf { node, value })
                .collect()
        } else {
            let k = 1 + rng.below(cfg.max_edits_per_step as u64) as usize;
            (0..k).map(|_| gen_edit(&mut rng, cfg, &cur)).collect()
        };
        for e in &edits {
            model.apply_edit(&mut cur, e);
        }
        let mut req = EvalRequest {
            roots: gen_roots(&mut rng, n),
            keep_going: rng.percent(50),
            parallelism: rng.pick(&[1, 1, 2, 4, 8]),
            interrupt_after_runs: None,
        };
        if cfg.exact {
            if !req.keep_going {
                let mut probe = model.clone();
                if probe.evaluate(&cur, &req).aborted {
                    req.keep_going = true;
                }
            }
            let run = model.evaluate(&cur, &req);
            assert!(run.exact, "seed {seed}: generator lost exactness");
        } else if rng.percent(cfg.interrupt_percent) {
            req.interrupt_after_runs = Some(rng.below(4) as u32);
            model.evaluate_interrupted(&cur, &req);
        } else {
            model.evaluate(&cur, &req);
        }
        steps.push((edits, req));
    }
    Scenario { seed, spec, steps }
}

fn gen_roots(rng: &mut Rng, n: u32) -> Vec<NodeId> {
    let k = 1 + rng.below(3);
    (0..k)
        .map(|_| {
            // Prefer the top half, where the function nodes that reach most of the graph are.
            if rng.percent(75) {
                n / 2 + rng.below((n - n / 2) as u64) as u32
            } else {
                rng.below(n as u64) as u32
            }
        })
        .collect()
}

/// A dep of node `me` not yet used by the program.
fn gen_dep(
    rng: &mut Rng,
    cfg: &GenConfig,
    me: NodeId,
    n: u32,
    used: &mut BTreeSet<NodeId>,
) -> Option<Dep> {
    for _ in 0..8 {
        let node = if cfg.cycles && rng.percent(6) {
            // A back edge: itself or a later node.
            me + rng.below((n - me) as u64) as u32
        } else if me == 0 {
            continue;
        } else {
            rng.below(me as u64) as u32
        };
        if used.insert(node) {
            let catch = rng.percent(20).then(|| rng.below(4) as i64);
            return Some(Dep { node, catch });
        }
    }
    None
}

fn gen_group(
    rng: &mut Rng,
    cfg: &GenConfig,
    me: NodeId,
    n: u32,
    used: &mut BTreeSet<NodeId>,
    max: u64,
) -> Vec<Dep> {
    let k = 1 + rng.below(max);
    (0..k)
        .filter_map(|_| gen_dep(rng, cfg, me, n, used))
        .collect()
}

pub fn gen_program(rng: &mut Rng, cfg: &GenConfig, me: NodeId, n: u32) -> Program {
    let mut used = BTreeSet::new();
    let mut steps = vec![];
    let first = gen_group(rng, cfg, me, n, &mut used, 2);
    let first_nodes: Vec<NodeId> = first.iter().map(|d| d.node).collect();
    let mut requested: Vec<NodeId> = first_nodes.clone();
    steps.push(Step::Request(first));
    let more = rng.below(3);
    for _ in 0..more {
        let conditional = cfg.guarded_requests || rng.percent(50);
        if conditional && !requested.is_empty() {
            let on = if cfg.guarded_requests {
                match first_nodes.first() {
                    Some(f) => *f,
                    None => break,
                }
            } else {
                rng.pick(&requested)
            };
            let then = gen_group(rng, cfg, me, n, &mut used, 2);
            let otherwise = if rng.percent(50) {
                gen_group(rng, cfg, me, n, &mut used, 1)
            } else {
                vec![]
            };
            steps.push(Step::IfEven {
                on,
                then,
                otherwise,
            });
        } else if !cfg.guarded_requests {
            let g = gen_group(rng, cfg, me, n, &mut used, 2);
            requested.extend(g.iter().map(|d| d.node));
            steps.push(Step::Request(g));
        }
    }
    let modulus = 2 + rng.below(3) as i64;
    let throws = rng.percent(15).then(|| Throw {
        transient: rng.percent(40),
        when: if rng.percent(30) {
            ThrowWhen::Always
        } else {
            ThrowWhen::ResultIs(rng.below(modulus as u64) as i64)
        },
    });
    Program {
        steps,
        modulus,
        bias: rng.below(modulus as u64) as i64,
        throws,
        warning: rng.percent(30),
        stored_event: rng.percent(30),
    }
}

fn mutate(rng: &mut Rng, cfg: &GenConfig, me: NodeId, n: u32, p: &Program) -> Program {
    let mut p = p.clone();
    match rng.below(6) {
        // A new program.
        0 => return gen_program(rng, cfg, me, n),
        // Start or stop failing.
        1 => {
            p.throws = match p.throws {
                Some(_) => None,
                None => Some(Throw {
                    transient: rng.percent(40),
                    when: ThrowWhen::Always,
                }),
            }
        }
        // A different result.
        2 => p.bias = (p.bias + 1).rem_euclid(p.modulus),
        // Create a cycle (or at least a back edge).
        3 if cfg.cycles => {
            let mut used: BTreeSet<NodeId> = (0..n).filter(|d| program_uses(&p, *d)).collect();
            let node = me + rng.below((n - me) as u64) as u32;
            if used.insert(node) {
                p.steps.push(Step::Request(vec![Dep { node, catch: None }]));
            }
        }
        // Break a cycle: drop every back edge.
        4 => {
            for s in &mut p.steps {
                match s {
                    Step::Request(g) => g.retain(|d| d.node < me),
                    Step::IfEven {
                        then, otherwise, ..
                    } => {
                        then.retain(|d| d.node < me);
                        otherwise.retain(|d| d.node < me);
                    }
                }
            }
            // An IfEven may now name a dep no Request asks for: drop such steps.
            let mut requested = BTreeSet::new();
            p.steps.retain(|s| match s {
                Step::Request(g) => {
                    requested.extend(g.iter().map(|d| d.node));
                    true
                }
                Step::IfEven { on, .. } => requested.contains(on),
            });
        }
        // Flip transience.
        _ => {
            if let Some(t) = &mut p.throws {
                t.transient = !t.transient;
            }
        }
    }
    p
}

fn program_uses(p: &Program, node: NodeId) -> bool {
    p.steps.iter().any(|s| match s {
        Step::Request(g) => g.iter().any(|d| d.node == node),
        Step::IfEven {
            then, otherwise, ..
        } => then.iter().chain(otherwise).any(|d| d.node == node),
    })
}

fn gen_edit(rng: &mut Rng, cfg: &GenConfig, spec: &GraphSpec) -> Edit {
    let n = spec.len();
    let leaves: Vec<NodeId> = spec.leaves().map(|(id, _)| id).collect();
    let functions: Vec<NodeId> = (0..n).filter(|i| spec.program(*i).is_some()).collect();
    loop {
        match rng.below(100) {
            0..30 => {
                return Edit::SetLeaf {
                    node: rng.pick(&leaves),
                    value: rng.below(4) as i64,
                };
            }
            30..55 if !functions.is_empty() => {
                let node = rng.pick(&functions);
                let program = mutate(rng, cfg, node, n, spec.program(node).unwrap());
                let mut s = spec.clone();
                s.nodes[node as usize].kind = NodeKind::Function(program.clone());
                if s.validate().is_ok() {
                    return Edit::SetProgram { node, program };
                }
            }
            55..70 => {
                return Edit::Invalidate {
                    node: rng.below(n as u64) as u32,
                };
            }
            70..80 => {
                let k = 1 + rng.below(2);
                return Edit::Delete {
                    nodes: (0..k).map(|_| rng.below(n as u64) as u32).collect(),
                };
            }
            80..88 => return Edit::DeleteDirty { age: rng.below(3) },
            88..100 => return Edit::InvalidateTransientErrors,
            _ => {}
        }
    }
}
