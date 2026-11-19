//! `CaffeineSpec` values and the `Converters.CaffeineSpecConverter` used by
//! `--directory_creation_cache`: both are ported in blaze_options (`caffeine_spec`,
//! `converters`), as Java's converter lives in `com.google.devtools.common.options`.

pub use blaze_options::CaffeineSpecConverter;
pub use blaze_options::caffeine_spec::CaffeineSpec;
