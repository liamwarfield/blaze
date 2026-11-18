#![cfg(feature = "wip")]
//! Port of `lib/remote/common/BulkTransferExceptionTest.java`.

use std::io;

use blaze_remote::common::bulk_transfer_exception::BulkTransferException;
use blaze_remote::common::errors;

#[test]
fn should_provide_generic_message_if_no_added_exception() {
    let bulk_transfer_exception = BulkTransferException::new();
    assert_eq!(
        bulk_transfer_exception.get_message(),
        "Unknown error during bulk transfer"
    );
}

#[test]
fn should_preserve_message_as_is_from_single_exception() {
    let mut bulk_transfer_exception = BulkTransferException::new();
    bulk_transfer_exception.add(errors::io_exception("Failure Type A"));
    assert_eq!(bulk_transfer_exception.get_message(), "Failure Type A");
}

#[test]
fn should_sort_and_remove_duplicates_when_aggregating_messages() {
    let mut bulk_transfer_exception = BulkTransferException::new();
    bulk_transfer_exception.add(errors::io_exception("Failure Type B"));
    bulk_transfer_exception.add(errors::io_exception("Failure Type A"));
    bulk_transfer_exception.add(errors::io_exception("Failure Type B"));
    assert_eq!(
        bulk_transfer_exception.get_message(),
        "Multiple errors during bulk transfer:\nFailure Type A\nFailure Type B"
    );
}

/// `new IOException()`: an exception whose `getMessage()` is null. The closest `io::Error` is
/// one without a payload.
fn io_exception_without_message() -> io::Error {
    io::Error::from(io::ErrorKind::Other)
}

#[test]
#[ignore = "an io::Error has no null message: crate::common::errors has no `new IOException()` and BulkTransferException::get_message does not filter messages (Java: filter(Objects::nonNull))"]
fn should_provide_generic_message_if_only_null_messages() {
    let mut bulk_transfer_exception = BulkTransferException::new();
    bulk_transfer_exception.add(io_exception_without_message());
    assert_eq!(
        bulk_transfer_exception.get_message(),
        "Unknown error during bulk transfer"
    );
}

#[test]
#[ignore = "an io::Error has no null message: crate::common::errors has no `new IOException()` and BulkTransferException::get_message does not filter messages (Java: filter(Objects::nonNull))"]
fn should_ignore_null_messages_when_getting_message() {
    let mut bulk_transfer_exception = BulkTransferException::new();
    bulk_transfer_exception.add(errors::io_exception("Failure Type A"));
    bulk_transfer_exception.add(io_exception_without_message());
    assert_eq!(bulk_transfer_exception.get_message(), "Failure Type A");
}
