//! Port of `packages/PackageLoadingListener.java`.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::package::package::Package;

/// The metrics of one package load.
// Java: com/google/devtools/build/lib/packages/PackageLoadingListener.java:53 PackageLoadingListener.Metrics
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    pub load_time_nanos: i64,
    pub glob_filesystem_operation_cost: i64,
}

/// Listener for package-loading events.
// Java: com/google/devtools/build/lib/packages/PackageLoadingListener.java:24 PackageLoadingListener
pub trait PackageLoadingListener: Send + Sync {
    /// Called after a package has been loaded successfully.
    ///
    /// PORT: the `StarlarkSemantics` and `LazyMacroExpansionPackages` parameters are not passed:
    /// the only listener of Bazel (`PackageMetricsPackageLoadingListener`) does not read them.
    // Java: com/google/devtools/build/lib/packages/PackageLoadingListener.java:49 PackageLoadingListener.onLoadingCompleteAndSuccessful
    fn on_loading_complete_and_successful(&self, pkg: &Package, metrics: &Metrics);
}

/// The server's listener: `PackageLoadingListener.create(modules' listeners)`, which
/// `BlazeRuntime` hands to its `PackageFactory`. `None` is `NOOP_LISTENER`.
///
/// PORT: the package factories of the port are built per command; the listener, which Java
/// builds once with the runtime, is the process's.
static LISTENER: RwLock<Option<Arc<dyn PackageLoadingListener>>> = RwLock::new(None);

/// Sets the server's listener (`BlazeRuntime.getPackageLoadingListener(modules, ...)`).
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1913 BlazeRuntime.getPackageLoadingListener
// Java: com/google/devtools/build/lib/packages/PackageLoadingListener.java:28 PackageLoadingListener.create
pub fn set_package_loading_listener(listeners: Vec<Arc<dyn PackageLoadingListener>>) {
    let listener: Option<Arc<dyn PackageLoadingListener>> = match listeners.len() {
        0 => None,
        1 => listeners.into_iter().next(),
        _ => Some(Arc::new(CompositeListener(listeners))),
    };
    *LISTENER.write() = listener;
}

/// The server's listener (`NOOP_LISTENER` when `None`).
pub fn package_loading_listener() -> Option<Arc<dyn PackageLoadingListener>> {
    LISTENER.read().clone()
}

/// `PackageLoadingListener.create` of more than one listener.
// Java: com/google/devtools/build/lib/packages/PackageLoadingListener.java:28 PackageLoadingListener.create
struct CompositeListener(Vec<Arc<dyn PackageLoadingListener>>);

impl PackageLoadingListener for CompositeListener {
    fn on_loading_complete_and_successful(&self, pkg: &Package, metrics: &Metrics) {
        for listener in &self.0 {
            listener.on_loading_complete_and_successful(pkg, metrics);
        }
    }
}
