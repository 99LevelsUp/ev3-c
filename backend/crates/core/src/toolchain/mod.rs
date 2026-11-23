//! The pinned Zig cross-compiler: obtaining it, running `zig cc` for the brick, and checking
//! that what it produced runs there.

mod compiler;
mod elf;
mod manager;

pub use compiler::{CompileRequest, CompileResult, compile, compile_args};
pub use elf::{check_elf, elf_problems};
pub use manager::{
    CompilerInfo, CompilerOrigin, CompilerOverride, Downloader, EnsureCompilerOptions,
    HostToolchain, ToolchainError, ToolchainManifest, ensure_compiler, host_key,
};

/// Keeps a console window from opening for a child process when the backend itself runs
/// without one, as Node's `windowsHide` did.
fn hide_window(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}
