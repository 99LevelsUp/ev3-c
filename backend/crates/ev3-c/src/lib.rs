//! The `ev3-c` program's parts: the commands of `ev3 c …`, the MCP server of `ev3 c mcp`,
//! and the link to EVƎ Cockpit's daemon they share. `main.rs` wires them to the process.

pub mod args;
pub mod cli;
pub mod cockpit;
pub mod daemon;
pub mod format;
pub mod mcp;
pub mod mcp_runs;
pub mod paths;

#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod testing;
