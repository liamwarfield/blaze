//! The `Target`-specific part of `query2/engine/QueryUtil.java`:
//! `LexicographicallySortedTargetAggregator`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use blaze_cmdline::Label;
use blaze_packages::target::Target;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::query_util::{AggregateAllCallback, AggregateAllOutputFormatterCallback};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};

/// PORT: Java keeps a `HashMap<Label, Target>` (the last target per label wins) and returns an
/// `ImmutableSortedSet` ordered by `Label.compareTo`; a `BTreeMap` keyed by label gives the same
/// result (blaze_cmdline's `Label: Ord` is Java's `compareTo`).
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:103 QueryUtil.LexicographicallySortedTargetAggregator
pub struct LexicographicallySortedTargetAggregator {
    result_map: Mutex<BTreeMap<Label, Target>>,
    io_exception: IoExceptionSlot,
}

impl Callback<Target> for LexicographicallySortedTargetAggregator {
    fn process(&self, partial_result: &[Target]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<Target> for LexicographicallySortedTargetAggregator {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:108 LexicographicallySortedTargetAggregator.processOutput
    fn process_output(&self, partial_result: &[Target]) -> Result<(), OutputFormatterError> {
        let mut result_map = self.result_map.lock().unwrap();
        for target in partial_result {
            result_map.insert(target.get_label().clone(), target.clone());
        }
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

impl ThreadSafeOutputFormatterCallback<Target> for LexicographicallySortedTargetAggregator {}

impl AggregateAllCallback<Target, Vec<Target>> for LexicographicallySortedTargetAggregator {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:115 LexicographicallySortedTargetAggregator.getResult
    fn get_result(&self) -> Vec<Target> {
        self.result_map.lock().unwrap().values().cloned().collect()
    }
}

impl AggregateAllOutputFormatterCallback<Target, Vec<Target>>
    for LexicographicallySortedTargetAggregator
{
}

/// Returns a fresh `AggregateAllOutputFormatterCallback` instance whose `get_result` returns all
/// the targets in the result sorted lexicographically by `Label`.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:141 QueryUtil.newLexicographicallySortedTargetAggregator
pub fn new_lexicographically_sorted_target_aggregator() -> LexicographicallySortedTargetAggregator {
    LexicographicallySortedTargetAggregator {
        result_map: Mutex::new(BTreeMap::new()),
        io_exception: IoExceptionSlot::new(),
    }
}
