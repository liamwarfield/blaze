//! Port of `com.google.devtools.build.lib.shell.ExecutionStatistics`, with a decoder for
//! `src/main/protobuf/execution_statistics.proto`.

use std::io::{self, Read};
use std::time::Duration;

use blaze_vfs::Path;

/// `tools.protos.ResourceUsage` (proto3).
// PORT: move to blaze_protos when that crate exists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProtosResourceUsage {
    pub utime_sec: i64,
    pub utime_usec: i64,
    pub stime_sec: i64,
    pub stime_usec: i64,
    pub maxrss: i64,
    pub ixrss: i64,
    pub idrss: i64,
    pub isrss: i64,
    pub minflt: i64,
    pub majflt: i64,
    pub nswap: i64,
    pub inblock: i64,
    pub oublock: i64,
    pub msgsnd: i64,
    pub msgrcv: i64,
    pub nsignals: i64,
    pub nvcsw: i64,
    pub nivcsw: i64,
}

/// `tools.protos.ExecutionStatistics` (proto3).
// PORT: move to blaze_protos when that crate exists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProtosExecutionStatistics {
    pub resource_usage: Option<ProtosResourceUsage>,
}

fn invalid(message: &str) -> io::Error {
    // Java: com.google.protobuf.InvalidProtocolBufferException (an IOException).
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

const TRUNCATED: &str = "While parsing a protocol message, the input ended unexpectedly in the middle of a field.  This could mean either that the input has been truncated or that an embedded message misreported its own length.";

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn varint(&mut self) -> io::Result<u64> {
        let mut result: u64 = 0;
        for i in 0..10 {
            let Some(&b) = self.buf.get(self.pos) else {
                return Err(invalid(TRUNCATED));
            };
            self.pos += 1;
            result |= ((b & 0x7f) as u64) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(result);
            }
        }
        Err(invalid("CodedInputStream encountered a malformed varint."))
    }

    fn bytes(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.buf.len() - self.pos < n {
            return Err(invalid(TRUNCATED));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn skip_field(&mut self, tag: u64) -> io::Result<()> {
        match tag & 7 {
            0 => {
                self.varint()?;
            }
            1 => {
                self.bytes(8)?;
            }
            2 => {
                let n = self.varint()? as usize;
                self.bytes(n)?;
            }
            3 => loop {
                if self.at_end() {
                    return Err(invalid(TRUNCATED));
                }
                let t = self.varint()?;
                if t & 7 == 4 {
                    if t >> 3 != tag >> 3 {
                        return Err(invalid(
                            "Protocol message end-group tag did not match expected tag.",
                        ));
                    }
                    break;
                }
                self.skip_field(t)?;
            },
            5 => {
                self.bytes(4)?;
            }
            _ => return Err(invalid("Protocol message tag had invalid wire type.")),
        }
        Ok(())
    }

    fn tag(&mut self) -> io::Result<u64> {
        let tag = self.varint()?;
        if tag >> 3 == 0 {
            return Err(invalid("Protocol message contained an invalid tag (zero)."));
        }
        Ok(tag)
    }
}

impl ProtosResourceUsage {
    fn merge_from(&mut self, buf: &[u8]) -> io::Result<()> {
        let mut r = Reader { buf, pos: 0 };
        while !r.at_end() {
            let tag = r.tag()?;
            if tag & 7 != 0 || !(1..=18).contains(&(tag >> 3)) {
                r.skip_field(tag)?;
                continue;
            }
            let v = r.varint()? as i64;
            let field = match tag >> 3 {
                1 => &mut self.utime_sec,
                2 => &mut self.utime_usec,
                3 => &mut self.stime_sec,
                4 => &mut self.stime_usec,
                5 => &mut self.maxrss,
                6 => &mut self.ixrss,
                7 => &mut self.idrss,
                8 => &mut self.isrss,
                9 => &mut self.minflt,
                10 => &mut self.majflt,
                11 => &mut self.nswap,
                12 => &mut self.inblock,
                13 => &mut self.oublock,
                14 => &mut self.msgsnd,
                15 => &mut self.msgrcv,
                16 => &mut self.nsignals,
                17 => &mut self.nvcsw,
                _ => &mut self.nivcsw,
            };
            *field = v;
        }
        Ok(())
    }

    /// Serializes in the canonical (field-number) order, skipping default values.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let fields = [
            self.utime_sec,
            self.utime_usec,
            self.stime_sec,
            self.stime_usec,
            self.maxrss,
            self.ixrss,
            self.idrss,
            self.isrss,
            self.minflt,
            self.majflt,
            self.nswap,
            self.inblock,
            self.oublock,
            self.msgsnd,
            self.msgrcv,
            self.nsignals,
            self.nvcsw,
            self.nivcsw,
        ];
        for (i, v) in fields.iter().enumerate() {
            if *v != 0 {
                write_varint(&mut out, (i as u64 + 1) << 3);
                write_varint(&mut out, *v as u64);
            }
        }
        out
    }
}

fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        if v < 0x80 {
            out.push(v as u8);
            return;
        }
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
}

impl ProtosExecutionStatistics {
    /// `ExecutionStatistics.parseFrom(InputStream)`.
    pub fn parse_from(buf: &[u8]) -> io::Result<ProtosExecutionStatistics> {
        let mut msg = ProtosExecutionStatistics::default();
        let mut r = Reader { buf, pos: 0 };
        while !r.at_end() {
            let tag = r.tag()?;
            if tag == (1 << 3) | 2 {
                let n = r.varint()? as usize;
                let bytes = r.bytes(n)?;
                msg.resource_usage
                    .get_or_insert_with(ProtosResourceUsage::default)
                    .merge_from(bytes)?;
            } else {
                r.skip_field(tag)?;
            }
        }
        Ok(msg)
    }

    /// `writeTo(OutputStream)`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(ru) = &self.resource_usage {
            let inner = ru.to_bytes();
            write_varint(&mut out, (1 << 3) | 2);
            write_varint(&mut out, inner.len() as u64);
            out.extend_from_slice(&inner);
        }
        out
    }
}

/// Provides execution statistics (e.g. resource usage) for external commands.
// Java: com/google/devtools/build/lib/shell/ExecutionStatistics.java:26 ExecutionStatistics
pub struct ExecutionStatistics;

impl ExecutionStatistics {
    /// Provides execution statistics based on a `execution_statistics.proto` file.
    // Java: com/google/devtools/build/lib/shell/ExecutionStatistics.java:33 ExecutionStatistics.getResourceUsage
    pub fn get_resource_usage(
        execution_statistics_proto_path: &Path,
    ) -> io::Result<Option<ExecutionStatisticsResourceUsage>> {
        let mut proto_input_stream = match execution_statistics_proto_path.get_input_stream() {
            Ok(s) => s,
            // Collecting resource usage is best-effort and the file may be missing if the wrapper
            // around the command terminated abnormally.
            // PORT: Java catches FileNotFoundException; blaze_vfs reports it as NotFound.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut buf = Vec::new();
        proto_input_stream.read_to_end(&mut buf)?;
        let execution_statistics_proto = ProtosExecutionStatistics::parse_from(&buf)?;
        Ok(execution_statistics_proto
            .resource_usage
            .map(ExecutionStatisticsResourceUsage::new))
    }
}

/// `Duration.ofSeconds(seconds, nanoAdjustment)`.
fn duration_of_seconds(seconds: i64, nano_adjustment: i64) -> Duration {
    let secs = seconds
        .checked_add(nano_adjustment.div_euclid(1_000_000_000))
        .expect("ArithmeticException: long overflow");
    let nanos = nano_adjustment.rem_euclid(1_000_000_000) as u32;
    // PORT: java.time.Duration may be negative; std::time::Duration may not. getrusage never
    // reports negative times.
    let secs = u64::try_from(secs).unwrap_or_else(|_| {
        blaze_util::unported!("negative java.time.Duration in ExecutionStatistics.ResourceUsage")
    });
    Duration::new(secs, nanos)
}

/// Provides resource usage statistics for command execution, derived from the getrusage()
/// system call.
// Java: com/google/devtools/build/lib/shell/ExecutionStatistics.java:55 ExecutionStatistics.ResourceUsage
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionStatisticsResourceUsage {
    resource_usage_proto: ProtosResourceUsage,
}

impl ExecutionStatisticsResourceUsage {
    pub fn new(resource_usage_proto: ProtosResourceUsage) -> Self {
        ExecutionStatisticsResourceUsage {
            resource_usage_proto,
        }
    }

    /// Returns the user time for command execution, if available.
    pub fn get_user_execution_time(&self) -> Duration {
        let p = &self.resource_usage_proto;
        duration_of_seconds(p.utime_sec, p.utime_usec.wrapping_mul(1000))
    }

    /// Returns the system time for command execution, if available.
    pub fn get_system_execution_time(&self) -> Duration {
        let p = &self.resource_usage_proto;
        duration_of_seconds(p.stime_sec, p.stime_usec.wrapping_mul(1000))
    }

    pub fn get_maximum_resident_set_size(&self) -> i64 {
        self.resource_usage_proto.maxrss
    }

    pub fn get_integral_shared_memory_size(&self) -> i64 {
        self.resource_usage_proto.ixrss
    }

    pub fn get_integral_unshared_data_size(&self) -> i64 {
        self.resource_usage_proto.idrss
    }

    pub fn get_integral_unshared_stack_size(&self) -> i64 {
        self.resource_usage_proto.isrss
    }

    pub fn get_page_reclaims(&self) -> i64 {
        self.resource_usage_proto.minflt
    }

    pub fn get_page_faults(&self) -> i64 {
        self.resource_usage_proto.majflt
    }

    pub fn get_swaps(&self) -> i64 {
        self.resource_usage_proto.nswap
    }

    pub fn get_block_input_operations(&self) -> i64 {
        self.resource_usage_proto.inblock
    }

    pub fn get_block_output_operations(&self) -> i64 {
        self.resource_usage_proto.oublock
    }

    pub fn get_ipc_messages_sent(&self) -> i64 {
        self.resource_usage_proto.msgsnd
    }

    pub fn get_ipc_messages_received(&self) -> i64 {
        self.resource_usage_proto.msgrcv
    }

    pub fn get_signals_received(&self) -> i64 {
        self.resource_usage_proto.nsignals
    }

    pub fn get_voluntary_context_switches(&self) -> i64 {
        self.resource_usage_proto.nvcsw
    }

    pub fn get_involuntary_context_switches(&self) -> i64 {
        self.resource_usage_proto.nivcsw
    }
}
