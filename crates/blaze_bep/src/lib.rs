//! Port of the build event protocol (BEP): `lib/buildeventstream` (events, ids, transports,
//! artifact uploader), the `BuildEventStreamer` of `lib/runtime`, and the `BuildEvent` classes
//! it streams (from `lib/runtime`, `lib/analysis`, `lib/buildtool`, `lib/pkgcache`,
//! `lib/metrics`, `lib/actions`).
//!
//! Entry point for the runtime: [`bep_module::BuildEventStreamerBuilder`] (see its docs for
//! when the runtime posts each event).

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod bep_module;
pub mod buildeventstream;
pub mod events;
pub mod metrics;
pub mod runtime;
pub mod util;
