//! Loading .bzl files for Bzlmod (Java `BzlLoadValue.keyForBzlmod` /
//! `keyForBzlmodBootstrap` through `env.getValueOrThrow(key, BzlLoadFailedException.class)`).

use std::sync::Arc;

use blaze_cmdline::Label;
pub use blaze_sky_loading::bzl_load::BzlLoadFailedException;
use blaze_sky_loading::bzl_load::BzlLoadKey;
pub use blaze_sky_loading::bzl_load::BzlLoadValue;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;

/// `env.getValueOrThrow(bootstrap ? BzlLoadValue.keyForBzlmodBootstrap(label) :
/// BzlLoadValue.keyForBzlmod(label), BzlLoadFailedException.class)`: `Ok(None)` when the value
/// is missing, `Ok(Some(Err(e)))` when loading failed.
pub fn load_bzl_for_bzlmod(
    env: &mut Environment<'_>,
    label: &Label,
    bootstrap: bool,
) -> Result<Option<Result<Arc<BzlLoadValue>, BzlLoadFailedException>>, Interrupted> {
    let key = if bootstrap {
        BzlLoadKey::key_for_bzlmod_bootstrap(label.clone())
    } else {
        BzlLoadKey::key_for_bzlmod(label.clone())
    };
    match env.get_value_or_throw::<BzlLoadKey, BzlLoadFailedException>(key)? {
        Ok(Some(v)) => Ok(Some(Ok(v))),
        Ok(None) => Ok(None),
        Err(e) => Ok(Some(Err(e))),
    }
}
