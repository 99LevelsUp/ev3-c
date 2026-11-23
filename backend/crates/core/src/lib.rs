//! EVƎ C's core: what building, uploading and running a C program on an EV3 brick takes,
//! without a front end. The `ev3-c` program serves it to a terminal, an agent and the VS Code
//! extension.

pub mod launcher;

mod json;
#[cfg(test)]
mod testutil;
