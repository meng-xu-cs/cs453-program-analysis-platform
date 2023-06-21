use anyhow::Result;

use crate::util_docker::Dock;
use crate::{tool_aflpp, tool_klee};

/// Provision all the tools
pub fn provision(force: bool) -> Result<()> {
    let mut dock = Dock::new();
    tool_aflpp::provision(&mut dock, force)?;
    tool_klee::provision(&mut dock, force)?;
    Ok(())
}
