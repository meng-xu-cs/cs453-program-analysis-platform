use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use once_cell::sync::Lazy;

use crate::packet::Packet;
use crate::util_docker::Dock;

/// Tag of the Docker image
const DOCKER_TAG: &str = "gcov";

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
pub fn run_baseline(dock: &mut Dock, packet: &mut Packet) -> Result<ResultBaseline> {
    let wks = packet.mk_wks("base")?;

    // build the program
    // docker_run(dock, packet)

    todo!()
}

/// Utility helper on invoking this Docker image
fn docker_run(dock: &mut Dock, packet: &mut Packet, cmd: Vec<String>) -> Result<bool> {
    let workdir = "/test";
    let mut binding = BTreeMap::new();
    binding.insert(packet.base.as_path(), workdir.to_string());
    dock.invoke(
        DOCKER_TAG,
        cmd,
        false,
        false,
        binding,
        Some(workdir.to_string()),
    )
}
