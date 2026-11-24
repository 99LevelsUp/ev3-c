//! The only way the core reaches a brick. The host adapts EVƎ Cockpit's client to the
//! [`BrickPort`] trait for the brick the author picked, so nothing in the core depends on a
//! front end or on cockpit's own types.
//!
//! The operations are async because cockpit's client is: a Run waits seconds for a program
//! to end while it listens to the brick's state. Every method returns a `Send` future, so a
//! host on a multi-threaded runtime can spawn the operations built on it.

use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Unknown,
}

/// One entry of a brick folder listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrickEntry {
    pub name: String,
    pub kind: EntryKind,
    pub bytes: Option<u64>,
    /// Hex MD5 of a file's content, when cockpit reports it.
    pub md5: Option<String>,
}

/// The user-slot state cockpit reports. `run_seq` numbers the starts cockpit sent to the
/// brick (0 before any): a state with a `run_seq` at least a start's own is about that run
/// or a later one, never an earlier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramState {
    pub running: bool,
    pub run_seq: u64,
}

/// What the state subscription delivers: a change of the user-slot state, or the brick
/// coming back after its connection was lost, when the state may have changed unseen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrickEvent {
    Program(ProgramState),
    Reconnected,
}

/// The brick's events, in the order heard, from the moment [`BrickPort::subscribe`]
/// returned. Dropping it ends the subscription.
#[derive(Debug)]
pub struct Subscription {
    events: mpsc::UnboundedReceiver<BrickEvent>,
}

impl Subscription {
    /// A subscription fed by the sending half of `events`; the port keeps that half and sends
    /// each event as it arrives. A failed send means the subscriber is gone.
    pub fn new(events: mpsc::UnboundedReceiver<BrickEvent>) -> Self {
        Subscription { events }
    }

    /// The next event; `None` once the port has dropped its sending half.
    pub async fn next(&mut self) -> Option<BrickEvent> {
        self.events.recv().await
    }

    /// An event already heard, without waiting for one.
    pub fn try_next(&mut self) -> Option<BrickEvent> {
        self.events.try_recv().ok()
    }
}

/// What a followed brick file says next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowEvent {
    /// The bytes the file grew by, from `offset` on, each growth starting where the last one
    /// ended. Bytes, not text: a character may be split between two.
    Appended { offset: u64, data: Vec<u8> },
    /// The follow ended without being dropped (the file was removed or replaced, the brick
    /// signed out, or the follow failed); the reason in cockpit's words. Nothing follows.
    Ended(String),
}

/// A brick file followed as it grows, from [`BrickPort::follow_file`]. Dropping it cancels the
/// follow, which frees one of the brick's few follows for another client.
#[derive(Debug)]
pub struct FileFollow {
    events: mpsc::UnboundedReceiver<FollowEvent>,
}

impl FileFollow {
    /// A follow fed by the sending half of `events`, as [`Subscription::new`].
    pub fn new(events: mpsc::UnboundedReceiver<FollowEvent>) -> Self {
        FileFollow { events }
    }

    /// The next growth or the end; `None` once the port has dropped its sending half.
    pub async fn next(&mut self) -> Option<FollowEvent> {
        self.events.recv().await
    }
}

/// File content to put on the brick: a file on this computer, or a few bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSource {
    LocalPath(PathBuf),
    Data(Vec<u8>),
}

/// One file of a deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployItem {
    pub brick_path: String,
    pub source: FileSource,
    /// Leave the brick's copy when its MD5 matches.
    pub skip_if_same: bool,
}

/// How a start ended: `Started` (seen running), `NotSeenRunning` (it may have ended at
/// once), or `Uncertain` (the reply was lost; read the state, never start again).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartOutcome {
    Started,
    NotSeenRunning,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartReport {
    pub outcome: StartOutcome,
    /// This start's number; see [`ProgramState::run_seq`].
    pub run_seq: u64,
    pub uploaded: Vec<String>,
    pub skipped: Vec<String>,
}

/// What a recursive removal left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoveReport {
    pub left_behind: Vec<String>,
}

/// A brick call that failed. `code` is cockpit's stable error code (`not_connected`,
/// `program_running`, `outcome_unknown`, …) when the failure has one; the core decides on
/// the code, never on the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrickError {
    pub code: Option<String>,
    pub message: String,
}

impl BrickError {
    pub fn new(code: Option<&str>, message: impl Into<String>) -> Self {
        BrickError {
            code: code.map(str::to_owned),
            message: message.into(),
        }
    }

    /// Cockpit's error code, if the failure carries one.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }
}

impl fmt::Display for BrickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BrickError {}

/// Reads brick files; a port is one. Kept apart so result readers need nothing else of the
/// port, and so a host can hand them a reader that spaces its reads out.
pub trait BrickFileReader: Sync {
    fn read_file(&self, path: &str) -> impl Future<Output = Result<Vec<u8>, BrickError>> + Send;
}

/// Brick operations the core needs. Paths are absolute brick paths shorter than 60 bytes.
/// Failures carry cockpit's stable `code` (see [`BrickError`]).
pub trait BrickPort: BrickFileReader {
    fn list(&self, path: &str) -> impl Future<Output = Result<Vec<BrickEntry>, BrickError>> + Send;

    fn exists(&self, path: &str) -> impl Future<Output = Result<bool, BrickError>> + Send;

    /// Writes one file; missing folders are created.
    fn write_file(
        &self,
        path: &str,
        source: &FileSource,
    ) -> impl Future<Output = Result<(), BrickError>> + Send;

    /// Removes several paths at once; reports what a recursive removal left behind.
    fn remove_paths(
        &self,
        paths: &[String],
        recursive: bool,
    ) -> impl Future<Output = Result<RemoveReport, BrickError>> + Send;

    /// Checks that no program runs, uploads the files, and starts `run_path`, as one
    /// operation no other client can interleave with. Refused with `program_running` while
    /// one runs.
    fn deploy_and_run(
        &self,
        files: &[DeployItem],
        run_path: &str,
    ) -> impl Future<Output = Result<StartReport, BrickError>> + Send;

    fn stop_program(&self) -> impl Future<Output = Result<ProgramState, BrickError>> + Send;

    fn emergency_stop(&self) -> impl Future<Output = Result<ProgramState, BrickError>> + Send;

    fn program_state(&self) -> impl Future<Output = Result<ProgramState, BrickError>> + Send;

    /// Follows the brick's program state; returns once the subscription is live.
    fn subscribe(&self) -> impl Future<Output = Result<Subscription, BrickError>> + Send;

    /// Follows a file as it grows, from its start. Refused with `file_not_found` while the
    /// file does not exist yet, and with `invalid_request` when the brick has no follow free
    /// (cockpit allows two per brick, all clients together).
    fn follow_file(
        &self,
        path: &str,
    ) -> impl Future<Output = Result<FileFollow, BrickError>> + Send;
}

/// Time source, replaceable in tests so waits take no real time.
pub trait Clock: Sync {
    fn now(&self) -> SystemTime;
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

/// The computer's clock and tokio's timer; the runtime needs its time driver enabled.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(duration)
    }
}
