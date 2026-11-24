//! Turns a project folder into the files that go onto the brick: the student's program
//! linked with libev3.a, the launcher with the project's rules in its supervisor command,
//! and the prebuilt supervisor. Every step that can refuse the project runs before the
//! compiler: name and rules, the runtime files, the presence of sources.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::launcher::{LauncherError, LauncherOptions, SUPERVISOR_FILE, build_launcher};
use crate::project::{ProjectConfig, ProjectError, load_project_config};
use crate::toolchain::{
    CompileRequest, CompileResult, CompilerOverride, EnsureCompilerOptions, ToolchainError,
    check_elf, compile, ensure_compiler,
};

/// Folder on the brick that holds one folder per project.
pub const BRICK_PROJECTS_DIR: &str = "/home/root/lms2012/prjs";

/// File name of the student's program inside the project folder; the supervisor runs it.
pub const PROGRAM_FILE: &str = "prog";

/// Build output folder inside the project folder.
pub const BUILD_DIR: &str = "build";

// Pulls ev3lib's start-up object into every program, also one that calls no ev3 function:
// its constructor sets line-buffered output before main(), and it registers the motor stop
// at exit.
const FORCE_EV3LIB_STARTUP: &str = "-Wl,-u,ev3_core_init";

/// The project's folder on the brick.
pub fn brick_project_dir(project: &str) -> String {
    format!("{BRICK_PROJECTS_DIR}/{project}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    Program,
    Launcher,
    Supervisor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildArtifact {
    pub kind: ArtifactKind,
    /// The file on this computer.
    pub local_path: PathBuf,
    /// Where the file goes on the brick, always shorter than 60 bytes.
    pub brick_path: String,
}

/// Where the compiler is cached, and a compiler path set by the host.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolchainSettings {
    pub cache_dir: PathBuf,
    pub compiler_override: Option<CompilerOverride>,
}

/// A replacement for the Zig compile step.
pub type CompileStep<'a> = dyn Fn(&CompileRequest) -> Result<CompileResult, ToolchainError> + 'a;

#[derive(Default)]
pub struct BuildProjectOptions<'a> {
    /// Folder with the student's .c files and an optional ev3c.json.
    pub project_dir: PathBuf,
    /// Root of the installed package; ev3lib's headers are in `ev3lib/include` below it.
    pub package_root: PathBuf,
    /// Folder with the prebuilt libev3.a and supervisor; defaults to `out/runtime` in the
    /// package.
    pub runtime_dir: Option<PathBuf>,
    pub toolchain: ToolchainSettings,
    /// Replaces the Zig compile step, for tests.
    pub compile: Option<&'a CompileStep<'a>>,
}

/// `CompileFailed` means the student's sources did not compile; `diagnostics` holds what the
/// compiler said. Anything else that stops a build is a [`BuildError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildResult {
    Built {
        project: ProjectConfig,
        /// Program, launcher, and supervisor, in that order.
        artifacts: Vec<BuildArtifact>,
        /// Compiler warnings, if any.
        diagnostics: String,
    },
    CompileFailed {
        diagnostics: String,
    },
}

/// What stops a build other than a compile error: invalid name or rules, missing runtime
/// files, no sources, no compiler, or a program the brick cannot run.
#[derive(Debug)]
pub enum BuildError {
    Project(ProjectError),
    RuntimeIncomplete { file: String, runtime_dir: PathBuf },
    NoSources { project_dir: PathBuf },
    Toolchain(ToolchainError),
    NotRunnable { problems: Vec<String> },
    Launcher(LauncherError),
    Io { context: String, source: io::Error },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::Project(error) => error.fmt(f),
            BuildError::RuntimeIncomplete { file, runtime_dir } => write!(
                f,
                "The EVƎ C runtime is incomplete: {file} is missing in {}.",
                runtime_dir.display()
            ),
            BuildError::NoSources { project_dir } => write!(
                f,
                "The project folder {} has no .c files to build.",
                project_dir.display()
            ),
            BuildError::Toolchain(error) => error.fmt(f),
            BuildError::NotRunnable { problems } => write!(
                f,
                "The compiled program cannot run on the brick:\n  {}",
                problems.join("\n  ")
            ),
            BuildError::Launcher(error) => error.fmt(f),
            BuildError::Io { context, source } => write!(f, "{context}: {source}"),
        }
    }
}

impl std::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BuildError::Project(error) => Some(error),
            BuildError::Toolchain(error) => Some(error),
            BuildError::Launcher(error) => Some(error),
            BuildError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<ProjectError> for BuildError {
    fn from(error: ProjectError) -> Self {
        BuildError::Project(error)
    }
}

impl From<ToolchainError> for BuildError {
    fn from(error: ToolchainError) -> Self {
        BuildError::Toolchain(error)
    }
}

impl From<LauncherError> for BuildError {
    fn from(error: LauncherError) -> Self {
        BuildError::Launcher(error)
    }
}

fn io_error(context: String) -> impl FnOnce(io::Error) -> BuildError {
    move |source| BuildError::Io { context, source }
}

pub fn build_project(options: &BuildProjectOptions) -> Result<BuildResult, BuildError> {
    let project_dir = &options.project_dir;
    let package_root = &options.package_root;
    let project = load_project_config(project_dir)?;

    let runtime_dir = options
        .runtime_dir
        .clone()
        .unwrap_or_else(|| package_root.join("out").join("runtime"));
    let library = runtime_dir.join("libev3.a");
    let supervisor = runtime_dir.join("supervisor");
    for (file, name) in [(&library, "libev3.a"), (&supervisor, "supervisor")] {
        if fs::metadata(file).is_err() {
            return Err(BuildError::RuntimeIncomplete {
                file: name.to_owned(),
                runtime_dir,
            });
        }
    }

    let mut sources = Vec::new();
    let entries = fs::read_dir(project_dir).map_err(io_error(format!(
        "Reading the project folder {} failed",
        project_dir.display()
    )))?;
    for entry in entries {
        let entry = entry.map_err(io_error(format!(
            "Reading the project folder {} failed",
            project_dir.display()
        )))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // file_type() does not follow links, like Node's Dirent.isFile().
        if entry.file_type().is_ok_and(|kind| kind.is_file()) && name.ends_with(".c") {
            sources.push(name);
        }
    }
    sources.sort();
    if sources.is_empty() {
        return Err(BuildError::NoSources {
            project_dir: project_dir.clone(),
        });
    }

    let compiler = ensure_compiler(&EnsureCompilerOptions {
        cache_dir: options.toolchain.cache_dir.clone(),
        compiler_override: options.toolchain.compiler_override.clone(),
        ..EnsureCompilerOptions::default()
    })?;

    // Remove this project's previous outputs first, so a failed build leaves nothing to deploy.
    let build_dir = project_dir.join(BUILD_DIR);
    let program_path = build_dir.join(PROGRAM_FILE);
    let launcher_file = format!("{}.rbf", project.name);
    let launcher_path = build_dir.join(&launcher_file);
    remove_file_if_present(&program_path)?;
    remove_file_if_present(&launcher_path)?;
    fs::create_dir_all(&build_dir)
        .map_err(io_error(format!("Creating {} failed", build_dir.display())))?;

    let request = CompileRequest {
        compiler_path: compiler.path,
        cwd: Some(project_dir.clone()),
        sources,
        include_dirs: vec![package_root.join("ev3lib").join("include")],
        // The request has no field for linker flags; libraries are passed as plain
        // arguments, so the flag goes in front of the archive it applies to.
        libraries: vec![
            OsString::from(FORCE_EV3LIB_STARTUP),
            library.into_os_string(),
        ],
        output: format!("{BUILD_DIR}/{PROGRAM_FILE}"),
    };
    let compiled = match options.compile {
        Some(step) => step(&request)?,
        None => compile(&request)?,
    };
    if !compiled.ok {
        remove_file_if_present(&program_path)?;
        return Ok(BuildResult::CompileFailed {
            diagnostics: compiled.diagnostics,
        });
    }

    let problems = check_elf(&program_path).map_err(io_error(format!(
        "Reading {} failed",
        program_path.display()
    )))?;
    if !problems.is_empty() {
        remove_file_if_present(&program_path)?;
        return Err(BuildError::NotRunnable { problems });
    }

    let launcher = build_launcher(&LauncherOptions {
        retention: project.retention,
        output_cap_bytes: project.output_cap_bytes,
    })?;
    fs::write(&launcher_path, launcher).map_err(io_error(format!(
        "Writing {} failed",
        launcher_path.display()
    )))?;

    let brick_dir = brick_project_dir(&project.name);
    let artifacts = vec![
        BuildArtifact {
            kind: ArtifactKind::Program,
            local_path: program_path,
            brick_path: format!("{brick_dir}/{PROGRAM_FILE}"),
        },
        BuildArtifact {
            kind: ArtifactKind::Launcher,
            local_path: launcher_path,
            brick_path: format!("{brick_dir}/{launcher_file}"),
        },
        BuildArtifact {
            kind: ArtifactKind::Supervisor,
            local_path: supervisor,
            brick_path: format!("{brick_dir}/{SUPERVISOR_FILE}"),
        },
    ];
    Ok(BuildResult::Built {
        project,
        artifacts,
        diagnostics: compiled.diagnostics,
    })
}

fn remove_file_if_present(path: &Path) -> Result<(), BuildError> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(BuildError::Io {
            context: format!("Deleting {} failed", path.display()),
            source: error,
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::launcher::RetentionRule;
    use crate::testutil::{repo_path, temp_dir};

    fn elf_fixture(name: &str) -> PathBuf {
        repo_path("backend/crates/core/fixtures/elf").join(name)
    }

    /// A package root with a fake runtime, a project folder, and a compiler path. The
    /// compile step is [`FakeCompiler`], which writes a fixture ELF instead of running Zig.
    struct Setup {
        temp: PathBuf,
        project_dir: PathBuf,
        package_root: PathBuf,
        runtime_dir: PathBuf,
        compiler_path: PathBuf,
    }

    fn setup(config: &str) -> Setup {
        let temp = temp_dir("ev3c-build-");
        let package_root = temp.join("package");
        let runtime_dir = package_root.join("out").join("runtime");
        fs::create_dir_all(&runtime_dir).unwrap();
        fs::write(runtime_dir.join("libev3.a"), "fake archive").unwrap();
        fs::write(runtime_dir.join("supervisor"), "fake supervisor").unwrap();
        fs::create_dir_all(package_root.join("ev3lib").join("include")).unwrap();

        let project_dir = temp.join("project");
        fs::create_dir_all(project_dir.join("notes")).unwrap();
        fs::write(project_dir.join("main.c"), "int main(void) { return 0; }\n").unwrap();
        fs::write(project_dir.join("drive.c"), "void drive(void) {}\n").unwrap();
        fs::write(project_dir.join("readme.txt"), "not a source\n").unwrap();
        fs::write(project_dir.join("notes").join("old.c"), "not compiled\n").unwrap();
        fs::write(project_dir.join("ev3c.json"), config).unwrap();

        let compiler_path = temp.join("zig");
        fs::write(&compiler_path, "").unwrap();
        Setup {
            temp,
            project_dir,
            package_root,
            runtime_dir,
            compiler_path,
        }
    }

    struct FakeCompiler {
        elf: &'static str,
        fail: Option<&'static str>,
        requests: RefCell<Vec<CompileRequest>>,
    }

    impl FakeCompiler {
        fn writing(elf: &'static str) -> Self {
            Self {
                elf,
                fail: None,
                requests: RefCell::new(Vec::new()),
            }
        }

        fn failing(diagnostics: &'static str) -> Self {
            Self {
                fail: Some(diagnostics),
                ..Self::writing("static-armv5te.elf")
            }
        }

        fn compile(&self, request: &CompileRequest) -> Result<CompileResult, ToolchainError> {
            self.requests.borrow_mut().push(request.clone());
            let elf_path = request
                .cwd
                .clone()
                .unwrap_or_default()
                .join(&request.output);
            if let Some(diagnostics) = self.fail {
                return Ok(CompileResult {
                    ok: false,
                    elf_path: request.output.clone(),
                    exit_code: Some(1),
                    diagnostics: diagnostics.to_owned(),
                });
            }
            fs::copy(elf_fixture(self.elf), elf_path).unwrap();
            Ok(CompileResult {
                ok: true,
                elf_path: request.output.clone(),
                exit_code: Some(0),
                diagnostics: String::new(),
            })
        }
    }

    impl Setup {
        fn options<'a>(&self, compile: &'a CompileStep<'a>) -> BuildProjectOptions<'a> {
            BuildProjectOptions {
                project_dir: self.project_dir.clone(),
                package_root: self.package_root.clone(),
                runtime_dir: None,
                toolchain: ToolchainSettings {
                    cache_dir: self.temp.join("cache"),
                    compiler_override: Some(CompilerOverride {
                        path: self.compiler_path.clone(),
                        setting: "test.compiler".into(),
                    }),
                },
                compile: Some(compile),
            }
        }
    }

    fn build(setup: &Setup, compiler: &FakeCompiler) -> Result<BuildResult, BuildError> {
        let step = |request: &CompileRequest| compiler.compile(request);
        build_project(&setup.options(&step))
    }

    #[test]
    fn a_successful_build_returns_program_launcher_and_supervisor_with_brick_paths_under_60_bytes()
    {
        let name = "abcdefghij-12_4";
        let setup = setup(&format!(
            r#"{{ "name": "{name}", "retention": {{ "keepMiB": 3 }}, "outputCapKiB": 32 }}"#
        ));
        let compiler = FakeCompiler::writing("static-armv5te.elf");

        let BuildResult::Built { artifacts, .. } = build(&setup, &compiler).unwrap() else {
            panic!("expected a successful build");
        };

        let folder = format!("/home/root/lms2012/prjs/{name}");
        let build_dir = setup.project_dir.join("build");
        assert_eq!(
            artifacts,
            [
                BuildArtifact {
                    kind: ArtifactKind::Program,
                    local_path: build_dir.join("prog"),
                    brick_path: format!("{folder}/prog"),
                },
                BuildArtifact {
                    kind: ArtifactKind::Launcher,
                    local_path: build_dir.join(format!("{name}.rbf")),
                    brick_path: format!("{folder}/{name}.rbf"),
                },
                BuildArtifact {
                    kind: ArtifactKind::Supervisor,
                    local_path: setup.runtime_dir.join("supervisor"),
                    brick_path: format!("{folder}/sup"),
                },
            ]
        );
        for artifact in &artifacts {
            assert!(artifact.brick_path.len() < 60, "{}", artifact.brick_path);
            assert!(
                artifact.local_path.exists(),
                "{}",
                artifact.local_path.display()
            );
        }
        let expected_launcher = build_launcher(&LauncherOptions {
            retention: RetentionRule::Megabytes { limit: 3 },
            output_cap_bytes: 32 * 1024,
        })
        .unwrap();
        assert_eq!(
            fs::read(&artifacts[1].local_path).unwrap(),
            expected_launcher
        );
        assert_eq!(
            fs::read(&artifacts[0].local_path).unwrap(),
            fs::read(elf_fixture("static-armv5te.elf")).unwrap()
        );
    }

    #[test]
    fn the_compiler_gets_the_project_sources_the_ev3lib_headers_the_forced_start_up_and_libev3_a() {
        let setup = setup(r#"{ "name": "robot" }"#);
        let compiler = FakeCompiler::writing("static-armv5te.elf");

        build(&setup, &compiler).unwrap();

        let requests = compiler.requests.borrow();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.compiler_path, setup.compiler_path);
        assert_eq!(request.cwd.as_ref(), Some(&setup.project_dir));
        assert_eq!(request.sources, ["drive.c", "main.c"]);
        assert_eq!(
            request.include_dirs,
            [setup.package_root.join("ev3lib").join("include")]
        );
        assert_eq!(
            request.libraries,
            [
                OsString::from("-Wl,-u,ev3_core_init"),
                setup.runtime_dir.join("libev3.a").into_os_string()
            ]
        );
        assert_eq!(request.output, "build/prog");
    }

    #[test]
    fn a_runtime_folder_given_by_the_caller_replaces_the_one_in_the_package() {
        let setup = setup(r#"{ "name": "robot" }"#);
        let elsewhere = setup.temp.join("runtime-copy");
        fs::create_dir(&elsewhere).unwrap();
        for file in ["libev3.a", "supervisor"] {
            fs::copy(setup.runtime_dir.join(file), elsewhere.join(file)).unwrap();
        }
        let compiler = FakeCompiler::writing("static-armv5te.elf");
        let step = |request: &CompileRequest| compiler.compile(request);

        let result = build_project(&BuildProjectOptions {
            runtime_dir: Some(elsewhere.clone()),
            ..setup.options(&step)
        })
        .unwrap();

        let BuildResult::Built { artifacts, .. } = result else {
            panic!("expected a successful build");
        };
        assert_eq!(artifacts[2].local_path, elsewhere.join("supervisor"));
        assert_eq!(
            compiler.requests.borrow()[0].libraries.last(),
            Some(&elsewhere.join("libev3.a").into_os_string())
        );
    }

    #[test]
    fn a_compile_error_returns_the_compiler_diagnostics_and_leaves_no_artifacts() {
        let setup = setup(r#"{ "name": "robot" }"#);
        let good = FakeCompiler::writing("static-armv5te.elf");
        assert!(matches!(
            build(&setup, &good).unwrap(),
            BuildResult::Built { .. }
        ));
        let diagnostics = "main.c:1:18: error: use of undeclared identifier 'x'\n";
        let failing = FakeCompiler::failing(diagnostics);

        let result = build(&setup, &failing).unwrap();

        assert_eq!(
            result,
            BuildResult::CompileFailed {
                diagnostics: diagnostics.to_owned()
            }
        );
        let left: Vec<_> = fs::read_dir(setup.project_dir.join("build"))
            .unwrap()
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn an_invalid_project_name_stops_the_build_before_compiling() {
        let setup = setup(r#"{ "name": "My Robot" }"#);
        let compiler = FakeCompiler::writing("static-armv5te.elf");
        let message = build(&setup, &compiler).unwrap_err().to_string();
        assert!(message.contains("at most 15 bytes"), "{message}");
        assert!(compiler.requests.borrow().is_empty());
    }

    #[test]
    fn a_missing_supervisor_in_the_runtime_folder_stops_the_build_with_a_clear_message() {
        let setup = setup(r#"{ "name": "robot" }"#);
        fs::remove_file(setup.runtime_dir.join("supervisor")).unwrap();
        let compiler = FakeCompiler::writing("static-armv5te.elf");
        let message = build(&setup, &compiler).unwrap_err().to_string();
        assert!(message.contains("supervisor"), "{message}");
        assert!(
            message.contains(&setup.runtime_dir.display().to_string()),
            "{message}"
        );
        assert!(compiler.requests.borrow().is_empty());
    }

    #[test]
    fn a_program_that_fails_the_elf_check_is_refused_and_no_launcher_is_written() {
        let setup = setup(r#"{ "name": "robot" }"#);
        let compiler = FakeCompiler::writing("armv7.elf");
        let message = build(&setup, &compiler).unwrap_err().to_string();
        assert!(message.contains("ARMv7"), "{message}");
        assert!(!setup.project_dir.join("build").join("robot.rbf").exists());
        assert!(!setup.project_dir.join("build").join("prog").exists());
    }

    #[test]
    fn a_project_without_c_sources_is_refused() {
        let setup = setup(r#"{ "name": "robot" }"#);
        fs::remove_file(setup.project_dir.join("main.c")).unwrap();
        fs::remove_file(setup.project_dir.join("drive.c")).unwrap();
        let compiler = FakeCompiler::writing("static-armv5te.elf");
        let message = build(&setup, &compiler).unwrap_err().to_string();
        assert!(message.contains("no .c files"), "{message}");
        assert!(compiler.requests.borrow().is_empty());
    }

    #[test]
    fn artifacts_serialize_with_the_typescript_field_names() {
        let artifact = BuildArtifact {
            kind: ArtifactKind::Launcher,
            local_path: PathBuf::from("robot.rbf"),
            brick_path: "/home/root/lms2012/prjs/robot/robot.rbf".into(),
        };
        assert_eq!(
            serde_json::to_value(&artifact).unwrap(),
            serde_json::json!({
                "kind": "launcher",
                "localPath": "robot.rbf",
                "brickPath": "/home/root/lms2012/prjs/robot/robot.rbf",
            })
        );
    }
}
