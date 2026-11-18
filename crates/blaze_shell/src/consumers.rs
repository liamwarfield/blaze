//! Port of `com.google.devtools.build.lib.shell.Consumers`.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread::JoinHandle;

use parking_lot::Mutex;

use crate::command_result::{ByteArrayOutputStream, CommandResult};
use crate::input_stream_sink::{InputStream, InputStreamSink, OutputStream, RunnableSink};

/// This class provides convenience methods for consuming (actively reading) output and error
/// streams with different consumption policies: accumulating and streaming.
// Java: com/google/devtools/build/lib/shell/Consumers.java:37 Consumers
pub struct Consumers;

impl Consumers {
    // Java: com/google/devtools/build/lib/shell/Consumers.java:45 Consumers.createAccumulatingConsumers
    pub fn create_accumulating_consumers() -> OutErrConsumers {
        OutErrConsumers::new(
            OutputConsumer::accumulating(),
            OutputConsumer::accumulating(),
        )
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:49 Consumers.createStreamingConsumers
    pub fn create_streaming_consumers(out: OutputStream, err: OutputStream) -> OutErrConsumers {
        OutErrConsumers::new(
            OutputConsumer::streaming(out),
            OutputConsumer::streaming(err),
        )
    }
}

/// The stdout and stderr consumers of one process.
// Java: com/google/devtools/build/lib/shell/Consumers.java:55 Consumers.OutErrConsumers
pub struct OutErrConsumers {
    out: OutputConsumer,
    err: OutputConsumer,
}

impl OutErrConsumers {
    fn new(out: OutputConsumer, err: OutputConsumer) -> OutErrConsumers {
        OutErrConsumers { out, err }
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:64 OutErrConsumers.registerInputs
    pub fn register_inputs(
        &mut self,
        out_input: InputStream,
        err_input: InputStream,
        close_streams: bool,
    ) {
        self.out.register_input(out_input, close_streams);
        self.err.register_input(err_input, close_streams);
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:71 OutErrConsumers.cancel
    pub fn cancel(&mut self) {
        self.out.cancel();
        self.err.cancel();
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:76 OutErrConsumers.waitForCompletion
    pub fn wait_for_completion(&mut self) -> io::Result<()> {
        self.out.wait_for_completion()?;
        self.err.wait_for_completion()
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:80 OutErrConsumers.getAccumulatedOut
    pub fn get_accumulated_out(&self) -> ByteArrayOutputStream {
        self.out.get_accumulated_out()
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:83 OutErrConsumers.getAccumulatedErr
    pub fn get_accumulated_err(&self) -> ByteArrayOutputStream {
        self.err.get_accumulated_out()
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:91 OutErrConsumers.logConsumptionStrategy
    pub fn log_consumption_strategy(&self) {
        // The creation methods guarantee that the consumption strategy is the same for out and
        // err - doesn't matter whether we call out or err, let's pick out.
        self.out.log_consumption_strategy();
    }
}

enum ConsumerKind {
    /// Java: StreamingConsumer: sends the input to a stream while consuming it.
    Streaming(OutputStream),
    /// Java: AccumulatingConsumer: sends the input to a ByteArrayOutputStream.
    Accumulating(ByteArrayOutputStream),
}

/// One consumer (Java `OutputConsumer` implemented by `FutureConsumption`).
// Java: com/google/devtools/build/lib/shell/Consumers.java:101 Consumers.OutputConsumer
struct OutputConsumer {
    kind: ConsumerKind,
    future: Option<JoinHandle<io::Result<()>>>,
}

static THREAD_INIT_NUMBER: AtomicI32 = AtomicI32::new(0);

impl OutputConsumer {
    fn streaming(out: OutputStream) -> OutputConsumer {
        OutputConsumer {
            kind: ConsumerKind::Streaming(out),
            future: None,
        }
    }

    fn accumulating() -> OutputConsumer {
        OutputConsumer {
            kind: ConsumerKind::Accumulating(ByteArrayOutputStream::new()),
            future: None,
        }
    }

    fn get_accumulated_out(&self) -> ByteArrayOutputStream {
        match &self.kind {
            ConsumerKind::Streaming(_) => CommandResult::NO_OUTPUT_COLLECTED,
            ConsumerKind::Accumulating(out) => out.clone(),
        }
    }

    fn log_consumption_strategy(&self) {
        // PORT: skipped, FINER-level java.log logging.
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:177 FutureConsumption.registerInput
    fn register_input(&mut self, input: InputStream, close_consumer: bool) {
        let sink = self.create_consuming_and_closing_sink(input, close_consumer);
        // Java: AccumulatorThreadFactory names the pool's threads.
        let name = format!(
            "Command-Accumulator-Thread-{}",
            THREAD_INIT_NUMBER.fetch_add(1, Ordering::SeqCst)
        );
        let handle = std::thread::Builder::new()
            .name(name)
            .spawn(sink)
            .expect("failed to spawn consumer thread");
        self.future = Some(handle);
    }

    fn create_consuming_and_closing_sink(
        &self,
        input: InputStream,
        close_consumer: bool,
    ) -> RunnableSink {
        match &self.kind {
            ConsumerKind::Streaming(out) => closing_sink(input, out.clone(), close_consumer),
            ConsumerKind::Accumulating(out) => {
                let out: OutputStream = Arc::new(Mutex::new(out.clone()));
                closing_sink(input, out, false)
            }
        }
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:185 FutureConsumption.cancel
    fn cancel(&mut self) {
        // PORT: Java's future.cancel(true) interrupts the consumer thread, which does not stop a
        // blocking read; the copy runs on until end-of-file. Detaching the thread is the same.
        if self.future.is_none() {
            panic!("NullPointerException: future");
        }
        self.future = None;
    }

    // Java: com/google/devtools/build/lib/shell/Consumers.java:190 FutureConsumption.waitForCompletion
    fn wait_for_completion(&mut self) -> io::Result<()> {
        let Some(future) = self.future.take() else {
            // Already waited for (or cancelled).
            return Ok(());
        };
        // Uninterruptibles.getUninterruptibly: joining a thread is not interruptible here.
        match future.join() {
            Ok(result) => result,
            // The sink panicked (Java: a RuntimeException or Error in the worker thread):
            // re-throw it in this thread.
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

/// A sink that closes its input stream once its done.
// Java: com/google/devtools/build/lib/shell/Consumers.java:237 Consumers.ClosingSink
fn closing_sink(input: InputStream, out: OutputStream, close: bool) -> RunnableSink {
    let sink = InputStreamSink::new_runnable_sink(input, out.clone());
    Box::new(move || {
        // The sink owns the input stream and drops (closes) it when done.
        let result = sink();
        if close {
            // PORT: a shared Rust writer cannot be closed; it is flushed, and closed when its last
            // owner drops it. Errors are ignored as in Java's silentClose.
            let _ = out.lock().flush();
        }
        result
    })
}
