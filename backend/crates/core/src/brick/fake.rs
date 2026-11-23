//! An in-memory brick behind the BrickPort trait, for the brick-operation tests. It behaves
//! like the cockpit daemon where the core depends on it: writes create missing folders,
//! listings carry each file's MD5, a start refuses while a program runs and numbers every
//! start, and every call fails with `not_connected` while disconnected. Its recursive removal
//! deletes one folder level only and reports what it left, so the core cannot rely on a
//! removal being complete.
//!
//! Time is tokio's, paused by the tests (`start_paused`): it moves only when every task
//! waits, straight to the next timer, so a 10 s wait takes no real time.
//!
//! Other crates' tests get it through the `test-support` feature.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::port::{
    BrickEntry, BrickError, BrickEvent, BrickFileReader, BrickPort, Clock, DeployItem, EntryKind,
    FileSource, ProgramState, RemoveReport, StartOutcome, StartReport, Subscription,
};

/// Virtual wall time on tokio's paused clock, starting at a fixed date.
#[derive(Debug, Clone, Copy)]
pub struct TestClock {
    base: SystemTime,
    start: Instant,
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl TestClock {
    /// Starts at the fixed date now, by tokio's clock; call it inside the test's runtime.
    pub fn new() -> Self {
        TestClock {
            base: UNIX_EPOCH + Duration::from_millis(1_700_000_000_000),
            start: Instant::now(),
        }
    }
}

impl Clock for TestClock {
    fn now(&self) -> SystemTime {
        self.base + self.start.elapsed()
    }

    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(duration)
    }
}

type Hook = Box<dyn FnMut(&mut FakeBrick) + Send>;
type Md5Rewrite = Box<dyn Fn(Option<String>) -> Option<String> + Send>;

/// The fake brick's state, which the tests script directly.
pub struct FakeBrick {
    pub files: BTreeMap<String, Vec<u8>>,
    pub dirs: BTreeSet<String>,
    /// Every call in order, as "<method> <path>".
    pub calls: Vec<String>,
    /// Virtual times at which `read_file` was called.
    pub read_times: Vec<SystemTime>,
    /// Error codes the next writes of a path fail with, one per write.
    write_failures: HashMap<String, VecDeque<String>>,
    pub running: bool,
    /// The number of the last start; every start counts, a refused one too.
    pub run_seq: u64,
    /// How the next starts end.
    pub start_outcome: StartOutcome,
    /// Error code a start fails with after its files are uploaded, if set.
    pub start_error: Option<String>,
    pub disconnected: bool,
    /// Called inside `deploy_and_run` once the program "runs", before the call returns.
    pub on_run: Option<Hook>,
    /// An event sent to each new subscriber at once, like a replay of an earlier state.
    pub replay_on_subscribe: Option<BrickEvent>,
    /// Called inside `remove_paths`, after the removal.
    pub on_remove: Option<Hook>,
    /// Rewrites the MD5 each listed file reports.
    pub list_md5: Option<Md5Rewrite>,
    listeners: Vec<mpsc::UnboundedSender<BrickEvent>>,
    clock: TestClock,
}

impl FakeBrick {
    pub fn put_file(&mut self, path: &str, content: impl AsRef<[u8]>) {
        self.mkdirs(parent_of(path));
        self.files
            .insert(path.to_owned(), content.as_ref().to_vec());
    }

    pub fn mkdirs(&mut self, path: &str) {
        let mut dir = path;
        while dir != "/" && !self.dirs.contains(dir) {
            self.dirs.insert(dir.to_owned());
            dir = parent_of(dir);
        }
    }

    pub fn text(&self, path: &str) -> Option<String> {
        self.files
            .get(path)
            .map(|data| String::from_utf8_lossy(data).into_owned())
    }

    pub fn has(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.dirs.contains(path)
    }

    pub fn fail_writes(&mut self, path: &str, codes: &[&str]) {
        self.write_failures.insert(
            path.to_owned(),
            codes.iter().map(|code| (*code).to_owned()).collect(),
        );
    }

    fn state(&self) -> ProgramState {
        ProgramState {
            running: self.running,
            run_seq: self.run_seq,
        }
    }

    pub fn emit(&mut self, event: BrickEvent) {
        self.listeners
            .retain(|listener| listener.send(event).is_ok());
    }

    /// Tells the subscribers the program state now.
    pub fn emit_state(&mut self) {
        self.emit(BrickEvent::Program(self.state()));
    }

    /// The program stops and the subscribers hear it.
    pub fn stop(&mut self) {
        self.running = false;
        self.emit_state();
    }

    /// Subscriptions still held by someone.
    pub fn listener_count(&self) -> usize {
        self.listeners
            .iter()
            .filter(|listener| !listener.is_closed())
            .count()
    }

    /// Ends every subscription from the port's side, as a client shutting down would.
    pub fn end_subscriptions(&mut self) {
        self.listeners.clear();
    }

    fn write(&mut self, path: &str, source: &FileSource) -> Result<(), BrickError> {
        if let Some(code) = self
            .write_failures
            .get_mut(path)
            .and_then(VecDeque::pop_front)
        {
            return Err(cockpit_error(&code));
        }
        self.put_file(path, content(source));
        Ok(())
    }
}

#[derive(Clone)]
pub struct FakeBrickPort {
    brick: Arc<Mutex<FakeBrick>>,
}

impl Default for FakeBrickPort {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBrickPort {
    /// A brick with only its root folder. Call it inside the test's runtime, whose paused
    /// clock it reads.
    pub fn new() -> Self {
        FakeBrickPort {
            brick: Arc::new(Mutex::new(FakeBrick {
                files: BTreeMap::new(),
                dirs: BTreeSet::from(["/".to_owned()]),
                calls: Vec::new(),
                read_times: Vec::new(),
                write_failures: HashMap::new(),
                running: false,
                run_seq: 0,
                start_outcome: StartOutcome::Started,
                start_error: None,
                disconnected: false,
                on_run: None,
                replay_on_subscribe: None,
                on_remove: None,
                list_md5: None,
                listeners: Vec::new(),
                clock: TestClock::new(),
            })),
        }
    }

    /// Runs `script` on the brick's state.
    pub fn with<T>(&self, script: impl FnOnce(&mut FakeBrick) -> T) -> T {
        script(&mut self.brick.lock().unwrap())
    }

    pub fn calls(&self) -> Vec<String> {
        self.with(|brick| brick.calls.clone())
    }

    /// The clock whose time `read_times` holds.
    pub fn clock(&self) -> TestClock {
        self.with(|brick| brick.clock)
    }

    /// A handle for hooks, which get the brick's state but not the port, to change the
    /// brick later. It does not keep the brick alive, so a hook holding it is no cycle.
    pub fn handle(&self) -> FakeHandle {
        FakeHandle(Arc::downgrade(&self.brick))
    }

    fn call<T>(
        &self,
        method: &str,
        path: &str,
        body: impl FnOnce(&mut FakeBrick) -> Result<T, BrickError>,
    ) -> Result<T, BrickError> {
        self.with(|brick| {
            brick.calls.push(if path.is_empty() {
                method.to_owned()
            } else {
                format!("{method} {path}")
            });
            if brick.disconnected {
                return Err(cockpit_error("not_connected"));
            }
            body(brick)
        })
    }
}

#[derive(Clone)]
pub struct FakeHandle(Weak<Mutex<FakeBrick>>);

impl FakeHandle {
    /// Runs `action` on the brick once `delay` has passed. With no delay it runs as soon as
    /// the task that scheduled it waits, like `setImmediate`.
    pub fn later(&self, delay: Duration, action: impl FnOnce(&mut FakeBrick) + Send + 'static) {
        let brick = self.0.clone();
        tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if let Some(brick) = brick.upgrade() {
                action(&mut brick.lock().unwrap());
            }
        });
    }
}

impl BrickFileReader for FakeBrickPort {
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, BrickError> {
        self.call("read_file", path, |brick| {
            let now = brick.clock.now();
            brick.read_times.push(now);
            brick
                .files
                .get(path)
                .cloned()
                .ok_or_else(|| BrickError::new(Some("protocol_failed"), format!("no file {path}")))
        })
    }
}

impl BrickPort for FakeBrickPort {
    async fn list(&self, path: &str) -> Result<Vec<BrickEntry>, BrickError> {
        self.call("list", path, |brick| {
            if !brick.dirs.contains(path) {
                return Err(BrickError::new(
                    Some("protocol_failed"),
                    format!("no folder {path}"),
                ));
            }
            let mut entries: Vec<BrickEntry> = brick
                .dirs
                .iter()
                .filter(|dir| *dir != "/" && parent_of(dir) == path)
                .map(|dir| BrickEntry {
                    name: name_of(dir).to_owned(),
                    kind: EntryKind::Directory,
                    bytes: None,
                    md5: None,
                })
                .collect();
            for (file, data) in &brick.files {
                if parent_of(file) == path {
                    let md5 = Some(md5_hex(data));
                    entries.push(BrickEntry {
                        name: name_of(file).to_owned(),
                        kind: EntryKind::File,
                        bytes: Some(data.len() as u64),
                        md5: match &brick.list_md5 {
                            Some(rewrite) => rewrite(md5),
                            None => md5,
                        },
                    });
                }
            }
            Ok(entries)
        })
    }

    async fn exists(&self, path: &str) -> Result<bool, BrickError> {
        self.call("exists", path, |brick| Ok(brick.has(path)))
    }

    async fn write_file(&self, path: &str, source: &FileSource) -> Result<(), BrickError> {
        self.call("write_file", path, |brick| brick.write(path, source))
    }

    async fn remove_paths(
        &self,
        paths: &[String],
        recursive: bool,
    ) -> Result<RemoveReport, BrickError> {
        let method = if recursive {
            "remove_paths -r"
        } else {
            "remove_paths"
        };
        self.call(method, &paths.join(" "), |brick| {
            let mut left_behind = Vec::new();
            for path in paths {
                if brick.files.remove(path).is_some() || !brick.dirs.contains(path) {
                    continue;
                }
                if recursive {
                    brick
                        .files
                        .retain(|file, _| parent_of(file) != path.as_str());
                }
                let has_children = brick
                    .files
                    .keys()
                    .chain(brick.dirs.iter())
                    .any(|child| child != path && parent_of(child) == path.as_str());
                if has_children {
                    left_behind.push(path.clone());
                } else {
                    brick.dirs.remove(path);
                }
            }
            if let Some(mut hook) = brick.on_remove.take() {
                hook(brick);
                brick.on_remove = Some(hook);
            }
            Ok(RemoveReport { left_behind })
        })
    }

    async fn deploy_and_run(
        &self,
        files: &[DeployItem],
        run_path: &str,
    ) -> Result<StartReport, BrickError> {
        self.call("deploy_and_run", run_path, |brick| {
            if brick.running {
                return Err(cockpit_error("program_running"));
            }
            let mut uploaded = Vec::new();
            let mut skipped = Vec::new();
            for item in files {
                let data = content(&item.source);
                let same = brick
                    .files
                    .get(&item.brick_path)
                    .is_some_and(|there| md5_hex(there) == md5_hex(&data));
                if item.skip_if_same && same {
                    skipped.push(item.brick_path.clone());
                    continue;
                }
                brick.write(&item.brick_path, &item.source)?;
                uploaded.push(item.brick_path.clone());
            }
            brick.run_seq += 1;
            if let Some(code) = &brick.start_error {
                return Err(cockpit_error(code));
            }
            brick.running = brick.start_outcome != StartOutcome::NotSeenRunning;
            if let Some(mut hook) = brick.on_run.take() {
                hook(brick);
                brick.on_run = Some(hook);
            }
            Ok(StartReport {
                outcome: brick.start_outcome,
                run_seq: brick.run_seq,
                uploaded,
                skipped,
            })
        })
    }

    // Cockpit tells the subscribers when a stop ends a program, as for any other end.
    async fn stop_program(&self) -> Result<ProgramState, BrickError> {
        self.call("stop_program", "", |brick| {
            brick.stop();
            Ok(brick.state())
        })
    }

    async fn emergency_stop(&self) -> Result<ProgramState, BrickError> {
        self.call("emergency_stop", "", |brick| {
            brick.stop();
            Ok(brick.state())
        })
    }

    async fn program_state(&self) -> Result<ProgramState, BrickError> {
        self.call("program_state", "", |brick| Ok(brick.state()))
    }

    async fn subscribe(&self) -> Result<Subscription, BrickError> {
        self.call("subscribe", "", |brick| {
            let (sender, receiver) = mpsc::unbounded_channel();
            if let Some(event) = brick.replay_on_subscribe {
                // Cannot fail: the receiver is right here.
                let _ = sender.send(event);
            }
            brick.listeners.push(sender);
            Ok(Subscription::new(receiver))
        })
    }
}

fn cockpit_error(code: &str) -> BrickError {
    BrickError::new(Some(code), code)
}

fn content(source: &FileSource) -> Vec<u8> {
    match source {
        FileSource::LocalPath(path) => std::fs::read(path).unwrap(),
        FileSource::Data(data) => data.clone(),
    }
}

pub fn md5_hex(data: &[u8]) -> String {
    format!("{:x}", Md5::digest(data))
}

fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(slash) if slash > 0 => &path[..slash],
        _ => "/",
    }
}

fn name_of(path: &str) -> &str {
    path.rfind('/').map_or(path, |slash| &path[slash + 1..])
}
