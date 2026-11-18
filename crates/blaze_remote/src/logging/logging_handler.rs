//! Port of `lib/remote/logging/LoggingHandler.java`.

use std::any::Any;
use std::marker::PhantomData;

use crate::reapi::remote_logging::RpcCallDetails;

/// An interface for building [`RpcCallDetails`]s specialized for a specific gRPC call.
///
/// * `ReqT`: request type of the gRPC call
/// * `RespT`: response type of the gRPC call
// Java: com/google/devtools/build/lib/remote/logging/LoggingHandler.java:25 LoggingHandler
pub trait LoggingHandler<ReqT, RespT>: Send {
    /// Handle logging for an issued message.
    ///
    /// * `message`: the issued request message
    // Java: com/google/devtools/build/lib/remote/logging/LoggingHandler.java:32 LoggingHandler.handleReq
    fn handle_req(&mut self, message: &ReqT);

    /// Handle logging for a received response.
    ///
    /// * `message`: the received response message
    // Java: com/google/devtools/build/lib/remote/logging/LoggingHandler.java:39 LoggingHandler.handleResp
    fn handle_resp(&mut self, message: &RespT);

    /// Returns a [`RpcCallDetails`] based on the requests and responses handled by this handler.
    ///
    /// PORT: `&mut self` because Java's handlers fill their builders in `getDetails()`.
    // Java: com/google/devtools/build/lib/remote/logging/LoggingHandler.java:44 LoggingHandler.getDetails
    fn get_details(&mut self) -> RpcCallDetails;
}

/// Java's raw type `LoggingHandler`, as `LoggingInterceptor.selectHandler` returns it: the
/// handler of a call whose message types are only known at run time.
///
/// PORT: the channel hands messages to its interceptors as `&dyn Any`
/// ([`crate::grpc::CallListener`]); a message of another type than the handler's is Java's
/// `ClassCastException`.
// Java: com/google/devtools/build/lib/remote/logging/LoggingHandler.java:25 LoggingHandler
pub trait RawLoggingHandler: Send {
    /// `handleReq(message)`.
    fn handle_req(&mut self, message: &(dyn Any + Send));

    /// `handleResp(message)`.
    fn handle_resp(&mut self, message: &(dyn Any + Send));

    /// `getDetails()`.
    fn get_details(&mut self) -> RpcCallDetails;
}

struct Raw<H, ReqT, RespT> {
    handler: H,
    _marker: PhantomData<fn(&ReqT, &RespT)>,
}

impl<H, ReqT, RespT> RawLoggingHandler for Raw<H, ReqT, RespT>
where
    H: LoggingHandler<ReqT, RespT>,
    ReqT: 'static,
    RespT: 'static,
{
    fn handle_req(&mut self, message: &(dyn Any + Send)) {
        match message.downcast_ref::<ReqT>() {
            Some(message) => self.handler.handle_req(message),
            None => panic!(
                "ClassCastException: request cannot be cast to {}",
                std::any::type_name::<ReqT>()
            ),
        }
    }

    fn handle_resp(&mut self, message: &(dyn Any + Send)) {
        match message.downcast_ref::<RespT>() {
            Some(message) => self.handler.handle_resp(message),
            None => panic!(
                "ClassCastException: response cannot be cast to {}",
                std::any::type_name::<RespT>()
            ),
        }
    }

    fn get_details(&mut self) -> RpcCallDetails {
        self.handler.get_details()
    }
}

/// A typed handler as the raw `LoggingHandler` the interceptor works with.
pub fn raw<H, ReqT, RespT>(handler: H) -> Box<dyn RawLoggingHandler>
where
    H: LoggingHandler<ReqT, RespT> + 'static,
    ReqT: 'static,
    RespT: 'static,
{
    Box::new(Raw {
        handler,
        _marker: PhantomData,
    })
}
