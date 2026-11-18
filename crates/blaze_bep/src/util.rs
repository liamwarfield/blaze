//! Small ports of protobuf-java-util helpers used by build events.

use blaze_protos::google::protobuf::{Duration, Timestamp};

/// `Timestamps.fromMillis`.
// Java: com/google/protobuf/util/Timestamps.java Timestamps.fromMillis (protobuf-java-util)
pub fn timestamp_from_millis(millis: i64) -> Timestamp {
    Timestamp {
        seconds: millis.div_euclid(1000),
        nanos: (millis.rem_euclid(1000) * 1_000_000) as i32,
    }
}

/// `Durations.fromMillis`: the sign of nanos follows the sign of seconds.
// Java: com/google/protobuf/util/Durations.java Durations.fromMillis (protobuf-java-util)
pub fn duration_from_millis(millis: i64) -> Duration {
    Duration {
        seconds: millis / 1000,
        nanos: ((millis % 1000) * 1_000_000) as i32,
    }
}

/// `Durations.fromSeconds`.
// Java: com/google/protobuf/util/Durations.java Durations.fromSeconds (protobuf-java-util)
pub fn duration_from_seconds(seconds: i64) -> Duration {
    Duration { seconds, nanos: 0 }
}

/// `Durations.fromNanos`.
// Java: com/google/protobuf/util/Durations.java Durations.fromNanos (protobuf-java-util)
pub fn duration_from_nanos(nanos: i64) -> Duration {
    Duration {
        seconds: nanos / 1_000_000_000,
        nanos: (nanos % 1_000_000_000) as i32,
    }
}
