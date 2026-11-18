//! Port of the build event protocol (BEP): `lib/buildeventstream` (events, ids, transports,
//! artifact uploader), the `BuildEventStreamer` of `lib/runtime`, and the `BuildEvent` classes
//! it streams (from `lib/runtime`, `lib/analysis`, `lib/buildtool`, `lib/pkgcache`,
//! `lib/metrics`, `lib/actions`).
//!
//! Entry point for the runtime: [`bep_module::BuildEventStreamerBuilder`] (see its docs for
//! when the runtime posts each event).

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
// Ported signatures and Java's control flow win (PORTING.md "Style"): maps keyed by `Artifact`
// (Java `Map<Artifact, ...>`), exceptions carried by value, Java method names (`from_*`), Java's
// branch structure.
#![allow(
    clippy::mutable_key_type,
    clippy::result_large_err,
    clippy::large_enum_variant,
    clippy::type_complexity,
    clippy::wrong_self_convention,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::if_same_then_else
)]

pub mod bep_module;
pub mod buildeventstream;
pub mod events;
pub mod metrics;
pub mod runtime;
pub mod util;
