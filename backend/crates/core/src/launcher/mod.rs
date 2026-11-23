//! The launcher .rbf that starts a project on the brick, and the bytecode encoder it is
//! written with.

pub mod bytecode;
mod template;

pub use bytecode::LauncherError;
pub use template::{
    LauncherOptions, RetentionRule, SUPERVISOR_FILE, assemble_launcher, build_launcher,
    supervisor_arguments,
};
