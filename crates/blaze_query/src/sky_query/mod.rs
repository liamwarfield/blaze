//! SkyQuery: `query` over a Skyframe graph prepopulated with a universe (`--universe_scope`,
//! `--infer_universe_scope`), with `allrdeps` and `rbuildfiles`.
//!
//! Java: com/google/devtools/build/lib/query2/ (SkyQueryEnvironment and its helpers).

pub mod expression_mappers;
pub mod parallel_sky_query_utils;
pub mod rbuild_files;
pub mod sky_query_environment;

pub use sky_query_environment::{SkyQueryEnvironment, WalkableGraphFactory};
