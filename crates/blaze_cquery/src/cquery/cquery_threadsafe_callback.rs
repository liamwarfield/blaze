//! Port of `query2/cquery/CqueryThreadsafeCallback.java`: the state shared by cquery's output
//! formatter callbacks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::cquery_options::CqueryOptions;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::callback::IoExceptionSlot;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use indexmap::IndexSet;

use crate::cquery::configured_target_query_environment::ConfiguredTargetQueryEnvironment;
use crate::cquery::cquery_options::CommonQueryOptionsView;

/// Parent class for cquery's output formatter callbacks.
///
/// Human-readable cquery outputters should output short configuration IDs via [`short_id`] for
/// easier reading. Machine-readable output, which are more focused on completeness, should
/// output full configuration checksums.
// Java: com/google/devtools/build/lib/query2/cquery/CqueryThreadsafeCallback.java:44 CqueryThreadsafeCallback
pub struct CqueryThreadsafeCallback<'a> {
    pub event_handler: Arc<dyn ExtendedEventHandler>,
    pub options: &'a CqueryOptions,
    /// Java `outputStream` / `printStream` (a `BufferedWriter` over it; the same bytes).
    pub out: Option<OutputStream>,
    // Skyframe calls incur a performance cost, even on cache hits. Consider this before exposing
    // direct executor access to child classes.
    skyframe_executor: &'a SkyframeExecutor,
    config_cache: RefCell<HashMap<BuildConfigurationKey, Option<Arc<BuildConfigurationValue>>>>,
    pub accessor: &'a ConfiguredTargetQueryEnvironment,
    result: RefCell<Vec<String>>,
    uniquify_results: bool,
    pub io_exception: IoExceptionSlot,
}

impl<'a> CqueryThreadsafeCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/cquery/CqueryThreadsafeCallback.java:62 CqueryThreadsafeCallback.<init>
    pub fn new(
        event_handler: Arc<dyn ExtendedEventHandler>,
        options: &'a CqueryOptions,
        out: Option<OutputStream>,
        skyframe_executor: &'a SkyframeExecutor,
        accessor: &'a ConfiguredTargetQueryEnvironment,
        uniquify_results: bool,
    ) -> Self {
        CqueryThreadsafeCallback {
            event_handler,
            options,
            out,
            skyframe_executor,
            config_cache: RefCell::new(HashMap::new()),
            accessor,
            result: RefCell::new(Vec::new()),
            uniquify_results,
            io_exception: IoExceptionSlot::new(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/cquery/CqueryThreadsafeCallback.java:81 CqueryThreadsafeCallback.addResult
    pub fn add_result(&self, string: String) {
        self.result.borrow_mut().push(string);
    }

    // Java: com/google/devtools/build/lib/query2/cquery/CqueryThreadsafeCallback.java:86 CqueryThreadsafeCallback.getResult
    pub fn get_result(&self) -> Vec<String> {
        self.result.borrow().clone()
    }

    // Java: com/google/devtools/build/lib/query2/cquery/CqueryThreadsafeCallback.java:91 CqueryThreadsafeCallback.close
    pub fn close(&self, fail_fast: bool) -> std::io::Result<()> {
        if !fail_fast {
            if let Some(out) = &self.out {
                let result = self.result.borrow();
                let results_to_print: Vec<&String> = if self.uniquify_results {
                    let set: IndexSet<&String> = result.iter().collect();
                    set.into_iter().collect()
                } else {
                    result.iter().collect()
                };
                let mut print_stream = out.borrow_mut();
                let line_terminator = self.options.get_line_terminator();
                for s in results_to_print {
                    print_stream.write_all(s.as_bytes())?;
                    print_stream.write_all(line_terminator.as_bytes())?;
                }
                print_stream.flush()?;
            }
        }
        Ok(())
    }

    /// The configuration of a key (cached).
    // Java: com/google/devtools/build/lib/query2/cquery/CqueryThreadsafeCallback.java:102 CqueryThreadsafeCallback.getConfiguration
    pub fn get_configuration(
        &self,
        config_key: Option<&BuildConfigurationKey>,
    ) -> Option<Arc<BuildConfigurationValue>> {
        // Experiments querying:
        //     cquery --output=graph "deps(//src:main/java/com/google/devtools/build/lib:runtime)"
        // 10 times on a warm Blaze instance show 7% less total query time when using this cache
        // vs. calling Skyframe directly (and relying on Skyframe's cache).
        let config_key = config_key?;
        if let Some(c) = self.config_cache.borrow().get(config_key) {
            return c.clone();
        }
        let value = self
            .skyframe_executor
            .get_configuration_for_key(self.event_handler.clone(), config_key);
        self.config_cache
            .borrow_mut()
            .insert(config_key.clone(), value.clone());
        value
    }
}

/// Returns a user-friendly configuration identifier, using special IDs for null configurations.
// Java: com/google/devtools/build/lib/query2/cquery/CqueryThreadsafeCallback.java:117 CqueryThreadsafeCallback.shortId
pub fn short_id(config: Option<&Arc<BuildConfigurationValue>>) -> String {
    match config {
        None => "null".to_owned(),
        Some(config) => config.short_id(),
    }
}
