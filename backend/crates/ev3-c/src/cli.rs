//! `ev3 c …`: EVƎ C from a terminal or an agent. EVƎ Cockpit's `ev3` command runs this as the
//! add-on `ev3-c`. It does what the VS Code commands do, through the same core and a
//! [`Connector`] to the cockpit's daemon, and prints text or, with `--json`, one JSON object.

use std::collections::HashMap;
use std::future::{Future, pending};
use std::path::{Path, PathBuf};
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::mpsc;

use ev3c_core::brick::deploy::deploy;
use ev3c_core::brick::live_output::{LiveEvent, OnOutput, OutputStream};
use ev3c_core::brick::returned_files::{
    DownloadError, ReturnedReport, download_file, download_returned_files,
};
use ev3c_core::brick::run_control::{
    RunOutcome, RunProjectOptions, emergency_stop, run_project, stop_program,
};
use ev3c_core::brick::run_folders::{DeleteRunsReport, delete_runs};
use ev3c_core::brick::run_results::{
    Launches, RunReadError, RunResult, list_runs, read_run_result,
};
use ev3c_core::build::{
    BuildArtifact, BuildError, BuildProjectOptions, BuildResult, CompileStep, ToolchainSettings,
    build_project,
};
use ev3c_core::port::{
    BrickError, BrickEvent, BrickFileReader, BrickPort, Clock, ProgramState, StartReport,
    Subscription, SystemClock,
};
use ev3c_core::project::{PROJECT_FILE, ProjectConfig, ProjectError, load_project_config};
use ev3c_core::results::{RunEnd, RunRecord, library_error};
use ev3c_core::returned::returned_dir;
use ev3c_core::toolchain::{
    CompileRequest, CompileResult, CompilerOrigin, CompilerOverride, EnsureCompilerOptions,
    ToolchainError, ensure_compiler,
};

use crate::args::{parse_arguments, wants_events};
use crate::cockpit::{
    BrickHandle, ClientKind, CockpitError, ConnectOptions, Connection, Connector, online_brick,
};
use crate::format::{
    format_no_result, format_returned, format_run_result, format_run_result_shown, how_it_ended,
    iso_time,
};
use crate::paths::resolve;

pub const USAGE: &str = "Usage: ev3 c <command> [project folder] [options]

Commands:
  build                 build the project
  upload                build, then upload what changed
  run                   build, upload and run; prints the run's output and exits with
                        the program's exit code (signal n: 128+n; stopped: 130)
  stop                  stop the program running on the brick
  emergency-stop        stop every program and motor on the brick
  runs                  list the project's runs on the brick
  runs show <n>         print run n's result
  runs delete [--keep <n>]
                        delete the run folders, all or all but the newest n
  get <brick path> [local path]
                        download a file from the brick (default: its name, here)
  toolchain             check the compiler, downloading it when needed
  watch                 follow the program state of the brick until Ctrl+C
  mcp                   serve the project commands as MCP tools on stdin and stdout

The project folder holds ev3c.json; it is the current folder when none is given.
Files a run returns land in the project's returned/run-<n> folder.

Options:
  --brick <name|id>     the brick (default: EV3C_BRICK, else the brick EVƎ Cockpit's
                        window shows, else the only connected brick)
  --toolchain <path>    a compiler to use instead of the pinned one (or EV3C_TOOLCHAIN)
  --json                print one JSON object instead of text
  --events              print one JSON event per line as the command proceeds; a line
                        \"interrupt\" on stdin, or its end, acts as Ctrl+C
  --help, --version";

/// Exit codes besides a run's own.
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_STOPPED: i32 = 130;

/// The name a terminal client connects under instead of `ev3 c <command> (terminal)`.
pub const CLIENT_NAME_VARIABLE: &str = "EV3C_CLIENT_NAME";
/// Runs started earlier with their start times, as a JSON object of launch ID to
/// milliseconds since 1970: a caller that keeps them from `started` events (EVƎ C's VS Code
/// extension) hands them back, so `runs` and `runs show` can tell when those runs started.
pub const LAUNCHES_VARIABLE: &str = "EV3C_LAUNCHES";
/// The name of the setting `EV3C_TOOLCHAIN` comes from, for messages: the VS Code extension
/// sets `ev3-c.toolchain.path`, which is what its user can change.
pub const TOOLCHAIN_SETTING_VARIABLE: &str = "EV3C_TOOLCHAIN_SETTING";

/// Where a command writes: one call per message, without the line end.
pub trait Io {
    fn out(&mut self, text: &str);
    fn err(&mut self, text: &str);

    /// A piece of the program's own stdout while it runs, written as it is: the line ends are
    /// the program's. Writers that keep whole messages take it as one.
    fn out_part(&mut self, text: &str) {
        self.out(text);
    }

    /// As [`Io::out_part`], for the program's stderr.
    fn err_part(&mut self, text: &str) {
        self.err(text);
    }
}

/// Replaces the Zig compile step, for tests.
pub type CompileHook =
    dyn Fn(&CompileRequest) -> Result<CompileResult, ToolchainError> + Send + Sync;

/// What the commands need from their surroundings; tests replace all of it.
pub struct Deps<C, K = SystemClock> {
    pub connector: C,
    pub clock: K,
    /// EVƎ C's install folder, where ev3lib's headers and the brick runtime are; why it was
    /// not found otherwise. Only a build needs it.
    pub package_root: Result<PathBuf, String>,
    pub toolchain_cache_dir: PathBuf,
    pub version: String,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub compile: Option<Arc<CompileHook>>,
}

/// Who runs the command, which names the connection to the cockpit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Client {
    /// A person or a script: `ev3 c <command> (terminal)`, kind addon.
    Terminal,
    /// The MCP server for the agent named `name`: `<name> (EVƎ C MCP)`, kind mcp.
    Mcp { name: String },
}

/// Ctrl+C, or what stands for it: an MCP cancel. Each message is one Ctrl+C; once the
/// sending side is gone, none comes any more.
pub struct Interrupts(Option<mpsc::UnboundedReceiver<()>>);

impl Interrupts {
    pub fn new(receiver: mpsc::UnboundedReceiver<()>) -> Self {
        Interrupts(Some(receiver))
    }

    pub fn none() -> Self {
        Interrupts(None)
    }

    /// The next interrupt; never, when none can come.
    async fn next(&mut self) {
        loop {
            match &mut self.0 {
                Some(receiver) => match receiver.recv().await {
                    Some(()) => return,
                    None => self.0 = None,
                },
                None => pending::<()>().await,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    Json,
    /// One JSON object per line as the command proceeds; see docs/cli.md.
    Events,
}

struct Options {
    mode: Mode,
    brick: String,
    toolchain: Option<String>,
    /// Where `toolchain` came from, as messages name it: `--toolchain`, else the setting
    /// `EV3C_TOOLCHAIN_SETTING` names (the VS Code extension's), else `EV3C_TOOLCHAIN`.
    toolchain_source: String,
    keep: Option<usize>,
}

/// A failure as printed: the message, cockpit's code, and why the daemon could not start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Failure {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<String>,
}

/// Why a command ends without its report.
#[derive(Debug)]
enum Stop {
    Usage(String),
    /// Ctrl+C ended the command; the line says what became of the work.
    Interrupted(&'static str),
    Failed(Failure),
}

impl Stop {
    fn message(message: impl Into<String>) -> Self {
        Stop::Failed(Failure {
            message: message.into(),
            code: None,
            failure: None,
        })
    }
}

impl From<CockpitError> for Stop {
    fn from(error: CockpitError) -> Self {
        Stop::Failed(Failure {
            message: error.message,
            code: error.code,
            failure: error.failure,
        })
    }
}

impl From<BrickError> for Stop {
    fn from(error: BrickError) -> Self {
        CockpitError::from(error).into()
    }
}

impl From<RunReadError> for Stop {
    fn from(error: RunReadError) -> Self {
        match error {
            RunReadError::Brick(error) => error.into(),
            RunReadError::Record(error) => Stop::message(error.to_string()),
        }
    }
}

impl From<DownloadError> for Stop {
    fn from(error: DownloadError) -> Self {
        match error {
            DownloadError::Brick(error) => error.into(),
            other => Stop::message(other.to_string()),
        }
    }
}

impl From<BuildError> for Stop {
    fn from(error: BuildError) -> Self {
        Stop::message(error.to_string())
    }
}

impl From<ToolchainError> for Stop {
    fn from(error: ToolchainError) -> Self {
        Stop::message(error.to_string())
    }
}

impl From<ProjectError> for Stop {
    fn from(error: ProjectError) -> Self {
        Stop::message(error.to_string())
    }
}

/// What a command printed on success, or on an outcome that is not an error but still not
/// a success (a run without result, a stop that left a program running).
struct Report {
    pretty: String,
    compact: String,
    lines: Vec<String>,
    code: i32,
    /// The terminal event under --events: `done`, `result`, `noResult` or `interrupted`.
    event: &'static str,
}

impl Report {
    fn ending_with(mut self, event: &'static str) -> Self {
        self.event = event;
        self
    }
}

fn report<T: Serialize>(value: &T, lines: Vec<String>, code: i32) -> Result<Report, Stop> {
    let printing = |error: serde_json::Error| {
        Stop::message(format!("The result could not be printed: {error}"))
    };
    Ok(Report {
        pretty: serde_json::to_string_pretty(value).map_err(printing)?,
        compact: serde_json::to_string(value).map_err(printing)?,
        lines,
        code,
        event: "done",
    })
}

/// One line of --events: `{"event":"<name>"`, the exit code of a terminal event, then the
/// fields of `object`, the compact JSON of an object.
fn event_line(name: &str, exit_code: Option<i32>, object: &str) -> String {
    let mut line = format!("{{\"event\":\"{name}\"");
    if let Some(code) = exit_code {
        line.push_str(&format!(",\"exitCode\":{code}"));
    }
    let fields = object
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or_default();
    if !fields.is_empty() {
        line.push(',');
        line.push_str(fields);
    }
    line.push('}');
    line
}

/// The --events line for a notice of the cockpit's client, such as a daemon replaced.
pub fn notice_event(text: &str) -> String {
    let fields = serde_json::to_string(&serde_json::json!({ "text": text }))
        .unwrap_or_else(|_| "{}".to_owned());
    event_line("notice", None, &fields)
}

const NOTHING_STARTED: &str = "Interrupted; nothing was started.";
const INTERRUPTED: &str = "Interrupted.";
const LEFT: &str = "Left without waiting for the result; \"ev3 c runs\" lists the run.";
const STOPPED_BEFORE_START: &str = "Stopped before the program started.";

type PortOf<C> = <<C as Connector>::Connection as Connection>::Port;
type ReaderOf<C> = <<C as Connector>::Connection as Connection>::Reader;
type Handle<C> = BrickHandle<PortOf<C>, ReaderOf<C>>;
/// A stop sent while a run goes on.
type StopFuture<'a> = Pin<Box<dyn Future<Output = Result<ProgramState, BrickError>> + 'a>>;

/// Runs one `ev3 c` command line and returns its exit code.
pub async fn run_cli<C: Connector, K: Clock>(
    argv: &[String],
    io: &mut dyn Io,
    deps: &Deps<C, K>,
    interrupts: &mut Interrupts,
    client: &Client,
) -> i32 {
    let events = wants_events(argv);
    let parsed = match parse_arguments(argv) {
        Ok(parsed) => parsed,
        Err(message) => return usage(io, &message, events),
    };
    if parsed.version {
        io.out(&deps.version);
        return EXIT_OK;
    }
    let mut positionals = parsed.positionals.into_iter();
    let command = positionals.next();
    let rest: Vec<String> = positionals.collect();
    let command = match command {
        Some(command) if !parsed.help && command != "help" => command,
        other => {
            io.out(USAGE);
            return if other.is_none() && !parsed.help {
                EXIT_USAGE
            } else {
                EXIT_OK
            };
        }
    };
    let keep = match parsed
        .keep
        .as_deref()
        .map(|keep| whole_number(keep, "--keep"))
    {
        Some(Err(Stop::Usage(message))) => return usage(io, &message, events),
        Some(Ok(keep)) => Some(keep),
        _ => None,
    };
    let variable = |name: &str| non_empty(deps.env.get(name).map(String::as_str));
    let options = Options {
        mode: match (parsed.events, parsed.json) {
            (true, _) => Mode::Events,
            (false, true) => Mode::Json,
            (false, false) => Mode::Text,
        },
        brick: non_empty(parsed.brick.as_deref())
            .or_else(|| variable("EV3C_BRICK"))
            .unwrap_or_else(|| "active".to_owned()),
        toolchain: non_empty(parsed.toolchain.as_deref()).or_else(|| variable("EV3C_TOOLCHAIN")),
        toolchain_source: if non_empty(parsed.toolchain.as_deref()).is_some() {
            "--toolchain".to_owned()
        } else {
            variable(TOOLCHAIN_SETTING_VARIABLE).unwrap_or_else(|| "EV3C_TOOLCHAIN".to_owned())
        },
        keep,
    };

    let mut cli = Cli {
        io,
        deps,
        interrupts,
        options,
        client,
        command: &command,
    };
    match cli.dispatch(&rest).await {
        Ok(report) => {
            cli.print(&report);
            report.code
        }
        Err(Stop::Usage(message)) => usage(cli.io, &message, events),
        Err(Stop::Interrupted(line)) => {
            match report(
                &Interruption { interrupted: true },
                vec![line.to_owned()],
                EXIT_STOPPED,
            ) {
                Ok(report) => cli.print(&report.ending_with("interrupted")),
                Err(_) => cli.io.err(line),
            }
            EXIT_STOPPED
        }
        Err(Stop::Failed(failure)) => {
            cli.fail(&failure);
            EXIT_FAILED
        }
    }
}

/// A wrong command line: the usage text on standard error, and under --events the terminal
/// event a caller waits for.
fn usage(io: &mut dyn Io, message: &str, events: bool) -> i32 {
    io.err(&format!("ev3 c: {message}\n\n{USAGE}"));
    if events {
        let failure = Failure {
            message: message.to_owned(),
            code: None,
            failure: None,
        };
        if let Ok(object) = serde_json::to_string(&ErrorJson { error: &failure }) {
            io.out(&event_line("error", Some(EXIT_USAGE), &object));
        }
    }
    EXIT_USAGE
}

struct Cli<'a, C, K> {
    io: &'a mut dyn Io,
    deps: &'a Deps<C, K>,
    interrupts: &'a mut Interrupts,
    options: Options,
    client: &'a Client,
    command: &'a str,
}

impl<C: Connector, K: Clock> Cli<'_, C, K> {
    async fn dispatch(&mut self, rest: &[String]) -> Result<Report, Stop> {
        match self.command {
            "build" => {
                let dir = self.project_dir(rest, 0)?;
                self.build(&dir).await
            }
            "upload" => {
                let dir = self.project_dir(rest, 0)?;
                self.upload(&dir).await
            }
            "run" => {
                let dir = self.project_dir(rest, 0)?;
                self.run(&dir).await
            }
            "stop" | "emergency-stop" => self.stop(rest).await,
            "runs" => self.runs(rest).await,
            "toolchain" => self.toolchain(rest).await,
            "get" => self.get(rest).await,
            "watch" => self.watch(rest).await,
            other => Err(Stop::Usage(format!("unknown command \"{other}\""))),
        }
    }

    /// Prints a result: the object with --json, the terminal event with --events, the lines
    /// otherwise.
    fn print(&mut self, report: &Report) {
        match self.options.mode {
            Mode::Json => self.io.out(&report.pretty),
            Mode::Events => {
                self.io.out(&event_line(
                    report.event,
                    Some(report.code),
                    &report.compact,
                ));
            }
            Mode::Text if !report.lines.is_empty() => self.io.out(&report.lines.join("\n")),
            Mode::Text => {}
        }
    }

    fn fail(&mut self, failure: &Failure) {
        let object = ErrorJson { error: failure };
        let printed = match self.options.mode {
            Mode::Json => serde_json::to_string_pretty(&object),
            Mode::Events => serde_json::to_string(&object)
                .map(|object| event_line("error", Some(EXIT_FAILED), &object)),
            Mode::Text => {
                let code = match (&failure.failure, &failure.code) {
                    (Some(why), Some(_)) => format!(" [{why}]"),
                    (None, Some(code)) => format!(" [{code}]"),
                    _ => String::new(),
                };
                self.io.err(&format!("ev3 c: {}{code}", failure.message));
                return;
            }
        };
        match printed {
            Ok(text) => self.io.out(&text),
            Err(_) => self.io.err(&format!("ev3 c: {}", failure.message)),
        }
    }

    /// A line for the person at the terminal, on standard error; with --json too, as the
    /// TypeScript backend printed compiler messages and the stop notice there. Under --events
    /// the event `event` with the `fields` stands for it, and standard error stays quiet.
    fn notice<T: Serialize>(&mut self, text: &str, event: &str, fields: &T) {
        if self.options.mode == Mode::Events {
            self.event(event, fields);
        } else {
            self.io.err(text);
        }
    }

    /// A progress event, under --events only.
    fn event<T: Serialize>(&mut self, name: &str, fields: &T) {
        if self.options.mode == Mode::Events
            && let Ok(object) = serde_json::to_string(fields)
        {
            self.io.out(&event_line(name, None, &object));
        }
    }

    async fn build(&mut self, dir: &Path) -> Result<Report, Stop> {
        let built = self.compile(dir).await?;
        let brick_paths: Vec<&str> = built
            .artifacts
            .iter()
            .map(|artifact| artifact.brick_path.as_str())
            .collect();
        report(
            &BuildJson {
                project: &built.project.name,
                artifacts: &built.artifacts,
                diagnostics: &built.diagnostics,
            },
            vec![format!(
                "Built \"{}\": {}",
                built.project.name,
                brick_paths.join(", ")
            )],
            EXIT_OK,
        )
    }

    async fn upload(&mut self, dir: &Path) -> Result<Report, Stop> {
        let built = self.compile(dir).await?;
        let (_connection, handle) = self.brick().await?;
        let uploaded = interruptible(
            self.interrupts,
            deploy(&handle.port, &built.artifacts),
            INTERRUPTED,
        )
        .await??;
        let paths = |artifacts: &[BuildArtifact]| -> Vec<String> {
            artifacts
                .iter()
                .map(|artifact| artifact.brick_path.clone())
                .collect()
        };
        let (done, same) = (paths(&uploaded.uploaded), paths(&uploaded.unchanged));
        report(
            &UploadJson {
                project: &built.project.name,
                brick_id: &handle.brick_id,
                uploaded: &done,
                unchanged: &same,
            },
            vec![format!(
                "Uploaded \"{}\" to {}: {} file(s) uploaded, {} unchanged.",
                built.project.name,
                handle.brick_id,
                done.len(),
                same.len()
            )],
            EXIT_OK,
        )
    }

    /// Ctrl+C while building or connecting ends the command; dropping the connection aborts
    /// its requests. From the upload on it stops the program: a stop during the upload
    /// cancels the start, a stop later ends the run, whose result is still printed. A second
    /// Ctrl+C leaves without waiting.
    async fn run(&mut self, dir: &Path) -> Result<Report, Stop> {
        let built = self.compile(dir).await?;
        let (_connection, handle) = self.brick().await?;
        let project = built.project.name.clone();
        if self.options.mode == Mode::Text {
            self.io.err(&format!(
                "Uploading and running \"{project}\" on {}. Ctrl+C stops it.",
                handle.brick_id
            ));
        }

        // The start is heard while the run goes on; the loop below reports it.
        let (started_sender, mut started) = mpsc::unbounded_channel::<StartReport>();
        let on_started = move |start: &StartReport| {
            // The receiver lives until the run is over.
            let _ = started_sender.send(start.clone());
        };
        // The program's output while it runs, in text and events mode. --json prints one
        // object at the end, so it follows nothing and leaves the brick's follows to others.
        let live = self.options.mode != Mode::Json;
        let (output_sender, mut output) = mpsc::unbounded_channel::<LiveEvent>();
        let on_output = move |event: LiveEvent| {
            // The receiver lives until the run is over.
            let _ = output_sender.send(event);
        };
        // Copied out of `self`, so the run borrows the dependencies, not the command.
        let deps = self.deps;
        let launches = Launches::new();
        let mut run = pin!(run_project(
            &handle.port,
            &handle.reader,
            &deps.clock,
            RunProjectOptions {
                project: &project,
                artifacts: &built.artifacts,
                launches: &launches,
                new_launch_id: None,
                on_started: Some(&on_started),
                on_output: live.then_some(&on_output as &OnOutput),
            },
        ));
        let mut open_line = OpenLine::default();
        let mut stopping = false;
        let mut stop: Option<StopFuture<'_>> = None;
        let outcome = loop {
            // The branch futures are gone before a branch's code runs, so that code may use
            // `self` and `stop` again. The start comes first, so it is told before the end.
            tokio::select! {
                biased;
                Some(start) = started.recv() => self.tell_started(&start, &launches),
                Some(event) = output.recv() => self.tell_output(event, &mut open_line),
                outcome = &mut run => break outcome,
                () = self.interrupts.next() => {
                    if stopping {
                        return Err(Stop::Interrupted(LEFT));
                    }
                    stopping = true;
                    self.notice(
                        "Stopping the program (Ctrl+C again to leave without waiting for its result)…",
                        "stopping",
                        &Empty {},
                    );
                    stop = Some(Box::pin(stop_program(&handle.port)));
                }
                // The run hears the stop as the end of the program; its own answer adds nothing.
                _ = finish(&mut stop) => stop = None,
            }
        };
        drop(stop);
        while let Ok(start) = started.try_recv() {
            self.tell_started(&start, &launches);
        }
        // The end of the output, which the run delivers as it ends.
        while let Ok(event) = output.try_recv() {
            self.tell_output(event, &mut open_line);
        }
        open_line.close(self.io);

        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) if stopping && error.code() == Some("cancelled") => {
                return Err(Stop::Interrupted(STOPPED_BEFORE_START));
            }
            Err(error) => return Err(error.into()),
        };
        match outcome {
            RunOutcome::NoResult {
                launch_id,
                started_at,
                start,
            } => Ok(report(
                &NoResultJson {
                    project: &project,
                    brick_id: &handle.brick_id,
                    start: &start,
                    no_result: NoResultInfo {
                        launch_id: &launch_id,
                        started_at: iso_time(started_at),
                    },
                },
                format_no_result(&project, &launch_id, started_at, "\"ev3 c runs\""),
                EXIT_FAILED,
            )?
            .ending_with("noResult")),
            RunOutcome::Result { result, start } => {
                let returned = return_files(&handle.reader, &project, dir, &result).await;
                // In text mode the output was shown as it came: the result does not repeat it.
                let mut lines = if self.options.mode == Mode::Text {
                    format_run_result_shown(&project, &result)
                } else {
                    format_run_result(&project, &result)
                };
                lines.extend(returned.iter().flat_map(format_returned));
                Ok(report(
                    &RunJson {
                        project: &project,
                        brick_id: &handle.brick_id,
                        start: &start,
                        result: result_json(&result),
                        returned_files: returned.as_ref(),
                    },
                    lines,
                    run_exit_code(&result.record),
                )?
                .ending_with("result"))
            }
        }
    }

    async fn stop(&mut self, rest: &[String]) -> Result<Report, Stop> {
        let emergency = self.command == "emergency-stop";
        if !rest.is_empty() {
            return Err(Stop::Usage(format!("\"{}\" takes no folder", self.command)));
        }
        let (_connection, handle) = self.brick().await?;
        let state = if emergency {
            interruptible(self.interrupts, emergency_stop(&handle.port), INTERRUPTED).await??
        } else {
            interruptible(self.interrupts, stop_program(&handle.port), INTERRUPTED).await??
        };
        let what = if emergency { "Emergency stop" } else { "Stop" };
        let running = if state.running {
            "a program still running"
        } else {
            "no program running"
        };
        report(
            &StopJson {
                brick_id: &handle.brick_id,
                running: state.running,
                run_seq: state.run_seq,
            },
            vec![format!("{what}: the brick reports {running}.")],
            if state.running { EXIT_FAILED } else { EXIT_OK },
        )
    }

    async fn runs(&mut self, rest: &[String]) -> Result<Report, Stop> {
        match rest.first().map(String::as_str) {
            Some("show") => self.show_run(&rest[1..]).await,
            Some("delete") => self.delete_runs(&rest[1..]).await,
            _ => self.list_runs(rest).await,
        }
    }

    async fn list_runs(&mut self, rest: &[String]) -> Result<Report, Stop> {
        let project = self.project_config(&self.project_dir(rest, 0)?)?.name;
        let (_connection, handle) = self.brick().await?;
        let launches = self.known_launches();
        let runs = interruptible(
            self.interrupts,
            list_runs(&handle.port, &handle.reader, &project, &launches),
            INTERRUPTED,
        )
        .await??;
        let lines = if runs.is_empty() {
            vec![format!("\"{project}\" has no runs on the brick.")]
        } else {
            runs.iter()
                .map(|summary| {
                    let ended = summary.record.as_ref().map_or_else(
                        || "no completion record (running, or interrupted)".to_owned(),
                        how_it_ended,
                    );
                    format!("Run {}: {ended}", summary.run)
                })
                .collect()
        };
        report(
            &RunsJson {
                project: &project,
                runs: runs
                    .iter()
                    .map(|summary| RunEntryJson {
                        run: summary.run,
                        record: summary.record.as_ref().map(record_json),
                        started_at: summary.started_at.map(iso_time),
                    })
                    .collect(),
            },
            lines,
            EXIT_OK,
        )
    }

    async fn show_run(&mut self, args: &[String]) -> Result<Report, Stop> {
        let Some(number) = args.first() else {
            return Err(Stop::Usage("\"runs show\" needs a run number".to_owned()));
        };
        let run = whole_number(number, "the run number")?;
        let dir = self.project_dir(&args[1..], 0)?;
        let project = self.project_config(&dir)?.name;
        let (_connection, handle) = self.brick().await?;
        let launches = self.known_launches();
        // A run number of at most nine digits always fits.
        let run = i64::try_from(run).unwrap_or(i64::MAX);
        let result = interruptible(
            self.interrupts,
            read_run_result(&handle.port, &handle.reader, &project, run, &launches),
            INTERRUPTED,
        )
        .await??;
        let Some(result) = result else {
            return report(
                &NoRecordJson {
                    project: &project,
                    run,
                    result: (),
                },
                vec![format!(
                    "Run {run} of \"{project}\" has no completion record yet."
                )],
                EXIT_FAILED,
            );
        };
        let returned = return_files(&handle.reader, &project, &dir, &result).await;
        let mut lines = format_run_result(&project, &result);
        lines.extend(returned.iter().flat_map(format_returned));
        let failed = returned
            .as_ref()
            .is_some_and(|returned| !returned.failed.is_empty());
        Ok(report(
            &ShowJson {
                project: &project,
                result: result_json(&result),
                returned_files: returned.as_ref(),
            },
            lines,
            if failed { EXIT_FAILED } else { EXIT_OK },
        )?
        .ending_with("result"))
    }

    async fn delete_runs(&mut self, args: &[String]) -> Result<Report, Stop> {
        let project = self.project_config(&self.project_dir(args, 0)?)?.name;
        let (_connection, handle) = self.brick().await?;
        let keep = self.options.keep.unwrap_or(0);
        let deleted = interruptible(
            self.interrupts,
            delete_runs(&handle.port, &project, keep),
            INTERRUPTED,
        )
        .await??;
        let list = |runs: &[i64]| -> String {
            runs.iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut line = format!(
            "Run folders of \"{project}\": deleted [{}], kept [{}]",
            list(&deleted.deleted),
            list(&deleted.kept)
        );
        if !deleted.still_present.is_empty() {
            line.push_str(&format!(
                ", could not delete [{}]",
                list(&deleted.still_present)
            ));
        }
        report(
            &DeleteJson {
                project: &project,
                report: &deleted,
            },
            vec![line],
            if deleted.still_present.is_empty() {
                EXIT_OK
            } else {
                EXIT_FAILED
            },
        )
    }

    async fn get(&mut self, rest: &[String]) -> Result<Report, Stop> {
        let valid = |path: &String| path.starts_with('/') && !path.ends_with('/');
        let brick_path = match rest {
            [brick_path] | [brick_path, _] if valid(brick_path) => brick_path,
            _ => {
                return Err(Stop::Usage(
                    "\"get\" needs a file's full brick path, and takes one local path at most"
                        .to_owned(),
                ));
            }
        };
        let name = rest
            .get(1)
            .map_or_else(|| posix_basename(brick_path), String::as_str);
        let target = resolve(&self.deps.cwd, Path::new(name));
        let (_connection, handle) = self.brick().await?;
        let bytes = interruptible(
            self.interrupts,
            download_file(&handle.reader, brick_path, &target),
            INTERRUPTED,
        )
        .await??;
        report(
            &GetJson {
                brick_path,
                local_path: &target,
                bytes,
            },
            vec![format!(
                "Downloaded {brick_path} ({bytes} bytes) → {}",
                target.display()
            )],
            EXIT_OK,
        )
    }

    async fn toolchain(&mut self, rest: &[String]) -> Result<Report, Stop> {
        if !rest.is_empty() {
            return Err(Stop::Usage("\"toolchain\" takes no folder".to_owned()));
        }
        let cache_dir = self.deps.toolchain_cache_dir.clone();
        let compiler_override = self.compiler_override();
        // Downloading the compiler blocks; Ctrl+C still ends the command meanwhile.
        let task = tokio::task::spawn_blocking(move || {
            ensure_compiler(&EnsureCompilerOptions {
                cache_dir,
                compiler_override,
                ..EnsureCompilerOptions::default()
            })
        });
        let compiler = interruptible(self.interrupts, task, INTERRUPTED)
            .await?
            .map_err(|error| Stop::message(format!("The compiler check stopped: {error}")))??;
        let how = match (&compiler.origin, &compiler.version) {
            (CompilerOrigin::Override, _) => {
                format!("given by \"{}\"", self.options.toolchain_source)
            }
            (_, version) => format!("version {}", version.as_deref().unwrap_or("?")),
        };
        report(
            &ToolchainJson {
                path: &compiler.path,
                origin: compiler.origin,
                version: compiler.version.as_deref(),
            },
            vec![format!("Compiler: {} ({how})", compiler.path.display())],
            EXIT_OK,
        )
    }

    /// Builds the project; compiler messages go to the person, a compile error ends the
    /// command.
    async fn compile(&mut self, dir: &Path) -> Result<Built, Stop> {
        let package_root = self.deps.package_root.clone().map_err(Stop::message)?;
        let project_dir = dir.to_path_buf();
        let toolchain = ToolchainSettings {
            cache_dir: self.deps.toolchain_cache_dir.clone(),
            compiler_override: self.compiler_override(),
        };
        let hook = self.deps.compile.clone();
        self.event("building", &BuildingEvent { folder: dir });
        // The compiler runs for seconds and blocks; Ctrl+C still ends the command meanwhile.
        let task = tokio::task::spawn_blocking(move || {
            let compile = hook.as_deref().map(|hook| hook as &CompileStep);
            build_project(&BuildProjectOptions {
                project_dir,
                package_root,
                runtime_dir: None,
                toolchain,
                compile,
            })
        });
        let built = interruptible(self.interrupts, task, NOTHING_STARTED)
            .await?
            .map_err(|error| Stop::message(format!("The build stopped: {error}")))??;
        let (project, artifacts, diagnostics, ok) = match built {
            BuildResult::Built {
                project,
                artifacts,
                diagnostics,
            } => (Some(project), artifacts, diagnostics, true),
            BuildResult::CompileFailed { diagnostics } => (None, Vec::new(), diagnostics, false),
        };
        if !diagnostics.trim().is_empty() {
            self.notice(
                diagnostics.trim_end(),
                "diagnostics",
                &DiagnosticsEvent {
                    text: diagnostics.trim_end(),
                },
            );
        }
        match (ok, project) {
            (true, Some(project)) => {
                self.event(
                    "built",
                    &BuiltEvent {
                        project: &project.name,
                        artifacts: &artifacts,
                    },
                );
                Ok(Built {
                    project,
                    artifacts,
                    diagnostics,
                })
            }
            _ => Err(Stop::message(
                "The build failed: the compiler reported errors.",
            )),
        }
    }

    /// Tells a run's start under --events, with the launch ID the run took.
    fn tell_started(&mut self, start: &StartReport, launches: &Launches) {
        let launch = launches.entries().into_iter().next();
        self.event(
            "started",
            &StartedEvent {
                start,
                started_at: launch.as_ref().map(|(_, at)| iso_time(*at)),
                launch_id: launch.map(|(launch_id, _)| launch_id),
            },
        );
    }

    /// The program's output while it runs: written as it is in text mode, an `output` event
    /// under --events. Live output that cannot be had is said once; the output then comes at
    /// the end of the run through the same way.
    fn tell_output(&mut self, event: LiveEvent, open_line: &mut OpenLine) {
        match event {
            LiveEvent::Output { stream, text } => {
                if self.options.mode == Mode::Events {
                    self.event(
                        "output",
                        &OutputEvent {
                            stream: match stream {
                                OutputStream::Out => "out",
                                OutputStream::Err => "err",
                            },
                            text: &text,
                        },
                    );
                    return;
                }
                open_line.write(self.io, stream, &text);
            }
            LiveEvent::Unavailable(reason) => self.notice(
                &format!("The program's output is shown when it ends: {reason}."),
                "outputAtEnd",
                &OutputAtEndEvent { reason: &reason },
            ),
        }
    }

    /// The runs `EV3C_LAUNCHES` names; none when it is unset or not such an object, which only
    /// leaves start times unknown.
    fn known_launches(&self) -> Launches {
        let launches = Launches::new();
        let given = self
            .deps
            .env
            .get(LAUNCHES_VARIABLE)
            .and_then(|text| serde_json::from_str::<HashMap<String, u64>>(text).ok())
            .unwrap_or_default();
        for (launch_id, millis) in given {
            launches.insert(launch_id, UNIX_EPOCH + Duration::from_millis(millis));
        }
        launches
    }

    /// Follows the program state of the brick the options name, for a status bar, until an
    /// interrupt. It never connects or holds a brick: one that is not online is told as such,
    /// and the brick is chosen again whenever the bricks, the windows or the daemon change.
    /// Each state is told once, when it differs from the one told before.
    async fn watch(&mut self, rest: &[String]) -> Result<Report, Stop> {
        if !rest.is_empty() {
            return Err(Stop::Usage("\"watch\" takes no folder".to_owned()));
        }
        if self.options.mode == Mode::Json {
            return Err(Stop::Usage(
                "\"watch\" tells each change as it comes: use --events, or no option for text"
                    .to_owned(),
            ));
        }
        let connection = self.connect().await?;
        let mut changes = connection.changes();
        let mut told: Option<Watched> = None;
        let mut followed: Option<Followed<PortOf<C>>> = None;
        loop {
            let chosen = match connection.view().await {
                Ok(view) => online_brick(&self.options.brick, &view),
                Err(error) => Err(error),
            };
            match chosen {
                Ok(brick_id) if followed.as_ref().is_some_and(|f| f.brick_id == brick_id) => {}
                Ok(brick_id) => {
                    followed = None;
                    let port = connection.port(&brick_id);
                    match port.subscribe().await {
                        Ok(events) => {
                            let watched = watched_state(&brick_id, &port).await;
                            self.tell(&mut told, watched);
                            followed = Some(Followed {
                                brick_id,
                                port,
                                events,
                            });
                        }
                        Err(error) => {
                            self.tell(&mut told, Watched::NoBrick(failure_of(error.into())));
                        }
                    }
                }
                Err(error) => {
                    followed = None;
                    self.tell(&mut told, Watched::NoBrick(failure_of(error)));
                }
            }
            loop {
                tokio::select! {
                    biased;
                    () = self.interrupts.next() => return report(&Empty {}, Vec::new(), EXIT_OK),
                    // The brick's own events first: what it told before a change is still told.
                    event = next_followed(&mut followed) => match (event, &followed) {
                        (Some(BrickEvent::Program(state)), Some(f)) => {
                            let watched = Watched::Program {
                                brick_id: f.brick_id.clone(),
                                running: state.running,
                                run_seq: state.run_seq,
                            };
                            self.tell(&mut told, watched);
                        }
                        (Some(BrickEvent::Reconnected), Some(f)) => {
                            let watched = watched_state(&f.brick_id, &f.port).await;
                            self.tell(&mut told, watched);
                        }
                        // The subscription ended: choose again.
                        _ => {
                            followed = None;
                            break;
                        }
                    },
                    change = changes.recv() => match change {
                        Some(()) => break,
                        None => return Err(Stop::message("The connection to EVƎ Cockpit ended.")),
                    },
                }
            }
        }
    }

    /// Tells a state `watch` sees, unless it was the last one told.
    fn tell(&mut self, told: &mut Option<Watched>, watched: Watched) {
        if told.as_ref() == Some(&watched) {
            return;
        }
        match &watched {
            Watched::Program {
                brick_id,
                running,
                run_seq,
            } => {
                if self.options.mode == Mode::Events {
                    self.event(
                        "program",
                        &ProgramEvent {
                            brick_id,
                            running: *running,
                            run_seq: *run_seq,
                        },
                    );
                } else if *running {
                    self.io.out(&format!(
                        "{brick_id}: a program is running (start {run_seq})."
                    ));
                } else {
                    self.io.out(&format!("{brick_id}: no program is running."));
                }
            }
            Watched::NoBrick(failure) => {
                if self.options.mode == Mode::Events {
                    self.event("noBrick", &ErrorJson { error: failure });
                } else {
                    self.io.out(&format!("No brick: {}", failure.message));
                }
            }
        }
        *told = Some(watched);
    }

    /// Connects to the cockpit, under the name `EV3C_CLIENT_NAME` gives (EVƎ C's VS Code
    /// extension names itself so), else one that says which command runs.
    async fn connect(&mut self) -> Result<C::Connection, Stop> {
        let (name, kind) = match self.client {
            Client::Terminal => (
                non_empty(self.deps.env.get(CLIENT_NAME_VARIABLE).map(String::as_str))
                    .unwrap_or_else(|| format!("ev3 c {} (terminal)", self.command)),
                ClientKind::Addon,
            ),
            Client::Mcp { name } => (format!("{name} (EVƎ C MCP)"), ClientKind::Mcp),
        };
        let options = ConnectOptions {
            name,
            version: self.deps.version.clone(),
            kind,
            channel: non_empty(self.deps.env.get("EV3_COCKPIT_CHANNEL").map(String::as_str)),
        };
        let deps = self.deps;
        interruptible(
            self.interrupts,
            deps.connector.connect(&options),
            NOTHING_STARTED,
        )
        .await?
        .map_err(Stop::from)
    }

    /// Connects to the cockpit and holds the brick the options name. The connection lasts as
    /// long as the caller keeps it.
    async fn brick(&mut self) -> Result<(C::Connection, Handle<C>), Stop> {
        let connection = self.connect().await?;
        let handle = interruptible(
            self.interrupts,
            connection.brick(&self.options.brick),
            NOTHING_STARTED,
        )
        .await??;
        self.event(
            "connected",
            &ConnectedEvent {
                brick_id: &handle.brick_id,
            },
        );
        Ok((connection, handle))
    }

    fn compiler_override(&self) -> Option<CompilerOverride> {
        self.options
            .toolchain
            .as_ref()
            .map(|path| CompilerOverride {
                path: PathBuf::from(path),
                setting: self.options.toolchain_source.clone(),
            })
    }

    fn project_config(&self, dir: &Path) -> Result<ProjectConfig, Stop> {
        Ok(load_project_config(dir)?)
    }

    /// The project folder: the positional at `index`, else the current folder; it must hold
    /// ev3c.json.
    fn project_dir(&self, rest: &[String], index: usize) -> Result<PathBuf, Stop> {
        if let Some(extra) = rest.get(index + 1) {
            return Err(Stop::Usage(format!("unexpected argument \"{extra}\"")));
        }
        let given = rest.get(index).map_or(".", String::as_str);
        let dir = resolve(&self.deps.cwd, Path::new(given));
        if !dir.join(PROJECT_FILE).exists() {
            return Err(Stop::message(format!(
                "{} is not an EVƎ C project: it has no {PROJECT_FILE}.",
                dir.display()
            )));
        }
        Ok(dir)
    }
}

struct Built {
    project: ProjectConfig,
    artifacts: Vec<BuildArtifact>,
    diagnostics: String,
}

/// Runs `future` unless Ctrl+C comes first, which ends the command with `line`.
async fn interruptible<F: Future>(
    interrupts: &mut Interrupts,
    future: F,
    line: &'static str,
) -> Result<F::Output, Stop> {
    tokio::select! {
        biased;
        output = future => Ok(output),
        () = interrupts.next() => Err(Stop::Interrupted(line)),
    }
}

/// The future in `slot` to its end; never, while there is none.
async fn finish<F: Future + Unpin>(slot: &mut Option<F>) -> F::Output {
    match slot {
        Some(future) => future.await,
        None => pending().await,
    }
}

/// Downloads the files a run returned into returned/run-<n>; `None` when it returned none.
async fn return_files(
    reader: &impl BrickFileReader,
    project: &str,
    dir: &Path,
    result: &RunResult,
) -> Option<ReturnedReport> {
    if result.returned.is_empty() {
        return None;
    }
    Some(
        download_returned_files(
            reader,
            project,
            &result.returned,
            &returned_dir(dir, result.run),
        )
        .await,
    )
}

/// A run's exit code for the shell: the program's own (0 to 255), 128+n for signal n, 130
/// when stopped, 1 when the slot ended before the program started.
pub fn run_exit_code(record: &RunRecord) -> i32 {
    match record.end {
        RunEnd::Program => match record.signal {
            Some(signal) => i32::try_from(128 + signal).unwrap_or(255),
            // Clamped into 0..=255, so the conversion cannot fail.
            None => i32::try_from(record.exit.unwrap_or(0).clamp(0, 255)).unwrap_or(255),
        },
        RunEnd::Slot => EXIT_STOPPED,
        RunEnd::BeforeStart => EXIT_FAILED,
    }
}

fn whole_number(text: &str, what: &str) -> Result<usize, Stop> {
    let valid = (1..=9).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
    match text.parse() {
        Ok(number) if valid => Ok(number),
        _ => Err(Stop::Usage(format!(
            "{what} must be a whole number, got \"{text}\""
        ))),
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn posix_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

// The JSON objects --json prints, with the TypeScript backend's field names and order.

#[derive(Serialize)]
struct ErrorJson<'a> {
    error: &'a Failure,
}

#[derive(Serialize)]
struct Interruption {
    interrupted: bool,
}

#[derive(Serialize)]
struct BuildJson<'a> {
    project: &'a str,
    artifacts: &'a [BuildArtifact],
    diagnostics: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadJson<'a> {
    project: &'a str,
    brick_id: &'a str,
    uploaded: &'a [String],
    unchanged: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordJson<'a> {
    #[serde(flatten)]
    record: &'a RunRecord,
    ended: String,
}

fn record_json(record: &RunRecord) -> RecordJson<'_> {
    RecordJson {
        record,
        ended: how_it_ended(record),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResultJson<'a> {
    run: i64,
    record: RecordJson<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    out: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    err: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    returned: &'a [String],
    /// When ev3lib stopped the program on a failed call: its error line, `file:line: function: why`.
    #[serde(skip_serializing_if = "Option::is_none")]
    library_error: Option<String>,
}

fn result_json(result: &RunResult) -> ResultJson<'_> {
    ResultJson {
        run: result.run,
        record: record_json(&result.record),
        out: result.out.as_deref(),
        err: result.err.as_deref(),
        started_at: result.started_at.map(iso_time),
        returned: &result.returned,
        library_error: library_error(&result.record, result.err.as_deref()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunJson<'a> {
    project: &'a str,
    brick_id: &'a str,
    start: &'a StartReport,
    result: ResultJson<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    returned_files: Option<&'a ReturnedReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NoResultInfo<'a> {
    launch_id: &'a str,
    started_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NoResultJson<'a> {
    project: &'a str,
    brick_id: &'a str,
    start: &'a StartReport,
    no_result: NoResultInfo<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowJson<'a> {
    project: &'a str,
    result: ResultJson<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    returned_files: Option<&'a ReturnedReport>,
}

/// `result` prints as `null`.
#[derive(Serialize)]
struct NoRecordJson<'a> {
    project: &'a str,
    run: i64,
    result: (),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunEntryJson<'a> {
    run: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    record: Option<RecordJson<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
}

#[derive(Serialize)]
struct RunsJson<'a> {
    project: &'a str,
    runs: Vec<RunEntryJson<'a>>,
}

#[derive(Serialize)]
struct DeleteJson<'a> {
    project: &'a str,
    #[serde(flatten)]
    report: &'a DeleteRunsReport,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StopJson<'a> {
    brick_id: &'a str,
    running: bool,
    run_seq: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetJson<'a> {
    brick_path: &'a str,
    local_path: &'a Path,
    bytes: u64,
}

#[derive(Serialize)]
struct ToolchainJson<'a> {
    path: &'a Path,
    origin: CompilerOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<&'a str>,
}

// The fields of the progress events of --events.

#[derive(Serialize)]
struct Empty {}

#[derive(Serialize)]
struct BuildingEvent<'a> {
    folder: &'a Path,
}

#[derive(Serialize)]
struct DiagnosticsEvent<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct BuiltEvent<'a> {
    project: &'a str,
    artifacts: &'a [BuildArtifact],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectedEvent<'a> {
    brick_id: &'a str,
}

/// A piece of the program's output while it runs: `stream` is "out" or "err", `text` whole
/// UTF-8 characters with the program's own line ends.
#[derive(Serialize)]
struct OutputEvent<'a> {
    stream: &'static str,
    text: &'a str,
}

/// The output cannot be followed while the program runs; it comes in `output` events as the
/// run ends.
#[derive(Serialize)]
struct OutputAtEndEvent<'a> {
    reason: &'a str,
}

/// Whether the program's last output on this terminal ended inside a line, per stream, so the
/// lines EVƎ C writes after it start on a line of their own.
#[derive(Default)]
struct OpenLine {
    out: bool,
    err: bool,
}

impl OpenLine {
    fn write(&mut self, io: &mut dyn Io, stream: OutputStream, text: &str) {
        let open = !text.ends_with('\n');
        match stream {
            OutputStream::Out => {
                io.out_part(text);
                self.out = open;
            }
            OutputStream::Err => {
                io.err_part(text);
                self.err = open;
            }
        }
    }

    fn close(&mut self, io: &mut dyn Io) {
        if std::mem::take(&mut self.out) {
            io.out_part("\n");
        }
        if std::mem::take(&mut self.err) {
            io.err_part("\n");
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartedEvent<'a> {
    start: &'a StartReport,
    /// The run's launch ID and when this computer started it, which a caller keeps and hands
    /// back in `EV3C_LAUNCHES`.
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgramEvent<'a> {
    brick_id: &'a str,
    running: bool,
    run_seq: u64,
}

/// The brick `watch` follows now, with its program subscription.
struct Followed<P> {
    brick_id: String,
    port: P,
    events: Subscription,
}

/// What `watch` tells: the program state of the brick it follows, or why it follows none.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Watched {
    Program {
        brick_id: String,
        running: bool,
        run_seq: u64,
    },
    NoBrick(Failure),
}

async fn watched_state<P: BrickPort>(brick_id: &str, port: &P) -> Watched {
    match port.program_state().await {
        Ok(state) => Watched::Program {
            brick_id: brick_id.to_owned(),
            running: state.running,
            run_seq: state.run_seq,
        },
        Err(error) => Watched::NoBrick(failure_of(error.into())),
    }
}

/// The next event of the followed brick; never, while none is followed.
async fn next_followed<P>(followed: &mut Option<Followed<P>>) -> Option<BrickEvent> {
    match followed {
        Some(followed) => followed.events.next().await,
        None => pending().await,
    }
}

fn failure_of(error: CockpitError) -> Failure {
    Failure {
        message: error.message,
        code: error.code,
        failure: error.failure,
    }
}
