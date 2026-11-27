//! Port of `DebugEventHelper`: building the debug server's `DebugEvent` protos.

use blaze_protos::starlark_debugging::{
    ContinueExecutionResponse, DebugEvent, Error, EvaluateResponse, Frame, GetChildrenResponse,
    ListFramesResponse, Location, PauseThreadResponse, PausedThread, Scope, SetBreakpointsResponse,
    StartDebuggingResponse, Stepping as SteppingProto, ThreadContinuedEvent, ThreadPausedEvent,
    Value as ValueProto, debug_event::Payload,
};
use blaze_starlark::debug::{DebugLocation, Stepping};
use starlark::values::Value;

use crate::serialization::{DebugObject, ThreadObjectMap, get_value_proto};

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:44 DebugEventHelper.NO_SEQUENCE_NUMBER
const NO_SEQUENCE_NUMBER: i64 = 0;

fn event(sequence_number: i64, payload: Payload) -> DebugEvent {
    DebugEvent {
        sequence_number,
        payload: Some(payload),
    }
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:46 DebugEventHelper.error
pub fn error(message: &str) -> DebugEvent {
    error_with(NO_SEQUENCE_NUMBER, message)
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:50 DebugEventHelper.error
pub fn error_with(sequence_number: i64, message: &str) -> DebugEvent {
    event(
        sequence_number,
        Payload::Error(Error {
            message: message.to_owned(),
        }),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:57 DebugEventHelper.setBreakpointsResponse
pub fn set_breakpoints_response(sequence_number: i64) -> DebugEvent {
    event(
        sequence_number,
        Payload::SetBreakpoints(SetBreakpointsResponse {}),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:64 DebugEventHelper.continueExecutionResponse
pub fn continue_execution_response(sequence_number: i64) -> DebugEvent {
    event(
        sequence_number,
        Payload::ContinueExecution(ContinueExecutionResponse {}),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:71 DebugEventHelper.evaluateResponse
pub fn evaluate_response(sequence_number: i64, value: ValueProto) -> DebugEvent {
    event(
        sequence_number,
        Payload::Evaluate(EvaluateResponse {
            result: Some(value),
        }),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:78 DebugEventHelper.listFramesResponse
pub fn list_frames_response(sequence_number: i64, frames: Vec<Frame>) -> DebugEvent {
    event(
        sequence_number,
        Payload::ListFrames(ListFramesResponse { frame: frames }),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:85 DebugEventHelper.startDebuggingResponse
pub fn start_debugging_response(sequence_number: i64) -> DebugEvent {
    event(
        sequence_number,
        Payload::StartDebugging(StartDebuggingResponse {}),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:92 DebugEventHelper.pauseThreadResponse
pub fn pause_thread_response(sequence_number: i64) -> DebugEvent {
    event(
        sequence_number,
        Payload::PauseThread(PauseThreadResponse {}),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:99 DebugEventHelper.getChildrenResponse
pub fn get_children_response(sequence_number: i64, children: Vec<ValueProto>) -> DebugEvent {
    event(
        sequence_number,
        Payload::GetChildren(GetChildrenResponse { children }),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:106 DebugEventHelper.threadPausedEvent
pub fn thread_paused_event(thread: PausedThread) -> DebugEvent {
    event(
        NO_SEQUENCE_NUMBER,
        Payload::ThreadPaused(ThreadPausedEvent {
            thread: Some(thread),
        }),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:112 DebugEventHelper.threadContinuedEvent
pub fn thread_continued_event(thread_id: i64) -> DebugEvent {
    event(
        NO_SEQUENCE_NUMBER,
        Payload::ThreadContinued(ThreadContinuedEvent { thread_id }),
    )
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:119 DebugEventHelper.getLocationProto
pub fn get_location_proto(location: Option<&DebugLocation>) -> Option<Location> {
    location.map(|l| Location {
        line_number: l.line as u32,
        column_number: l.column as u32,
        path: l.file.clone(),
    })
}

/// A frame's name, location and the bindings of its scopes (`local` then `global`, or only
/// `global` when it has no locals).
// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:131 DebugEventHelper.getFrameProto
pub fn get_frame_proto<'v>(
    object_map: &mut ThreadObjectMap<'v>,
    function_name: &str,
    location: Option<&DebugLocation>,
    locals: Vec<(String, Value<'v>)>,
    module_vars: Vec<(String, Value<'v>)>,
) -> Frame {
    Frame {
        function_name: function_name.to_owned(),
        scope: get_scopes(object_map, locals, module_vars),
        location: get_location_proto(location),
    }
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:139 DebugEventHelper.getScopes
fn get_scopes<'v>(
    object_map: &mut ThreadObjectMap<'v>,
    local_vars: Vec<(String, Value<'v>)>,
    module_vars: Vec<(String, Value<'v>)>,
) -> Vec<Scope> {
    if local_vars.is_empty() {
        return vec![get_scope(object_map, "global", module_vars)];
    }
    let global_vars: Vec<(String, Value<'v>)> = module_vars
        .into_iter()
        .filter(|(name, _)| !local_vars.iter().any(|(l, _)| l == name))
        .collect();
    vec![
        get_scope(object_map, "local", local_vars),
        get_scope(object_map, "global", global_vars),
    ]
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:154 DebugEventHelper.getScope
fn get_scope<'v>(
    object_map: &mut ThreadObjectMap<'v>,
    name: &str,
    bindings: Vec<(String, Value<'v>)>,
) -> Scope {
    Scope {
        name: name.to_owned(),
        binding: bindings
            .into_iter()
            .map(|(s, o)| get_value_proto(object_map, &s, DebugObject::Value(o)))
            .collect(),
    }
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebugEventHelper.java:163 DebugEventHelper.convertSteppingEnum
pub fn convert_stepping_enum(stepping: SteppingProto) -> Stepping {
    match stepping {
        SteppingProto::Into => Stepping::Into,
        SteppingProto::Out => Stepping::Out,
        SteppingProto::Over => Stepping::Over,
        SteppingProto::None => Stepping::None,
    }
}
