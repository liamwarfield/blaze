//! `ProcMeminfoParser` (from `lib/unix`).
//!
//! PORT: Java's `com.google.devtools.build.lib.unix.ProcMeminfoParser`; no other crate had a
//! port, and only `ServerWatcherRunnable` uses it, so it lives here.

use std::collections::HashMap;
use std::io;

use blaze_util::string_encoding::latin1_decode;

/// Exception thrown when /proc/meminfo does not have a requested key. Should be tolerated.
// Java: com/google/devtools/build/lib/unix/ProcMeminfoParser.java:108 ProcMeminfoParser.KeywordNotFoundException
#[derive(Debug, thiserror::Error)]
#[error("Can't locate {0} in the /proc/meminfo")]
pub struct KeywordNotFoundException(String);

/// Parse and return information from /proc/meminfo. In case of duplicate entries the first one
/// is used and other values are skipped.
// Java: com/google/devtools/build/lib/unix/ProcMeminfoParser.java:32 ProcMeminfoParser
#[derive(Clone, Debug)]
pub struct ProcMeminfoParser {
    mem_info: HashMap<String, i64>,
}

impl ProcMeminfoParser {
    // Java: com/google/devtools/build/lib/unix/ProcMeminfoParser.java:34 ProcMeminfoParser.FILE
    pub const FILE: &'static str = "/proc/meminfo";

    /// Populates memory information by reading /proc/meminfo.
    // Java: com/google/devtools/build/lib/unix/ProcMeminfoParser.java:42 ProcMeminfoParser.<init>
    pub fn new() -> io::Result<ProcMeminfoParser> {
        Self::from_file(Self::FILE)
    }

    // Java: com/google/devtools/build/lib/unix/ProcMeminfoParser.java:47 ProcMeminfoParser.<init>
    pub fn from_file(file_name: &str) -> io::Result<ProcMeminfoParser> {
        // Files.readLines(new File(fileName), Charset.defaultCharset()): the server's default
        // charset is ISO-8859-1. The file name is an internal string.
        let bytes = std::fs::read(&*blaze_util::string_encoding::internal_to_platform(
            file_name,
        ))?;
        Ok(Self::from_content(&latin1_decode(&bytes)))
    }

    /// The parsing half of the constructor, on the file's contents.
    pub fn from_content(content: &str) -> ProcMeminfoParser {
        let mut new_mem_info = HashMap::new();
        for line in read_lines(content) {
            let Some(colon) = line.find(':') else {
                continue;
            };
            let keyword = &line[..colon];
            let val_string = &line[colon + 1..];
            // Long.parseLong(CharMatcher.inRange('0', '9').retainFrom(valString))
            let digits: String = val_string.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(val) = digits.parse::<i64>() {
                new_mem_info.entry(keyword.to_string()).or_insert(val);
            }
            // Otherwise ignore: we'll fail later if somebody tries to capture this value.
        }
        ProcMeminfoParser {
            mem_info: new_mem_info,
        }
    }

    /// Gets a named field in KB.
    // Java: com/google/devtools/build/lib/unix/ProcMeminfoParser.java:68 ProcMeminfoParser.getRamKb
    pub fn get_ram_kb(&self, keyword: &str) -> Result<i64, KeywordNotFoundException> {
        self.mem_info
            .get(keyword)
            .copied()
            .ok_or_else(|| KeywordNotFoundException(keyword.to_string()))
    }

    /// Return the total physical memory.
    // Java: com/google/devtools/build/lib/unix/ProcMeminfoParser.java:77 ProcMeminfoParser.getTotalKb
    pub fn get_total_kb(&self) -> Result<i64, KeywordNotFoundException> {
        self.get_ram_kb("MemTotal")
    }

    /// Convert KB to MB.
    // Java: com/google/devtools/build/lib/unix/ProcMeminfoParser.java:84 ProcMeminfoParser.kbToMb
    pub fn kb_to_mb(kb: i64) -> f64 {
        (kb >> 10) as f64
    }

    /// Reads the amount of *available* memory as reported by the kernel. This corresponds to the
    /// MemAvailable line in /proc/meminfo.
    // Java: com/google/devtools/build/lib/unix/ProcMeminfoParser.java:93 ProcMeminfoParser.getFreeRamKb
    pub fn get_free_ram_kb(&self) -> Result<i64, KeywordNotFoundException> {
        if self.mem_info.contains_key("MemAvailable") {
            return self.get_ram_kb("MemAvailable");
        }
        // We have no MemAvailable in /proc/meminfo; fall back to the previous estimation.
        Ok(self.get_ram_kb("MemTotal")?
            - self.get_ram_kb("Active")?
            // Blaze doesn't want to use more than a third of inactive ram...
            - (self.get_ram_kb("Inactive")? as f64 * 0.3) as i64
            // ...and doesn't want to assume more than 80% of the slab memory can be reallocated.
            - (self.get_ram_kb("Slab")? as f64 * 0.8) as i64)
        // That said, this estimate will be more inaccurate as it diverges from kernel internals.
    }
}

/// Guava `Files.readLines`: lines split at `\n`, `\r` or `\r\n`, without a trailing empty line.
fn read_lines(content: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = content;
    while !rest.is_empty() {
        match rest.find(['\n', '\r']) {
            None => {
                lines.push(rest);
                break;
            }
            Some(i) => {
                lines.push(&rest[..i]);
                let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[i + skip..];
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/unix/ProcMeminfoParserTest.java
    use super::*;

    #[test]
    fn mem_info() {
        let meminfo_content = [
            "MemTotal:      3091732 kB",
            "MemFree:       2167344 kB",
            "MemAvailable:   14717640 kB",
            "Buffers:         60644 kB",
            "Cached:         509940 kB",
            "SwapCached:          0 kB",
            "Active:         636892 kB",
            "Inactive:       212760 kB",
            "HighTotal:           0 kB",
            "HighFree:            0 kB",
            "LowTotal:      3091732 kB",
            "LowFree:       2167344 kB",
            "SwapTotal:     9124880 kB",
            "SwapFree:      9124880 kB",
            "Dirty:               0 kB",
            "Writeback:           0 kB",
            "AnonPages:      279028 kB",
            "Mapped:          54404 kB",
            "Slab:            42820 kB",
            "PageTables:       5184 kB",
            "NFS_Unstable:        0 kB",
            "Bounce:              0 kB",
            "CommitLimit:  10670744 kB",
            "Committed_AS:   665840 kB",
            "VmallocTotal: 34359738367 kB",
            "VmallocUsed:    300484 kB",
            "VmallocChunk: 34359437307 kB",
            "HugePages_Total:     0",
            "HugePages_Free:      0",
            "HugePages_Rsvd:      0",
            "Hugepagesize:     2048 kB",
            "Bogus: not_a_number",
            "Bogus2: 1000000000000000000000000000000000000000000000000 kB",
            "Writeback:         123 kB",
            "Not even a valid line",
        ]
        .join("\n");

        let dir = tempfile::tempdir().unwrap();
        let meminfo_file = dir.path().join("test_meminfo");
        std::fs::write(&meminfo_file, meminfo_content).unwrap();
        let mem_info = ProcMeminfoParser::from_file(meminfo_file.to_str().unwrap()).unwrap();

        assert_eq!(mem_info.get_free_ram_kb().unwrap(), 14717640);
        assert_eq!(mem_info.get_ram_kb("Cached").unwrap(), 509940);
        assert_eq!(mem_info.get_total_kb().unwrap(), 3091732);
        assert_eq!(mem_info.get_ram_kb("Writeback").unwrap(), 0);
        assert!(mem_info.get_ram_kb("Bogus").is_err());
        assert!(mem_info.get_ram_kb("Bogus2").is_err());
    }

    #[test]
    fn test_old_kernel_fallback() {
        let meminfo_content = [
            "MemTotal:      3091732 kB",
            "Active:         636892 kB",
            "Inactive:       212760 kB",
            "Slab:            42820 kB",
        ]
        .join("\n");

        let dir = tempfile::tempdir().unwrap();
        let meminfo_file = dir.path().join("test_meminfo");
        std::fs::write(&meminfo_file, meminfo_content).unwrap();
        let mem_info = ProcMeminfoParser::from_file(meminfo_file.to_str().unwrap()).unwrap();
        assert_eq!(mem_info.get_free_ram_kb().unwrap(), 2356756);
    }
}
