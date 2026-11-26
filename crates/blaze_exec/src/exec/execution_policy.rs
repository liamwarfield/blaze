//! Port of `lib/exec/ExecutionPolicy.java`.

use std::fmt;

/// `ExecutionPolicy.Locality`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Locality {
    LocalOnly,
    RemoteOnly,
    Both,
}

/// Determines whether a Spawn is executable locally, remotely, or both.
// Java: com/google/devtools/build/lib/exec/ExecutionPolicy.java:22 ExecutionPolicy
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionPolicy {
    locality: Locality,
}

impl ExecutionPolicy {
    // Java: com/google/devtools/build/lib/exec/ExecutionPolicy.java:29 ExecutionPolicy.LOCAL_EXECUTION_ONLY
    pub const LOCAL_EXECUTION_ONLY: ExecutionPolicy = ExecutionPolicy {
        locality: Locality::LocalOnly,
    };
    // Java: com/google/devtools/build/lib/exec/ExecutionPolicy.java:32 ExecutionPolicy.REMOTE_EXECUTION_ONLY
    pub const REMOTE_EXECUTION_ONLY: ExecutionPolicy = ExecutionPolicy {
        locality: Locality::RemoteOnly,
    };
    // Java: com/google/devtools/build/lib/exec/ExecutionPolicy.java:35 ExecutionPolicy.ANYWHERE
    pub const ANYWHERE: ExecutionPolicy = ExecutionPolicy {
        locality: Locality::Both,
    };

    // Java: com/google/devtools/build/lib/exec/ExecutionPolicy.java:43 ExecutionPolicy.canRunRemotelyOnly
    pub fn can_run_remotely_only(&self) -> bool {
        self.locality == Locality::RemoteOnly
    }

    // Java: com/google/devtools/build/lib/exec/ExecutionPolicy.java:47 ExecutionPolicy.canRunRemotely
    pub fn can_run_remotely(&self) -> bool {
        self.locality != Locality::LocalOnly
    }

    // Java: com/google/devtools/build/lib/exec/ExecutionPolicy.java:51 ExecutionPolicy.canRunLocallyOnly
    pub fn can_run_locally_only(&self) -> bool {
        self.locality == Locality::LocalOnly
    }

    // Java: com/google/devtools/build/lib/exec/ExecutionPolicy.java:55 ExecutionPolicy.canRunLocally
    pub fn can_run_locally(&self) -> bool {
        self.locality != Locality::RemoteOnly
    }
}

impl fmt::Display for ExecutionPolicy {
    // Java: com/google/devtools/build/lib/exec/ExecutionPolicy.java:59 ExecutionPolicy.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.locality {
            Locality::LocalOnly => "LOCAL_ONLY",
            Locality::RemoteOnly => "REMOTE_ONLY",
            Locality::Both => "BOTH",
        })
    }
}
