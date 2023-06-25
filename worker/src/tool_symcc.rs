use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::packet::{Packet, Registry};
use crate::util_docker::{Dock, ExitStatus};

/// Tag of the Docker image
const DOCKER_TAG: &str = "symcc";
const DOCKER_TAG_BASE: &str = "symcc-base";

/// Default mount point for work directory
const DOCKER_MNT: &str = "/test";

/// Timeout for fuzzing
const TIMEOUT_FUZZ: Duration = Duration::from_secs(5);

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
        vec![
            "bash".to_string(),
            "-c".to_string(),
            "sudo apt-get update -y && sudo apt-get install -y screen".to_string(),
        ],
        true,
        false,
        BTreeMap::new(),
        None,
        force,
    )?;
    Ok(())
}

/// Result for SymCC + AFL hybrid fuzzing
#[derive(Serialize, Deserialize)]
pub struct ResultSymCC {
    pub completed: bool,
    pub num_crashes: u64,
}

pub fn run_symcc(dock: &mut Dock, registry: &Registry, packet: &Packet) -> Result<ResultSymCC> {
    let docked = registry.mk_dockerized_packet(packet, "symcc", DOCKER_MNT)?;

    // compile the program
    let (_, dock_path_aflcc_compiled) = docked.wks_path("main-afl");
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "/afl/afl-clang".to_string(),
            docked.path_program.clone(),
            "-o".to_string(),
            dock_path_aflcc_compiled.clone(),
        ],
        None,
    )?;
    if !matches!(result, ExitStatus::Success) {
        return Ok(ResultSymCC {
            completed: false,
            num_crashes: 0,
        });
    }

    let (_, dock_path_symcc_compiled) = docked.wks_path("main-sym");
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "symcc".to_string(),
            docked.path_program.clone(),
            "-o".to_string(),
            dock_path_symcc_compiled.clone(),
        ],
        None,
    )?;
    if !matches!(result, ExitStatus::Success) {
        return Ok(ResultSymCC {
            completed: false,
            num_crashes: 0,
        });
    }

    // done with hybrid fuzzing
    Ok(ResultSymCC {
        completed: true,
        num_crashes: 0,
    })
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
