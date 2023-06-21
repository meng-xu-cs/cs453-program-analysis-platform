use std::path::PathBuf;

use anyhow::Result;
use once_cell::sync::Lazy;

use crate::util_docker::Dock;

/// Tag of the Docker image
const DOCKER_TAG: &str = "klee";

/// Path to the build directory
static DOCKER_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("deps");
    path.push("klee");
    path
});

/// Provision the KLEE tool
pub fn provision(dock: &mut Dock, force: bool) -> Result<()> {
    dock.build(DOCKER_PATH.as_path(), DOCKER_TAG, force)?;
    Ok(())
}
