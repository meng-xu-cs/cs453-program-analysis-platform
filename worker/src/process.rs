use std::path::Path;

use anyhow::Result;

use crate::packet::Registry;
use crate::tool_gcov::run_baseline;
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

/// Analyze a packet
pub fn analyze<P: AsRef<Path>>(registry: &Registry, src: P) -> Result<()> {
    let (packet, existed) = registry.register(src)?;
    if existed {
        return Ok(());
    }

    // analysis
    let mut dock = Dock::new()?;
    run_baseline(&mut dock, registry, &packet)?;
    Ok(())
}
