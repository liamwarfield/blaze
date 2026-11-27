//! `WorkspaceRuleEvent`: the events of `repository_ctx` / `module_ctx` calls that
//! `--experimental_workspace_rules_log_file` writes (blaze_runtime `WorkspaceRuleModule`).
//!
//! Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java
//!
//! The strings are Blaze's internal strings (one char per byte), as Java's are, so the proto's
//! `string` fields get the same bytes.

use blaze_events::{Location, Postable};
use blaze_protos::workspace_log::{
    self, DeleteEvent, DownloadAndExtractEvent, DownloadEvent, ExecuteEvent, ExecuteWasmEvent,
    ExtractEvent, FileEvent, LoadWasmEvent, OsEvent, PatchEvent, ReadEvent, RenameEvent,
    SymlinkEvent, TemplateEvent, WhichEvent, WorkspaceEvent, workspace_event::Event,
};

/// An event to record events happening during workspace rule resolution.
// Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:34 WorkspaceRuleEvent
#[derive(Debug, Clone)]
pub struct WorkspaceRuleEvent {
    event: WorkspaceEvent,
}

impl Postable for WorkspaceRuleEvent {}

/// `putAll` into a proto map field: Java's `MapField` keeps insertion order, and a key put
/// again keeps its place with the new value.
fn put_all<E>(
    entries: &mut Vec<E>,
    map: impl IntoIterator<Item = (String, String)>,
    key: fn(&mut E) -> &mut String,
    value: fn(&mut E) -> &mut String,
    new: fn(String, String) -> E,
) {
    for (k, v) in map {
        let mut found = false;
        for e in entries.iter_mut() {
            if *key(e) == k {
                *value(e) = v.clone();
                found = true;
                break;
            }
        }
        if !found {
            entries.push(new(k, v));
        }
    }
}

impl WorkspaceRuleEvent {
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:37 WorkspaceRuleEvent.getLogEvent
    pub fn get_log_event(&self) -> &WorkspaceEvent {
        &self.event
    }

    /// The `WorkspaceEvent` with its location and context (both optional in Java).
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:41 WorkspaceRuleEvent.<init>
    fn new(event: Event, context: Option<&str>, location: Option<&Location>) -> Self {
        WorkspaceRuleEvent {
            event: WorkspaceEvent {
                location: location.map(|l| l.to_string()).unwrap_or_default(),
                context: context.map(str::to_owned).unwrap_or_default(),
                event: Some(event),
            },
        }
    }

    /// Creates a new WorkspaceRuleEvent for an execution event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:46 WorkspaceRuleEvent.newExecuteEvent
    #[allow(clippy::too_many_arguments)]
    pub fn new_execute_event(
        args: impl IntoIterator<Item = String>,
        timeout: i32,
        common_environment: Option<Vec<(String, String)>>,
        custom_environment: Option<Vec<(String, String)>>,
        output_directory: &str,
        quiet: bool,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let mut e = ExecuteEvent {
            timeout_seconds: timeout,
            output_directory: output_directory.to_owned(),
            quiet,
            ..Default::default()
        };
        for env in [common_environment, custom_environment]
            .into_iter()
            .flatten()
        {
            put_all(
                &mut e.environment,
                env,
                |e| &mut e.key,
                |e| &mut e.value,
                |key, value| workspace_log::execute_event::EnvironmentEntry { key, value },
            );
        }
        e.arguments.extend(args);
        Self::new(Event::ExecuteEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a download event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:85 WorkspaceRuleEvent.newDownloadEvent
    pub fn new_download_event(
        urls: impl IntoIterator<Item = String>,
        output: &str,
        sha256: &str,
        integrity: &str,
        executable: bool,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let e = DownloadEvent {
            url: urls.into_iter().collect(),
            output: output.to_owned(),
            sha256: sha256.to_owned(),
            executable,
            integrity: integrity.to_owned(),
        };
        Self::new(Event::DownloadEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for an extract event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:116 WorkspaceRuleEvent.newExtractEvent
    pub fn new_extract_event(
        archive: &str,
        output: &str,
        strip_prefix: &str,
        rename_files: impl IntoIterator<Item = (String, String)>,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let mut e = ExtractEvent {
            archive: archive.to_owned(),
            output: output.to_owned(),
            strip_prefix: strip_prefix.to_owned(),
            ..Default::default()
        };
        put_all(
            &mut e.rename_files,
            rename_files,
            |e| &mut e.key,
            |e| &mut e.value,
            |key, value| workspace_log::extract_event::RenameFilesEntry { key, value },
        );
        Self::new(Event::ExtractEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a download and extract event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:144 WorkspaceRuleEvent.newDownloadAndExtractEvent
    #[allow(clippy::too_many_arguments)]
    pub fn new_download_and_extract_event(
        urls: impl IntoIterator<Item = String>,
        output: &str,
        sha256: &str,
        integrity: &str,
        type_: &str,
        strip_prefix: &str,
        rename_files: impl IntoIterator<Item = (String, String)>,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let mut e = DownloadAndExtractEvent {
            output: output.to_owned(),
            sha256: sha256.to_owned(),
            integrity: integrity.to_owned(),
            r#type: type_.to_owned(),
            strip_prefix: strip_prefix.to_owned(),
            ..Default::default()
        };
        put_all(
            &mut e.rename_files,
            rename_files,
            |e| &mut e.key,
            |e| &mut e.value,
            |key, value| workspace_log::download_and_extract_event::RenameFilesEntry { key, value },
        );
        e.url.extend(urls);
        Self::new(Event::DownloadAndExtractEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a file event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:179 WorkspaceRuleEvent.newFileEvent
    pub fn new_file_event(
        path: &str,
        content: &str,
        executable: bool,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let e = FileEvent {
            path: path.to_owned(),
            content: content.to_owned(),
            executable,
        };
        Self::new(Event::FileEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a file read event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:201 WorkspaceRuleEvent.newReadEvent
    pub fn new_read_event(path: &str, context: Option<&str>, location: Option<&Location>) -> Self {
        let e = ReadEvent {
            path: path.to_owned(),
        };
        Self::new(Event::ReadEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a file delete event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:218 WorkspaceRuleEvent.newDeleteEvent
    pub fn new_delete_event(
        path: &str,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let e = DeleteEvent {
            path: path.to_owned(),
        };
        Self::new(Event::DeleteEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a patch event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:235 WorkspaceRuleEvent.newPatchEvent
    pub fn new_patch_event(
        patch_file: &str,
        strip: i32,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let e = PatchEvent {
            patch_file: patch_file.to_owned(),
            strip,
        };
        Self::new(Event::PatchEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for an os event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:253 WorkspaceRuleEvent.newOsEvent
    pub fn new_os_event(context: Option<&str>, location: Option<&Location>) -> Self {
        Self::new(Event::OsEvent(OsEvent {}), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a rename event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:269 WorkspaceRuleEvent.newRenameEvent
    pub fn new_rename_event(
        src: &str,
        dst: &str,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let e = RenameEvent {
            src: src.to_owned(),
            dst: dst.to_owned(),
        };
        Self::new(Event::RenameEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a symlink event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:286 WorkspaceRuleEvent.newSymlinkEvent
    pub fn new_symlink_event(
        from: &str,
        to: &str,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let e = SymlinkEvent {
            target: from.to_owned(),
            path: to.to_owned(),
        };
        Self::new(Event::SymlinkEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a template event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:304 WorkspaceRuleEvent.newTemplateEvent
    pub fn new_template_event(
        path: &str,
        template: &str,
        substitutions: impl IntoIterator<Item = (String, String)>,
        executable: bool,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let mut e = TemplateEvent {
            path: path.to_owned(),
            template: template.to_owned(),
            executable,
            ..Default::default()
        };
        put_all(
            &mut e.substitutions,
            substitutions,
            |e| &mut e.key,
            |e| &mut e.value,
            |key, value| workspace_log::template_event::SubstitutionsEntry { key, value },
        );
        Self::new(Event::TemplateEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a which event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:332 WorkspaceRuleEvent.newWhichEvent
    pub fn new_which_event(
        program: &str,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let e = WhichEvent {
            program: program.to_owned(),
        };
        Self::new(Event::WhichEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for a load_wasm event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:348 WorkspaceRuleEvent.newLoadWasmEvent
    pub fn new_load_wasm_event(
        module_path: &str,
        allocate_fn: &str,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let e = LoadWasmEvent {
            module_path: module_path.to_owned(),
            allocate_fn: allocate_fn.to_owned(),
        };
        Self::new(Event::LoadWasmEvent(e), context, location)
    }

    /// Creates a new WorkspaceRuleEvent for an execute_wasm event.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleEvent.java:367 WorkspaceRuleEvent.newExecuteWasmEvent
    #[allow(clippy::too_many_arguments)]
    pub fn new_execute_wasm_event(
        module_path: &str,
        function: &str,
        input: &[u8],
        timeout: i32,
        memory_limit: i64,
        context: Option<&str>,
        location: Option<&Location>,
    ) -> Self {
        let e = ExecuteWasmEvent {
            module_path: module_path.to_owned(),
            function: function.to_owned(),
            input: input.to_vec(),
            timeout_seconds: timeout,
            memory_limit_bytes: memory_limit,
        };
        Self::new(Event::ExecuteWasmEvent(e), context, location)
    }
}

// PORT: skipped, debug text of the proto (only Java's logging calls it): WorkspaceRuleEvent.logMessage
