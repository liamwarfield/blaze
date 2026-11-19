//! `InfoItemHandler` (how info items are output), `StdoutInfoItemHandler` and
//! `RemoteRequestedInfoItemHandler`.

use std::io;

use blaze_events::OutErr;
#[allow(unused_imports)]
use blaze_events::OutputStream as _;
use blaze_optdefs::command::info_item_handler::InfoItemOutputType;

use crate::command_environment::CommandEnvironment;
use crate::module::AbruptExitException;

/// What `InfoItemHandler.addInfoItem` throws (`AbruptExitException`, `IOException`; Blaze does
/// not model `InterruptedException` here).
#[derive(Debug)]
pub enum InfoItemHandlerException {
    AbruptExit(AbruptExitException),
    Io(io::Error),
}

impl From<AbruptExitException> for InfoItemHandlerException {
    fn from(e: AbruptExitException) -> Self {
        InfoItemHandlerException::AbruptExit(e)
    }
}

impl From<io::Error> for InfoItemHandlerException {
    fn from(e: io::Error) -> Self {
        InfoItemHandlerException::Io(e)
    }
}

/// Handles how `InfoItem`s are outputted.
// Java: com/google/devtools/build/lib/runtime/commands/info/InfoItemHandler.java:22 InfoItemHandler
pub trait InfoItemHandler {
    /// Stores `InfoItem` information.
    // Java: com/google/devtools/build/lib/runtime/commands/info/InfoItemHandler.java:24 InfoItemHandler.addInfoItem
    fn add_info_item(&mut self, key: &str, value: &[u8]) -> Result<(), InfoItemHandlerException>;

    /// Flushes any internal state.
    // Java: com/google/devtools/build/lib/runtime/commands/info/InfoItemHandler.java:29 InfoItemHandler.close
    fn close(&mut self) -> io::Result<()>;
}

/// Contains method to create the correct type of `InfoItemHandler`.
// Java: com/google/devtools/build/lib/runtime/commands/info/InfoItemHandler.java:47 InfoItemHandler.InfoItemHandlerFactory
pub trait InfoItemHandlerFactory: Send + Sync {
    // Java: com/google/devtools/build/lib/runtime/commands/info/InfoItemHandler.java:48 InfoItemHandlerFactory.create
    fn create(
        &self,
        env: &CommandEnvironment,
        info_item_output_type: InfoItemOutputType,
        print_keys: bool,
    ) -> Box<dyn InfoItemHandler>;
}

/// Implementation of `InfoItemHandlerFactory` that creates `InfoItemHandler` instances based on
/// the provided `InfoItemOutputType`.
// Java: com/google/devtools/build/lib/runtime/commands/info/InfoItemHandler.java:56 InfoItemHandler.InfoItemHandlerFactoryImpl
pub struct InfoItemHandlerFactoryImpl;

impl InfoItemHandlerFactory for InfoItemHandlerFactoryImpl {
    // Java: com/google/devtools/build/lib/runtime/commands/info/InfoItemHandler.java:48 InfoItemHandlerFactoryImpl.create
    fn create(
        &self,
        env: &CommandEnvironment,
        info_item_output_type: InfoItemOutputType,
        print_keys: bool,
    ) -> Box<dyn InfoItemHandler> {
        match info_item_output_type {
            InfoItemOutputType::STDOUT => Box::new(StdoutInfoItemHandler::new(
                env.get_reporter_out_err().clone(),
                print_keys,
            )),
            InfoItemOutputType::RESPONSE_PROTO => {
                Box::new(RemoteRequestedInfoItemHandler::new(env, print_keys))
            }
        }
    }
}

/// Prints `InfoItem`s to the console.
// Java: com/google/devtools/build/lib/runtime/commands/info/StdoutInfoItemHandler.java:22 StdoutInfoItemHandler
pub struct StdoutInfoItemHandler {
    out_err: OutErr,
    print_keys: bool,
}

impl StdoutInfoItemHandler {
    // Java: com/google/devtools/build/lib/runtime/commands/info/StdoutInfoItemHandler.java:26 StdoutInfoItemHandler.<init>
    pub fn new(out_err: OutErr, print_keys: bool) -> Self {
        StdoutInfoItemHandler {
            out_err,
            print_keys,
        }
    }
}

impl InfoItemHandler for StdoutInfoItemHandler {
    // Java: com/google/devtools/build/lib/runtime/commands/info/StdoutInfoItemHandler.java:32 StdoutInfoItemHandler.addInfoItem
    fn add_info_item(&mut self, key: &str, value: &[u8]) -> Result<(), InfoItemHandlerException> {
        let out = self.out_err.get_output_stream();
        if self.print_keys {
            out.write(&blaze_util::string_encoding::latin1_encode(key))?;
            out.write(b":")?;
            out.write(b" ")?;
        }
        out.write(value)?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/commands/info/StdoutInfoItemHandler.java:42 StdoutInfoItemHandler.close
    fn close(&mut self) -> io::Result<()> {
        self.out_err.get_output_stream().flush()
    }
}

/// Collects `InfoItem`s and sends them to the remote client via response extensions.
///
/// PORT: response extensions and `PathToReplaceUtils` are not ported; creating this handler
/// (`--info_output_type=response_proto`) fails loudly.
// Java: com/google/devtools/build/lib/runtime/commands/info/RemoteRequestedInfoItemHandler.java:28 RemoteRequestedInfoItemHandler
pub struct RemoteRequestedInfoItemHandler;

impl RemoteRequestedInfoItemHandler {
    // Java: com/google/devtools/build/lib/runtime/commands/info/RemoteRequestedInfoItemHandler.java:35 RemoteRequestedInfoItemHandler.<init>
    pub fn new(_env: &CommandEnvironment, _print_keys: bool) -> Self {
        blaze_util::unported!(
            "RemoteRequestedInfoItemHandler.<init>",
            "response extensions (InfoResponse) are not ported"
        )
    }
}

impl InfoItemHandler for RemoteRequestedInfoItemHandler {
    // Java: com/google/devtools/build/lib/runtime/commands/info/RemoteRequestedInfoItemHandler.java:42 RemoteRequestedInfoItemHandler.addInfoItem
    fn add_info_item(&mut self, _key: &str, _value: &[u8]) -> Result<(), InfoItemHandlerException> {
        blaze_util::unported!("RemoteRequestedInfoItemHandler.addInfoItem")
    }

    // Java: com/google/devtools/build/lib/runtime/commands/info/RemoteRequestedInfoItemHandler.java:48 RemoteRequestedInfoItemHandler.close
    fn close(&mut self) -> io::Result<()> {
        blaze_util::unported!("RemoteRequestedInfoItemHandler.close")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_events::RecordingOutErr;

    // Java: src/test/java/com/google/devtools/build/lib/runtime/commands/info/StdoutInfoItemHandlerTest.java
    #[test]
    #[ignore = "needs a CommandEnvironment test fixture (Java uses a Mockito mock)"]
    fn test_std_output_item_handler_creation() {}

    #[test]
    fn test_std_output_item_handler_add_one_item_without_printing_key() {
        let out_err = RecordingOutErr::new();
        {
            let mut handler = StdoutInfoItemHandler::new(out_err.out_err().clone(), false);
            handler.add_info_item("info-1", b"value-1\n").unwrap();
            handler.close().unwrap();
        }
        assert_eq!(out_err.out_as_latin1(), "value-1\n");
    }

    #[test]
    fn test_std_output_item_handler_add_two_item_with_printing_key() {
        let out_err = RecordingOutErr::new();
        {
            let mut handler = StdoutInfoItemHandler::new(out_err.out_err().clone(), true);
            handler.add_info_item("foo", b"value-foo\n").unwrap();
            handler.add_info_item("bar", b"value-bar\n").unwrap();
            handler.close().unwrap();
        }
        assert_eq!(out_err.out_as_latin1(), "foo: value-foo\nbar: value-bar\n");
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/commands/info/RemoteRequestedInfoItemHandlerTest.java
    #[test]
    #[ignore = "needs response extensions (InfoResponse) and a CommandEnvironment fixture"]
    fn test_remote_requested_info_item_handler_creation() {}

    #[test]
    #[ignore = "needs response extensions (InfoResponse) and a CommandEnvironment fixture"]
    fn test_remote_requested_info_item_handler_no_extension_if_nothing_added() {}

    #[test]
    #[ignore = "needs response extensions (InfoResponse) and a CommandEnvironment fixture"]
    fn test_remote_requested_info_item_handler_add_one_item_without_print_key() {}

    #[test]
    #[ignore = "needs response extensions (InfoResponse) and a CommandEnvironment fixture"]
    fn test_remote_requested_info_item_handler_add_two_items() {}
}
