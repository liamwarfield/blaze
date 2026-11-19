//! Port of `lib/remote/util/RxUtils.java`.
//!
//! PORT: a `Completable` transfer is a future of `io::Result<()>`. Java's
//! `InterruptedException` and `CancellationException` are errors of kind `Interrupted`
//! (`crate::common::errors`); "other errors" (anything but an `IOException`) have no counterpart,
//! since every failure of a transfer is an `io::Error` and an unchecked exception is a panic.

use std::future::Future;
use std::io;

use futures::stream::{FuturesUnordered, StreamExt};

use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::errors;

/// The result of a transfer.
// Java: com/google/devtools/build/lib/remote/util/RxUtils.java:31 RxUtils.TransferResult
#[derive(Debug)]
pub enum TransferResult {
    /// `TransferResult.ok()`.
    Ok,
    /// `TransferResult.interrupted()`.
    Interrupted,
    /// `TransferResult.error(error)`.
    Error(io::Error),
}

impl TransferResult {
    /// `isOk()`.
    pub fn is_ok(&self) -> bool {
        matches!(self, TransferResult::Ok)
    }

    /// `isError()`.
    pub fn is_error(&self) -> bool {
        matches!(self, TransferResult::Error(_))
    }

    /// `isInterrupted()`.
    pub fn is_interrupted(&self) -> bool {
        matches!(self, TransferResult::Interrupted)
    }

    /// `getError()`.
    pub fn get_error(&self) -> Option<&io::Error> {
        match self {
            TransferResult::Error(e) => Some(e),
            _ => None,
        }
    }
}

/// Converts the outcome of a transfer to a [`TransferResult`]: an IO error is an error, an
/// interruption or cancellation is an interruption.
// Java: com/google/devtools/build/lib/remote/util/RxUtils.java:82 RxUtils.toTransferResult
pub fn to_transfer_result(result: io::Result<()>) -> TransferResult {
    match result {
        Ok(()) => TransferResult::Ok,
        // A transfer is only ever canceled as part of tearing down the operation it belongs
        // to, so this is reported as an interruption rather than propagated downstream.
        Err(error) if errors::is_interrupted(&error) => TransferResult::Interrupted,
        Err(error) => TransferResult::Error(error),
    }
}

/// Collects transfer results into one outcome.
// Java: com/google/devtools/build/lib/remote/util/RxUtils.java:101 RxUtils.BulkTransferExceptionCollector
#[derive(Default)]
pub struct BulkTransferExceptionCollector {
    bulk_transfer_exception: Option<BulkTransferException>,
    interrupted: bool,
}

impl BulkTransferExceptionCollector {
    /// A collector without results.
    pub fn new() -> BulkTransferExceptionCollector {
        BulkTransferExceptionCollector::default()
    }

    // Java: com/google/devtools/build/lib/remote/util/RxUtils.java:105 RxUtils.BulkTransferExceptionCollector.onResult
    pub fn on_result(&mut self, result: TransferResult) {
        match result {
            TransferResult::Ok => {}
            TransferResult::Interrupted => self.interrupted = true,
            TransferResult::Error(error) => self
                .bulk_transfer_exception
                .get_or_insert_with(BulkTransferException::new)
                .add(error),
        }
    }

    // Java: com/google/devtools/build/lib/remote/util/RxUtils.java:124 RxUtils.BulkTransferExceptionCollector.toCompletable
    pub fn into_result(self) -> io::Result<()> {
        if self.interrupted {
            return Err(errors::interrupted());
        }

        if let Some(bulk_transfer_exception) = self.bulk_transfer_exception {
            return Err(bulk_transfer_exception.into());
        }

        Ok(())
    }
}

/// Completes when all `transfers` complete. Their IO errors are wrapped in a
/// `BulkTransferException`, in the order in which the transfers fail; an interrupted or
/// cancelled transfer makes the whole an interruption. Dropping the returned future cancels the
/// transfers that are still running.
// Java: com/google/devtools/build/lib/remote/util/RxUtils.java:160 RxUtils.mergeBulkTransfer
// Java: com/google/devtools/build/lib/remote/util/RxUtils.java:142 RxUtils.mergeBulkTransfer
pub async fn merge_bulk_transfer<I, F>(transfers: I) -> io::Result<()>
where
    I: IntoIterator<Item = F>,
    F: Future<Output = io::Result<()>>,
{
    // `Flowable.fromArray(transfers).flatMapSingle(RxUtils::toTransferResult)`: the results
    // arrive in the order in which the transfers complete.
    let mut transfers: FuturesUnordered<F> = transfers.into_iter().collect();
    let mut collector = BulkTransferExceptionCollector::new();
    while let Some(result) = transfers.next().await {
        collector.on_result(to_transfer_result(result));
    }
    collector.into_result()
}
