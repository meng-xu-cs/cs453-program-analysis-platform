use std::path::Path;

use anyhow::Result;

use crate::packet::Packet;
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
pub fn analyze<SRC: AsRef<Path>, DST: AsRef<Path>>(src: SRC, dst: DST) -> Result<()> {
    let (hash, pkt) = Packet::new(src, dst)?;
    Ok(())
}
