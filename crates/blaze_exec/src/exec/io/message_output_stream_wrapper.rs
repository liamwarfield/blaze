//! Port of `lib/util/io/MessageOutputStreamWrapper.java`.

use std::io::{self, Write};
use std::marker::PhantomData;

use blaze_protos::JavaMessage;
use blaze_protos::json::{JsonFormat, Printer};

use super::MessageOutputStream;

fn stream_closed() -> io::Error {
    io::Error::other("Stream Closed")
}

/// Writes the messages in length-delimited protobuf wire format.
// Java: com/google/devtools/build/lib/util/io/MessageOutputStreamWrapper.java:31 MessageOutputStreamWrapper.BinaryOutputStreamWrapper
pub struct BinaryOutputStreamWrapper<T> {
    stream: Option<Box<dyn Write + Send>>,
    _marker: PhantomData<fn(&T)>,
}

impl<T> BinaryOutputStreamWrapper<T> {
    // Java: com/google/devtools/build/lib/util/io/MessageOutputStreamWrapper.java:35 BinaryOutputStreamWrapper.<init>
    pub fn new(stream: Box<dyn Write + Send>) -> BinaryOutputStreamWrapper<T> {
        BinaryOutputStreamWrapper {
            stream: Some(stream),
            _marker: PhantomData,
        }
    }
}

impl<T: JavaMessage> MessageOutputStream<T> for BinaryOutputStreamWrapper<T> {
    // Java: com/google/devtools/build/lib/util/io/MessageOutputStreamWrapper.java:40 BinaryOutputStreamWrapper.write
    fn write(&mut self, m: &T) -> io::Result<()> {
        let stream = self.stream.as_mut().ok_or_else(stream_closed)?;
        let mut buf = Vec::with_capacity(m.java_encoded_len() + 5);
        m.write_delimited_to(&mut buf);
        stream.write_all(&buf)
    }

    // Java: com/google/devtools/build/lib/util/io/MessageOutputStreamWrapper.java:46 BinaryOutputStreamWrapper.close
    fn close(&mut self) -> io::Result<()> {
        match self.stream.take() {
            None => Ok(()),
            Some(mut stream) => stream.flush(),
        }
    }
}

/// Writes the messages in concatenated JSON text format.
// Java: com/google/devtools/build/lib/util/io/MessageOutputStreamWrapper.java:53 MessageOutputStreamWrapper.JsonOutputStreamWrapper
pub struct JsonOutputStreamWrapper<T> {
    stream: Option<Box<dyn Write + Send>>,
    printer: Printer,
    _marker: PhantomData<fn(&T)>,
}

impl<T> JsonOutputStreamWrapper<T> {
    // Java: com/google/devtools/build/lib/util/io/MessageOutputStreamWrapper.java:59 JsonOutputStreamWrapper.<init>
    pub fn new(stream: Box<dyn Write + Send>) -> JsonOutputStreamWrapper<T> {
        JsonOutputStreamWrapper {
            stream: Some(stream),
            // Java: com/google/devtools/build/lib/util/io/MessageOutputStreamWrapper.java:54 JsonOutputStreamWrapper.PRINTER
            printer: JsonFormat::printer().always_print_fields_with_no_presence(),
            _marker: PhantomData,
        }
    }
}

impl<T: JavaMessage> MessageOutputStream<T> for JsonOutputStreamWrapper<T> {
    // Java: com/google/devtools/build/lib/util/io/MessageOutputStreamWrapper.java:64 JsonOutputStreamWrapper.write
    fn write(&mut self, m: &T) -> io::Result<()> {
        let stream = self.stream.as_mut().ok_or_else(stream_closed)?;
        // `InvalidProtocolBufferException` is an `IOException` in Java.
        let text = self
            .printer
            .print(m)
            .map_err(|e| io::Error::other(e.get_message().to_string()))?;
        // `getBytes(UTF_8)`: the printed text holds the message's strings as Java holds them.
        stream.write_all(text.as_bytes())
    }

    // Java: com/google/devtools/build/lib/util/io/MessageOutputStreamWrapper.java:70 JsonOutputStreamWrapper.close
    fn close(&mut self) -> io::Result<()> {
        match self.stream.take() {
            None => Ok(()),
            Some(mut stream) => stream.flush(),
        }
    }
}
