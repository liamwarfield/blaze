//! Ports of `Callback.java`, `OutputFormatterCallback.java`,
//! `ThreadSafeOutputFormatterCallback.java` and `SynchronizedDelegatingOutputFormatterCallback.java`.

use std::io;
use std::sync::{Arc, Mutex};

use crate::query_exception::{InterruptedException, QueryError};

/// Query callback to be called by a `QueryExpression` when it has part of the computation result.
/// Assuming the `QueryEnvironment` supports it, it would allow the caller to stream the results.
///
/// PORT: Java's `Iterable<T>` batch is a slice; lazily filtered iterables
/// (`Iterables.filter`) are materialised before the call, which is equivalent because the
/// filters are pure. Closures `Fn(&[T]) -> Result<(), QueryError>` are callbacks.
// Java: com/google/devtools/build/lib/query2/engine/Callback.java:24 Callback
pub trait Callback<T> {
    /// According to the `BatchCallback` interface, repeated elements may be passed in here.
    /// However, `QueryExpression`s calling the callback do not need to maintain this property, as
    /// the `QueryEnvironment` should filter out duplicates.
    // Java: com/google/devtools/build/lib/query2/engine/Callback.java:32 Callback.process
    fn process(&self, partial_result: &[T]) -> Result<(), QueryError>;
}

impl<T, F> Callback<T> for F
where
    F: Fn(&[T]) -> Result<(), QueryError>,
{
    fn process(&self, partial_result: &[T]) -> Result<(), QueryError> {
        self(partial_result)
    }
}

/// Gives a closure the callback signature (so its argument and error types are inferred).
pub fn callback_fn<T, F>(f: F) -> F
where
    F: Fn(&[T]) -> Result<(), QueryError>,
{
    f
}

/// Java's `IOException` as kept by an `OutputFormatterCallback`.
pub type IoException = Arc<io::Error>;

/// What `processOutput` and `close` throw: `IOException` or `InterruptedException`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum OutputFormatterError {
    #[error(transparent)]
    Io(IoException),
    #[error(transparent)]
    Interrupted(#[from] InterruptedException),
}

impl From<io::Error> for OutputFormatterError {
    fn from(e: io::Error) -> Self {
        OutputFormatterError::Io(Arc::new(e))
    }
}

/// The `ioException` field of Java's `OutputFormatterCallback`.
#[derive(Debug, Default)]
pub struct IoExceptionSlot(Mutex<Option<IoException>>);

impl IoExceptionSlot {
    pub fn new() -> Self {
        IoExceptionSlot(Mutex::new(None))
    }

    pub fn get(&self) -> Option<IoException> {
        self.0.lock().unwrap().clone()
    }

    pub fn set(&self, e: IoException) {
        *self.0.lock().unwrap() = Some(e);
    }
}

/// A callback that can receive a finish event when there are no more partial results.
///
/// PORT: Java's abstract class implements `process` by calling `processOutput` and turning an
/// `IOException` into a stored exception plus an `InterruptedException`. Implementors write
/// `Callback::process` as a call to [`output_formatter_callback_process`] and keep an
/// [`IoExceptionSlot`].
// Java: com/google/devtools/build/lib/query2/engine/OutputFormatterCallback.java:24 OutputFormatterCallback
pub trait OutputFormatterCallback<T>: Callback<T> {
    /// This method will be called before any partial result are available.
    ///
    /// It should be used for opening resources or sending a header to the output.
    // Java: com/google/devtools/build/lib/query2/engine/OutputFormatterCallback.java:33 OutputFormatterCallback.start
    fn start(&self) -> Result<(), IoException> {
        Ok(())
    }

    /// Flushes remaining output and cleans up resources if necessary.
    ///
    /// This method is called whether or not there was an error.
    ///
    /// `fail_fast` indicates whether or not this method is being called after an error. When
    /// true implementations should prefer cleaning up resources and avoiding throwing unnecessary
    /// exceptions over completing the output.
    // Java: com/google/devtools/build/lib/query2/engine/OutputFormatterCallback.java:45 OutputFormatterCallback.close
    fn close(&self, _fail_fast: bool) -> Result<(), OutputFormatterError> {
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/engine/OutputFormatterCallback.java:65 OutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[T]) -> Result<(), OutputFormatterError>;

    /// The slot behind `getIoException`.
    fn io_exception_slot(&self) -> &IoExceptionSlot;

    // Java: com/google/devtools/build/lib/query2/engine/OutputFormatterCallback.java:68 OutputFormatterCallback.getIoException
    fn get_io_exception(&self) -> Option<IoException> {
        self.io_exception_slot().get()
    }
}

/// Java `OutputFormatterCallback.process`.
///
/// Note that `Callback` does not throw IOExceptions. What this implementation does instead is
/// throw `InterruptedException` and store the `IOException` in the `ioException` field. Users of
/// this class should check on InterruptedException the field to disambiguate between real
/// interruptions or IO Exceptions.
// Java: com/google/devtools/build/lib/query2/engine/OutputFormatterCallback.java:55 OutputFormatterCallback.process
pub fn output_formatter_callback_process<T, C>(
    this: &C,
    partial_result: &[T],
) -> Result<(), QueryError>
where
    C: OutputFormatterCallback<T> + ?Sized,
{
    match this.process_output(partial_result) {
        Ok(()) => Ok(()),
        Err(OutputFormatterError::Io(e)) => {
            this.io_exception_slot().set(e);
            Err(QueryError::Interrupted(InterruptedException::with_message(
                "Interrupting due to a IOException in the OutputFormatter",
            )))
        }
        Err(OutputFormatterError::Interrupted(e)) => Err(QueryError::Interrupted(e)),
    }
}

/// Use an `OutputFormatterCallback` with an already computed set of targets. Note that this does
/// not work in stream mode, as the `targets` would already be computed.
///
/// The intended usage of this method is to use `StreamedFormatter` formatters in non streaming
/// contexts.
// Java: com/google/devtools/build/lib/query2/engine/OutputFormatterCallback.java:79 OutputFormatterCallback.processAllTargets
pub fn process_all_targets<T>(
    callback: &dyn OutputFormatterCallback<T>,
    targets: &[T],
) -> Result<(), OutputFormatterError> {
    let mut fail_fast = true;
    let result = (|| -> Result<(), OutputFormatterError> {
        callback.start().map_err(OutputFormatterError::Io)?;
        match callback.process(targets) {
            Ok(()) => {}
            Err(QueryError::Interrupted(e)) => {
                if let Some(io_exception) = callback.get_io_exception() {
                    return Err(OutputFormatterError::Io(io_exception));
                }
                return Err(OutputFormatterError::Interrupted(e));
            }
            Err(QueryError::Query(e)) => panic!(
                "IllegalStateException: This should not happen, as we are not running any query, \
                 only printing the results:{}",
                e.get_message()
            ),
            Err(QueryError::Cancellation) => panic!("java.util.concurrent.CancellationException"),
        }
        fail_fast = false;
        Ok(())
    })();
    // finally { callback.close(failFast); }: an exception from close replaces the pending one.
    callback.close(fail_fast)?;
    result
}

/// A marker parent class for a thread-safe `OutputFormatterCallback`.
// Java: com/google/devtools/build/lib/query2/engine/ThreadSafeOutputFormatterCallback.java:20 ThreadSafeOutputFormatterCallback
pub trait ThreadSafeOutputFormatterCallback<T>: OutputFormatterCallback<T> {}

/// A `ThreadSafeOutputFormatterCallback` wrapper around a `OutputFormatterCallback` delegate.
// Java: com/google/devtools/build/lib/query2/engine/SynchronizedDelegatingOutputFormatterCallback.java:21 SynchronizedDelegatingOutputFormatterCallback
pub struct SynchronizedDelegatingOutputFormatterCallback<T> {
    delegate: Box<dyn OutputFormatterCallback<T>>,
    lock: Mutex<()>,
    io_exception: IoExceptionSlot,
}

impl<T> SynchronizedDelegatingOutputFormatterCallback<T> {
    // Java: com/google/devtools/build/lib/query2/engine/SynchronizedDelegatingOutputFormatterCallback.java:25 SynchronizedDelegatingOutputFormatterCallback.<init>
    pub fn new(delegate: Box<dyn OutputFormatterCallback<T>>) -> Self {
        SynchronizedDelegatingOutputFormatterCallback {
            delegate,
            lock: Mutex::new(()),
            io_exception: IoExceptionSlot::new(),
        }
    }
}

impl<T> Callback<T> for SynchronizedDelegatingOutputFormatterCallback<T> {
    // Java: com/google/devtools/build/lib/query2/engine/SynchronizedDelegatingOutputFormatterCallback.java:40 SynchronizedDelegatingOutputFormatterCallback.process
    fn process(&self, partial_result: &[T]) -> Result<(), QueryError> {
        let _guard = self.lock.lock().unwrap();
        self.delegate.process(partial_result)
    }
}

impl<T> OutputFormatterCallback<T> for SynchronizedDelegatingOutputFormatterCallback<T> {
    // Java: com/google/devtools/build/lib/query2/engine/SynchronizedDelegatingOutputFormatterCallback.java:30 SynchronizedDelegatingOutputFormatterCallback.start
    fn start(&self) -> Result<(), IoException> {
        let _guard = self.lock.lock().unwrap();
        self.delegate.start()
    }

    // Java: com/google/devtools/build/lib/query2/engine/SynchronizedDelegatingOutputFormatterCallback.java:35 SynchronizedDelegatingOutputFormatterCallback.close
    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        let _guard = self.lock.lock().unwrap();
        self.delegate.close(fail_fast)
    }

    // Java: com/google/devtools/build/lib/query2/engine/SynchronizedDelegatingOutputFormatterCallback.java:46 SynchronizedDelegatingOutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[T]) -> Result<(), OutputFormatterError> {
        let _guard = self.lock.lock().unwrap();
        self.delegate.process_output(partial_result)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }

    // Java: com/google/devtools/build/lib/query2/engine/SynchronizedDelegatingOutputFormatterCallback.java:53 SynchronizedDelegatingOutputFormatterCallback.getIoException
    fn get_io_exception(&self) -> Option<IoException> {
        self.delegate.get_io_exception()
    }
}

impl<T> ThreadSafeOutputFormatterCallback<T> for SynchronizedDelegatingOutputFormatterCallback<T> {}
