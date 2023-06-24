use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::packet::{Packet, Registry};
use crate::util_docker::{Dock, ExitStatus};

/// Tag of the Docker image
const DOCKER_TAG: &str = "klee";

/// Default mount point for work directory
const DOCKER_MNT: &str = "/test";

/// Timeout for symbolic execution
const TIMEOUT_EXEC: Duration = Duration::from_secs(60 * 15);

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

/// Result for KLEE execution
#[derive(Serialize, Deserialize)]
pub struct ResultKLEE {
    pub completed: bool,
    pub num_crashes: u64,
}

pub fn run_klee(dock: &mut Dock, registry: &Registry, packet: &Packet) -> Result<ResultKLEE> {
    let docked = registry.mk_dockerized_packet(packet, "klee", DOCKER_MNT)?;

    // compile the program
    let (_, dock_path_bc) = docked.wks_path("main.bc");
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "clang".to_string(),
            "-emit-llvm".to_string(),
            "-g".to_string(),
            "-O0".to_string(),
            "-c".to_string(),
            docked.path_program.clone(),
            "-o".to_string(),
            dock_path_bc.clone(),
        ],
        None,
    )?;
    if !matches!(result, ExitStatus::Success) {
        return Ok(ResultKLEE {
            completed: false,
            num_crashes: 0,
        });
    }

    // symbolic exploration
    let (_host_path_klee_out, dock_path_klee_out) = docked.wks_path("output");
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "klee".to_string(),
            "--libc=klee".to_string(),
            "--posix-runtime".to_string(),
            format!("--output-dir={}", dock_path_klee_out),
            dock_path_bc,
            "-sym-stdin".to_string(),
            "1024".to_string(),
        ],
        Some(TIMEOUT_EXEC),
    )?;
    if matches!(result, ExitStatus::Failure) {
        return Ok(ResultKLEE {
            completed: false,
            num_crashes: 0,
        });
    }

    // done with KLEE execution
    Ok(ResultKLEE {
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
