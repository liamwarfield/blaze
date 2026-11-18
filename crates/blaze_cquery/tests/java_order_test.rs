//! Oracle-driven tests of the order of post-analysis query results (integrity audit S1-1,
//! analysis review S2-5).
//!
//! Bazel's `cquery "deps(...)"` prints its result in the iteration order of Java hash collections
//! keyed by `ConfiguredTargetKey` (see `blaze_cquery::post_analysis_query_environment`). The
//! fixtures in `tests/testdata/*.fixture` were extracted from the oracle (Bazel 9.3.2, a machine
//! with 16 available processors): the keys and their direct deps come from
//! `bazel dump --skyframe=deps`, the expected order (`E`) is the output of
//! `cquery "deps(...)" --output=label_kind`. Each test replays `DepsFunction.eval` and
//! `PostAnalysisQueryEnvironment.getFwdDeps` over the fixture with the crate's order functions
//! (`configured_target_key_java_hash_code`, `targets_by_key_order`,
//! `delegating_walkable_graph_map_order`, `java_hash_map_key_order`) and the engine's
//! `JavaConcurrentHashMap`, and compares with the order the oracle printed.
//!
//! What the replay does not cover: `targetifyValues` (the fixtures hold its result) and the
//! glue of the environment; `scripts/w4/scenarios/query-order` covers them end to end.

use std::collections::HashSet;

use blaze_cmdline::Label;
use blaze_cquery::post_analysis_query_environment::{
    configured_target_key_java_hash_code, delegating_walkable_graph_map_order,
    java_hash_map_key_order, targets_by_key_order,
};
use blaze_query_engine::java_concurrent_hash_map::{JavaConcurrentHashMap, JavaHashCode};

/// `Runtime.getRuntime().availableProcessors()` of the machine the oracle ran on
/// (`SkyQueryEnvironment.DEFAULT_THREAD_COUNT`).
const ORACLE_AVAILABLE_PROCESSORS: i32 = 16;

struct Fixture {
    /// Java `ConfiguredTargetKey.hashCode()` by node id.
    hash_codes: Vec<i32>,
    /// What `targetifyValues` yields for the node's direct deps, in order, by node id.
    deps: Vec<Vec<usize>>,
    /// The result of the query argument, in order.
    start: Vec<usize>,
    /// The order the oracle printed.
    expected: Vec<usize>,
    /// The order the oracle printed with its server restricted to 8 processors (`E8`), if
    /// recorded.
    expected_with_8_processors: Option<Vec<usize>>,
}

fn parse_ids(field: Option<&str>) -> Vec<usize> {
    field
        .unwrap_or("")
        .split(' ')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("id"))
        .collect()
}

fn parse_label(raw: &str) -> Label {
    Label::parse_canonical(raw).unwrap_or_else(|e| panic!("label {raw}: {e:?}"))
}

fn load(content: &str) -> Fixture {
    let mut checksums: Vec<String> = Vec::new();
    let mut hash_codes: Vec<Option<i32>> = Vec::new();
    let mut deps: Vec<Option<Vec<usize>>> = Vec::new();
    let mut start: Vec<usize> = Vec::new();
    let mut expected: Vec<usize> = Vec::new();
    let mut expected_with_8_processors: Option<Vec<usize>> = None;
    for line in content.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        match fields.next().expect("record type") {
            "C" => {
                let index: usize = fields.next().expect("index").parse().expect("index");
                assert_eq!(index, checksums.len());
                checksums.push(fields.next().expect("checksum").to_owned());
            }
            "N" => {
                let id: usize = fields.next().expect("id").parse().expect("id");
                let configuration = fields.next().expect("configuration");
                let should_apply_rule_transition = fields.next().expect("flag") == "1";
                let execution_platform_label = fields.next().expect("execution platform");
                let label = parse_label(fields.next().expect("label"));
                let checksum: Option<&str> = match configuration {
                    "-" => None,
                    index => Some(checksums[index.parse::<usize>().expect("index")].as_str()),
                };
                let execution_platform_label: Option<Label> = match execution_platform_label {
                    "-" => None,
                    raw => Some(parse_label(raw)),
                };
                if hash_codes.len() <= id {
                    hash_codes.resize(id + 1, None);
                }
                hash_codes[id] = Some(configured_target_key_java_hash_code(
                    &label,
                    checksum,
                    execution_platform_label.as_ref(),
                    should_apply_rule_transition,
                ));
            }
            "D" => {
                let id: usize = fields.next().expect("id").parse().expect("id");
                if deps.len() <= id {
                    deps.resize(id + 1, None);
                }
                deps[id] = Some(parse_ids(fields.next()));
            }
            "S" => start = parse_ids(fields.next()),
            "E" => expected = parse_ids(fields.next()),
            "E8" => expected_with_8_processors = Some(parse_ids(fields.next())),
            other => panic!("unknown record type {other:?}"),
        }
    }
    assert!(!start.is_empty() && !expected.is_empty());
    Fixture {
        hash_codes: hash_codes
            .into_iter()
            .map(|h| h.expect("N record"))
            .collect(),
        deps: deps.into_iter().map(|d| d.expect("D record")).collect(),
        start,
        expected,
        expected_with_8_processors,
    }
}

/// A node of the fixture as the key of a query set.
#[derive(Clone, Copy, PartialEq, Eq)]
struct NodeKey {
    id: usize,
    hash_code: i32,
}

impl JavaHashCode for NodeKey {
    fn java_hash_code(&self) -> i32 {
        self.hash_code
    }
}

impl Fixture {
    fn key(&self, id: usize) -> NodeKey {
        NodeKey {
            id,
            hash_code: self.hash_codes[id],
        }
    }

    /// `createThreadSafeMutableSet()`: a `ThreadSafeMutableKeyExtractorBackedSetImpl` over
    /// `new ConcurrentHashMap<>(concurrencyLevel, 0.75f)`.
    fn create_thread_safe_mutable_set(processors: i32) -> JavaConcurrentHashMap<NodeKey, ()> {
        JavaConcurrentHashMap::with_capacity(processors, 0.75, 1)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:287 PostAnalysisQueryEnvironment.getFwdDeps
    fn get_fwd_deps(&self, targets: &[usize], processors: i32) -> Vec<usize> {
        let hash_code = |id: &usize| self.hash_codes[*id];
        // Map<SkyKey, T> targetsByKey = Maps.newHashMapWithExpectedSize(Iterables.size(targets));
        let targets_by_key = targets_by_key_order(targets.len(), targets.to_vec(), &hash_code);
        // graph.getDirectDeps(targetsByKey.keySet())
        let direct_deps_of_keys = delegating_walkable_graph_map_order(
            targets_by_key.into_iter().map(|k| (k, ())).collect(),
            &hash_code,
        );
        // targetifyValues(targetsByKey, ...): `new HashMap<>()` filled in that order.
        let direct_deps = java_hash_map_key_order(
            16,
            direct_deps_of_keys.into_iter().map(|(k, ())| k).collect(),
            &hash_code,
        );
        let mut result = Self::create_thread_safe_mutable_set(processors);
        for key in direct_deps {
            for dep in &self.deps[key] {
                result.put_if_absent(self.key(*dep), ());
            }
        }
        result.keys().map(|k| k.id).collect()
    }

    // Java: com/google/devtools/build/lib/query2/engine/DepsFunction.java:59 DepsFunction.eval
    fn deps(&self, processors: i32) -> Vec<usize> {
        let mut output: Vec<usize> = Vec::new();
        // MinDepthUniquifier (the depth is unbounded here: an element is new exactly once).
        let mut seen: HashSet<usize> = HashSet::new();
        let mut current = Self::create_thread_safe_mutable_set(processors);
        for id in &self.start {
            current.put_if_absent(self.key(*id), ());
        }
        loop {
            let to_process: Vec<usize> = current
                .keys()
                .map(|k| k.id)
                .filter(|id| seen.insert(*id))
                .collect();
            output.extend(to_process.iter().copied());
            current = Self::create_thread_safe_mutable_set(processors);
            for id in self.get_fwd_deps(&to_process, processors) {
                current.put_if_absent(self.key(id), ());
            }
            if current.is_empty() {
                // Exit when there are no more nodes to visit.
                break;
            }
        }
        output
    }
}

fn assert_oracle_order(content: &str, nodes: usize) {
    let fixture = load(content);
    assert_eq!(fixture.expected.len(), nodes);
    let got = fixture.deps(ORACLE_AVAILABLE_PROCESSORS);
    assert_eq!(got.len(), fixture.expected.len());
    for (position, (got, expected)) in got.iter().zip(&fixture.expected).enumerate() {
        assert_eq!(
            got, expected,
            "result {position} differs from the oracle's output"
        );
    }
}

// Oracle: corpus project brotli, `cquery "deps(//...)" --output=label_kind` (176 lines, equal to
// oracle-records/oracle-9.3.2/brotli/raw/cquery_deps_label_kind.stdout).
#[test]
fn cquery_deps_order_of_brotli_matches_the_oracle() {
    assert_oracle_order(include_str!("testdata/brotli_deps.fixture"), 176);
}

// Oracle: corpus project re2, `cquery "deps(//:all)" --output=label_kind` (820 lines: four
// configurations, toolchain dependency keys with an execution platform label).
#[test]
fn cquery_deps_order_of_re2_matches_the_oracle() {
    assert_oracle_order(include_str!("testdata/re2_all_deps.fixture"), 820);
}

// Oracle: scripts/w4/scenarios/query-order, step 1 (`cquery "deps(//...)" --output=label_kind`).
#[test]
fn cquery_deps_order_of_the_query_order_scenario_matches_the_oracle() {
    assert_oracle_order(include_str!("testdata/query_order_deps.fixture"), 39);
}

// SkyQueryEnvironment.DEFAULT_THREAD_COUNT is Runtime.availableProcessors(): the order Bazel
// prints depends on it. Oracle: the same brotli query with the Bazel server restricted to 8
// processors (`taskset -c 0-7`); its output differs from the 16-processor one in the order only.
#[test]
fn cquery_deps_order_depends_on_the_available_processors() {
    let fixture = load(include_str!("testdata/brotli_deps.fixture"));
    let expected = fixture
        .expected_with_8_processors
        .as_ref()
        .expect("E8 record");
    assert_ne!(*expected, fixture.expected);
    assert_eq!(fixture.deps(8), *expected);
}
