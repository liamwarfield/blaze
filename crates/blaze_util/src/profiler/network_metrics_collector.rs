//! Ports of `NetworkMetricsCollector.java` and `SystemNetworkStats.java`.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::OnceLock;

use blaze_protos::build_event_stream::build_metrics::NetworkMetrics;
use blaze_protos::build_event_stream::build_metrics::network_metrics::SystemNetworkStats as SystemNetworkStatsProto;
use parking_lot::Mutex;

/// Value class for network IO counters.
// Java: com/google/devtools/build/lib/profiler/SystemNetworkStats.java:43 SystemNetworkStats.NetIoCounter
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetIoCounter {
    pub bytes_sent: i64,
    pub bytes_recv: i64,
    pub packets_sent: i64,
    pub packets_recv: i64,
}

impl NetIoCounter {
    pub fn create(bytes_sent: i64, bytes_recv: i64, packets_sent: i64, packets_recv: i64) -> Self {
        NetIoCounter {
            bytes_sent,
            bytes_recv,
            packets_sent,
            packets_recv,
        }
    }
}

/// Returns a map from network interface name to the respective I/O counters.
// Java: com/google/devtools/build/lib/profiler/SystemNetworkStats.java:51 SystemNetworkStats.getNetIoCounters
pub fn get_net_io_counters() -> io::Result<HashMap<String, NetIoCounter>> {
    let mut counters_map = HashMap::new();
    get_net_io_counters_linux(&mut counters_map, "/proc/net/dev")?;
    Ok(counters_map)
}

/// `getNetIoCountersLinux`, reading `file` (Java: `/proc/net/dev`).
// Java: com/google/devtools/build/lib/profiler/SystemNetworkStats.java:64 SystemNetworkStats.getNetIoCountersLinux
pub fn get_net_io_counters_linux(
    counters_map: &mut HashMap<String, NetIoCounter>,
    file: &str,
) -> io::Result<()> {
    let bytes = std::fs::read(file)?;
    let text = crate::string_encoding::java_new_string_utf8(&bytes);
    let lines: Vec<&str> = split_lines(&text);

    // skip table header (first 2 lines)
    for line in lines.iter().skip(2) {
        let Some(colon_at) = line.find(':') else {
            continue;
        };
        let name = line[..colon_at].trim().to_string();
        // Splitter.on(" ").omitEmptyStrings().trimResults(), Long::parseUnsignedLong.
        let mut fields = Vec::new();
        for field in line[colon_at + 1..].split(' ') {
            let field = field.trim();
            if field.is_empty() {
                continue;
            }
            match field.parse::<u64>() {
                Ok(v) => fields.push(v as i64),
                Err(_) => {
                    return Err(io::Error::other(format!(
                        "NumberFormatException: For input string: \"{field}\""
                    )));
                }
            }
        }
        if fields.len() > 9 {
            let bytes_recv = fields[0];
            let packets_recv = fields[1];
            let bytes_sent = fields[8];
            let packets_sent = fields[9];
            counters_map.insert(
                name,
                NetIoCounter::create(bytes_sent, bytes_recv, packets_sent, packets_recv),
            );
        }
    }
    Ok(())
}

/// `Files.readAllLines`: lines split at `\n`, `\r\n` or `\r`.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find(['\n', '\r']) {
            Some(i) => {
                lines.push(&rest[..i]);
                let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[i + skip..];
            }
            None => {
                lines.push(rest);
                break;
            }
        }
    }
    lines
}

/// Aggregated system network usages over all interfaces except local loopback.
// Java: com/google/devtools/build/lib/profiler/NetworkMetricsCollector.java:140 NetworkMetricsCollector.SystemNetworkUsages
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemNetworkUsages {
    pub bytes_sent_per_sec: f64,
    pub bytes_recv_per_sec: f64,
    pub packets_sent_per_sec: f64,
    pub packets_recv_per_sec: f64,
}

impl SystemNetworkUsages {
    pub fn megabits_sent_per_sec(&self) -> f64 {
        bytes_per_sec_to_megabits_per_sec(self.bytes_sent_per_sec)
    }

    pub fn megabits_recv_per_sec(&self) -> f64 {
        bytes_per_sec_to_megabits_per_sec(self.bytes_recv_per_sec)
    }
}

#[derive(Default)]
struct State {
    loopback_interface_names: Option<HashSet<String>>,
    previous_network_io_counters: Option<HashMap<String, NetIoCounter>>,
    system_network_stats: SystemNetworkStatsProto,
}

/// Collects and populates network metrics during an invocation.
// Java: com/google/devtools/build/lib/profiler/NetworkMetricsCollector.java:29 NetworkMetricsCollector
pub struct NetworkMetricsCollector {
    state: Mutex<State>,
}

static INSTANCE: OnceLock<NetworkMetricsCollector> = OnceLock::new();

impl NetworkMetricsCollector {
    /// The metrics collector (a static singleton instance). Inactive by default.
    pub fn instance() -> &'static NetworkMetricsCollector {
        INSTANCE.get_or_init(|| NetworkMetricsCollector {
            state: Mutex::new(State::default()),
        })
    }

    // Java: com/google/devtools/build/lib/profiler/NetworkMetricsCollector.java:47 NetworkMetricsCollector.collectMetrics
    pub fn collect_metrics(&self) -> Option<NetworkMetrics> {
        let state = self.state.lock();
        if state.system_network_stats.bytes_recv == 0 || state.system_network_stats.bytes_sent == 0
        {
            return None;
        }

        Some(NetworkMetrics {
            system_network_stats: Some(state.system_network_stats),
        })
    }

    // Java: com/google/devtools/build/lib/profiler/NetworkMetricsCollector.java:56 NetworkMetricsCollector.collectSystemNetworkUsages
    pub fn collect_system_network_usages(&self, delta_nanos: f64) -> Option<SystemNetworkUsages> {
        let mut state = self.state.lock();
        if state.loopback_interface_names.is_none() {
            // PORT: Java logs a warning (GoogleLogger) when this fails; Blaze has no java.log.
            state.loopback_interface_names = get_loopback_interface_names().ok();
        }

        // PORT: Java logs a warning when this fails.
        let next_network_io_counters = get_net_io_counters().ok();

        if state.previous_network_io_counters.is_none() {
            state.previous_network_io_counters = next_network_io_counters.clone();
        }

        let mut usages = None;
        if let (Some(previous), Some(next_counters)) = (
            state.previous_network_io_counters.as_ref(),
            next_network_io_counters.as_ref(),
        ) {
            let mut delta_bytes_sent: i64 = 0;
            let mut delta_bytes_recv: i64 = 0;
            let mut delta_packets_sent: i64 = 0;
            let mut delta_packets_recv: i64 = 0;
            let loopback = state
                .loopback_interface_names
                .as_ref()
                .expect("NullPointerException: loopbackInterfaceNames");
            for (name, previous) in previous {
                if loopback.contains(name) {
                    continue;
                }
                if let Some(next) = next_counters.get(name) {
                    delta_bytes_sent += calc_delta(previous.bytes_sent, next.bytes_sent);
                    delta_bytes_recv += calc_delta(previous.bytes_recv, next.bytes_recv);
                    delta_packets_sent += calc_delta(previous.packets_sent, next.packets_sent);
                    delta_packets_recv += calc_delta(previous.packets_recv, next.packets_recv);
                }
            }

            let stats = &mut state.system_network_stats;
            stats.bytes_recv = stats.bytes_recv.wrapping_add(delta_bytes_recv as u64);
            stats.bytes_sent = stats.bytes_sent.wrapping_add(delta_bytes_sent as u64);
            stats.packets_recv = stats.packets_recv.wrapping_add(delta_packets_recv as u64);
            stats.packets_sent = stats.packets_sent.wrapping_add(delta_packets_sent as u64);

            let u = SystemNetworkUsages {
                bytes_sent_per_sec: calc_value_per_sec(delta_bytes_sent, delta_nanos),
                bytes_recv_per_sec: calc_value_per_sec(delta_bytes_recv, delta_nanos),
                packets_sent_per_sec: calc_value_per_sec(delta_packets_sent, delta_nanos),
                packets_recv_per_sec: calc_value_per_sec(delta_packets_recv, delta_nanos),
            };

            // Java compares a double with the (signed) long getter.
            if u.bytes_sent_per_sec > stats.peak_bytes_sent_per_sec as i64 as f64 {
                stats.peak_bytes_sent_per_sec = u.bytes_sent_per_sec as i64 as u64;
            }
            if u.bytes_recv_per_sec > stats.peak_bytes_recv_per_sec as i64 as f64 {
                stats.peak_bytes_recv_per_sec = u.bytes_recv_per_sec as i64 as u64;
            }
            if u.packets_sent_per_sec > stats.peak_packets_sent_per_sec as i64 as f64 {
                stats.peak_packets_sent_per_sec = u.packets_sent_per_sec as i64 as u64;
            }
            if u.packets_recv_per_sec > stats.peak_packets_recv_per_sec as i64 as f64 {
                stats.peak_packets_recv_per_sec = u.packets_recv_per_sec as i64 as u64;
            }
            usages = Some(u);
        }

        state.previous_network_io_counters = next_network_io_counters;

        usages
    }
}

/// `NetworkInterface.getNetworkInterfaces()` filtered by `isLoopback()`.
///
/// PORT: getifaddrs(3) and `IFF_LOOPBACK`, which is what the JDK reads on Linux.
// Java: com/google/devtools/build/lib/profiler/NetworkMetricsCollector.java:162 NetworkMetricsCollector.getLoopbackInterfaceNames
fn get_loopback_interface_names() -> io::Result<HashSet<String>> {
    let mut result = HashSet::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs allocates the list it returns; freeifaddrs releases it below.
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut cur = ifap;
    while !cur.is_null() {
        // SAFETY: cur points into the list getifaddrs returned.
        let ifa = unsafe { &*cur };
        if ifa.ifa_flags & (libc::IFF_LOOPBACK as u32) != 0 && !ifa.ifa_name.is_null() {
            // SAFETY: ifa_name is a NUL-terminated string owned by the list.
            let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) };
            result.insert(
                crate::string_encoding::latin1_decode(name.to_bytes()).into_owned(),
            );
        }
        cur = ifa.ifa_next;
    }
    // SAFETY: ifap came from getifaddrs.
    unsafe { libc::freeifaddrs(ifap) };
    Ok(result)
}

// Java: com/google/devtools/build/lib/profiler/NetworkMetricsCollector.java:174 NetworkMetricsCollector.calcDelta
fn calc_delta(prev: i64, next: i64) -> i64 {
    if (next as u64) < (prev as u64) {
        return next;
    }
    next.wrapping_sub(prev)
}

// Java: com/google/devtools/build/lib/profiler/NetworkMetricsCollector.java:181 NetworkMetricsCollector.calcValuePerSec
fn calc_value_per_sec(delta_value: i64, delta_nanos: f64) -> f64 {
    delta_value as f64 / delta_nanos * 1e9
}

// Java: com/google/devtools/build/lib/profiler/NetworkMetricsCollector.java:185 NetworkMetricsCollector.bytesPerSecToMegabitsPerSec
fn bytes_per_sec_to_megabits_per_sec(bytes_per_sec: f64) -> f64 {
    bytes_per_sec / 1e6 * 8.0
}
