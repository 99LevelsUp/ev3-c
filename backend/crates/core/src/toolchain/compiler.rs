//! Cross-compiles C sources for the brick with `zig cc`. The target flags come from
//! toolchain-manifest.json, the single place that defines what a brick binary is.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::manager::{ToolchainError, ToolchainManifest, io_error};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileRequest {
    /// Compiler executable, as returned by `ensure_compiler`.
    pub compiler_path: PathBuf,
    pub sources: Vec<String>,
    pub include_dirs: Vec<PathBuf>,
    /// Static libraries (for example libev3.a), passed to the linker by path, and any linker
    /// flag that has to come right before one.
    pub libraries: Vec<OsString>,
    pub output: String,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileResult {
    pub ok: bool,
    pub elf_path: String,
    /// The compiler's exit code; `None` when a signal ended it.
    pub exit_code: Option<i32>,
    /// Everything the compiler wrote: its stdout, then its stderr.
    pub diagnostics: String,
}

/// The arguments after the compiler executable: `cc`, the manifest's target flags, the
/// include folders, the sources, the libraries, and the output.
pub fn compile_args(request: &CompileRequest, target_flags: &[String]) -> Vec<OsString> {
    let mut args = vec![OsString::from("cc")];
    args.extend(target_flags.iter().map(OsString::from));
    args.extend(request.include_dirs.iter().map(|dir| {
        let mut flag = OsString::from("-I");
        flag.push(dir);
        flag
    }));
    args.extend(request.sources.iter().map(OsString::from));
    args.extend(request.libraries.iter().cloned());
    args.push(OsString::from("-o"));
    args.push(OsString::from(&request.output));
    args
}

/// Runs the compiler. A compile error is `Ok` with `ok: false` and the compiler's messages;
/// `Err` means the compiler could not be started.
pub fn compile(request: &CompileRequest) -> Result<CompileResult, ToolchainError> {
    let manifest = ToolchainManifest::bundled()?;
    let mut command = Command::new(&request.compiler_path);
    command
        .args(compile_args(request, &manifest.target_flags))
        .stdin(Stdio::null());
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    super::hide_window(&mut command);
    let output = command.output().map_err(io_error(format!(
        "Starting {} failed",
        request.compiler_path.display()
    )))?;
    let mut diagnostics = String::from_utf8_lossy(&output.stdout).into_owned();
    diagnostics.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(CompileResult {
        ok: output.status.success(),
        elf_path: request.output.clone(),
        exit_code: output.status.code(),
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiler_gets_cc_the_target_flags_includes_sources_libraries_and_output_in_order() {
        let request = CompileRequest {
            compiler_path: PathBuf::from("zig"),
            sources: vec!["drive.c".into(), "main.c".into()],
            include_dirs: vec![PathBuf::from("inc")],
            libraries: vec!["-Wl,-u,ev3_core_init".into(), "libev3.a".into()],
            output: "build/prog".into(),
            cwd: None,
        };
        let manifest = ToolchainManifest::bundled().unwrap();
        let args = compile_args(&request, &manifest.target_flags);
        let args: Vec<&str> = args.iter().map(|arg| arg.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "cc",
                "-target",
                "arm-linux-musleabi",
                "-mcpu=arm926ej_s",
                "-static",
                "-Os",
                "-ffunction-sections",
                "-fdata-sections",
                "-Wl,--gc-sections",
                "-Iinc",
                "drive.c",
                "main.c",
                "-Wl,-u,ev3_core_init",
                "libev3.a",
                "-o",
                "build/prog",
            ]
        );
    }

    #[test]
    fn a_compiler_that_cannot_be_started_is_an_error() {
        let request = CompileRequest {
            compiler_path: crate::testutil::temp_dir("ev3c-compile-").join("no-such-zig"),
            sources: vec!["main.c".into()],
            include_dirs: Vec::new(),
            libraries: Vec::new(),
            output: "prog".into(),
            cwd: None,
        };
        assert!(compile(&request).is_err());
    }
}
