//! Where the runtime starts and stops the profiler: `BlazeRuntime.initProfiler`,
//! `BlazeRuntime.manageProfiles`, the `ProfilerStartedEvent` and the local
//! `InstrumentationOutput` the profile is written to (decisions.md 2026-11-19, profiling ruling
//! (3): Bazel writes `command.profile.gz` on every command).

use std::io;
use std::sync::Arc;

use blaze_events::Clock as _;
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_options::OptionsParser;
use blaze_options::rc::handler::{HandlerEvent, StoredEventHandler};
use blaze_util::profiler::{Format, ProfilePhase, Profiler, ProfilerTask, ProfilerTaskSet};
use blaze_vfs::{Path, Symlinks};
use parking_lot::Mutex;

use crate::collect_local_resource_usage::CollectLocalResourceUsage;
use crate::command_environment::CommandEnvironment;

/// The profiler's clock: Blaze's `BlazeClock` (`blaze_events::JavaClock`), whose `nanoTime`
/// origin the rest of Blaze uses (`blaze_events::ui::nano_time_of`).
// Java: com/google/devtools/build/lib/clock/JavaClock.java:22 JavaClock
pub struct ProfilerClock;

impl blaze_util::profiler::Clock for ProfilerClock {
    fn current_time_millis(&self) -> i64 {
        blaze_events::JavaClock.current_time_millis()
    }

    fn nano_time(&self) -> i64 {
        blaze_events::JavaClock.nano_time()
    }
}

/// Used when instrumentation output is written to a local file.
// Java: com/google/devtools/build/lib/runtime/LocalInstrumentationOutput.java:27 LocalInstrumentationOutput
pub struct LocalInstrumentationOutput {
    path: Path,
    name: String,
    convenience_name: Option<String>,
    append: Option<bool>,
    internal: Option<bool>,
    create_parent: bool,
}

impl LocalInstrumentationOutput {
    // Java: com/google/devtools/build/lib/runtime/LocalInstrumentationOutput.java:35 LocalInstrumentationOutput.<init>
    pub fn new(
        name: impl Into<String>,
        path: Path,
        convenience_name: Option<String>,
        append: Option<bool>,
        internal: Option<bool>,
        create_parent: bool,
    ) -> Self {
        LocalInstrumentationOutput {
            path,
            name: name.into(),
            convenience_name,
            append,
            internal,
            create_parent,
        }
    }

    /// `publish(buildToolLogCollection)`.
    // Java: com/google/devtools/build/lib/runtime/LocalInstrumentationOutput.java:51 LocalInstrumentationOutput.publish
    pub fn publish(
        &self,
        build_tool_log_collection: &mut blaze_buildtool::buildtool::build_result::BuildToolLogCollection,
    ) {
        build_tool_log_collection.add_local_file(&self.name, self.path.clone());
    }

    // Java: com/google/devtools/build/lib/runtime/LocalInstrumentationOutput.java:55 LocalInstrumentationOutput.makeConvenienceLink
    pub fn make_convenience_link(&self) -> io::Result<()> {
        if let Some(convenience_name) = &self.convenience_name {
            let link = self
                .path
                .get_parent_directory()
                .expect("parent directory")
                .get_child(convenience_name);
            link.delete()?;
            link.create_symbolic_link_to_fragment(&blaze_vfs::PathFragment::create(
                self.path.get_base_name(),
            ))?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/LocalInstrumentationOutput.java:64 LocalInstrumentationOutput.createOutputStream
    pub fn create_output_stream(&self) -> io::Result<Box<dyn io::Write + Send>> {
        if self.create_parent {
            self.path
                .get_parent_directory()
                .expect("parent directory")
                .create_directory_and_parents()?;
        }
        if let (Some(append), Some(internal)) = (self.append, self.internal) {
            return self.path.get_output_stream_with(append, internal);
        }
        if let Some(append) = self.append {
            return self.path.get_output_stream_with(append, false);
        }
        self.path.get_output_stream()
    }

    // Java: com/google/devtools/build/lib/runtime/LocalInstrumentationOutput.java:78 LocalInstrumentationOutput.getPathString
    pub fn get_path_string(&self) -> &str {
        self.path.get_path_string()
    }
}

// The callers build each output with `LocalInstrumentationOutput::new` (all six fields; `name`
// and `path` are the parameters Builder.build checks for null). Bazel's InstrumentationOutputFactory
// holds only the local builder, the BEP artifact builder (unported, behind
// --experimental_stream_log_file_uploads) and no redirect builder, so the factory's methods are
// inlined where they are called (see `init_profiler` and the command log in bazel_modules.rs).
// PORT: skipped, see above: LocalInstrumentationOutput.Builder.setName, LocalInstrumentationOutput.Builder.setPath, LocalInstrumentationOutput.Builder.setConvenienceName, LocalInstrumentationOutput.Builder.setAppend, LocalInstrumentationOutput.Builder.setInternal, LocalInstrumentationOutput.Builder.setCreateParent, LocalInstrumentationOutput.Builder.build
// PORT: skipped, see above: InstrumentationOutputBuilder.setName, InstrumentationOutputBuilder.setDestination, InstrumentationOutputBuilder.setDestinationRelatedToType, InstrumentationOutputBuilder.setOptions, InstrumentationOutputBuilder.setCreateParent, InstrumentationOutputBuilder.build
// PORT: skipped, see above: InstrumentationOutputFactory.<init>, InstrumentationOutputFactory.Builder.setLocalInstrumentationOutputBuilderSupplier, InstrumentationOutputFactory.Builder.setBuildEventArtifactInstrumentationOutputBuilderSupplier, InstrumentationOutputFactory.Builder.setRedirectInstrumentationOutputBuilderSupplier, InstrumentationOutputFactory.Builder.setLocalTempLoggingDirPathStr, InstrumentationOutputFactory.Builder.build

/// `ProfilerStartedEvent`: the profile the command writes, if any.
// Java: com/google/devtools/build/lib/buildtool/buildevent/ProfilerStartedEvent.java:21 ProfilerStartedEvent
#[derive(Clone, Default)]
pub struct ProfilerStartedEvent {
    profile: Option<Arc<LocalInstrumentationOutput>>,
}

impl ProfilerStartedEvent {
    // Java: com/google/devtools/build/lib/buildtool/buildevent/ProfilerStartedEvent.java:29 ProfilerStartedEvent.getProfile
    pub fn get_profile(&self) -> Option<&Arc<LocalInstrumentationOutput>> {
        self.profile.as_ref()
    }
}

/// `BuildSummaryStatsModule.profileEvent`: the command's `ProfilerStartedEvent`, which
/// `BuildSummaryStatsModule.profileStarting` keeps for `buildComplete`.
///
/// PORT: the module's subscriber field; the dispatcher posts here and clears it after the
/// command.
static PROFILE_EVENT: Mutex<Option<ProfilerStartedEvent>> = Mutex::new(None);

/// `storedEventHandler.post(profilerStartedEvent)` reaching
/// `BuildSummaryStatsModule.profileStarting`.
// Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:127 BuildSummaryStatsModule.profileStarting
pub fn profile_starting(event: ProfilerStartedEvent) {
    *PROFILE_EVENT.lock() = Some(event);
}

/// The part of `BuildSummaryStatsModule.buildComplete` that stops the profiler and lists the
/// profile in the build tool logs. Returns the error message to report, if any.
// Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:154 BuildSummaryStatsModule.buildComplete
pub fn stop_and_publish_profile(
    build_tool_log_collection: &mut blaze_buildtool::buildtool::build_result::BuildToolLogCollection,
) -> Option<String> {
    let profile = PROFILE_EVENT
        .lock()
        .as_ref()
        .and_then(|e| e.get_profile().cloned());
    if let Some(profile) = profile {
        // The profiler has to be stopped before `BuildEventServiceModule#afterCommand` is called,
        // especially when it is a bep artifact. An unstopped bep artifact could lead to a deadlock
        // in `BuildEventServiceModule#afterCommand`.
        //
        // We choose to stop profiler here instead of in `BuildSummaryStatsModule#afterCommand` so
        // that no ordering between GoogleBuildSummaryStatsModule and BuildEventServiceModule's
        // `afterCommand`s needs to be assumed. See b/253394502.
        //
        // Stopping the profiler here leads to missing the afterCommand profiles of the other
        // modules in the profile, which is a compromise we are willing to make.
        match Profiler::instance().stop() {
            Ok(()) => profile.publish(build_tool_log_collection),
            Err(e) => return Some(format!("Error while writing profile file: {e}")),
        }
    }
    None
}

/// The dispatcher's `finally`: the profiler might still be running when an exception is thrown
/// before BuildCompleteEvent is emitted or BlazeModule#completeCommand() is called. So we still
/// need to try to stop the profiler here. Returns the error message to report, if any.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:363 BlazeCommandDispatcher.execExclusively
pub fn stop_and_clear_current() -> Option<String> {
    let profiler_started_event = PROFILE_EVENT.lock().clone().unwrap_or_default();
    let mut error = None;
    let result =
        Profiler::instance()
            .stop()
            .and_then(|()| match profiler_started_event.get_profile() {
                Some(profile) => profile.make_convenience_link(),
                None => Ok(()),
            });
    if let Err(e) = result {
        error = Some(format!("Error while writing profile file: {e}"));
    }
    Profiler::instance().clear();
    *PROFILE_EVENT.lock() = None;
    error
}

/// Implements automatic profile management: computes the path to write the profile for the
/// current command to, and does garbage collection for profiles from previous commands based on
/// the `--profiles_to_retain` flag. Returns the path this command's profile should be written to.
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:339 BlazeRuntime.manageProfiles
pub fn manage_profiles(dir: &Path, command_id: &str, retention_window: i32) -> io::Result<Path> {
    let prefix = "command-";
    let suffix = ".profile.gz";
    let mut old: Vec<(Path, i64)> = Vec::new();
    for dirent in dir.readdir(Symlinks::Follow)? {
        if dirent.get_name().starts_with(prefix) && dirent.get_name().ends_with(suffix) {
            let path = dir.get_child(dirent.get_name());
            let mtime = path.stat()?.get_last_modified_time();
            old.push((path, mtime));
        }
    }
    // old.sort(Comparator.comparingLong(PathAndMtime::mtime)): a stable sort.
    old.sort_by_key(|(_, mtime)| *mtime);
    let to_remove = (old.len() as i64 - retention_window as i64 + 1).max(0) as usize;
    for (path, _) in old.iter().take(to_remove) {
        path.delete()?;
    }
    let profile_name = format!("{prefix}{command_id}{suffix}");
    Ok(dir.get_child(&profile_name))
}

/// Whether the command records a profile (`tracerEnabled`).
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:363 BlazeCommandDispatcher.execExclusively
pub fn tracer_enabled(
    common_options: &CommonCommandOptions,
    command_name: &str,
    analyzes: bool,
) -> bool {
    use blaze_options::TriState;
    let mut tracer_enabled = false;
    if common_options.enable_tracer == TriState::YES {
        tracer_enabled = true;
    } else if common_options.enable_tracer == TriState::AUTO {
        let command_supports_profile = command_name == "query" || analyzes;
        tracer_enabled = command_supports_profile || common_options.profile_path.is_some();
    }
    tracer_enabled
}

/// Configure profiling based on the provided options.
///
/// PORT: `--experimental_stream_log_file_uploads` (a build event artifact as the profile) and
/// `--redirect_local_instrumentation_output_writes` with a redirecting builder (Bazel has none;
/// it warns and writes locally, as here) are the other destinations.
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:360 BlazeRuntime.initProfiler
pub fn init_profiler(
    tracer_enabled: bool,
    event_handler: &mut StoredEventHandler,
    options: &OptionsParser,
    env: &CommandEnvironment,
    exec_start_time_nanos: i64,
    wait_time_in_ms: i64,
    release_name: &str,
) -> ProfilerStartedEvent {
    let bep_options = options.get_options::<BuildEventProtocolOptions>();
    let command_options = options
        .get_options::<CommonCommandOptions>()
        .expect("CommonCommandOptions");
    let mut out: Option<Box<dyn io::Write + Send>> = None;
    let mut record_full_profiler_data = command_options.record_full_profiler_data;
    let mut profiled_tasks_builder = ProfilerTaskSet::empty();
    let mut format = Format::JSON_TRACE_FILE_FORMAT;
    let mut profile: Option<Arc<LocalInstrumentationOutput>> = None;
    let result: io::Result<()> = (|| {
        if tracer_enabled {
            match &command_options.profile_path {
                None => {
                    let profile_name = "command.profile.gz";
                    format = Format::JSON_TRACE_FILE_COMPRESSED_FORMAT;
                    if bep_options
                        .as_ref()
                        .is_some_and(|o| o.streaming_log_file_uploads)
                    {
                        blaze_util::unported!(
                            "InstrumentationOutputFactory.createBuildEventArtifactInstrumentationOutput",
                            "--experimental_stream_log_file_uploads"
                        );
                    } else if command_options.redirect_local_instrumentation_output_writes {
                        // InstrumentationOutputFactory.createInstrumentationOutput(profileName,
                        // PathFragment.create(profileName), OUTPUT_BASE, env, eventHandler,
                        // null, null): Bazel has no redirecting builder.
                        // Java: com/google/devtools/build/lib/runtime/InstrumentationOutputFactory.java:128 InstrumentationOutputFactory.createInstrumentationOutput
                        event_handler.handle(HandlerEvent::warn(
                            "Redirecting to write Instrumentation Output on a different machine \
                             is not supported. Defaulting to writing output locally.",
                        ));
                        profile = Some(Arc::new(LocalInstrumentationOutput::new(
                            profile_name,
                            env.get_output_base().get_relative_str(profile_name),
                            None,
                            None,
                            None,
                            false,
                        )));
                    } else {
                        let profile_path = manage_profiles(
                            env.get_output_base(),
                            env.get_command_id(),
                            command_options.profiles_to_retain,
                        )?;
                        // createLocalOutputWithConvenientName(profileName, profilePath,
                        // profileName).
                        // Java: com/google/devtools/build/lib/runtime/InstrumentationOutputFactory.java:65 InstrumentationOutputFactory.createLocalOutputWithConvenientName
                        profile = Some(Arc::new(LocalInstrumentationOutput::new(
                            profile_name,
                            profile_path,
                            Some(profile_name.to_string()),
                            None,
                            None,
                            false,
                        )));
                    }
                }
                Some(profile_path) => {
                    let profile_path = profile_path.get();
                    format = if profile_path.get_path_string().ends_with(".gz") {
                        Format::JSON_TRACE_FILE_COMPRESSED_FORMAT
                    } else {
                        Format::JSON_TRACE_FILE_FORMAT
                    };
                    // createInstrumentationOutput(..., WORKSPACE_OR_HOME, ..., append=false,
                    // internal=true).
                    if command_options.redirect_local_instrumentation_output_writes {
                        event_handler.handle(HandlerEvent::warn(
                            "Redirecting to write Instrumentation Output on a different machine \
                             is not supported. Defaulting to writing output locally.",
                        ));
                    }
                    let base = match env.get_workspace() {
                        Some(w) => w.clone(),
                        None => blaze_util::unported!(
                            "InstrumentationOutputFactory.createInstrumentationOutput (no workspace: home directory)"
                        ),
                    };
                    profile = Some(Arc::new(LocalInstrumentationOutput::new(
                        if format == Format::JSON_TRACE_FILE_COMPRESSED_FORMAT {
                            "command.profile.gz"
                        } else {
                            "command.profile.json"
                        },
                        base.get_relative(profile_path),
                        None,
                        Some(false),
                        Some(true),
                        false,
                    )));
                }
            }
            out = Some(profile.as_ref().unwrap().create_output_stream()?);
            for profiler_task in ProfilerTask::VALUES {
                if !profiler_task.is_vfs()
                    // CRITICAL_PATH corresponds to writing the file.
                    && *profiler_task != ProfilerTask::CRITICAL_PATH
                    && *profiler_task != ProfilerTask::SKYFUNCTION
                {
                    profiled_tasks_builder.add(*profiler_task);
                }
            }
            for task in &command_options.additional_profile_tasks {
                use blaze_options::converters::JavaEnum;
                profiled_tasks_builder
                    .add(ProfilerTask::value_of(task.name()).expect("ProfilerTask constant"));
            }
            if command_options.record_full_profiler_data {
                for t in ProfilerTask::VALUES {
                    profiled_tasks_builder.add(*t);
                }
            }
        } else if command_options.always_profile_slow_operations {
            record_full_profiler_data = false;
            out = None;
            for profiler_task in ProfilerTask::VALUES {
                if profiler_task.collects_slowest_instances() {
                    profiled_tasks_builder.add(*profiler_task);
                }
            }
        }
        let profiled_tasks = profiled_tasks_builder;
        if !profiled_tasks.is_empty() {
            if command_options.slim_profile && command_options.include_primary_output {
                event_handler.handle(HandlerEvent::warn(
                    "Enabling both --slim_profile and \
                     --experimental_profile_include_primary_output: the \"out\" field \
                     will be omitted in merged actions.",
                ));
            }
            let profiler = Profiler::instance();
            let worker_process_metrics_collector =
                blaze_exec::worker::worker_process_metrics::WorkerProcessMetricsCollector::instance(
                );
            worker_process_metrics_collector.set_clock(Arc::new(blaze_events::JavaClock));

            let clock: Arc<dyn blaze_util::profiler::Clock> = Arc::new(ProfilerClock);
            profiler.start(
                profiled_tasks,
                out.take(),
                Some(format),
                env.get_output_base().get_path_string().to_string(),
                env.get_command_id().to_string(),
                record_full_profiler_data,
                clock.clone(),
                exec_start_time_nanos,
                command_options.slim_profile,
                command_options.include_primary_output,
                command_options.profile_include_target_label,
                command_options.profile_include_target_configuration,
                command_options.always_profile_slow_operations,
                Arc::new(CollectLocalResourceUsage::new(
                    worker_process_metrics_collector,
                    // env.getLocalResourceManager(): see CollectLocalResourceUsage.
                    None,
                    command_options.collect_worker_data_in_profiler,
                    command_options.collect_load_average_in_profiler,
                    command_options.collect_system_network_usage,
                    command_options.collect_resource_estimation,
                    command_options.collect_pressure_stall_indicators,
                    command_options.collect_skyframe_counts,
                    clock,
                )),
                release_name.to_string(),
            )?;
            // Instead of logEvent() we're calling the low level function to pass the timings we
            // took in the launcher. We're setting the INIT phase marker so that it follows
            // immediately the LAUNCH phase.
            let startup_time_nanos = command_options.startup_time * 1000000;
            let wait_time_nanos = wait_time_in_ms * 1000000;
            let client_start_time_nanos =
                exec_start_time_nanos - startup_time_nanos - wait_time_nanos;
            profiler.log_simple_task_duration(
                client_start_time_nanos,
                startup_time_nanos,
                ProfilerTask::PHASE,
                ProfilePhase::LAUNCH.description(),
            );
            if command_options.extract_data_time > 0 {
                profiler.log_simple_task_duration(
                    client_start_time_nanos,
                    command_options.extract_data_time * 1_000_000,
                    ProfilerTask::PHASE,
                    "Extracting Bazel binary",
                );
            }
            if command_options.wait_time > 0 {
                profiler.log_simple_task_duration(
                    client_start_time_nanos,
                    command_options.wait_time * 1_000_000,
                    ProfilerTask::PHASE,
                    "Blocking on busy Bazel server (in client)",
                );
            }
            if wait_time_in_ms > 0 {
                profiler.log_simple_task_duration(
                    client_start_time_nanos + startup_time_nanos,
                    wait_time_in_ms * 1_000_000,
                    ProfilerTask::PHASE,
                    "Blocking on busy Bazel server (in server)",
                );
            }
            profiler.log_simple_task_duration(
                exec_start_time_nanos,
                0,
                ProfilerTask::PHASE,
                ProfilePhase::INIT.description(),
            );
        }
        Ok(())
    })();
    if let Err(e) = result {
        event_handler.handle(HandlerEvent::error(format!(
            "Error while creating profile file: {e}"
        )));
        profile = None;
    }
    ProfilerStartedEvent { profile }
}
