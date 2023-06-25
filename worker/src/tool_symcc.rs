use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{fs, thread};

use anyhow::{bail, Result};
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
pub fn provision(dock: &Dock, force: bool) -> Result<()> {
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

pub fn run_symcc(dock: &Dock, registry: &Registry, packet: &Packet) -> Result<ResultSymCC> {
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

    // prepare output
    let (host_path_symcc_out, dock_path_symcc_out) = docked.wks_path("output");

    // launch the fuzzing thread
    let side_dock = dock.duplicate()?;
    let side_base = docked.host_base.clone();
    let side_output = dock_path_symcc_out.clone();
    let handle = thread::spawn(move || {
        docker_run(
            &side_dock,
            &side_base,
            vec![
                "/afl/afl-fuzz".to_string(),
                "-M".to_string(),
                "afl-0".to_string(),
                "-i".to_string(),
                docked.path_input.clone(),
                "-o".to_string(),
                side_output,
                "--".to_string(),
                dock_path_aflcc_compiled,
            ],
            Some(TIMEOUT_FUZZ),
        )
    });

    // wait for readiness
    let afl_dir = host_path_symcc_out.join("afl-0");
    let afl_dir_queue = afl_dir.join("queue");
    while !afl_dir_queue.exists() {
        if handle.is_finished() {
            bail!("AFL not started on the sideline");
        }
        thread::sleep(Duration::from_secs(1));
    }

    // spawn SymCC
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "symcc_fuzzing_helper".to_string(),
            "-v".to_string(),
            "-o".to_string(),
            dock_path_symcc_out,
            "-a".to_string(),
            "afl-0".to_string(),
            "-n".to_string(),
            "symcc".to_string(),
            "--".to_string(),
            dock_path_symcc_compiled,
        ],
        Some(TIMEOUT_FUZZ),
    )?;
    if !matches!(result, ExitStatus::Timeout) {
        return Ok(ResultSymCC {
            completed: false,
            num_crashes: 0,
        });
    }

    // check result
    match handle.join() {
        Ok(result) => {
            let result = result?;
            if !matches!(result, ExitStatus::Timeout) {
                return Ok(ResultSymCC {
                    completed: false,
                    num_crashes: 0,
                });
            }
        }
        Err(err) => {
            bail!("failed to execute AFL on the sideline: {:?}", err);
        }
    }

    // done with hybrid fuzzing
    let afl_dir_crash = afl_dir.join("crashes");
    if !afl_dir_crash.exists() {
        bail!("unable to find the AFL crash directory on host system");
    }

    let mut num_crashes = 0;
    for item in fs::read_dir(afl_dir_crash)? {
        let item = item?;
        if item
            .file_name()
            .to_str()
            .map_or(true, |s| s != "README.txt")
        {
            num_crashes += 1;
        }
    }

    Ok(ResultSymCC {
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
