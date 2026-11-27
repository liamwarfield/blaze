//! The tests' `exec`: Java's tests evaluate a .bzl in a `BazelEvaluationTestCase` under a fake
//! label; here the file is written to a scratch workspace and loaded through Skyframe
//! (`BzlLoadValue`), as a `starlark_doc_extract` target loads it, and the doc comments are
//! computed the way the rule computes them.
#![allow(dead_code)]

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_protos::stardoc_output::ModuleInfo;
use blaze_sky_loading::bzl_load::{BzlLoadKey, BzlLoadValue};
use blaze_sky_top::testing::BuildViewTestCase;
use blaze_skyframe::SkyKey;
use blaze_starlark::syntax::java::{
    JavaOptions, PredeclaredModule, parse_file, program_doc_comments,
};
use blaze_starlarkdocextract::extraction_exception::ExtractionException;
use blaze_starlarkdocextract::label_renderer::LabelRenderer;
use blaze_starlarkdocextract::module_info_extractor::{DocumentedModule, ModuleInfoExtractor};

/// The label of the evaluated file (Java: the fake label of `BazelEvaluationTestCase`).
pub const FAKE_LABEL_STRING: &str = "//test:test.bzl";

/// A loaded module and the harness that keeps it alive.
pub struct TestModule {
    pub t: BuildViewTestCase,
    pub value: Arc<BzlLoadValue>,
    pub source: String,
}

/// Java's `exec(lines...)`.
pub fn exec(source: &str) -> TestModule {
    exec_with_options(&[], source)
}

/// Java's `execWithOptions(options, lines...)`: `options` are Starlark semantics flags.
pub fn exec_with_options(options: &[&str], source: &str) -> TestModule {
    let mut t = BuildViewTestCase::new();
    if !options.is_empty() {
        t.set_build_language_options(options);
    }
    t.scratch.file("test/BUILD", &[""]);
    t.scratch.file("test/test.bzl", &[source]);
    let key = BzlLoadKey::key_for_build(Label::parse_canonical(FAKE_LABEL_STRING).unwrap());
    let result = t.evaluate(&[SkyKey::new(key.clone())], false);
    let value = result.get(&key).unwrap_or_else(|| {
        panic!(
            "loading {FAKE_LABEL_STRING} failed: {:?}",
            result.error_map()
        )
    });
    TestModule {
        t,
        value,
        source: source.to_owned(),
    }
}

/// Java's `getExtractor()` (every name wanted, the default label renderer).
pub fn extract(module: &TestModule) -> Result<ModuleInfo, ExtractionException> {
    extract_with(module, &|_| true, LabelRenderer::default_renderer(), false)
}

/// `extractor.extractFrom(module)`.
pub fn extract_with(
    module: &TestModule,
    is_wanted: &dyn Fn(&str) -> bool,
    renderer: LabelRenderer,
    allow_unused_doc_comments: bool,
) -> Result<ModuleInfo, ExtractionException> {
    let mut extractor = ModuleInfoExtractor::new(is_wanted, renderer);
    if allow_unused_doc_comments {
        extractor = extractor.allow_unused_doc_comments();
    }
    let module_context = module.value.get_module_context();
    let parsed = parse_file(
        &module_context.filename,
        &module.source,
        JavaOptions::DEFAULT,
    );
    let guarded = |_: &str| None;
    let predeclared = PredeclaredModule::new(Vec::new(), &guarded);
    let doc_comments = program_doc_comments(&parsed, &predeclared, JavaOptions::DEFAULT);
    extractor.extract_from(
        &DocumentedModule {
            module: module.value.get_module(),
            filename: Some(module_context.filename.clone()),
            label: Some(module_context.label().clone()),
        },
        &doc_comments.doc_comments_map,
        &doc_comments.unused_doc_comment_lines,
    )
}
