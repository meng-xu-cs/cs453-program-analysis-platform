use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use once_cell::sync::Lazy;

use crate::util_docker::Dock;

/// Tag of the Docker image
const DOCKER_TAG: &str = "symcc";
const DOCKER_TAG_BASE: &str = "symcc-base";

/// Path to the build directory
static DOCKER_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("deps");
    path.push("symcc");
    path
});

/// Provision the SymCC tool
pub fn provision(dock: &mut Dock, force: bool) -> Result<()> {
    dock.build(DOCKER_PATH.as_path(), DOCKER_TAG_BASE, force)?;
    dock.commit(
        DOCKER_TAG_BASE,
        DOCKER_TAG,
        "/usr/bin/bash -c \"sudo apt-get update -y && sudo apt-get install -y screen\"".to_string(),
        BTreeMap::new(),
        None,
        true,
    )?;
    Ok(())
}
