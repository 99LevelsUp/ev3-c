//! The `ev3-c` program's parts: the commands of `ev3 c …`, the MCP server of `ev3 c mcp`,
//! and the link to EVƎ Cockpit's daemon they share. `main.rs` wires them to the process.

pub mod cockpit;
pub mod paths;

#[cfg(test)]
mod testing;
