use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::packet::{Packet, Registry};
use crate::tool_aflpp::{run_aflpp, ResultAFLpp};
use crate::tool_gcov::{run_baseline, run_gcov, ResultBaseline, ResultGcov};
use crate::tool_klee::{run_klee, ResultKLEE};
use crate::util_docker::Dock;
use crate::{tool_aflpp, tool_gcov, tool_klee};

/// Provision all the tools
pub fn provision(force: bool) -> Result<()> {
    let dock = Dock::new("provision".to_string())?;
    tool_gcov::provision(&dock, force)?;
    tool_aflpp::provision(&dock, force)?;
    tool_klee::provision(&dock, force)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct AnalysisResult {
    result_baseline: ResultBaseline,
    result_gcov: ResultGcov,
    result_aflpp: ResultAFLpp,
    result_klee: ResultKLEE,
}

impl AnalysisResult {
    pub fn to_human_readable(&self) -> String {
        vec![
            "==== Baseline ====".to_string(),
            self.result_baseline.to_human_readable(),
            String::new(),
            "==== GCOV ====".to_string(),
            self.result_gcov.to_human_readable(),
            String::new(),
            "==== AFL++ ====".to_string(),
            self.result_aflpp.to_human_readable(),
            String::new(),
            "==== KLEE ====".to_string(),
            self.result_klee.to_human_readable(),
            String::new(),
        ]
        .join("\n")
    }
}

/// Analyze a packet
pub fn analyze(dock: &Dock, registry: &Registry, packet: &Packet) -> Result<AnalysisResult> {
    let result_baseline = run_baseline(dock, registry, packet)?;
    let result_gcov = run_gcov(dock, registry, packet)?;
    let result_aflpp = run_aflpp(dock, registry, packet)?;
    let result_klee = run_klee(dock, registry, packet)?;

    // collect and dump result
    Ok(AnalysisResult {
        result_baseline,
        result_gcov,
        result_aflpp,
        result_klee,
    })
}
