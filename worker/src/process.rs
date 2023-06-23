use std::path::Path;

use anyhow::Result;
use log::info;

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

/// Schedule a packet
pub fn schedule<P: AsRef<Path>>(registry: &Registry, src: P) -> Result<(String, bool)> {
    let (packet, existed) = registry.register(src)?;

    // shortcut on previously submitted packet
    if existed {
        info!("packet already exists: {}", packet.id());
        return Ok((packet.id().to_string(), false));
    }

    // analysis
    info!("scheduling analysis for packet: {}", packet.id());
    let mut dock = Dock::new()?;
    run_baseline(&mut dock, registry, &packet)?;
    Ok((packet.id().to_string(), true))
}
