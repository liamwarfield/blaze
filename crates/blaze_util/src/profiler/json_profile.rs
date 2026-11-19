//! Ports of `JsonProfile.java`, `TraceEvent.java` and `statistics/PhaseSummaryStatistics.java`:
//! reading a JSON trace profile back.
//!
//! PORT: Java reads with Gson's streaming `JsonReader`; this reads the whole document with
//! `serde_json` (insertion-ordered objects) and walks it the same way. Object `args` values stay
//! JSON values (Java: `Boolean`, `Double`, `String`, nested `ImmutableMap` / `ImmutableList`).

use std::io::{self, Read};
use std::path::Path;

use indexmap::IndexMap;
use serde_json::Value;

use super::profile_phase::ProfilePhase;
use super::profiler_task::ProfilerTask;

/// Represents a single trace event in a JSON profile.
// Java: com/google/devtools/build/lib/profiler/TraceEvent.java:31 TraceEvent
#[derive(Clone, Debug, PartialEq)]
pub struct TraceEvent {
    pub category: Option<String>,
    pub name: String,
    pub type_: Option<String>,
    /// Microsecond timestamps as nanoseconds (`Duration`).
    pub timestamp: Option<i64>,
    pub duration: Option<i64>,
    pub process_id: i64,
    pub thread_id: i64,
    pub args: Option<IndexMap<String, Value>>,
    pub primary_output_path: Option<String>,
    pub target_label: Option<String>,
    pub mnemonic: Option<String>,
    pub configuration: Option<String>,
}

impl TraceEvent {
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn type_(&self) -> Option<&str> {
        self.type_.as_deref()
    }

    pub fn timestamp(&self) -> Option<i64> {
        self.timestamp
    }

    pub fn duration(&self) -> Option<i64> {
        self.duration
    }

    pub fn thread_id(&self) -> i64 {
        self.thread_id
    }

    pub fn args(&self) -> Option<&IndexMap<String, Value>> {
        self.args.as_ref()
    }

    pub fn primary_output_path(&self) -> Option<&str> {
        self.primary_output_path.as_deref()
    }

    pub fn target_label(&self) -> Option<&str> {
        self.target_label.as_deref()
    }

    pub fn mnemonic(&self) -> Option<&str> {
        self.mnemonic.as_deref()
    }

    pub fn configuration(&self) -> Option<&str> {
        self.configuration.as_deref()
    }

    // Java: com/google/devtools/build/lib/profiler/TraceEvent.java:86 TraceEvent.createFromJsonReader
    fn create_from_json(value: &Value) -> io::Result<TraceEvent> {
        let obj = value
            .as_object()
            .ok_or_else(|| io::Error::other("Expected BEGIN_OBJECT"))?;
        let mut category = None;
        let mut name = None;
        let mut timestamp = None;
        let mut duration = None;
        let mut process_id = -1;
        let mut thread_id = -1;
        let mut primary_output_path = None;
        let mut target_label = None;
        let mut mnemonic = None;
        let mut type_ = None;
        let mut configuration = None;
        let mut args = None;
        for (key, v) in obj {
            match key.as_str() {
                "cat" => category = Some(next_string(v)?),
                "name" => name = Some(next_string(v)?),
                "ph" => type_ = Some(next_string(v)?),
                // Duration has no microseconds :-/.
                "ts" => timestamp = Some(next_long(v)? * 1000),
                "dur" => duration = Some(next_long(v)? * 1000),
                "pid" => process_id = next_long(v)?,
                "tid" => thread_id = next_long(v)?,
                "out" => primary_output_path = Some(next_string(v)?),
                "args" => {
                    let map: IndexMap<String, Value> = v
                        .as_object()
                        .ok_or_else(|| io::Error::other("Expected BEGIN_OBJECT"))?
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    target_label = map.get("target").and_then(|t| t.as_str()).map(str::to_string);
                    mnemonic = map
                        .get("mnemonic")
                        .and_then(|t| t.as_str())
                        .map(str::to_string);
                    configuration = map
                        .get("configuration")
                        .and_then(|t| t.as_str())
                        .map(str::to_string);
                    args = Some(map);
                }
                _ => {}
            }
        }
        Ok(TraceEvent {
            category,
            name: name.ok_or_else(|| io::Error::other("NullPointerException: name"))?,
            type_,
            timestamp,
            duration,
            process_id,
            thread_id,
            args,
            primary_output_path,
            target_label,
            mnemonic,
            configuration,
        })
    }

    // Java: com/google/devtools/build/lib/profiler/TraceEvent.java:184 TraceEvent.parseTraceEvents
    pub fn parse_trace_events(value: &Value) -> io::Result<Vec<TraceEvent>> {
        let array = value
            .as_array()
            .ok_or_else(|| io::Error::other("Expected BEGIN_ARRAY"))?;
        array.iter().map(Self::create_from_json).collect()
    }
}

fn next_string(v: &Value) -> io::Result<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        // Gson's nextString also accepts numbers.
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(io::Error::other(format!("Expected a string but was {v}"))),
    }
}

fn next_long(v: &Value) -> io::Result<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| io::Error::other(format!("Expected a long but was {n}"))),
        Value::String(s) => s
            .parse()
            .map_err(|_| io::Error::other(format!("Expected a long but was {s}"))),
        _ => Err(io::Error::other(format!("Expected a long but was {v}"))),
    }
}

/// Extracts and keeps summary statistics from all [`ProfilePhase`]s for formatting to various
/// outputs.
// Java: com/google/devtools/build/lib/profiler/statistics/PhaseSummaryStatistics.java:27 PhaseSummaryStatistics
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhaseSummaryStatistics {
    total_duration_nanos: i64,
    /// `EnumMap<ProfilePhase, Long>`: iterated in declaration order.
    durations: Vec<(ProfilePhase, i64)>,
}

impl PhaseSummaryStatistics {
    /// Add a single profile phase.
    // Java: com/google/devtools/build/lib/profiler/statistics/PhaseSummaryStatistics.java:38 PhaseSummaryStatistics.addProfilePhase
    pub fn add_profile_phase(&mut self, phase: ProfilePhase, duration_nanos: i64) {
        self.total_duration_nanos += duration_nanos;
        match self.durations.iter_mut().find(|(p, _)| *p == phase) {
            Some(entry) => entry.1 = duration_nanos,
            None => {
                self.durations.push((phase, duration_nanos));
                let pos = |p: &ProfilePhase| ProfilePhase::VALUES.iter().position(|v| v == p);
                self.durations.sort_by_key(|(p, _)| pos(p));
            }
        }
    }

    // Java: com/google/devtools/build/lib/profiler/statistics/PhaseSummaryStatistics.java:52 PhaseSummaryStatistics.getDurationNanos
    pub fn get_duration_nanos(&self, phase: ProfilePhase) -> i64 {
        match self.durations.iter().find(|(p, _)| *p == phase) {
            Some((_, d)) => *d,
            None => panic!("NoSuchElementException: Phase {phase:?} was not executed"),
        }
    }

    // Java: com/google/devtools/build/lib/profiler/statistics/PhaseSummaryStatistics.java:61 PhaseSummaryStatistics.getRelativeDuration
    pub fn get_relative_duration(&self, phase: ProfilePhase) -> f64 {
        self.get_duration_nanos(phase) as f64 / self.total_duration_nanos as f64
    }

    pub fn get_total_duration(&self) -> i64 {
        self.total_duration_nanos
    }

    /// The executed phases, in declaration order.
    pub fn phases(&self) -> impl Iterator<Item = ProfilePhase> + '_ {
        self.durations.iter().map(|(p, _)| *p)
    }
}

/// Value class to hold build metadata (id, date, output base) if available.
// Java: com/google/devtools/build/lib/profiler/JsonProfile.java:136 JsonProfile.BuildMetadata
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildMetadata {
    pub build_id: Option<String>,
    pub date: Option<String>,
    pub output_base: Option<String>,
}

/// Utility class to handle parsing the JSON trace profiles.
// Java: com/google/devtools/build/lib/profiler/JsonProfile.java:38 JsonProfile
#[derive(Clone, Debug, PartialEq)]
pub struct JsonProfile {
    build_metadata: Option<BuildMetadata>,
    phase_summary_statistics: Option<PhaseSummaryStatistics>,
    trace_events: Vec<TraceEvent>,
}

impl JsonProfile {
    /// `new JsonProfile(File)`: gunzips files whose name ends in `.gz`.
    // Java: com/google/devtools/build/lib/profiler/JsonProfile.java:43 JsonProfile.<init>
    pub fn from_file(profile_file: &Path) -> io::Result<JsonProfile> {
        let file = std::fs::File::open(profile_file)?;
        let is_gz = profile_file
            .file_name()
            .is_some_and(|n| n.as_encoded_bytes().ends_with(b".gz"));
        if is_gz {
            Self::from_reader(flate2::read::GzDecoder::new(file))
        } else {
            Self::from_reader(file)
        }
    }

    // Java: com/google/devtools/build/lib/profiler/JsonProfile.java:47 JsonProfile.<init>
    pub fn from_reader(mut input_stream: impl Read) -> io::Result<JsonProfile> {
        let mut bytes = Vec::new();
        input_stream.read_to_end(&mut bytes)?;
        // InputStreamReader(inputStream, ISO_8859_1).
        let text = crate::string_encoding::latin1_decode(&bytes);
        let document: Value =
            serde_json::from_str(&text).map_err(|e| io::Error::other(e.to_string()))?;
        let mut build_metadata = None;
        let mut phase_summary_statistics = None;
        let mut trace_events = None;
        if let Value::Object(obj) = &document {
            for (object_key, value) in obj {
                if object_key == "otherData" {
                    build_metadata = Some(parse_build_metadata(value)?);
                } else if object_key == "traceEvents" {
                    let events = TraceEvent::parse_trace_events(value)?;
                    let mut stats = PhaseSummaryStatistics::default();
                    let mut last_phase_event: Option<&TraceEvent> = None;
                    let mut max_end_time: i64 = 0;
                    for trace_event in &events {
                        if let Some(ts) = trace_event.timestamp {
                            let mut cur_end_time = ts;
                            if let Some(d) = trace_event.duration {
                                cur_end_time += d;
                            }
                            if cur_end_time > max_end_time {
                                max_end_time = cur_end_time;
                            }
                        }
                        if trace_event.category() == Some(ProfilerTask::PHASE.description()) {
                            if let Some(last) = last_phase_event {
                                stats.add_profile_phase(
                                    ProfilePhase::get_phase_from_description(&last.name),
                                    trace_event.timestamp.expect("timestamp")
                                        - last.timestamp.expect("timestamp"),
                                );
                            }
                            last_phase_event = Some(trace_event);
                        }
                    }
                    if let Some(last) = last_phase_event {
                        stats.add_profile_phase(
                            ProfilePhase::get_phase_from_description(&last.name),
                            max_end_time - last.timestamp.expect("timestamp"),
                        );
                    }
                    phase_summary_statistics = Some(stats);
                    trace_events = Some(events);
                }
            }
        }
        match trace_events {
            Some(trace_events) => Ok(JsonProfile {
                build_metadata,
                phase_summary_statistics,
                trace_events,
            }),
            None => Err(io::Error::other(
                "Corrupted profile file: couldn't find 'traceEvents'.",
            )),
        }
    }

    pub fn get_phase_summary_statistics(&self) -> Option<&PhaseSummaryStatistics> {
        self.phase_summary_statistics.as_ref()
    }

    pub fn get_trace_events(&self) -> &[TraceEvent] {
        &self.trace_events
    }

    pub fn get_build_metadata(&self) -> Option<&BuildMetadata> {
        self.build_metadata.as_ref()
    }
}

// Java: com/google/devtools/build/lib/profiler/JsonProfile.java:110 JsonProfile.parseBuildMetadata
fn parse_build_metadata(value: &Value) -> io::Result<BuildMetadata> {
    let obj = value
        .as_object()
        .ok_or_else(|| io::Error::other("Expected BEGIN_OBJECT"))?;
    let mut metadata = BuildMetadata::default();
    for (key, v) in obj {
        match key.as_str() {
            "build_id" => metadata.build_id = Some(next_string(v)?),
            "date" => metadata.date = Some(next_string(v)?),
            "output_base" => metadata.output_base = Some(next_string(v)?),
            _ => {}
        }
    }
    Ok(metadata)
}
