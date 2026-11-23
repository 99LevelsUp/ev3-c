//! The `ev3 c` commands against the fake cockpit, with the real core: a build writes the
//! fixture ELF, and a run goes through the core's Run on the fake brick.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use ev3c_core::brick::fake::{FakeBrickPort, TestClock};
use ev3c_core::results::{RunEnd, RunRecord};

use crate::cli::{
    Client, Deps, EXIT_FAILED, EXIT_OK, EXIT_STOPPED, EXIT_USAGE, Interrupts, run_cli,
    run_exit_code,
};
use crate::cockpit::{CockpitError, Unavailable};
use crate::testing::{
    BRICK_ID, Captured, Ending, FakeCockpit, FakeConnector, OTHER_ID, compiler, deps, project,
    script_run,
};

const PRJ: &str = "/home/root/lms2012/prjs/hello";

struct Setup {
    cockpit: Arc<FakeCockpit>,
    deps: Deps<FakeConnector, TestClock>,
    interrupt: RefCell<Option<mpsc::UnboundedSender<()>>>,
}

struct Output {
    code: i32,
    out: Vec<String>,
    err: Vec<String>,
}

impl Output {
    fn json(&self) -> Value {
        assert_eq!(self.out.len(), 1, "one object: {:?}", self.out);
        serde_json::from_str(&self.out[0]).unwrap()
    }

    fn text(&self) -> String {
        self.out.join("\n")
    }
}

impl Setup {
    fn new() -> Self {
        let cockpit = FakeCockpit::new();
        let deps = deps(&cockpit);
        project(&deps.cwd, "hello");
        Setup {
            cockpit,
            deps,
            interrupt: RefCell::new(None),
        }
    }

    /// The project folder, which the commands find as "hello" below the working folder.
    fn dir(&self) -> std::path::PathBuf {
        self.deps.cwd.join("hello")
    }

    fn port(&self) -> FakeBrickPort {
        self.cockpit.port(BRICK_ID)
    }

    async fn cli(&self, args: &[&str]) -> Output {
        let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let (sender, receiver) = mpsc::unbounded_channel();
        *self.interrupt.borrow_mut() = Some(sender);
        let mut io = Captured::default();
        let code = run_cli(
            &argv,
            &mut io,
            &self.deps,
            &mut Interrupts::new(receiver),
            &Client::Terminal,
        )
        .await;
        Output {
            code,
            out: io.out,
            err: io.err,
        }
    }

    /// Ctrl+C for the command running now.
    fn interrupt(&self) {
        if let Some(sender) = &*self.interrupt.borrow() {
            sender.send(()).unwrap();
        }
    }

    /// Waits until the program runs on the brick, without moving paused time.
    async fn until_running(&self) {
        while !self.port().with(|brick| brick.running) {
            tokio::task::yield_now().await;
        }
    }
}

fn exits_with(record: &'static str) -> Ending {
    Ending {
        record,
        out: "hello\n",
        returned: None,
        stops: true,
    }
}

#[tokio::test]
async fn version_help_and_a_missing_or_unknown_command() {
    let s = Setup::new();
    let version = s.cli(&["--version"]).await;
    assert_eq!(
        (version.code, version.out),
        (EXIT_OK, vec!["1.2.3".to_owned()])
    );
    let help = s.cli(&["--help"]).await;
    assert_eq!(help.code, EXIT_OK);
    assert!(help.out[0].starts_with("Usage: ev3 c <command>"));
    assert_eq!(s.cli(&[]).await.code, EXIT_USAGE);
    let unknown = s.cli(&["fly"]).await;
    assert_eq!(unknown.code, EXIT_USAGE);
    assert!(unknown.err[0].starts_with("ev3 c: unknown command \"fly\"\n\nUsage:"));
    assert_eq!(
        s.cli(&["runs", "delete", "--keep", "many"]).await.code,
        EXIT_USAGE
    );
    assert_eq!(s.cli(&["stop", "--brick"]).await.code, EXIT_USAGE);
    assert!(
        s.cockpit.names().is_empty(),
        "no connection for any of these"
    );
}

#[tokio::test]
async fn stop_names_the_command_and_the_terminal_to_the_cockpit_holds_the_brick_and_stops_it() {
    let s = Setup::new();
    let stopped = s.cli(&["stop"]).await;
    assert_eq!(stopped.code, EXIT_OK);
    assert_eq!(s.cockpit.names(), ["ev3 c stop (terminal)"]);
    assert_eq!(s.cockpit.calls(), [format!("bricks/connect {BRICK_ID}")]);
    assert_eq!(s.port().calls(), ["stop_program"]);
    assert_eq!(
        stopped.text(),
        "Stop: the brick reports no program running."
    );
    assert_eq!(
        s.cockpit.open.load(Ordering::Relaxed),
        0,
        "the connection is closed at the end"
    );

    let emergency = s.cli(&["emergency-stop", "--json"]).await;
    assert_eq!(emergency.code, EXIT_OK);
    assert_eq!(
        emergency.json(),
        json!({ "brickId": BRICK_ID, "running": false, "runSeq": 0 })
    );
    assert!(
        emergency.out[0].starts_with("{\n  \"brickId\""),
        "pretty-printed with two spaces, in the TypeScript backend's order"
    );
    assert_eq!(
        s.cli(&["stop", "robot"]).await.code,
        EXIT_USAGE,
        "stop takes no folder"
    );
}

#[tokio::test]
async fn the_brick_comes_from_brick_else_ev3c_brick() {
    let mut s = Setup::new();
    s.cockpit.add_brick(OTHER_ID, "BBB");
    assert_eq!(s.cli(&["stop", "--brick", "bbb"]).await.code, EXIT_OK);
    s.deps.env.insert("EV3C_BRICK".to_owned(), "BBB".to_owned());
    assert_eq!(s.cli(&["emergency-stop"]).await.code, EXIT_OK);
    assert_eq!(
        s.cockpit.port(OTHER_ID).calls(),
        ["stop_program", "emergency_stop"]
    );
    assert!(s.port().calls().is_empty());

    s.deps.env.remove("EV3C_BRICK");
    let ambiguous = s.cli(&["stop"]).await;
    assert_eq!(ambiguous.code, EXIT_FAILED);
    assert!(
        ambiguous.err[0].ends_with("[brick_ambiguous]"),
        "{}",
        ambiguous.err[0]
    );
}

#[tokio::test]
async fn build_builds_without_connecting_and_reports_the_artifacts() {
    let mut s = Setup::new();
    s.deps.compile = Some(compiler("main.c:1: warning: unused", true));
    let built = s.cli(&["build", "hello"]).await;
    assert_eq!(built.code, EXIT_OK);
    assert_eq!(
        built.text(),
        format!("Built \"hello\": {PRJ}/prog, {PRJ}/hello.rbf, {PRJ}/sup")
    );
    assert_eq!(
        built.err,
        ["main.c:1: warning: unused"],
        "compiler messages go to stderr"
    );
    assert!(s.cockpit.names().is_empty());

    let json = s.cli(&["build", "hello", "--json"]).await.json();
    assert_eq!(json["project"], "hello");
    assert_eq!(json["diagnostics"], "main.c:1: warning: unused");
    assert_eq!(json["artifacts"][0]["kind"], "program");
    assert_eq!(json["artifacts"][0]["brickPath"], format!("{PRJ}/prog"));
    let local = s.dir().join("build").join("prog");
    assert_eq!(
        json["artifacts"][0]["localPath"],
        local.to_string_lossy().as_ref()
    );
}

#[tokio::test]
async fn a_compile_error_fails_the_build_after_the_compilers_messages() {
    let mut s = Setup::new();
    s.deps.compile = Some(compiler("main.c:1: error: expected ';'", false));
    let failed = s.cli(&["build", "hello"]).await;
    assert_eq!(failed.code, EXIT_FAILED);
    assert_eq!(
        failed.err,
        [
            "main.c:1: error: expected ';'",
            "ev3 c: The build failed: the compiler reported errors."
        ]
    );
}

#[tokio::test]
async fn upload_uploads_what_changed() {
    let s = Setup::new();
    let first = s.cli(&["upload", "hello"]).await;
    assert_eq!(first.code, EXIT_OK);
    assert_eq!(
        first.text(),
        format!("Uploaded \"hello\" to {BRICK_ID}: 3 file(s) uploaded, 0 unchanged.")
    );
    let again = s.cli(&["upload", "hello", "--json"]).await.json();
    assert_eq!(
        again,
        json!({
            "project": "hello",
            "brickId": BRICK_ID,
            "uploaded": [],
            "unchanged": [format!("{PRJ}/prog"), format!("{PRJ}/hello.rbf"), format!("{PRJ}/sup")],
        })
    );
}

#[tokio::test]
async fn run_prints_the_result_and_exits_with_the_programs_own_code() {
    let s = Setup::new();
    script_run(&s.port(), "hello", 1, exits_with("end=program\nexit=3\n"));
    let ran = s.cli(&["run", "hello"]).await;
    assert_eq!(ran.code, 3);
    let text = ran.text();
    assert!(text.contains("--- hello, run 1 ---"), "{text}");
    assert!(
        text.contains("Ended: the program exited with code 3"),
        "{text}"
    );
    assert!(text.contains("Output (stdout):\n  hello"), "{text}");
    assert!(
        text.contains(" (PC time)"),
        "the run was started here: {text}"
    );
    assert_eq!(
        ran.err,
        [format!(
            "Uploading and running \"hello\" on {BRICK_ID}. Ctrl+C stops it."
        )]
    );
    assert!(
        s.port()
            .calls()
            .contains(&format!("deploy_and_run {PRJ}/hello.rbf")),
        "the build's artifacts go with the start"
    );
}

#[tokio::test]
async fn run_json_prints_one_object_with_the_start_and_the_result() {
    let s = Setup::new();
    script_run(&s.port(), "hello", 1, exits_with("end=program\nexit=0\n"));
    let ran = s.cli(&["run", "hello", "--json"]).await;
    assert_eq!(ran.code, EXIT_OK);
    let printed = ran.json();
    assert_eq!(printed["project"], "hello");
    assert_eq!(printed["brickId"], BRICK_ID);
    assert_eq!(printed["start"]["outcome"], "started");
    assert_eq!(printed["start"]["runSeq"], 1);
    assert_eq!(printed["result"]["run"], 1);
    assert_eq!(printed["result"]["out"], "hello\n");
    assert_eq!(printed["result"]["record"]["exit"], 0);
    assert_eq!(
        printed["result"]["record"]["ended"],
        "the program exited with code 0"
    );
    assert_eq!(printed["result"]["returned"], json!([]));
    let started = printed["result"]["startedAt"].as_str().unwrap();
    assert!(started.ends_with('Z') && started.len() == 24, "{started}");
    assert!(printed.get("returnedFiles").is_none());
    assert!(
        ran.err.is_empty(),
        "nothing but the object, not even the start line"
    );
}

#[test]
fn a_runs_exit_code_is_the_programs_128_plus_n_for_a_signal_130_when_stopped() {
    let record = |end, exit, signal| RunRecord {
        run: 1,
        end,
        exit,
        signal,
        out_truncated: false,
        err_truncated: false,
        launch: None,
    };
    assert_eq!(run_exit_code(&record(RunEnd::Program, Some(7), None)), 7);
    assert_eq!(
        run_exit_code(&record(RunEnd::Program, Some(300), None)),
        255
    );
    assert_eq!(run_exit_code(&record(RunEnd::Program, Some(-1), None)), 0);
    assert_eq!(run_exit_code(&record(RunEnd::Program, None, Some(11))), 139);
    assert_eq!(
        run_exit_code(&record(RunEnd::Slot, None, Some(15))),
        EXIT_STOPPED
    );
    assert_eq!(
        run_exit_code(&record(RunEnd::BeforeStart, None, None)),
        EXIT_FAILED
    );
}

#[tokio::test]
async fn ctrl_c_during_a_run_stops_the_program_and_still_prints_its_result() {
    let s = Setup::new();
    script_run(
        &s.port(),
        "hello",
        1,
        Ending {
            record: "end=slot\nsignal=15\n",
            out: "",
            returned: None,
            stops: false,
        },
    );
    let (ran, ()) = tokio::join!(s.cli(&["run", "hello"]), async {
        s.until_running().await;
        s.interrupt();
    });
    assert_eq!(ran.code, EXIT_STOPPED);
    assert!(
        ran.err
            .iter()
            .any(|line| line.starts_with("Stopping the program"))
    );
    assert!(
        ran.text()
            .contains("the program was stopped when its slot ended (signal 15)"),
        "{}",
        ran.text()
    );
    assert!(s.port().calls().contains(&"stop_program".to_owned()));
}

#[tokio::test]
async fn a_second_ctrl_c_leaves_without_waiting_for_the_result() {
    let s = Setup::new();
    script_run(
        &s.port(),
        "hello",
        1,
        Ending {
            record: "end=slot\nsignal=15\n",
            out: "",
            returned: None,
            stops: false,
        },
    );
    let (ran, ()) = tokio::join!(s.cli(&["run", "hello", "--json"]), async {
        s.until_running().await;
        // The brick is out of reach: the stop fails and the program goes on.
        s.port().with(|brick| brick.disconnected = true);
        s.interrupt();
        while !s.port().calls().contains(&"stop_program".to_owned()) {
            tokio::task::yield_now().await;
        }
        s.interrupt();
    });
    assert_eq!(ran.code, EXIT_STOPPED);
    assert_eq!(ran.json(), json!({ "interrupted": true }));
    assert!(
        ran.err[0].starts_with("Stopping the program"),
        "the stop notice goes to stderr with --json too"
    );
}

#[tokio::test]
async fn ctrl_c_while_building_ends_the_command_and_starts_nothing() {
    let mut s = Setup::new();
    let building = Arc::new(AtomicBool::new(false));
    let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let (seen, gate) = (building.clone(), release.clone());
    let real = compiler("", true);
    s.deps.compile = Some(Arc::new(move |request| {
        seen.store(true, Ordering::Relaxed);
        let (lock, wake) = &*gate;
        let released = lock.lock().unwrap();
        let _unused = wake
            .wait_timeout_while(released, Duration::from_secs(10), |released| !*released)
            .unwrap();
        real(request)
    }));
    let (ran, ()) = tokio::join!(s.cli(&["run", "hello"]), async {
        while !building.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(1));
            tokio::task::yield_now().await;
        }
        s.interrupt();
    });
    *release.0.lock().unwrap() = true;
    release.1.notify_all();
    assert_eq!(ran.code, EXIT_STOPPED);
    assert_eq!(ran.text(), "Interrupted; nothing was started.");
    assert!(s.cockpit.names().is_empty());
}

#[tokio::test]
async fn a_cockpit_refusal_is_printed_with_its_code_and_json_prints_it_as_an_object() {
    let s = Setup::new();
    *s.cockpit.brick_failure.lock().unwrap() = Some(CockpitError::new("brick_busy", "busy"));
    let busy = s.cli(&["stop"]).await;
    assert_eq!(busy.code, EXIT_FAILED);
    assert_eq!(busy.err, ["ev3 c: busy [brick_busy]"]);

    *s.cockpit.brick_failure.lock().unwrap() = Some(CockpitError::new("timeout", "no answer"));
    let timeout = s.cli(&["stop", "--json"]).await;
    assert_eq!(timeout.code, EXIT_FAILED);
    assert_eq!(
        timeout.json(),
        json!({ "error": { "message": "no answer", "code": "timeout" } })
    );

    // A brick call's own failure keeps its code too.
    s.port().with(|brick| brick.disconnected = true);
    let lost = s.cli(&["runs", "show", "1", "hello"]).await;
    assert_eq!(lost.err, ["ev3 c: not_connected [not_connected]"]);
}

#[tokio::test]
async fn a_daemon_that_cannot_start_says_why() {
    let s = Setup::new();
    *s.cockpit.connect_failure.lock().unwrap() = Some(
        CockpitError::new("daemon_unavailable", "EVƎ Cockpit is not installed.")
            .with_failure("missing_binary"),
    );
    let failed = s.cli(&["stop"]).await;
    assert_eq!(failed.code, EXIT_FAILED);
    assert_eq!(
        failed.err,
        ["ev3 c: EVƎ Cockpit is not installed. [missing_binary]"]
    );
}

#[tokio::test]
async fn a_folder_without_ev3c_json_is_refused_before_anything_is_built_or_connected() {
    let s = Setup::new();
    let empty = crate::testing::temp_dir("ev3c-empty-");
    let refused = s.cli(&["build", empty.to_str().unwrap()]).await;
    assert_eq!(refused.code, EXIT_FAILED);
    assert!(
        refused.err[0].ends_with("is not an EVƎ C project: it has no ev3c.json."),
        "{}",
        refused.err[0]
    );
    assert!(!empty.join("build").exists());
    assert!(s.cockpit.names().is_empty());
    assert_eq!(s.cli(&["build", "hello", "more"]).await.code, EXIT_USAGE);
}

#[tokio::test]
async fn runs_lists_shows_and_deletes_with_the_project_from_ev3c_json() {
    let s = Setup::new();
    s.port().with(|brick| {
        brick.put_file(&format!("{PRJ}/runs/next"), "5");
        brick.mkdirs(&format!("{PRJ}/runs/3"));
        brick.put_file(&format!("{PRJ}/runs/3/out"), "");
        brick.put_file(
            &format!("{PRJ}/runs/4/res"),
            "run=4\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n",
        );
        brick.put_file(&format!("{PRJ}/runs/4/out"), "hi\n");
    });
    // From inside the project folder, as a person would type it.
    let mut inside = Setup::new();
    inside.cockpit = s.cockpit.clone();
    inside.deps.connector = FakeConnector(s.cockpit.clone());
    inside.deps.cwd = s.dir();

    let listed = inside.cli(&["runs"]).await;
    assert_eq!(listed.code, EXIT_OK);
    assert_eq!(
        listed.text(),
        "Run 3: no completion record (running, or interrupted)\nRun 4: the program exited with code 0"
    );
    let json = s.cli(&["runs", "hello", "--json"]).await.json();
    assert_eq!(json["runs"][0], json!({ "run": 3 }));
    assert_eq!(
        json["runs"][1]["record"]["ended"],
        "the program exited with code 0"
    );

    let shown = s.cli(&["runs", "show", "4", "hello"]).await;
    assert_eq!(shown.code, EXIT_OK);
    assert!(
        shown
            .text()
            .contains("Started: not started by EVƎ C in this session")
    );
    // A run folder without its record yet.
    s.port()
        .with(|brick| brick.mkdirs(&format!("{PRJ}/runs/9")));
    let missing = s.cli(&["runs", "show", "9", "hello", "--json"]).await;
    assert_eq!(missing.code, EXIT_FAILED);
    assert_eq!(
        missing.json(),
        json!({ "project": "hello", "run": 9, "result": null })
    );
    assert_eq!(s.cli(&["runs", "show"]).await.code, EXIT_USAGE);
    assert_eq!(
        s.cli(&["runs", "show", "x1", "hello"]).await.code,
        EXIT_USAGE
    );

    let deleted = s
        .cli(&["runs", "delete", "hello", "--keep", "1", "--json"])
        .await;
    assert_eq!(deleted.code, EXIT_OK);
    assert_eq!(
        deleted.json(),
        json!({ "project": "hello", "deleted": [3, 4], "kept": [9], "stillPresent": [] })
    );
    let again = s.cli(&["runs", "delete", "hello"]).await;
    assert_eq!(
        again.text(),
        "Run folders of \"hello\": deleted [9], kept []"
    );
}

#[tokio::test]
async fn run_downloads_the_files_the_run_returned_into_returned_run_n_and_reports_them() {
    let s = Setup::new();
    let ending = |stops| Ending {
        record: "end=program\nexit=0\n",
        out: "",
        returned: Some("/home/root/lms2012/prjs/hello/log.csv\n"),
        stops,
    };
    script_run(&s.port(), "hello", 4, ending(true));
    s.port()
        .with(|brick| brick.put_file(&format!("{PRJ}/log.csv"), "1,2\n"));
    let ran = s.cli(&["run", "hello", "--json"]).await;
    assert_eq!(ran.code, EXIT_OK);
    let printed = ran.json();
    let local = s.dir().join("returned").join("run-4").join("log.csv");
    assert_eq!(
        printed["result"]["returned"],
        json!([format!("{PRJ}/log.csv")])
    );
    assert_eq!(
        printed["returnedFiles"],
        json!({ "downloaded": [{ "brickPath": format!("{PRJ}/log.csv"), "localPath": local }], "failed": [] })
    );
    assert_eq!(std::fs::read_to_string(&local).unwrap(), "1,2\n");

    script_run(&s.port(), "hello", 5, ending(true));
    let text = s.cli(&["run", "hello"]).await.text();
    assert!(
        text.contains(&format!("Returned files:\n  {PRJ}/log.csv")),
        "{text}"
    );
    assert!(
        text.contains(&format!("Returned {PRJ}/log.csv → ")),
        "{text}"
    );
}

#[tokio::test]
async fn a_run_that_returned_nothing_downloads_nothing() {
    let s = Setup::new();
    script_run(&s.port(), "hello", 1, exits_with("end=program\nexit=0\n"));
    assert_eq!(s.cli(&["run", "hello"]).await.code, EXIT_OK);
    assert!(!s.dir().join("returned").exists());
}

#[tokio::test(start_paused = true)]
async fn a_run_whose_record_never_appears_has_no_result_and_fails() {
    let s = Setup::new();
    let handle = s.port().handle();
    s.port().with(|brick| {
        brick.on_run = Some(Box::new(move |_| {
            handle.later(Duration::ZERO, ev3c_core::brick::fake::FakeBrick::stop);
        }));
    });
    let ran = s.cli(&["run", "hello", "--json"]).await;
    assert_eq!(ran.code, EXIT_FAILED);
    let printed = ran.json();
    assert_eq!(printed["start"]["outcome"], "started");
    assert!(
        printed["noResult"]["launchId"]
            .as_str()
            .is_some_and(|id| id.len() == 16)
    );
    assert!(
        printed["noResult"]["startedAt"]
            .as_str()
            .is_some_and(|at| at.ends_with('Z'))
    );
    let text = s.cli(&["run", "hello"]).await.text();
    assert!(
        text.contains("\"ev3 c runs\" lists the run if its record appears later"),
        "{text}"
    );
}

#[tokio::test]
async fn get_downloads_one_brick_file_by_default_under_its_own_name_here() {
    let s = Setup::new();
    s.port()
        .with(|brick| brick.put_file(&format!("{PRJ}/log.csv"), "1,2\n"));
    let got = s.cli(&["get", &format!("{PRJ}/log.csv")]).await;
    assert_eq!(got.code, EXIT_OK);
    assert_eq!(
        std::fs::read_to_string(s.deps.cwd.join("log.csv")).unwrap(),
        "1,2\n"
    );
    assert!(
        got.text()
            .starts_with(&format!("Downloaded {PRJ}/log.csv (4 bytes) → "))
    );

    let copy = s
        .cli(&["get", &format!("{PRJ}/log.csv"), "copy.csv", "--json"])
        .await;
    assert_eq!(copy.json()["bytes"], 4);
    assert_eq!(
        std::fs::read_to_string(s.deps.cwd.join("copy.csv")).unwrap(),
        "1,2\n"
    );
    assert_eq!(
        s.cli(&["get", "log.csv"]).await.code,
        EXIT_USAGE,
        "a brick path is absolute"
    );
    assert_eq!(s.cli(&["get"]).await.code, EXIT_USAGE);
    assert_eq!(s.cli(&["get", "/a/", "x"]).await.code, EXIT_USAGE);
}

#[tokio::test]
async fn toolchain_reports_the_compiler_given_by_the_setting() {
    let s = Setup::new();
    let checked = s.cli(&["toolchain", "--json"]).await;
    assert_eq!(checked.code, EXIT_OK);
    let printed = checked.json();
    assert_eq!(printed["origin"], "override");
    assert!(printed.get("version").is_none());
    let text = s.cli(&["toolchain"]).await.text();
    assert!(
        text.ends_with("(given by --toolchain or EV3C_TOOLCHAIN)"),
        "{text}"
    );
    let missing = s.cli(&["toolchain", "--toolchain", "/no/such/zig"]).await;
    assert_eq!(missing.code, EXIT_FAILED);
    assert!(
        missing.err[0].contains("\"--toolchain\" does not exist"),
        "{}",
        missing.err[0]
    );
}

#[tokio::test]
async fn without_a_cockpit_connection_builds_work_and_brick_commands_fail_cleanly() {
    let fake = Setup::new();
    let deps = Deps {
        connector: Unavailable,
        clock: TestClock::new(),
        package_root: fake.deps.package_root.clone(),
        toolchain_cache_dir: fake.deps.toolchain_cache_dir.clone(),
        version: "1.2.3".to_owned(),
        cwd: fake.deps.cwd.clone(),
        env: fake.deps.env.clone(),
        compile: fake.deps.compile.clone(),
    };
    let run = async |args: &[&str]| {
        let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let mut io = Captured::default();
        let code = run_cli(
            &argv,
            &mut io,
            &deps,
            &mut Interrupts::none(),
            &Client::Terminal,
        )
        .await;
        (code, io)
    };
    assert_eq!(run(&["build", "hello"]).await.0, EXIT_OK);
    let (code, io) = run(&["run", "hello", "--json"]).await;
    assert_eq!(code, EXIT_FAILED);
    let printed: Value = serde_json::from_str(&io.out[0]).unwrap();
    assert_eq!(printed["error"]["code"], "daemon_unavailable");
    assert!(
        printed["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("issue #13 U4"))
    );
}

#[tokio::test]
async fn a_missing_install_folder_fails_only_the_build() {
    let mut s = Setup::new();
    s.deps.package_root = Err("EVƎ C's files were not found.".to_owned());
    let failed = s.cli(&["build", "hello"]).await;
    assert_eq!(failed.err, ["ev3 c: EVƎ C's files were not found."]);
    assert_eq!(s.cli(&["stop"]).await.code, EXIT_OK);
}

/// The events a command printed, checked to be one JSON object per line that ends with
/// exactly one terminal event: the only one with an exit code, which is the command's.
fn events(output: &Output) -> Vec<Value> {
    let events: Vec<Value> = output
        .out
        .iter()
        .map(|line| {
            assert!(!line.contains('\n'), "one line per event: {line}");
            serde_json::from_str(line).unwrap()
        })
        .collect();
    let terminal: Vec<&Value> = events
        .iter()
        .filter(|event| event.get("exitCode").is_some())
        .collect();
    assert_eq!(terminal.len(), 1, "one terminal event: {events:?}");
    let last = events.last().unwrap();
    assert_eq!(
        last["exitCode"], output.code,
        "the terminal event comes last"
    );
    events
}

fn names(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event["event"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn run_events_tell_the_build_the_brick_the_start_and_end_with_the_result() {
    let mut s = Setup::new();
    s.deps.compile = Some(compiler("main.c:1: warning: unused", true));
    script_run(&s.port(), "hello", 1, exits_with("end=program\nexit=3\n"));
    let ran = s.cli(&["run", "hello", "--events"]).await;
    assert_eq!(ran.code, 3);
    assert!(
        ran.err.is_empty(),
        "standard error stays quiet: {:?}",
        ran.err
    );
    let events = events(&ran);
    assert_eq!(
        names(&events),
        [
            "building",
            "diagnostics",
            "built",
            "connected",
            "started",
            "result"
        ]
    );
    assert!(
        ran.out[0].starts_with("{\"event\":\"building\""),
        "the name comes first"
    );
    assert_eq!(events[0]["folder"], s.dir().to_string_lossy().as_ref());
    assert_eq!(events[1]["text"], "main.c:1: warning: unused");
    assert_eq!(events[2]["project"], "hello");
    assert_eq!(events[2]["artifacts"][1]["kind"], "launcher");
    assert_eq!(events[3]["brickId"], BRICK_ID);
    assert_eq!(events[4]["start"]["runSeq"], 1);
    assert_eq!(events[5]["exitCode"], 3);
    assert_eq!(
        events[5]["result"]["record"]["ended"],
        "the program exited with code 3"
    );
    assert_eq!(events[5]["start"], events[4]["start"]);
}

#[tokio::test]
async fn an_interrupt_under_events_stops_the_program_and_the_result_still_ends_the_events() {
    let s = Setup::new();
    script_run(
        &s.port(),
        "hello",
        1,
        Ending {
            record: "end=slot\nsignal=15\n",
            out: "",
            returned: None,
            stops: false,
        },
    );
    let (ran, ()) = tokio::join!(s.cli(&["run", "hello", "--events", "--json"]), async {
        s.until_running().await;
        s.interrupt();
    });
    assert_eq!(ran.code, EXIT_STOPPED);
    let events = events(&ran);
    assert_eq!(
        names(&events),
        [
            "building",
            "built",
            "connected",
            "started",
            "stopping",
            "result"
        ],
        "--events wins over --json"
    );
    assert!(ran.err.is_empty());
}

#[tokio::test]
async fn every_way_a_command_ends_is_one_terminal_event() {
    let mut s = Setup::new();
    let stopped = s.cli(&["stop", "--events"]).await;
    let stop_events = events(&stopped);
    assert_eq!(names(&stop_events), ["connected", "done"]);
    assert_eq!(stop_events[1]["running"], false);

    let usage = s.cli(&["fly", "--events"]).await;
    assert_eq!(usage.code, EXIT_USAGE);
    assert_eq!(
        events(&usage),
        [
            json!({ "event": "error", "exitCode": 2, "error": { "message": "unknown command \"fly\"" } })
        ]
    );
    assert!(
        usage.err[0].contains("Usage:"),
        "the usage text still goes to stderr"
    );
    assert_eq!(
        events(&s.cli(&["--brick", "--events"]).await)[0]["exitCode"],
        2
    );

    *s.cockpit.brick_failure.lock().unwrap() = Some(CockpitError::new("brick_busy", "busy"));
    let busy = s.cli(&["runs", "hello", "--events"]).await;
    assert_eq!(
        events(&busy),
        [
            json!({ "event": "error", "exitCode": 1, "error": { "message": "busy", "code": "brick_busy" } })
        ]
    );

    s.deps.compile = Some(compiler("main.c:1: error: expected ';'", false));
    let failed = s.cli(&["build", "hello", "--events"]).await;
    let events_of_failure = events(&failed);
    assert_eq!(
        names(&events_of_failure),
        ["building", "diagnostics", "error"]
    );
    assert_eq!(
        events_of_failure[2]["error"]["message"],
        "The build failed: the compiler reported errors."
    );

    s.deps.compile = Some(compiler("", true));
    let showed = s.cli(&["runs", "show", "1", "hello", "--events"]).await;
    assert_eq!(
        names(&events(&showed)),
        ["connected", "error"],
        "no run folder 1 yet"
    );
}

#[tokio::test]
async fn an_interrupt_before_the_start_ends_the_events_with_interrupted() {
    let s = Setup::new();
    let (sender, receiver) = mpsc::unbounded_channel();
    sender.send(()).unwrap();
    let argv: Vec<String> = ["run", "hello", "--events"].map(str::to_owned).to_vec();
    let mut io = Captured::default();
    let code = run_cli(
        &argv,
        &mut io,
        &s.deps,
        &mut Interrupts::new(receiver),
        &Client::Terminal,
    )
    .await;
    let output = Output {
        code,
        out: io.out,
        err: io.err,
    };
    assert_eq!(code, EXIT_STOPPED);
    let events = events(&output);
    assert_eq!(
        events.last().unwrap(),
        &json!({ "event": "interrupted", "exitCode": 130, "interrupted": true })
    );
}
