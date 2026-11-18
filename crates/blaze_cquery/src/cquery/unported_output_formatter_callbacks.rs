//! The cquery output formatter callbacks that are not ported yet:
//! `TransitionsOutputFormatterCallback` (needs `CqueryTransitionResolver`),
//! `BuildOutputFormatterCallback`, `GraphOutputFormatterCallback`,
//! `StarlarkOutputFormatterCallback` and `FilesOutputFormatterCallback`. They keep their names
//! (so `--output` validation lists them as Java does) and fail loudly when selected.

use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};

use crate::cquery::configured_target_query_environment::CqueryNode;
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// A named cquery output formatter whose Java class is not ported.
pub struct UnportedOutputFormatterCallback {
    name: &'static str,
    java_class: &'static str,
    io_exception: IoExceptionSlot,
}

impl UnportedOutputFormatterCallback {
    // Java: com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterCallback.java:47 TransitionsOutputFormatterCallback
    pub fn transitions() -> Self {
        Self::new("transitions", "TransitionsOutputFormatterCallback")
    }

    // Java: com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallback.java:40 BuildOutputFormatterCallback
    pub fn build() -> Self {
        Self::new("build", "BuildOutputFormatterCallback")
    }

    // Java: com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallback.java:30 GraphOutputFormatterCallback
    pub fn graph() -> Self {
        Self::new("graph", "GraphOutputFormatterCallback")
    }

    // Java: com/google/devtools/build/lib/query2/cquery/StarlarkOutputFormatterCallback.java:63 StarlarkOutputFormatterCallback
    pub fn starlark() -> Self {
        Self::new("starlark", "StarlarkOutputFormatterCallback")
    }

    // Java: com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallback.java:36 FilesOutputFormatterCallback
    pub fn files() -> Self {
        Self::new("files", "FilesOutputFormatterCallback")
    }

    fn new(name: &'static str, java_class: &'static str) -> Self {
        UnportedOutputFormatterCallback {
            name,
            java_class,
            io_exception: IoExceptionSlot::new(),
        }
    }
}

impl NamedThreadSafeOutputFormatterCallback<CqueryNode> for UnportedOutputFormatterCallback {
    fn get_name(&self) -> String {
        self.name.to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<CqueryNode> for UnportedOutputFormatterCallback {}

impl Callback<CqueryNode> for UnportedOutputFormatterCallback {
    fn process(&self, partial_result: &[CqueryNode]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<CqueryNode> for UnportedOutputFormatterCallback {
    fn start(&self) -> Result<(), blaze_query_engine::callback::IoException> {
        blaze_util::unported!(format!("{}.start", self.java_class))
    }

    fn process_output(&self, _partial_result: &[CqueryNode]) -> Result<(), OutputFormatterError> {
        blaze_util::unported!(format!("{}.processOutput", self.java_class))
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}
