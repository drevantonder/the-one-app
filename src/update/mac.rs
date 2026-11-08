//! What a Mac that built its app publishes, for the others.

use super::*;

/// Publishes the Mac app at `zip` for this Mac's platform: `<cli> publish-app`, run by the job after a build here.
pub(super) fn publish_app(zip: &Path, config: &str) -> Result<()> {
    build::publish_latest(&crate::node(), &release_key()?, &[(one::latest::Thing::app_name(TARGET), Some(zip.to_path_buf()), Version::this(), config.to_string())])
}
