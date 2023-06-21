use anyhow::Result;

use crate::tool_aflpp;
use crate::util_docker::Dock;

/// Provision all the tools
pub fn provision(force: bool) -> Result<()> {
    let mut dock = Dock::new();
    tool_aflpp::provision(&mut dock, force)?;
    Ok(())
}
