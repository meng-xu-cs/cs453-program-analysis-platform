use std::collections::BTreeMap;
use std::fs;
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

/// Timeout for fuzzing
const TIMEOUT_FUZZ: Duration = Duration::from_secs(5);

/// Path to the build directory
static DOCKER_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("deps");
    path.push("AFLplusplus");
    path
});

/// Provision the AFL++ tool
pub fn provision(dock: &Dock, force: bool) -> Result<()> {
    dock.build(DOCKER_PATH.as_path(), DOCKER_TAG, force)?;
    Ok(())
}

/// Result for AFL++ fuzzing
#[derive(Serialize, Deserialize)]
pub struct ResultAFLpp {
    pub completed: bool,
    pub num_crashes: u64,
}

pub fn run_aflpp(dock: &Dock, registry: &Registry, packet: &Packet) -> Result<ResultAFLpp> {
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
        return Ok(ResultAFLpp {
            completed: false,
            num_crashes: 0,
        });
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
        return Ok(ResultAFLpp {
            completed: false,
            num_crashes: 0,
        });
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

    // check number of crashes
    let host_path_crash_dir = host_path_afl_out.join("default").join("crashes");
    if !host_path_crash_dir.exists() {
        bail!("unable to find the AFL++ crash directory on host system");
    }

    let mut num_crashes = 0;
    for item in fs::read_dir(host_path_crash_dir)? {
        let item = item?;
        if item
            .file_name()
            .to_str()
            .map_or(true, |s| s != "README.txt")
        {
            num_crashes += 1;
        }
    }

    // done with AFL++ fuzzing
    Ok(ResultAFLpp {
        completed: true,
        num_crashes,
    })
}

/// Utility helper on invoking this Docker image
fn docker_run(
    dock: &Dock,
    base: &Path,
    cmd: Vec<String>,
    timeout: Option<Duration>,
) -> Result<ExitStatus> {
    let mut binding = BTreeMap::new();
    binding.insert(base, DOCKER_MNT.to_string());
    dock.sandbox(DOCKER_TAG, cmd, timeout, binding, None)
}
