//! Helpers shared by the unit tests: a fake cockpit behind the [`Connector`] trait with the
//! core's fake brick behind each port, a project folder, and an install folder whose
//! compiler writes a fixture ELF instead of running Zig.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ev3c_core::brick::fake::{FakeBrick, FakeBrickPort, TestClock};
use ev3c_core::build::brick_project_dir;
use ev3c_core::toolchain::CompileResult;

use tokio::sync::mpsc;

use crate::cli::{CompileHook, Deps, Io};
use crate::cockpit::{
    BrickHandle, BrickInfo, CockpitError, ConnectOptions, Connection, Connector, View,
    resolve_brick,
};

/// A new, empty folder below the system's temporary folder, with links resolved so it reads
/// as the commands print it. Tests run in parallel threads, so the name carries the process
/// ID and a counter.
pub(crate) fn temp_dir(prefix: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "{prefix}{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // Left over from an earlier run with the same process ID.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    crate::paths::real_path(&dir)
}

pub(crate) const BRICK_ID: &str = "usb-0016530aaaaa";
pub(crate) const OTHER_ID: &str = "bt-001653000bbb";

/// The cockpit's daemon as the commands see it: a brick list, the bricks the windows show,
/// one fake brick per id, and every connection and hold it was asked for.
pub(crate) struct FakeCockpit {
    pub bricks: Mutex<Vec<BrickInfo>>,
    pub shown: Mutex<Vec<String>>,
    ports: Mutex<HashMap<String, FakeBrickPort>>,
    pub connects: Mutex<Vec<ConnectOptions>>,
    /// Every hold, as "bricks/connect <id>".
    pub calls: Mutex<Vec<String>>,
    /// The next connection fails with this.
    pub connect_failure: Mutex<Option<CockpitError>>,
    /// The next hold fails with this.
    pub brick_failure: Mutex<Option<CockpitError>>,
    /// Connections not yet dropped.
    pub open: AtomicUsize,
    /// Reading the bricks fails with this while it is set (the daemon is gone).
    pub view_failure: Mutex<Option<CockpitError>>,
    /// Who hears [`FakeCockpit::changed`].
    watchers: Mutex<Vec<mpsc::UnboundedSender<()>>>,
}

impl FakeCockpit {
    /// One connected brick, AAA. Call it inside the test's runtime, whose clock the brick
    /// reads.
    pub fn new() -> Arc<Self> {
        let cockpit = Arc::new(FakeCockpit {
            bricks: Mutex::new(Vec::new()),
            shown: Mutex::new(Vec::new()),
            ports: Mutex::new(HashMap::new()),
            connects: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            connect_failure: Mutex::new(None),
            brick_failure: Mutex::new(None),
            open: AtomicUsize::new(0),
            view_failure: Mutex::new(None),
            watchers: Mutex::new(Vec::new()),
        });
        cockpit.add_brick(BRICK_ID, "AAA");
        cockpit
    }

    pub fn add_brick(&self, brick_id: &str, name: &str) {
        self.bricks.lock().unwrap().push(BrickInfo {
            brick_id: brick_id.to_owned(),
            name: name.to_owned(),
            connected: true,
            reconnecting: false,
            link_up: true,
        });
        self.ports
            .lock()
            .unwrap()
            .insert(brick_id.to_owned(), FakeBrickPort::new());
    }

    pub fn port(&self, brick_id: &str) -> FakeBrickPort {
        self.ports.lock().unwrap()[brick_id].clone()
    }

    /// Tells every connection's [`Connection::changes`] that something changed.
    pub fn changed(&self) {
        self.watchers
            .lock()
            .unwrap()
            .retain(|watcher| watcher.send(()).is_ok());
    }

    /// Sets whether the brick `brick_id` is connected.
    pub fn set_connected(&self, brick_id: &str, connected: bool) {
        for brick in self.bricks.lock().unwrap().iter_mut() {
            if brick.brick_id == brick_id {
                brick.connected = connected;
                brick.link_up = connected;
            }
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    pub fn names(&self) -> Vec<String> {
        self.connects
            .lock()
            .unwrap()
            .iter()
            .map(|options| options.name.clone())
            .collect()
    }
}

#[derive(Clone)]
pub(crate) struct FakeConnector(pub Arc<FakeCockpit>);

pub(crate) struct FakeConnection(Arc<FakeCockpit>);

impl Drop for FakeConnection {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Connector for FakeConnector {
    type Connection = FakeConnection;

    async fn connect(&self, options: &ConnectOptions) -> Result<FakeConnection, CockpitError> {
        self.0.connects.lock().unwrap().push(options.clone());
        if let Some(error) = self.0.connect_failure.lock().unwrap().take() {
            return Err(error);
        }
        self.0.open.fetch_add(1, Ordering::Relaxed);
        Ok(FakeConnection(self.0.clone()))
    }
}

impl Connection for FakeConnection {
    type Port = FakeBrickPort;
    type Reader = FakeBrickPort;

    async fn brick(
        &self,
        setting: &str,
    ) -> Result<BrickHandle<FakeBrickPort, FakeBrickPort>, CockpitError> {
        let cockpit = &self.0;
        let brick_id = resolve_brick(
            setting,
            &cockpit.bricks.lock().unwrap(),
            &cockpit.shown.lock().unwrap(),
        )?;
        if let Some(error) = cockpit.brick_failure.lock().unwrap().take() {
            return Err(error);
        }
        let port = cockpit
            .ports
            .lock()
            .unwrap()
            .get(&brick_id)
            .cloned()
            .ok_or_else(|| CockpitError::new("brick_not_found", format!("No brick {brick_id}.")))?;
        cockpit
            .calls
            .lock()
            .unwrap()
            .push(format!("bricks/connect {brick_id}"));
        Ok(BrickHandle {
            brick_id,
            port: port.clone(),
            reader: port,
        })
    }

    async fn view(&self) -> Result<View, CockpitError> {
        if let Some(error) = self.0.view_failure.lock().unwrap().clone() {
            return Err(error);
        }
        Ok(View {
            bricks: self.0.bricks.lock().unwrap().clone(),
            shown: self.0.shown.lock().unwrap().clone(),
        })
    }

    fn port(&self, brick_id: &str) -> FakeBrickPort {
        self.0.port(brick_id)
    }

    fn changes(&self) -> mpsc::UnboundedReceiver<()> {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.0.watchers.lock().unwrap().push(sender);
        receiver
    }
}

/// An install folder with ev3lib's headers, the brick runtime, and a file that stands for
/// the compiler.
pub(crate) fn package_root() -> PathBuf {
    let root = temp_dir("ev3c-package-");
    std::fs::create_dir_all(root.join("ev3lib").join("include")).unwrap();
    let runtime = root.join("out").join("runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::write(runtime.join("libev3.a"), "fake library").unwrap();
    std::fs::write(runtime.join("supervisor"), "fake supervisor").unwrap();
    std::fs::write(root.join("zig"), "fake compiler").unwrap();
    root
}

/// A project folder named `name` below `parent`, with ev3c.json and one source.
pub(crate) fn project(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ev3c.json"), format!("{{\"name\":\"{name}\"}}")).unwrap();
    std::fs::write(dir.join("main.c"), "int main(void) { return 0; }\n").unwrap();
    dir
}

/// A compile step that writes the fixture ELF where the compiler would, after saying
/// `diagnostics`; with `ok` false it fails with them instead.
pub(crate) fn compiler(diagnostics: &'static str, ok: bool) -> Arc<CompileHook> {
    Arc::new(move |request| {
        if ok {
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../core/fixtures/elf/static-armv5te.elf");
            let output = request
                .cwd
                .clone()
                .unwrap_or_default()
                .join(&request.output);
            std::fs::copy(fixture, output).unwrap();
        }
        Ok(CompileResult {
            ok,
            elf_path: request.output.clone(),
            exit_code: Some(if ok { 0 } else { 1 }),
            diagnostics: diagnostics.to_owned(),
        })
    })
}

/// The commands' surroundings: the fake cockpit, a working folder holding the project
/// `hello`, the install folder above, and the fake compiler.
pub(crate) fn deps(cockpit: &Arc<FakeCockpit>) -> Deps<FakeConnector, TestClock> {
    let cwd = temp_dir("ev3c-work-");
    let root = package_root();
    let mut env = HashMap::new();
    env.insert(
        "EV3C_TOOLCHAIN".to_owned(),
        root.join("zig").to_string_lossy().into_owned(),
    );
    Deps {
        connector: FakeConnector(cockpit.clone()),
        clock: TestClock::new(),
        package_root: Ok(root),
        toolchain_cache_dir: cwd.join("cache"),
        version: "1.2.3".to_owned(),
        cwd,
        env,
        compile: Some(compiler("", true)),
    }
}

/// Messages a command wrote, one entry per message.
#[derive(Debug, Default)]
pub(crate) struct Captured {
    pub out: Vec<String>,
    pub err: Vec<String>,
}

impl Io for Captured {
    fn out(&mut self, text: &str) {
        self.out.push(text.to_owned());
    }

    fn err(&mut self, text: &str) {
        self.err.push(text.to_owned());
    }
}

/// How a scripted run ends on the brick.
pub(crate) struct Ending {
    /// The record's lines after `run=`, such as "end=program\nexit=3\n".
    pub record: &'static str,
    pub out: &'static str,
    /// The return list, if the run returned files.
    pub returned: Option<&'static str>,
    /// Whether the program stops on its own right after the start; otherwise only a stop
    /// ends it.
    pub stops: bool,
}

/// What the supervisor does on the brick when `project` starts: takes the launch ID from
/// .req, and writes run `run`'s folder with that ID in its record.
pub(crate) fn script_run(port: &FakeBrickPort, project: &str, run: i64, ending: Ending) {
    let dir = brick_project_dir(project);
    let handle = port.handle();
    port.with(|brick| {
        brick.on_run = Some(Box::new(move |brick: &mut FakeBrick| {
            let request = format!("{dir}/.req");
            let launch = brick.text(&request).unwrap_or_default();
            brick.files.remove(&request);
            let launch = launch.trim_end();
            let runs = format!("{dir}/runs/{run}");
            brick.put_file(&format!("{dir}/runs/next"), (run + 1).to_string());
            brick.put_file(&format!("{runs}/out"), ending.out);
            brick.put_file(&format!("{runs}/err"), "");
            if let Some(list) = ending.returned {
                brick.put_file(&format!("{runs}/ret"), list);
            }
            brick.put_file(
                &format!("{runs}/res"),
                format!(
                    "run={run}\n{}out_truncated=0\nerr_truncated=0\nlaunch={launch}\n",
                    ending.record
                ),
            );
            if ending.stops {
                handle.later(Duration::ZERO, FakeBrick::stop);
            }
        }));
    });
}
