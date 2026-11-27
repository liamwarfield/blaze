//! A scratch workspace + output base + fake install base and registry, and an evaluator over
//! them (the Rust stand-in for Java's `BuildViewTestCase` / `FoundationTestCase` setup used by
//! the repository and module extension tests).

#![allow(dead_code)]

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use blaze_bzlmod::skyframe::after_command;
use blaze_cmdline::RepositoryName;
use blaze_events::Event;
use blaze_events::EventHandler;
use blaze_events::ExtendedEventHandler;
use blaze_events::Postable;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_options::OptionsClass;
use blaze_options::OptionsParser;
use blaze_options::PriorityCategory;
use blaze_repo_rules::registration::RepoPrecomputedValues;
use blaze_repo_rules::setup::FetchEvaluator;
use blaze_repo_rules::setup::FetchSetup;
use blaze_repo_rules::setup::create_evaluator;
use blaze_sky_loading::repos::RepositoryDirectoryKey;
use blaze_sky_loading::repos::RepositoryDirectoryValue;
use blaze_skyframe::EvaluationContext;
use blaze_skyframe::SkyKey;
use blaze_starlark::StarlarkSemantics;
use blaze_util::string_encoding::platform_to_internal;
use indexmap::IndexMap;

/// Records events (`<kind>: <message>`) and posts (`POST: <Debug form>`), in the order the
/// handler receives them.
#[derive(Default)]
pub struct Events(pub Mutex<Vec<String>>);

impl EventHandler for Events {
    fn handle(&self, event: &Event) {
        self.0
            .lock()
            .unwrap()
            .push(format!("{:?}: {}", event.get_kind(), event.get_message()));
    }
    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for Events {
    fn post(&self, obj: Arc<dyn Postable>) {
        self.0.lock().unwrap().push(format!("POST: {:?}", obj));
    }
}

/// The test's directories.
pub struct Scratch {
    pub _tmp: tempfile::TempDir,
    pub workspace: PathBuf,
    pub output_base: PathBuf,
    pub install_base: PathBuf,
    pub registry: PathBuf,
    pub events: Arc<Events>,
}

impl Scratch {
    pub fn new() -> Scratch {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let workspace = root.join("ws");
        let output_base = root.join("ob");
        let install_base = root.join("install");
        let registry = root.join("registry");
        for d in [
            &workspace,
            &output_base,
            &install_base.join("embedded_tools"),
            &registry,
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            install_base.join("embedded_tools/MODULE.bazel"),
            "module(name = \"bazel_tools\")\n",
        )
        .unwrap();
        std::fs::write(install_base.join("embedded_tools/REPO.bazel"), "").unwrap();
        std::fs::write(registry.join("bazel_registry.json"), "{}").unwrap();
        Scratch {
            _tmp: tmp,
            workspace,
            output_base,
            install_base,
            registry,
            events: Arc::new(Events::default()),
        }
    }

    /// Writes a workspace file.
    pub fn file(&self, rel: &str, content: &str) {
        let p = self.workspace.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    pub fn evaluator(&self, extra_options: &[&str]) -> FetchEvaluator {
        self.evaluator_with_precomputed(extra_options, RepoPrecomputedValues::default())
    }

    /// The client environment of [`Scratch::evaluator`]: `PATH` and `HOME`, as internal strings.
    pub fn default_env() -> IndexMap<String, String> {
        [
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
            (
                "HOME".to_owned(),
                std::env::var_os("HOME")
                    .map(|h| platform_to_internal(&h).into_owned())
                    .unwrap_or_default(),
            ),
        ]
        .into_iter()
        .collect()
    }

    /// [`Scratch::evaluator`] with the given precomputed values (`--nofetch`, ...).
    pub fn evaluator_with_precomputed(
        &self,
        extra_options: &[&str],
        precomputed: RepoPrecomputedValues,
    ) -> FetchEvaluator {
        self.evaluator_with_env(extra_options, precomputed, Self::default_env())
    }

    /// [`Scratch::evaluator_with_precomputed`] with the given client (and repo) environment,
    /// whose keys and values are internal strings.
    pub fn evaluator_with_env(
        &self,
        extra_options: &[&str],
        precomputed: RepoPrecomputedValues,
        env: IndexMap<String, String>,
    ) -> FetchEvaluator {
        self.evaluator_with_semantics(
            extra_options,
            precomputed,
            env,
            StarlarkSemantics::default(),
        )
    }

    /// [`Scratch::evaluator_with_env`] with the given Starlark semantics.
    pub fn evaluator_with_semantics(
        &self,
        extra_options: &[&str],
        precomputed: RepoPrecomputedValues,
        env: IndexMap<String, String>,
        starlark_semantics: StarlarkSemantics,
    ) -> FetchEvaluator {
        let mut parser = OptionsParser::builder()
            .options_classes(&[RepositoryOptions::class()])
            .build()
            .unwrap_or_else(|e| e.throw());
        let mut args: Vec<String> = vec![
            format!("--registry=file://{}", self.registry.display()),
            "--repository_cache=".to_owned(),
        ];
        args.extend(extra_options.iter().map(|s| s.to_string()));
        parser
            .parse_with_priority(PriorityCategory::COMMAND_LINE, None, args)
            .unwrap();
        let repo_options: RepositoryOptions = parser.get_options().unwrap();
        create_evaluator(FetchSetup {
            workspace: &self.workspace,
            working_directory: &self.workspace,
            output_base: &self.output_base,
            install_base: &self.install_base,
            repo_options: &repo_options,
            starlark_semantics,
            client_env: env.clone(),
            repo_env: env.clone(),
            nonstrict_repo_env: env,
            precomputed,
            reporter: self.events.clone(),
        })
        .unwrap()
    }

    /// Fetches `@@name` and returns its directory, or the error / failure message.
    pub fn fetch(&self, ev: &FetchEvaluator, name: &str) -> Result<PathBuf, String> {
        let key = SkyKey::new(RepositoryDirectoryKey::key(
            RepositoryName::create_unvalidated(name),
        ));
        let ctx = EvaluationContext {
            keep_going: true,
            event_handler: self.events.clone(),
            ..EvaluationContext::default()
        };
        let result = ev
            .evaluator
            .evaluate(std::slice::from_ref(&key), &ctx)
            .unwrap();
        if let Some(e) = result.get_error(&key) {
            return Err(e
                .get_exception()
                .map(|x| x.to_string())
                .unwrap_or_else(|| format!("{e:?}")));
        }
        let v = result.get_dyn(&key).expect("value");
        match v.downcast_ref::<RepositoryDirectoryValue>().unwrap() {
            RepositoryDirectoryValue::Success { .. } => {
                Ok(self.output_base.join("external").join(name))
            }
            RepositoryDirectoryValue::Failure { error_msg } => Err(error_msg.clone()),
        }
    }

    pub fn write_lockfile(&self, ev: &FetchEvaluator) {
        after_command(
            &ev.evaluator,
            &self.workspace,
            &self.output_base,
            ev.lockfile_mode,
        );
    }

    pub fn read(&self, p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    pub fn events(&self) -> Vec<String> {
        self.events.0.lock().unwrap().clone()
    }
}

/// Runs `f` on a thread with the evaluator's stack size.
pub fn run<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    blaze_starlark::eval::run_on_evaluation_thread(f)
}
