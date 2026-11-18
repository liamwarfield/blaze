//! Platform SkyFunctions (agent P; crates/blaze_analysis/DESIGN.md): ports of
//! `analysis/platform/PlatformFunction.java` (behind the
//! `blaze_build_config::platform::PlatformValueProvider` seam) and
//! `analysis/producers/PlatformProducer.java`.

pub mod platform_function;
pub mod platform_producer;

pub use platform_function::{PlatformFunction, platform_function};
pub use platform_producer::{PlatformProducerError, produce_platform};
