use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::packet::{Packet, Registry};
use crate::util_docker::{Dock, ExitStatus};

/// Tag of the Docker image
const DOCKER_TAG: &str = "gcov";

/// Default mount point for work directory
const DOCKER_MNT: &str = "/test";

/// Timeout for testcase execution
const TIMEOUT_TEST_CASE: Duration = Duration::from_secs(10);

/// Path to the build directory
static DOCKER_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("deps");
    path.push("gcov");
    path
});

/// Provision the GCOV tool
pub fn provision(dock: &Dock, force: bool) -> Result<()> {
    dock.build(DOCKER_PATH.as_path(), DOCKER_TAG, force)?;
    Ok(())
}

/// Result for baseline evaluation
#[derive(Serialize, Deserialize)]
pub struct ResultBaseline {
    pub compiled: bool,
    pub input_pass: usize,
    pub input_fail: usize,
    pub crash_pass: usize,
    pub crash_fail: usize,
}

/// Run user-provided test cases
pub fn run_baseline(dock: &Dock, registry: &Registry, packet: &Packet) -> Result<ResultBaseline> {
    let docked = registry.mk_dockerized_packet(packet, "baseline", DOCKER_MNT)?;

    // compile the program
    let (_, dock_path_compiled) = docked.wks_path("main");
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "gcc".to_string(),
            docked.path_program.clone(),
            "-o".to_string(),
            dock_path_compiled.clone(),
        ],
        None,
    )?;
    if !matches!(result, ExitStatus::Success) {
        return Ok(ResultBaseline {
            compiled: false,
            input_pass: 0,
            input_fail: 0,
            crash_pass: 0,
            crash_fail: 0,
        });
    }

    // run each tests in input directory
    let mut input_pass = 0;
    let mut input_fail = 0;
    for test in docked.path_input_cases.iter() {
        let result = docker_run(
            dock,
            &docked.host_base,
            vec![
                "bash".to_string(),
                "-c".to_string(),
                format!("{} < {}", dock_path_compiled, test),
            ],
            Some(TIMEOUT_TEST_CASE),
        )?;
        if matches!(result, ExitStatus::Success) {
            input_pass += 1;
        } else {
            input_fail += 1;
        }
    }

    let mut crash_pass = 0;
    let mut crash_fail = 0;
    for test in docked.path_crash_cases.iter() {
        let result = docker_run(
            dock,
            &docked.host_base,
            vec![
                "bash".to_string(),
                "-c".to_string(),
                format!("{} < {}", dock_path_compiled, test),
            ],
            Some(TIMEOUT_TEST_CASE),
        )?;
        if matches!(result, ExitStatus::Failure) {
            crash_pass += 1;
        } else {
            crash_fail += 1;
        }
    }

    // done with baseline testing
    Ok(ResultBaseline {
        compiled: true,
        input_pass,
        input_fail,
        crash_pass,
        crash_fail,
    })
}

/// Result for baseline evaluation
#[derive(Serialize, Deserialize)]
pub struct ResultGcov {
    pub completed: bool,
    pub num_blocks: u64,
    pub cov_blocks: u64,
}

pub fn run_gcov(dock: &Dock, registry: &Registry, packet: &Packet) -> Result<ResultGcov> {
    let docked = registry.mk_dockerized_packet(packet, "gcov", DOCKER_MNT)?;

    // compile the program
    let (_, dock_path_compiled) = docked.wks_path("main");
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "gcc".to_string(),
            "-fprofile-arcs".to_string(),
            "-ftest-coverage".to_string(),
            "-g".to_string(),
            docked.path_program.clone(),
            "-o".to_string(),
            dock_path_compiled.clone(),
        ],
        None,
    )?;
    if !matches!(result, ExitStatus::Success) {
        return Ok(ResultGcov {
            completed: false,
            num_blocks: 0,
            cov_blocks: 0,
        });
    }

    // run each tests in input directory
    for test in docked.path_input_cases.iter() {
        docker_run(
            dock,
            &docked.host_base,
            vec![
                "bash".to_string(),
                "-c".to_string(),
                format!("{} < {}", dock_path_compiled, test),
            ],
            None,
        )?;
    }

    // calculate GCOV in json format
    let (host_path_gcov_report, dock_path_gcov_report) = docked.wks_path("report.json");
    let result = docker_run(
        dock,
        &docked.host_base,
        vec![
            "bash".to_string(),
            "-c".to_string(),
            format!(
                "gcov -o {} -n main.c -j -t > {}",
                docked.path_output, dock_path_gcov_report
            ),
        ],
        None,
    )?;
    if !matches!(result, ExitStatus::Success) {
        return Ok(ResultGcov {
            completed: false,
            num_blocks: 0,
            cov_blocks: 0,
        });
    }
    if !host_path_gcov_report.exists() {
        bail!("unable to find the GCOV report on host system");
    }
    let report: Value = serde_json::from_reader(File::open(host_path_gcov_report)?)?;
    let (num_blocks, cov_blocks) = match parse_gcov_json_report(&report) {
        None => {
            bail!("unable to parse the GCOV report");
        }
        Some((n, c)) => (n, c),
    };

    // done with GCOV testing
    Ok(ResultGcov {
        completed: true,
        num_blocks,
        cov_blocks,
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

fn parse_gcov_json_report(v: &Value) -> Option<(u64, u64)> {
    let mut num_blocks = 0;
    let mut cov_blocks = 0;
    let report = v.as_object()?;
    for item_file in report.get("files")?.as_array()? {
        for item_func in item_file.as_object()?.get("functions")?.as_array()? {
            let item_func = item_func.as_object()?;
            num_blocks += item_func.get("blocks")?.as_u64()?;
            cov_blocks += item_func.get("blocks_executed")?.as_u64()?;
        }
    }
    Some((num_blocks, cov_blocks))
}
