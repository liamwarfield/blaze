//! Review loading-2 round 2, R2: every typed parameter of every builtin Blaze registers in its
//! Starlark environments has Java's type error text in the vendored crate's table
//! (`third_party/starlark-rust/starlark/src/values/java_param_types.tsv`, printed by Bazel's own
//! `MethodDescriptor.of` / `ParamDescriptor.getTypeErrorMessage`, `scripts/java-param-types/run`).
//! A parameter without a row would get starlark-rust's derived text (patch 0052's
//! `java_allowed_types`), which is a guess. The walk covers the BUILD, .bzl, MODULE.bazel,
//! builtins-.bzl and .scl environments of the Bazel environment the server builds
//! (`skyframe_executor.rs`), the `native` module, and every namespace and method reachable from
//! them by attribute access. Values created only during analysis or fetching (`ctx`,
//! `repository_ctx`, ...) are not reachable here.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;

use starlark::docs::DocItem;
use starlark::docs::DocMember;
use starlark::environment::Module;
use starlark::values::Value;

/// Function or method name, then parameter: why the table has no row for it.
const ALLOWED_MISSING: &[(&str, &str, &str)] = &[];

/// The table, (method, param) -> [(receiver, want)].
fn table() -> HashMap<(String, String), Vec<(String, String)>> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../third_party/starlark-rust/starlark/src/values/java_param_types.tsv"
    );
    let mut m: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if let [receiver, method, param, want] = cols[..] {
            m.entry((method.to_owned(), param.to_owned()))
                .or_default()
                .push((receiver.to_owned(), want.to_owned()));
        }
    }
    m
}

/// The name a builtin's errors use: `<built-in function NAME>`, `<built-in method NAME of T
/// value>` (patch 0009's repr).
fn builtin_name(v: Value) -> Option<String> {
    let r = v.to_repr();
    if let Some(rest) = r.strip_prefix("<built-in function ") {
        return rest.strip_suffix('>').map(str::to_owned);
    }
    if let Some(rest) = r.strip_prefix("<built-in method ") {
        return rest.split(' ').next().map(str::to_owned);
    }
    None
}

#[derive(Default)]
struct Walk {
    /// (receiver or "", name, param, typ) of every typed parameter seen.
    params: BTreeSet<(String, String, String, String)>,
    /// (receiver or "", name) of every builtin seen, and its parameter count.
    builtins: BTreeMap<(String, String), usize>,
    seen: BTreeSet<String>,
}

impl Walk {
    fn value<'v>(
        &mut self,
        v: Value<'v>,
        path: &str,
        receiver: Option<&str>,
        depth: usize,
        heap: starlark::values::Heap<'v>,
    ) {
        if let Some(name) = builtin_name(v) {
            if let DocItem::Member(DocMember::Function(f)) = v.documentation() {
                let p = &f.params;
                self.builtins.insert(
                    (receiver.unwrap_or("").to_owned(), name.clone()),
                    p.pos_only.len() + p.pos_or_named.len() + p.named_only.len(),
                );
                for param in p
                    .pos_only
                    .iter()
                    .chain(&p.pos_or_named)
                    .chain(&p.named_only)
                {
                    let typ = param.typ.to_string();
                    if typ == "typing.Any" {
                        continue;
                    }
                    self.params.insert((
                        receiver.unwrap_or("").to_owned(),
                        name.clone(),
                        param.name.clone(),
                        typ,
                    ));
                }
            }
            return;
        }
        if depth > 3 {
            return;
        }
        let ty = v.get_type().to_owned();
        if !self.seen.insert(format!("{ty}:{path}")) {
            return;
        }
        for attr in v.dir_attr() {
            // Some attributes exist only for documentation and panic when read (Java's
            // `UnsupportedOperationException` stubs); they have no parameters to check.
            let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                v.get_attr(&attr, heap).ok().flatten()
            }));
            let Ok(Some(a)) = got else {
                continue;
            };
            // A method is called on a value of type `ty`; a struct's field is a function.
            let is_method = a.to_repr().starts_with("<built-in method ");
            self.value(
                a,
                &format!("{path}.{attr}"),
                if is_method { Some(&ty) } else { None },
                depth + 1,
                heap,
            );
        }
    }
}

#[test]
fn every_registered_builtin_parameter_has_javas_type_text() {
    let rule_class_provider = blaze_packages::native_rules::RuleClassProvider::create_bazel();
    let env = blaze_rules::bazel_environment::bazel_starlark_environment_with(
        &rule_class_provider,
        &blaze_repo_rules::registration::add_repository_module_toplevels,
    );
    let table = table();
    let mut walk = Walk::default();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    Module::with_temp_heap(|module| {
        let heap = module.heap();
        for (what, e) in [
            ("bzl", env.get_uninjected_build_bzl_env()),
            ("BUILD", env.get_uninjected_build_env()),
            ("MODULE", env.get_uninjected_module_bzl_env()),
            ("builtins", env.get_builtins_bzl_env()),
            ("scl", env.get_scl_toplevels()),
            ("native", env.get_uninjected_build_bzl_native_bindings()),
        ] {
            for (name, entry) in e {
                let v = blaze_starlark::eval::frozen_into(&module, &entry.value);
                walk.value(v, &format!("{what}:{name}"), None, 0, heap);
            }
        }
    });
    std::panic::set_hook(hook);
    let allowed: BTreeMap<(&str, &str), &str> = ALLOWED_MISSING
        .iter()
        .map(|(f, p, why)| ((*f, *p), *why))
        .collect();
    let mut missing = Vec::new();
    let mut ambiguous = Vec::new();
    let mut covered = 0;
    for (receiver, name, param, typ) in &walk.params {
        match table.get(&(name.clone(), param.clone())) {
            None => {
                if !allowed.contains_key(&(name.as_str(), param.as_str())) {
                    missing.push(format!("{receiver}\t{name}\t{param}\t(Blaze type {typ})"));
                }
            }
            Some(rows) => {
                // As `java_param_type_error_message`: the receiver's row (a function's is ""),
                // else the text every row agrees on.
                let own = rows.iter().find(|(r, _)| r == receiver);
                if own.is_none() && rows.iter().any(|(_, w)| *w != rows[0].1) {
                    ambiguous.push(format!("{receiver}\t{name}\t{param}\t{rows:?}"));
                } else {
                    covered += 1;
                }
            }
        }
    }
    let stale: Vec<_> = allowed
        .keys()
        .filter(|(f, p)| !walk.params.iter().any(|(_, n, q, _)| n == f && q == p))
        .collect();
    eprintln!(
        "{} builtins with {} parameters reached; {} typed parameters: {covered} with Java's text, \
         {} missing, {} ambiguous, {} allowed",
        walk.builtins.len(),
        walk.builtins.values().sum::<usize>(),
        walk.params.len(),
        missing.len(),
        ambiguous.len(),
        allowed.len()
    );
    assert!(
        missing.is_empty() && ambiguous.is_empty() && stale.is_empty(),
        "parameters without Java's type text (add a row by regenerating the table, or an \
         ALLOWED_MISSING entry with the reason):\n{}\nambiguous (no row for the receiver, rows \
         disagree):\n{}\nallowed but not registered: {stale:?}",
        missing.join("\n"),
        ambiguous.join("\n")
    );
}
