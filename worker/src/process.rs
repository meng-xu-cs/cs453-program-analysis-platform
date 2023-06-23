use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::packet::{Packet, Registry};
use crate::tool_gcov::{run_baseline, run_gcov, ResultBaseline, ResultGcov};
use crate::util_docker::Dock;
use crate::{tool_aflpp, tool_gcov, tool_klee, tool_symcc};

/// Provision all the tools
pub fn provision(force: bool) -> Result<()> {
    let mut dock = Dock::new()?;
    tool_gcov::provision(&mut dock, force)?;
    tool_aflpp::provision(&mut dock, force)?;
    tool_klee::provision(&mut dock, force)?;
    tool_symcc::provision(&mut dock, force)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct AnalysisResult {
    result_baseline: ResultBaseline,
    result_gcov: ResultGcov,
}

/// Analyze a packet
pub fn analyze(registry: &Registry, packet: &Packet) -> Result<AnalysisResult> {
    let mut dock = Dock::new()?;
    let result_baseline = run_baseline(&mut dock, registry, packet)?;
    let result_gcov = run_gcov(&mut dock, registry, packet)?;

    // collect and dump result
    Ok(AnalysisResult {
        result_baseline,
        result_gcov,
    })
}
