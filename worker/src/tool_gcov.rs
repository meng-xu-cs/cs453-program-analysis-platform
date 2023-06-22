use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use once_cell::sync::Lazy;

use crate::packet::{Packet, Registry};
use crate::util_docker::Dock;

/// Tag of the Docker image
const DOCKER_TAG: &str = "gcov";

/// Default mount point for work directory
const DOCKER_MNT: &str = "/test";

/// Path to the build directory
static DOCKER_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("deps");
    path.push("gcov");
    path
});

/// Provision the GCOV tool
pub fn provision(dock: &mut Dock, force: bool) -> Result<()> {
    dock.build(DOCKER_PATH.as_path(), DOCKER_TAG, force)?;
    Ok(())
}

/// Result for baseline evaluation
pub struct ResultBaseline {
    pub input_pass: usize,
    pub input_fail: usize,
    pub crash_pass: usize,
    pub crash_fail: usize,
}

/// Run user-provided test cases
pub fn run_baseline(
    dock: &mut Dock,
    registry: &Registry,
    packet: &Packet,
) -> Result<ResultBaseline> {
    let docked = registry.mk_dockerized_packet(packet, "baseline", DOCKER_MNT)?;

    // build the program
    docker_run(
        dock,
        &docked.host_base,
        vec![
            "gcc".to_string(),
            docked.path_program.clone(),
            "-o".to_string(),
            docked.wks_path("main"),
        ],
    )?;

    todo!()
}

/// Utility helper on invoking this Docker image
fn docker_run(dock: &mut Dock, base: &Path, cmd: Vec<String>) -> Result<bool> {
    let mut binding = BTreeMap::new();
    binding.insert(base, DOCKER_MNT.to_string());
    dock.invoke(DOCKER_TAG, cmd, false, false, binding, None)
}
