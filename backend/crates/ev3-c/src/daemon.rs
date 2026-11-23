//! The [`Connector`] that reaches EVƎ Cockpit's daemon through the cockpit's own Rust client,
//! `ev3-client`. This is the only module that knows the client and the contract's types; the
//! commands see [`Connector`], [`Connection`] and [`BrickPort`] only.
//!
//! - A connection starts the daemon the installed cockpit recorded (`install.json`) when none
//!   runs, as an `addon` client (or `mcp` for the MCP server, which can confirm nothing).
//! - [`Connection::brick`] resolves the brick setting against the daemon's brick list and the
//!   bricks the cockpit's VS Code windows show, and holds the brick for EVƎ C
//!   (`bricks/connect` without `hold`): it stays connected for EVƎ C while the connection lasts.
//! - The connection is the cockpit's [`Cockpit`]: when the daemon is replaced while a command
//!   runs (an update of the cockpit), it connects again, holds the brick again and renews the
//!   program subscription before any other request goes out.
//! - Dropping the connection shuts it down, which ends the hold.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ev3_client::contract::{
    BrickParams, BricksConnectParams, ClientKind as ContractKind, ContractBrick,
    ContractChannelState, DeployFile, DeployParams, EmptyParams, FilesReadParams,
    FilesRemovePathsParams, FilesWriteParams, FsEntryKind, Notification, PathParams,
    ProgramsStatusParams, StartOutcome as ContractOutcome,
};
use ev3_client::{
    ClientError, Cockpit, CockpitEvent, CockpitOptions, Launch, Paths, method, user_paths,
};
use ev3c_core::port::{
    BrickEntry, BrickError, BrickEvent, BrickFileReader, BrickPort, DeployItem, EntryKind,
    FileSource, ProgramState, RemoveReport, StartOutcome, StartReport, Subscription,
};
use tokio::sync::{broadcast, mpsc};

use crate::cockpit::{
    BrickHandle, BrickInfo, ClientKind, ClientStatus, CockpitError, ConnectOptions, Connection,
    Connector, resolve_brick, shown_bricks,
};

/// Inline content of a file write is limited to this many bytes.
pub const INLINE_WRITE_LIMIT: usize = 4096;

/// Connects to the user's cockpit daemon, or to the test channel the options name.
#[derive(Debug, Clone, Default)]
pub struct CockpitConnector {
    /// More arguments for a daemon this connector starts; empty outside tests, where a replay
    /// daemon gets `--mode replay --fixtures <dir>`.
    pub daemon_args: Vec<String>,
    /// The cockpit's state root (where `install.json` and the channels live); the user's own
    /// when `None`. A test points it at a folder of its own.
    pub state_root: Option<PathBuf>,
}

impl Connector for CockpitConnector {
    type Connection = CockpitConnection;

    fn connect(
        &self,
        options: &ConnectOptions,
    ) -> impl Future<Output = Result<CockpitConnection, CockpitError>> + Send {
        let options = options.clone();
        let daemon_args = self.daemon_args.clone();
        let state_root = self.state_root.clone();
        async move {
            let paths = match state_root {
                Some(root) => Paths::new(&root, options.channel.as_deref()),
                None => user_paths(options.channel.as_deref()).map_err(cockpit_error)?,
            };
            let cockpit = Cockpit::connect(CockpitOptions {
                name: options.name,
                kind: match options.kind {
                    ClientKind::Addon => "addon",
                    ClientKind::Mcp => "mcp",
                },
                version: options.version,
                build_kind: "release".to_owned(),
                daemon: Launch::Installed,
                paths,
                daemon_args,
                // Without one, every question of the daemon is left unanswered (`keep`).
                asker: None,
                notice: Arc::new(|text: &str| eprintln!("{text}")),
            })
            .await
            .map_err(cockpit_error)?;
            Ok(CockpitConnection { cockpit })
        }
    }
}

/// One connection to the daemon. Dropping it shuts the connection down, so the daemon ends
/// what it held for EVƎ C; the ports of its bricks fail from then on.
pub struct CockpitConnection {
    cockpit: Cockpit,
}

impl Drop for CockpitConnection {
    fn drop(&mut self) {
        // The cockpit's tasks keep the connection open until it is shut down: a dropped
        // connection alone would leave the brick held for as long as the process runs
        // (ev3 c mcp). A command's process ends right after, which closes it anyway.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let cockpit = self.cockpit.clone();
            runtime.spawn(async move { cockpit.shutdown().await });
        }
    }
}

impl Connection for CockpitConnection {
    type Port = CockpitPort;
    type Reader = CockpitPort;

    async fn brick(
        &self,
        setting: &str,
    ) -> Result<BrickHandle<CockpitPort, CockpitPort>, CockpitError> {
        let bricks: Vec<BrickInfo> = self
            .cockpit
            .bricks()
            .await
            .map_err(cockpit_error)?
            .iter()
            .map(brick_info)
            .collect();
        // A daemon that cannot say what the windows show counts as showing nothing.
        let shown = match self
            .cockpit
            .call::<method::DaemonStatus>(EmptyParams(serde_json::Map::new()))
            .await
        {
            Ok(status) => shown_bricks(
                &status
                    .clients
                    .iter()
                    .map(|client| ClientStatus {
                        kind: kind_name(client.kind).to_owned(),
                        shows: client.shows.clone(),
                    })
                    .collect::<Vec<_>>(),
            ),
            Err(_) => Vec::new(),
        };
        let wanted = resolve_brick(setting, &bricks, &shown)?;
        // Without a hold: the brick is EVƎ C's own on this connection, held again after a
        // reconnect and let go when the connection ends.
        let brick = self
            .cockpit
            .call::<method::BricksConnect>(BricksConnectParams {
                brick: Some(wanted),
                hold: None,
            })
            .await
            .map_err(cockpit_error)?;
        let port = CockpitPort {
            cockpit: self.cockpit.clone(),
            brick_id: brick.brick_id.clone(),
        };
        Ok(BrickHandle {
            brick_id: brick.brick_id,
            reader: port.clone(),
            port,
        })
    }
}

/// A brick in the daemon's list, as far as choosing one needs it.
fn brick_info(brick: &ContractBrick) -> BrickInfo {
    BrickInfo {
        brick_id: brick.brick_id.clone(),
        name: brick.name.clone(),
        connected: brick.connected,
        reconnecting: brick.reconnecting.unwrap_or(false),
        link_up: brick
            .channels
            .iter()
            .any(|channel| matches!(channel.state, ContractChannelState::Connected)),
    }
}

fn kind_name(kind: ContractKind) -> &'static str {
    match kind {
        ContractKind::Vscode => "vscode",
        ContractKind::Cli => "cli",
        ContractKind::Mcp => "mcp",
        ContractKind::Addon => "addon",
    }
}

fn cockpit_error(error: ClientError) -> CockpitError {
    CockpitError {
        code: error.code().map(str::to_owned),
        message: error.message().to_owned(),
        failure: error.failure().map(|failure| failure.as_str().to_owned()),
    }
}

fn brick_error(error: ClientError) -> BrickError {
    BrickError::new(error.code(), error.message())
}

/// The operations on one brick, bound to it by id.
#[derive(Clone)]
pub struct CockpitPort {
    cockpit: Cockpit,
    brick_id: String,
}

impl CockpitPort {
    fn brick(&self) -> Option<String> {
        Some(self.brick_id.clone())
    }
}

impl BrickFileReader for CockpitPort {
    /// Reads through a private temporary file the daemon writes. An inline read is refused
    /// above 64 KiB, and a run's output may be larger, so every read goes to a local file.
    fn read_file(&self, path: &str) -> impl Future<Output = Result<Vec<u8>, BrickError>> + Send {
        let cockpit = self.cockpit.clone();
        let brick = self.brick();
        let path = path.to_owned();
        async move {
            let dir = PrivateTempDir::new()?;
            let local = dir.path().join("file");
            cockpit
                .call::<method::FilesRead>(FilesReadParams {
                    brick,
                    inline: None,
                    path,
                    to_local_path: Some(local.display().to_string()),
                })
                .await
                .map_err(brick_error)?;
            std::fs::read(&local).map_err(|error| {
                BrickError::new(
                    None,
                    format!("The file read from the brick cannot be opened: {error}"),
                )
            })
        }
    }
}

impl BrickPort for CockpitPort {
    async fn list(&self, path: &str) -> Result<Vec<BrickEntry>, BrickError> {
        let listing = self
            .cockpit
            .call::<method::FilesList>(PathParams {
                brick: self.brick(),
                path: path.to_owned(),
            })
            .await
            .map_err(brick_error)?;
        Ok(listing
            .entries
            .into_iter()
            .map(|entry| BrickEntry {
                name: entry.name,
                kind: match entry.kind {
                    FsEntryKind::File => EntryKind::File,
                    FsEntryKind::Directory => EntryKind::Directory,
                    FsEntryKind::Unknown => EntryKind::Unknown,
                },
                bytes: entry.bytes,
                md5: entry.md5,
            })
            .collect())
    }

    async fn exists(&self, path: &str) -> Result<bool, BrickError> {
        let answer = self
            .cockpit
            .call::<method::FilesExists>(PathParams {
                brick: self.brick(),
                path: path.to_owned(),
            })
            .await
            .map_err(brick_error)?;
        Ok(answer.exists)
    }

    async fn write_file(&self, path: &str, source: &FileSource) -> Result<(), BrickError> {
        let (from_local_path, content) = match write_source(source)? {
            Source::Local(local) => (Some(local), None),
            Source::Inline(content) => (None, Some(content)),
        };
        self.cockpit
            .call::<method::FilesWrite>(FilesWriteParams {
                brick: self.brick(),
                content,
                from_local_path,
                handle: None,
                path: path.to_owned(),
            })
            .await
            .map_err(brick_error)?;
        Ok(())
    }

    async fn remove_paths(
        &self,
        paths: &[String],
        recursive: bool,
    ) -> Result<RemoveReport, BrickError> {
        let report = self
            .cockpit
            .call::<method::FilesRemovePaths>(FilesRemovePathsParams {
                brick: self.brick(),
                paths: paths.to_vec(),
                recursive: Some(recursive),
            })
            .await
            .map_err(brick_error)?;
        Ok(RemoveReport {
            left_behind: report.left_behind,
        })
    }

    async fn deploy_and_run(
        &self,
        files: &[DeployItem],
        run_path: &str,
    ) -> Result<StartReport, BrickError> {
        let files = files
            .iter()
            .map(|item| {
                let (local_path, content) = match write_source(&item.source)? {
                    Source::Local(local) => (Some(local), None),
                    Source::Inline(content) => (None, Some(content)),
                };
                Ok(DeployFile {
                    content,
                    local_path,
                    remote_path: item.brick_path.clone(),
                    skip_if_same_md5: item.skip_if_same.then_some(true),
                })
            })
            .collect::<Result<Vec<_>, BrickError>>()?;
        let result = self
            .cockpit
            .call::<method::ProgramsDeployAndRun>(DeployParams {
                brick: self.brick(),
                files,
                run_path: run_path.to_owned(),
            })
            .await
            .map_err(brick_error)?;
        Ok(StartReport {
            outcome: match result.outcome {
                ContractOutcome::Started => StartOutcome::Started,
                ContractOutcome::NotSeenRunning => StartOutcome::NotSeenRunning,
                ContractOutcome::Uncertain => StartOutcome::Uncertain,
            },
            run_seq: result.run_seq,
            uploaded: result.uploaded,
            skipped: result.skipped,
        })
    }

    async fn stop_program(&self) -> Result<ProgramState, BrickError> {
        let state = self
            .cockpit
            .call::<method::ProgramsStop>(BrickParams {
                brick: self.brick(),
            })
            .await
            .map_err(brick_error)?;
        Ok(program_state(&state))
    }

    async fn emergency_stop(&self) -> Result<ProgramState, BrickError> {
        let state = self
            .cockpit
            .call::<method::ProgramsEmergencyStop>(BrickParams {
                brick: self.brick(),
            })
            .await
            .map_err(brick_error)?;
        Ok(program_state(&state))
    }

    async fn program_state(&self) -> Result<ProgramState, BrickError> {
        let state = self
            .cockpit
            .call::<method::ProgramsStatus>(ProgramsStatusParams {
                brick: self.brick(),
                wait_ms: None,
            })
            .await
            .map_err(brick_error)?;
        Ok(program_state(&state))
    }

    /// Follows the brick's program state with the cockpit's program subscription: every change
    /// in order, none lost before the daemon's answer, renewed after a reconnect with the
    /// fresh state first. Besides, [`BrickEvent::Reconnected`] says the state may have changed
    /// unseen, and the core reads it again: the brick's link came back (`bricks/reconnected`),
    /// the cockpit connected to a new daemon, or this side fell behind the cockpit's events.
    /// Dropping the [`Subscription`] cancels the daemon's subscription.
    async fn subscribe(&self) -> Result<Subscription, BrickError> {
        // Heard from before the subscription is made, so a reconnect meanwhile is not missed.
        let mut heard = self.cockpit.events();
        let mut programs = self
            .cockpit
            .subscribe_programs(&self.brick_id)
            .await
            .map_err(brick_error)?;
        let (sender, receiver) = mpsc::unbounded_channel();
        let brick_id = self.brick_id.clone();
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    () = sender.closed() => break,
                    state = programs.next() => match state {
                        Some(state) => BrickEvent::Program(program_state(&state)),
                        // The cockpit was shut down.
                        None => break,
                    },
                    heard = heard.recv() => match heard {
                        Ok(event) => match brick_event(&event, &brick_id) {
                            Some(event) => event,
                            None => continue,
                        },
                        Err(broadcast::error::RecvError::Lagged(_)) => BrickEvent::Reconnected,
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                };
                if sender.send(event).is_err() {
                    break;
                }
            }
            // `programs` goes with the task, which cancels the subscription on the daemon.
        });
        Ok(Subscription::new(receiver))
    }
}

/// What a cockpit event means for a subscription to `brick_id`'s program, if anything.
fn brick_event(event: &CockpitEvent, brick_id: &str) -> Option<BrickEvent> {
    match event {
        CockpitEvent::Notification(Notification::BricksReconnected(brick))
            if brick.brick_id == brick_id =>
        {
            Some(BrickEvent::Reconnected)
        }
        CockpitEvent::Reconnected(_) => Some(BrickEvent::Reconnected),
        _ => None,
    }
}

fn program_state(state: &ev3_client::contract::ProgramState) -> ProgramState {
    ProgramState {
        running: state.running,
        run_seq: state.run_seq,
    }
}

enum Source {
    Local(String),
    Inline(String),
}

/// A file on this computer by its absolute path (the daemon's working folder is not ours), or
/// a few bytes inline, base64-encoded.
fn write_source(source: &FileSource) -> Result<Source, BrickError> {
    match source {
        FileSource::LocalPath(path) => std::path::absolute(path)
            .map(|path| Source::Local(path.display().to_string()))
            .map_err(|error| {
                BrickError::new(
                    Some("invalid_request"),
                    format!("{} has no absolute path: {error}", path.display()),
                )
            }),
        FileSource::Data(data) if data.len() > INLINE_WRITE_LIMIT => Err(BrickError::new(
            Some("invalid_request"),
            format!("Inline content is limited to {INLINE_WRITE_LIMIT} bytes."),
        )),
        FileSource::Data(data) => Ok(Source::Inline(base64(data))),
    }
}

/// Standard base64 with padding (RFC 4648), as the contract's `content` carries bytes.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().enumerate().fold(0u32, |group, (i, byte)| {
            group | u32::from(*byte) << (16 - 8 * i)
        });
        for i in 0..4 {
            if i <= chunk.len() {
                text.push(char::from(ALPHABET[(group >> (18 - 6 * i)) as usize & 63]));
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// A folder only this user can read, removed with everything in it when dropped.
struct PrivateTempDir(PathBuf);

impl PrivateTempDir {
    fn new() -> Result<Self, BrickError> {
        let dir = std::env::temp_dir().join(format!(
            "ev3c-read-{}",
            ev3_channel::hex(&ev3_channel::random_bytes::<8>())
        ));
        ev3_channel::fsutil::create_private_dir_all(&dir).map_err(|error| {
            BrickError::new(
                None,
                format!("A temporary folder for the read cannot be made: {error}"),
            )
        })?;
        Ok(PrivateTempDir(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encodes_as_rfc_4648_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn inline_content_above_the_limit_is_refused_and_a_local_file_goes_by_absolute_path() {
        let refused = write_source(&FileSource::Data(vec![0; INLINE_WRITE_LIMIT + 1]))
            .err()
            .expect("too large");
        assert_eq!(refused.code(), Some("invalid_request"));
        assert!(matches!(
            write_source(&FileSource::Data(vec![0; INLINE_WRITE_LIMIT])),
            Ok(Source::Inline(_))
        ));
        let Ok(Source::Local(local)) = write_source(&FileSource::LocalPath("a/b.rbf".into()))
        else {
            panic!("a local file");
        };
        assert!(Path::new(&local).is_absolute(), "{local}");
    }

    #[test]
    fn only_its_own_bricks_reconnect_or_a_new_daemon_tells_a_subscription_to_read_again() {
        let back = |brick: &str| {
            CockpitEvent::Notification(Notification::BricksReconnected(
                ev3_client::contract::BrickIdPayload {
                    brick_id: brick.to_owned(),
                },
            ))
        };
        assert_eq!(
            brick_event(&back("AAA"), "AAA"),
            Some(BrickEvent::Reconnected)
        );
        assert_eq!(brick_event(&back("BBB"), "AAA"), None, "another brick");
        assert_eq!(
            brick_event(&CockpitEvent::Lost(ClientError::unavailable("gone")), "AAA"),
            None,
            "a lost daemon tries again with the next request"
        );
    }

    #[test]
    fn a_start_failure_keeps_its_code_and_says_why() {
        let error = cockpit_error(ClientError::start(
            ev3_client::Failure::MissingBinary,
            "install EVƎ Cockpit",
        ));
        assert_eq!(error.code.as_deref(), Some("daemon_unavailable"));
        assert_eq!(error.failure.as_deref(), Some("missing_binary"));
        let error = cockpit_error(ClientError::contract("brick_busy", "held"));
        assert_eq!(error.code.as_deref(), Some("brick_busy"));
        assert_eq!(error.failure, None);
    }
}
