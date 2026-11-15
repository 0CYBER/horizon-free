//! Validate API injection pins against a game's point catalog; no slot files are written.
use std::path::Path;
pub fn run(args: &[String]) -> Result<(), String> {
    let [flag, repo] = args else {
        return Err("usage: cargo horizon-mod inject sync --repo <game checkout>".into());
    };
    if flag != "--repo" {
        return Err("expected --repo".into());
    }
    super::inject::update::sync(Path::new("."), Path::new(repo))
}
