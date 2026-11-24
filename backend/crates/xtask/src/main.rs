//! The repository's build tasks, run from `backend/` as `cargo run -p xtask -- <task>` (npm
//! runs them as `build:runtime` and `test:native`):
//!
//! - `build-runtime` builds the brick runtime that ships with EVƎ C into `out/runtime/`:
//!   libev3.a and the supervisor. Zig comes from the core's toolchain manager and the target
//!   flags from its manifest, the ones a student's build uses. A small program linked against
//!   libev3.a must then pass the ELF check, which proves the library links into an executable
//!   the brick can run.
//! - `native-tests` builds ev3lib and the supervisor for this computer and runs their C tests.
//!   The tests fork and exec processes and map device files, so they need Linux or macOS, and
//!   the supervisor tests also read /proc, so they need Linux. Elsewhere it says what it skips
//!   (CI runs everything on Linux).
//!
//! Paths passed to Zig are relative to the repository root, so the objects do not record where
//! the repository happens to live.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use ev3c_core::toolchain::{
    CompileRequest, EnsureCompilerOptions, ToolchainManifest, check_elf, compile, ensure_compiler,
};

type Result<T = ()> = std::result::Result<T, String>;

const USAGE: &str = "usage: cargo run -p xtask -- build-runtime | native-tests";

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let done = match task.as_deref() {
        Some("build-runtime") => build_runtime(),
        Some("native-tests") => native_tests(),
        _ => Err(USAGE.to_owned()),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The repository root: `backend/crates/xtask` is three folders below it.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// The pinned Zig, downloaded into the repository's `toolchain/cache` on first use.
fn ensure_zig() -> Result<PathBuf> {
    let info = ensure_compiler(&EnsureCompilerOptions {
        cache_dir: root().join("toolchain").join("cache"),
        ..EnsureCompilerOptions::default()
    })
    .map_err(|error| error.to_string())?;
    println!(
        "zig {} from {:?}: {}",
        info.version.as_deref().unwrap_or("(override)"),
        info.origin,
        info.path.display()
    );
    Ok(info.path)
}

/// C sources under `dir` (relative to the repository root), sorted, with forward slashes.
fn sources_in(dir: &str) -> Result<Vec<String>> {
    fn walk(root: &Path, relative: &str, found: &mut Vec<String>) -> Result {
        let absolute = root.join(relative);
        if !absolute.exists() {
            return Ok(());
        }
        let entries = fs::read_dir(&absolute)
            .map_err(|error| format!("{} cannot be read: {error}", absolute.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = format!("{relative}/{name}");
            if entry.path().is_dir() {
                walk(root, &child, found)?;
            } else if name.ends_with(".c") {
                found.push(child);
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(&root(), dir, &mut found)?;
    found.sort();
    Ok(found)
}

/// Runs Zig in the repository root; a failure names the arguments.
fn zig<S: AsRef<str>>(compiler: &Path, args: &[S]) -> Result {
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let status = Command::new(compiler)
        .args(&args)
        .current_dir(root())
        .status()
        .map_err(|error| format!("{} cannot be started: {error}", compiler.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("zig {} failed ({status}).", args.join(" ")))
    }
}

fn recreate(relative: &str) -> Result {
    let dir = root().join(relative);
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .map_err(|error| format!("{} cannot be removed: {error}", dir.display()))?;
    }
    fs::create_dir_all(&dir).map_err(|error| format!("{} cannot be made: {error}", dir.display()))
}

/// Compiles every ev3lib source with `flags` and archives the objects into `archive`.
fn build_ev3lib(compiler: &Path, flags: &[String], obj_dir: &str, archive: &str) -> Result {
    recreate(obj_dir)?;
    let mut objects = Vec::new();
    for source in sources_in("ev3lib/src")? {
        let name = source
            .trim_start_matches("ev3lib/src/")
            .replace('/', "_")
            .replace(".c", ".o");
        let object = format!("{obj_dir}/{name}");
        let mut args: Vec<String> = vec!["cc".into(), "-c".into()];
        args.extend(flags.iter().cloned());
        args.extend(
            [
                "-Wall",
                "-Wextra",
                "-Werror",
                "-Iev3lib/include",
                &source,
                "-o",
                &object,
            ]
            .map(str::to_owned),
        );
        zig(compiler, &args)?;
        objects.push(object);
    }
    let archive_path = root().join(archive);
    let _ = fs::remove_file(&archive_path);
    if let Some(parent) = archive_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut args = vec!["ar".to_owned(), "rcs".to_owned(), archive.to_owned()];
    args.extend(objects.iter().cloned());
    zig(compiler, &args)?;
    println!("built {archive} from {} source file(s)", objects.len());
    Ok(())
}

/// Uses ev3lib's whole surface and malloc, which fails to link for ARMv5 with toolchains whose
/// allocator needs atomic instructions the brick lacks.
const CHECK_PROGRAM: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include "ev3.h"

int main(void)
{
    char *line = malloc(32);
    if (line == NULL) {
        return 1;
    }
    snprintf(line, 32, "run folder %s", ev3_core_run_dir() ? ev3_core_run_dir() : "-");
    puts(line);
    free(line);
    ev3_button_take_back();
    ev3_motor_set_power(EV3_PORT_A | EV3_PORT_B, 50);
    while (!ev3_button_is_pressed(EV3_BUTTON_BACK)) {
        ev3_sleep_ms(10);
    }
    ev3_motor_stop(EV3_PORT_ALL, EV3_STOP_BRAKE);
    if (ev3_motor_try_run_degrees(EV3_PORT_A, 50, 90) != EV3_OK) {
        fprintf(stderr, "%s\n", ev3_error_message());
    }
    ev3_button_release_back();
    ev3_core_exit();
    return 0;
}
"#;

/// Links for the brick with the core's compile step, then checks the ELF as a student's build
/// does.
fn checked_link(request: CompileRequest, label: &str) -> Result {
    let output = request.output.clone();
    let result = compile(&request).map_err(|error| error.to_string())?;
    if !result.diagnostics.trim().is_empty() {
        print!("{}", result.diagnostics);
    }
    if !result.ok {
        return Err(format!(
            "{label}: linking {output} failed with exit code {:?}.",
            result.exit_code
        ));
    }
    let problems = check_elf(&root().join(&output))
        .map_err(|error| format!("{label}: {output} cannot be read: {error}"))?;
    if !problems.is_empty() {
        return Err(format!(
            "{label}: {output} fails the ELF check:\n  {}",
            problems.join("\n  ")
        ));
    }
    println!("{label}: {output} passes the ELF check (static ARMv5TE soft-float)");
    Ok(())
}

fn build_runtime() -> Result {
    let compiler = ensure_zig()?;
    // Linker flags stay out of the object builds: Zig applies -Wl,--gc-sections to a single
    // object compiled with -c and keeps only what the constructor reaches, which leaves the
    // library without its public functions.
    let object_flags: Vec<String> = ToolchainManifest::bundled()
        .map_err(|error| error.to_string())?
        .target_flags
        .iter()
        .filter(|flag| !flag.starts_with("-Wl,"))
        .cloned()
        .collect();
    let (out_dir, work_dir) = ("out/runtime", "build/runtime");
    recreate(out_dir)?;
    recreate(work_dir)?;

    let archive = format!("{out_dir}/libev3.a");
    build_ev3lib(
        &compiler,
        &object_flags,
        &format!("{work_dir}/obj"),
        &archive,
    )?;

    let supervisor_sources = sources_in("supervisor/src")?;
    if supervisor_sources.is_empty() {
        println!("supervisor: skipped, supervisor/src has no C sources yet");
    } else {
        checked_link(
            CompileRequest {
                compiler_path: compiler.clone(),
                sources: supervisor_sources,
                include_dirs: vec![PathBuf::from("supervisor/src")],
                libraries: Vec::new(),
                output: format!("{out_dir}/supervisor"),
                cwd: Some(root()),
            },
            "supervisor",
        )?;
    }

    let check_dir = format!("{work_dir}/check");
    fs::create_dir_all(root().join(&check_dir)).map_err(|error| error.to_string())?;
    fs::write(root().join(&check_dir).join("check.c"), CHECK_PROGRAM)
        .map_err(|error| error.to_string())?;
    checked_link(
        CompileRequest {
            compiler_path: compiler.clone(),
            sources: vec![format!("{check_dir}/check.c")],
            include_dirs: vec![PathBuf::from("ev3lib/include")],
            libraries: vec![OsString::from(&archive)],
            output: format!("{check_dir}/check"),
            cwd: Some(root()),
        },
        "libev3.a",
    )?;

    let mut built: Vec<String> = fs::read_dir(root().join(out_dir))
        .map_err(|error| error.to_string())?
        .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
        .collect();
    built.sort();
    println!("runtime ready in {out_dir}/: {}", built.join(", "));
    Ok(())
}

fn native_tests() -> Result {
    if cfg!(windows) {
        println!(
            "Native tests need POSIX (fork, exec, mmap) and run on Linux or macOS; skipped on Windows."
        );
        return Ok(());
    }
    let compiler = ensure_zig()?;
    let build_dir = "build/native";
    let archive = format!("{build_dir}/libev3.a");
    build_ev3lib(
        &compiler,
        &["-g".to_owned()],
        &format!("{build_dir}/obj"),
        &archive,
    )?;

    let tests: Vec<String> = sources_in("ev3lib/tests")?
        .into_iter()
        .filter(|file| {
            Path::new(file)
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("test_"))
        })
        .collect();
    if tests.is_empty() {
        return Err("no test_*.c files in ev3lib/tests".to_owned());
    }
    // (test source, executable, arguments)
    let mut runs: Vec<(String, String, Vec<String>)> = Vec::new();
    for test in tests {
        let stem = Path::new(&test)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let executable = format!("{build_dir}/{stem}");
        // Linked through the archive, as programs on the brick are, so a test also proves that
        // ev3lib's start-up and exit code gets linked in.
        zig(
            &compiler,
            &[
                "cc",
                "-g",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-Iev3lib/include",
                &test,
                &archive,
                "-o",
                &executable,
            ],
        )?;
        runs.push((test, executable, Vec::new()));
    }

    // The supervisor: test_runfolder is linked with the run-folder code, and test_supervisor
    // drives a host build of the whole supervisor as a separate process. Zig's debug builds
    // trap undefined behaviour, which would turn the test's deliberate null dereference into
    // SIGILL instead of SIGSEGV.
    if cfg!(target_os = "linux") {
        let flags = [
            "cc",
            "-g",
            "-fno-sanitize=undefined",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Isupervisor/src",
        ];
        let with = |more: &[&str]| -> Vec<String> {
            flags
                .iter()
                .chain(more.iter())
                .map(|arg| (*arg).to_owned())
                .collect()
        };
        let host_supervisor = format!("{build_dir}/supervisor");
        let mut args = with(&[]);
        args.extend(sources_in("supervisor/src")?);
        args.extend(["-o".to_owned(), host_supervisor.clone()]);
        zig(&compiler, &args)?;
        let runfolder = format!("{build_dir}/test_runfolder");
        zig(
            &compiler,
            &with(&[
                "supervisor/tests/test_runfolder.c",
                "supervisor/src/runfolder.c",
                "-o",
                &runfolder,
            ]),
        )?;
        let supervisor_test = format!("{build_dir}/test_supervisor");
        zig(
            &compiler,
            &with(&["supervisor/tests/test_supervisor.c", "-o", &supervisor_test]),
        )?;
        runs.push((
            "supervisor/tests/test_runfolder.c".to_owned(),
            runfolder,
            Vec::new(),
        ));
        runs.push((
            "supervisor/tests/test_supervisor.c".to_owned(),
            supervisor_test,
            vec![host_supervisor],
        ));
    } else {
        println!("Supervisor tests read /proc and run on Linux only; skipped.");
    }

    let mut failed = Vec::new();
    for (test, executable, args) in &runs {
        println!("running {executable}");
        match Command::new(root().join(executable))
            .args(args)
            .current_dir(root())
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => failed.push(format!("{test} ({status})")),
            Err(error) => failed.push(format!("{test} ({error})")),
        }
    }
    if !failed.is_empty() {
        return Err(format!("native tests failed:\n  {}", failed.join("\n  ")));
    }
    println!("native tests passed: {} file(s)", runs.len());
    Ok(())
}
