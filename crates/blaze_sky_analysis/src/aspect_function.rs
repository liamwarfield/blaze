//! `skyframe/AspectFunction.java` (agent C).
//!
//! Not ported: neither corpus project attaches aspects through attributes it analyzes
//! (no `aspects = [...]` on the rules_cc / rules_python / rules_shell attributes reached by
//! `//...`, and `--exec_aspects` / `--aspects` are unset), and neither project's aquery output
//! has aspect-owned actions. `DependencyResolutionHelpers` and the producers keep their aspect
//! parameters and call `unported!` where aspects would change the result.

/// Java `AspectFunction.compute`.
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:247 AspectFunction.compute
pub fn compute() -> ! {
    blaze_util::unported!(
        "AspectFunction.compute",
        "aspects are not ported (no aspect-owned actions in the corpus projects)"
    )
}
