//! Port of `lib/analysis/actions/BuildInfoFileWriteAction.java` (analysis-time parts).

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use blaze_starlark::StarlarkSemantics;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::eval::{EvalOptions, frozen_into, with_evaluator};
use blaze_starlark::thread::{Phase, StarlarkEvents, ThreadContext};
use blaze_starlark::{EvalError, Location};
use starlark::values::dict::{AllocDict, DictRef};

use blaze_collect::{NestedSetBuilder, Order};
use blaze_util::{Fingerprint, check_argument};
use starlark::values::OwnedFrozenValue;

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_lookup::ArtifactOwner;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::analysis::workspace_status_action::BUILD_INFO_FUNCTION_NAME;

// Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:65 BuildInfoFileWriteAction.GUID
const GUID: &str = "7e4657a6-dd09-435e-9423-51d4846aad4a";

/// Translates workspace status text files (`ctx.info_file` and `ctx.version_file`) to a language
/// consumable file and writes its contents to output. Keys and values are translated by the
/// callback translation_func Starlark method, and the output file format is generated according
/// to the template.
///
/// Action takes text file as an input and transforms it with a user provided Starlark callback
/// function to a dictionary of strings to strings. This dictionary is then used as substitutions
/// to expand the user provided template file.
// Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:63 BuildInfoFileWriteAction
pub struct BuildInfoFileWriteAction {
    base: AbstractAction,
    /// The frozen `StarlarkFunction` (only called at execution time).
    translation_callback: OwnedFrozenValue,
    template: Artifact,
    is_volatile: bool,
    /// `BazelModuleContext.of(translationCallback.getModule()).bzlTransitiveDigest()`.
    bzl_transitive_digest: Vec<u8>,
    /// The semantics of the thread that created the action (the callback's thread uses them).
    semantics: Arc<StarlarkSemantics>,
}

/// Why `execute` could not translate the status values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslateError {
    /// The callback failed: `EvalException.toString()` (its message with the call stack).
    Callback(String),
    /// The callback's value is not a dict of strings to strings: the `Dict.cast` message.
    NotStringDict(String),
    /// The thread was cancelled.
    Interrupted,
}

impl BuildInfoFileWriteAction {
    /// PORT: Java takes the `StarlarkFunction` and a `StarlarkSemantics` (used only by
    /// `execute`, wave W6) and reads the defining module's `.bzl` transitive digest from the
    /// function at key time. Here the analysis lead passes `bzl_transitive_digest`: the
    /// `bzlTransitiveDigest` of the `BazelModuleContext` of the module that defines
    /// `translation_callback`.
    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:72 BuildInfoFileWriteAction.<init>
    pub fn new(
        owner: ActionOwner,
        input: Artifact,
        output: Artifact,
        translation_callback: OwnedFrozenValue,
        template: Artifact,
        is_volatile: bool,
        bzl_transitive_digest: Vec<u8>,
        semantics: Arc<StarlarkSemantics>,
    ) -> BuildInfoFileWriteAction {
        let base = AbstractAction::new(
            owner,
            NestedSetBuilder::create(Order::STABLE_ORDER, [input.clone(), template.clone()]),
            [output],
        );
        // Java: `input.getArtifactOwner() instanceof BuildInfoKey`.
        // PORT: `BuildInfoKey` is the only `ActionLookupKey` whose function is BUILD_INFO.
        let owned_by_build_info_key = matches!(
            input.get_artifact_owner(),
            ArtifactOwner::ActionLookupKey(k) if k.function_name() == BUILD_INFO_FUNCTION_NAME
        );
        check_argument!(
            owned_by_build_info_key,
            "input artifact of BuildInfoFileWriteAction must be one of workspace status artifacts: ctx.info_file or ctx.version_file"
        );
        BuildInfoFileWriteAction {
            base,
            translation_callback,
            template,
            is_volatile,
            bzl_transitive_digest,
            semantics,
        }
    }

    /// The template artifact.
    pub fn get_template(&self) -> &Artifact {
        &self.template
    }

    /// The Starlark translation callback.
    pub fn get_translation_callback(&self) -> &OwnedFrozenValue {
        &self.translation_callback
    }

    /// The part of `execute` that calls the Starlark callback: `values` (in the iteration order
    /// of Java's `HashMap`) become the dict passed positionally to the translation callback on a
    /// new thread with the action's semantics; the result is cast to a dict of strings to strings
    /// and returned in its iteration order.
    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:97 BuildInfoFileWriteAction.execute
    pub fn call_translation_callback(
        &self,
        values: &[(String, String)],
    ) -> Result<Vec<(String, String)>, TranslateError> {
        // Since the result of this thread is a String to String Dict, it should not result in
        // any reference-equals objects.
        // PORT: Java names the thread "transform_version_file callback" /
        // "transform_info_file callback"; the name appears nowhere in its output.
        let ctx = ThreadContext::new(Phase::Other, self.semantics.clone(), Arc::new(StderrPrint));
        let outcome = with_evaluator(&ctx, None, &EvalOptions::default(), |module, eval| {
            let f = frozen_into(module, &self.translation_callback);
            // Dict.immutableCopyOf(values)
            let dict = eval.heap().alloc(AllocDict(
                values.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            ));
            let ret = eval
                .eval_function(f, &[dict], &[])
                .map_err(|e| EvalError::from_starlark(&e))?;
            // Dict.cast(substitutionDictObject, String.class, String.class, "substitution_dict")
            let Some(d) = DictRef::from_value(ret) else {
                return Ok(Err(format!(
                    "got {} for 'substitution_dict', want dict",
                    ret.get_type()
                )));
            };
            let mut out = Vec::with_capacity(d.len());
            for (k, v) in d.iter() {
                match (k.unpack_str(), v.unpack_str()) {
                    (Some(k), Some(v)) => out.push((k.to_string(), v.to_string())),
                    _ => {
                        return Ok(Err(format!(
                            "got dict<{}, {}> for 'substitution_dict', want dict<string, string>",
                            k.get_type(),
                            v.get_type()
                        )));
                    }
                }
            }
            Ok(Ok(out))
        });
        match outcome {
            Ok(Ok(out)) => Ok(out),
            Ok(Err(message)) => Err(TranslateError::NotStringDict(message)),
            Err(e) if e.kind == EvalErrorKind::Cancelled => Err(TranslateError::Interrupted),
            Err(e) => Err(TranslateError::Callback(e.message_with_stack())),
        }
    }

    /// Whether this action translates the volatile status file.
    pub fn is_volatile_status(&self) -> bool {
        self.is_volatile
    }
}

/// The default print handler of a `StarlarkThread` (`StarlarkThread.create` sets none): `print`
/// writes `<caller location>: <message>` to the server's standard error.
// Java: net/starlark/java/eval/StarlarkThread.java:366 StarlarkThread.defaultPrintHandler
struct StderrPrint;

impl StarlarkEvents for StderrPrint {
    fn debug(&self, location: Option<&Location>, message: &str) {
        match location {
            Some(location) => eprintln!("{location}: {message}"),
            None => eprintln!("<builtin>: {message}"),
        }
    }

    fn warn(&self, location: Option<&Location>, message: &str) {
        self.debug(location, message);
    }
}

impl fmt::Debug for BuildInfoFileWriteAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuildInfoFileWriteAction")
            .field("base", &self.base)
            .field("template", &self.template)
            .field("is_volatile", &self.is_volatile)
            .finish_non_exhaustive()
    }
}

impl Action for BuildInfoFileWriteAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:200 BuildInfoFileWriteAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "TranslateBuildInfo".to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:189 BuildInfoFileWriteAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        fp.add_boolean(self.is_volatile);
        // Add Starlark function to the fingerprint.
        fp.add_bytes(&self.bzl_transitive_digest);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:205 BuildInfoFileWriteAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        if self.is_volatile {
            Some("Translating volatile BuildInfo file".to_string())
        } else {
            Some("Translating stable BuildInfo file".to_string())
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:214 BuildInfoFileWriteAction.executeUnconditionally
    fn execute_unconditionally(&self) -> bool {
        self.is_volatile
    }

    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:219 BuildInfoFileWriteAction.isVolatile
    fn is_volatile(&self) -> bool {
        self.is_volatile
    }
}
