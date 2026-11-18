//! Port of `com.google.devtools.build.lib.util.CPU`.
// PORT: move to blaze_util.

use std::sync::OnceLock;

/// Detects the CPU of the running process and returns a describing enum value.
// Java: com/google/devtools/build/lib/util/CPU.java:23 CPU
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CPU {
    X86_32,
    X86_64,
    Ppc,
    Arm,
    Aarch64,
    S390x,
    Mips64,
    Riscv64,
    Unknown,
}

impl CPU {
    /// `CPU.values()`, in declaration order.
    pub const VALUES: [CPU; 9] = [
        CPU::X86_32,
        CPU::X86_64,
        CPU::Ppc,
        CPU::Arm,
        CPU::Aarch64,
        CPU::S390x,
        CPU::Mips64,
        CPU::Riscv64,
        CPU::Unknown,
    ];

    // Java: com/google/devtools/build/lib/util/CPU.java:42 CPU.getCanonicalName
    pub fn get_canonical_name(self) -> &'static str {
        match self {
            CPU::X86_32 => "x86_32",
            CPU::X86_64 => "x86_64",
            CPU::Ppc => "ppc",
            CPU::Arm => "arm",
            CPU::Aarch64 => "aarch64",
            CPU::S390x => "s390x",
            CPU::Mips64 => "mips64",
            CPU::Riscv64 => "riscv64",
            CPU::Unknown => "unknown",
        }
    }

    fn archs(self) -> &'static [&'static str] {
        match self {
            CPU::X86_32 => &["i386", "i486", "i586", "i686", "i786", "x86"],
            CPU::X86_64 => &["amd64", "x86_64", "x64"],
            CPU::Ppc => &["ppc", "ppc64", "ppc64le"],
            CPU::Arm => &["arm", "armv7l"],
            CPU::Aarch64 => &["aarch64"],
            CPU::S390x => &["s390x", "s390"],
            CPU::Mips64 => &["mips64el", "mips64"],
            CPU::Riscv64 => &["riscv64"],
            CPU::Unknown => &[],
        }
    }

    /// The current CPU.
    // Java: com/google/devtools/build/lib/util/CPU.java:51 CPU.getCurrent
    pub fn get_current() -> CPU {
        static HOST_CPU: OnceLock<CPU> = OnceLock::new();
        *HOST_CPU.get_or_init(CPU::determine_current_cpu)
    }

    /// The JVM's `os.arch` on Linux for the architecture this binary was built for.
    fn os_arch() -> &'static str {
        match std::env::consts::ARCH {
            "x86_64" => "amd64",
            "x86" => "i386",
            "powerpc64" if cfg!(target_endian = "little") => "ppc64le",
            "powerpc64" => "ppc64",
            "powerpc" => "ppc",
            "mips64" if cfg!(target_endian = "little") => "mips64el",
            other => other,
        }
    }

    // Java: com/google/devtools/build/lib/util/CPU.java:55 CPU.determineCurrentCpu
    fn determine_current_cpu() -> CPU {
        let current_arch = CPU::os_arch();
        for cpu in CPU::VALUES {
            if cpu.archs().contains(&current_arch) {
                return cpu;
            }
        }
        CPU::Unknown
    }
}
