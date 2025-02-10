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

impl ResultBaseline {
    pub fn to_human_readable(&self) -> String {
        if !self.compiled {
            return "[failure] unable to compile the program".to_string();
        }
        if self.input_pass == 0 {
            return format!(
                "[failure] none of the {} test case(s) under 'input/' directory executes successfully",
                self.input_pass + self.input_fail,
            );
        }
        if self.input_fail != 0 {
            return format!(
                "[failure] {} out of {} test case(s) under 'input/' directory crash or timeout",
                self.input_fail,
                self.input_pass + self.input_fail
            );
        }
        if self.crash_pass == 0 {
            return format!(
                "[failure] none of the {} test case(s) under 'crash/' directory actually crash the program",
                self.crash_pass + self.crash_fail
            );
        }
        "[success] baseline check passed".to_string()
    }
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
                format!(
                    "timeout {} {} < {}",
                    TIMEOUT_TEST_CASE.as_secs(),
                    dock_path_compiled,
                    test
                ),
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
                format!(
                    "timeout {} {} < {}",
                    TIMEOUT_TEST_CASE.as_secs(),
                    dock_path_compiled,
                    test
                ),
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
    pub num_blocks: usize,
    pub cov_blocks: usize,
}

impl ResultGcov {
    pub fn to_human_readable(&self) -> String {
        if !self.completed {
            return "[failure] unable to complete GCOV measurement".to_string();
        }
        if self.num_blocks > self.cov_blocks {
            return format!(
                "[failure] GCOV coverage at {:.2}%",
                (self.cov_blocks as f64) / (self.num_blocks as f64) * 100.0
            );
        }
        "[success] 100% GCOV coverage".to_string()
    }
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
                format!(
                    "timeout {} {} < {}",
                    TIMEOUT_TEST_CASE.as_secs(),
                    dock_path_compiled,
                    test
                ),
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
                "gcov -a -b -o {} -n main.c -j -t > {}",
                docked.path_base, dock_path_gcov_report
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

fn parse_gcov_json_report(v: &Value) -> Option<(usize, usize)> {
    let mut total_num_blocks = 0;
    let mut total_cov_blocks = 0;

    let report = v.as_object()?;
    for item_file in report.get("files")?.as_array()? {
        let item_file = item_file.as_object()?;

        let mut stats = BTreeMap::new();
        for item_func in item_file.get("functions")?.as_array()? {
            let item_func = item_func.as_object()?;
            let item_name = item_func.get("name")?.as_str()?;
            let item_num_blocks = item_func.get("blocks")?.as_u64()? as usize;
            let item_cov_blocks = item_func.get("blocks_executed")?.as_u64()? as usize;
            stats.insert(item_name, (item_num_blocks, item_cov_blocks));
        }

        for item_line in item_file.get("lines")?.as_array()? {
            let item_line = item_line.as_object()?;
            let item_func_name = match item_line.get("function_name") {
                None => {
                    continue;
                }
                Some(v) => v.as_str()?,
            };
            let (_, cov_block) = stats.get_mut(item_func_name)?;
            for item_branch in item_line.get("branches")?.as_array()? {
                let item_branch = item_branch.as_object()?;
                let item_count = item_branch.get("count")?.as_u64()?;
                if item_count == 0 {
                    *cov_block += 1;
                }
            }
        }

        // aggregate the result
        for (num_blocks, cov_blocks) in stats.values() {
            total_num_blocks += *num_blocks;
            total_cov_blocks += *cov_blocks;
        }
    }

    Some((total_num_blocks, total_cov_blocks))
}
