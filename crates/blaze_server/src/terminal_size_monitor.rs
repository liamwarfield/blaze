//! `TerminalSizeMonitor`: tracks terminal size updates for a single running command.

use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;

/// Listener for terminal size updates.
// Java: com/google/devtools/build/lib/server/TerminalSizeMonitor.java:26 TerminalSizeMonitor.Listener
pub trait TerminalSizeMonitorListener: Send + Sync {
    fn terminal_size_changed(&self, columns: i32, rows: i32);
}

impl<F: Fn(i32, i32) + Send + Sync> TerminalSizeMonitorListener for F {
    fn terminal_size_changed(&self, columns: i32, rows: i32) {
        self(columns, rows)
    }
}

struct Locked {
    listeners: Vec<Arc<dyn TerminalSizeMonitorListener>>,
    columns: i32,
    rows: i32,
}

/// Tracks terminal size updates for a single running command.
// Java: com/google/devtools/build/lib/server/TerminalSizeMonitor.java:21 TerminalSizeMonitor
pub struct TerminalSizeMonitor {
    lock: Mutex<Locked>,
    enabled: bool,
}

impl TerminalSizeMonitor {
    // Java: com/google/devtools/build/lib/server/TerminalSizeMonitor.java:22 TerminalSizeMonitor.NOOP
    pub fn noop() -> Arc<TerminalSizeMonitor> {
        static NOOP: LazyLock<Arc<TerminalSizeMonitor>> =
            LazyLock::new(|| Arc::new(TerminalSizeMonitor::with_enabled(false)));
        NOOP.clone()
    }

    // Java: com/google/devtools/build/lib/server/TerminalSizeMonitor.java:35 TerminalSizeMonitor.<init>
    pub fn new() -> TerminalSizeMonitor {
        TerminalSizeMonitor::with_enabled(true)
    }

    // Java: com/google/devtools/build/lib/server/TerminalSizeMonitor.java:39 TerminalSizeMonitor.<init>
    fn with_enabled(enabled: bool) -> TerminalSizeMonitor {
        TerminalSizeMonitor {
            lock: Mutex::new(Locked {
                listeners: Vec::new(),
                columns: 0,
                rows: 0,
            }),
            enabled,
        }
    }

    // Java: com/google/devtools/build/lib/server/TerminalSizeMonitor.java:43 TerminalSizeMonitor.addListener
    pub fn add_listener(&self, listener: Arc<dyn TerminalSizeMonitorListener>) {
        if !self.enabled {
            return;
        }
        let (columns_snapshot, rows_snapshot) = {
            let mut locked = self.lock.lock();
            locked.listeners.push(listener.clone());
            (locked.columns, locked.rows)
        };
        if columns_snapshot > 0 {
            listener.terminal_size_changed(columns_snapshot, rows_snapshot);
        }
    }

    // Java: com/google/devtools/build/lib/server/TerminalSizeMonitor.java:59 TerminalSizeMonitor.updateTerminalSize
    pub fn update_terminal_size(&self, columns: i32, rows: i32) {
        if !self.enabled || columns <= 0 {
            return;
        }
        let listeners_snapshot = {
            let mut locked = self.lock.lock();
            if locked.columns == columns && locked.rows == rows {
                return;
            }
            locked.columns = columns;
            locked.rows = rows;
            locked.listeners.clone()
        };
        for listener in listeners_snapshot {
            listener.terminal_size_changed(columns, rows);
        }
    }
}
