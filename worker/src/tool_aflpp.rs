use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::packet::{Packet, Registry};
use crate::util_docker::{Dock, ExitStatus};

/// Tag of the Docker image
const DOCKER_TAG: &str = "afl";

/// Default mount point for work directory
const DOCKER_MNT: &str = "/test";

/// Timeout for testcase execution
const TIMEOUT_FUZZ: Duration = Duration::from_secs(60 * 15);

/// Path to the build directory
static DOCKER_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("deps");
    path.push("AFLplusplus");
    path
});

/// Provision the AFL++ tool
pub fn provision(dock: &mut Dock, force: bool) -> Result<()> {
    dock.build(DOCKER_PATH.as_path(), DOCKER_TAG, force)?;
    Ok(())
}

/// Result for baseline evaluation
#[derive(Serialize, Deserialize)]
pub struct ResultAFLpp {
    pub completed: bool,
}

pub fn run_aflpp(dock: &mut Dock, registry: &Registry, packet: &Packet) -> Result<ResultAFLpp> {
    let docked = registry.mk_dockerized_packet(packet, "aflpp", DOCKER_MNT)?;

    // compile the program
    let (_, dock_path_compiled) = docked.wks_path("main");
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "afl-cc".to_string(),
            docked.path_program.clone(),
            "-o".to_string(),
            dock_path_compiled.clone(),
        ],
        None,
    )?;
    if !matches!(result, ExitStatus::Success) {
        return Ok(ResultAFLpp { completed: false });
    }

    // fuzz the program
    let (host_path_afl_out, dock_path_afl_out) = docked.wks_path("output");
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "afl-fuzz".to_string(),
            "-i".to_string(),
            docked.path_input,
            "-o".to_string(),
            dock_path_afl_out.clone(),
            "--".to_string(),
            dock_path_compiled,
        ],
        Some(TIMEOUT_FUZZ),
    )?;
    if !matches!(result, ExitStatus::Timeout) {
        return Ok(ResultAFLpp { completed: false });
    }
    if !host_path_afl_out.exists() {
        bail!("unable to find the AFL++ output directory on host system");
    }

    // enable host access to the output directory
    docker_run(
        dock,
        &docked.host_base,
        vec![
            "chmod".to_string(),
            "-R".to_string(),
            "777".to_string(),
            dock_path_afl_out,
        ],
        None,
    )?;

    // done with AFL++ fuzzing
    Ok(ResultAFLpp { completed: true })
}

/// Utility helper on invoking this Docker image
fn docker_run(
    dock: &mut Dock,
    base: &Path,
    cmd: Vec<String>,
    timeout: Option<Duration>,
) -> Result<ExitStatus> {
    let mut binding = BTreeMap::new();
    binding.insert(base, DOCKER_MNT.to_string());
    dock.sandbox(DOCKER_TAG, cmd, timeout, binding, None)
}
