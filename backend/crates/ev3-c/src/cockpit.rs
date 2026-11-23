//! The commands' link to EVƎ Cockpit's daemon. The commands see the daemon only through
//! [`Connector`] and [`Connection`], so the transport, the cockpit's client library and its
//! types stay out of them: a test hands in a fake, and the program hands in
//! [`crate::daemon::CockpitConnector`], built on EVƎ Cockpit's own client.
//!
//! What a connection does for a command, as the TypeScript `CockpitSession` did:
//! - it resolves the brick setting with [`resolve_brick`] against the daemon's brick list and
//!   the bricks EVƎ Cockpit's VS Code windows show ([`shown_bricks`]);
//! - it holds that brick for EVƎ C (`bricks/connect` without a hold), so the brick stays
//!   connected for EVƎ C while the connection lasts;
//! - it gives the brick's [`BrickPort`] and the reader the result readers use.
//!
//! Dropping the connection closes it, which ends the hold and aborts its requests.

use std::fmt;
use std::future::Future;

use ev3c_core::port::{BrickError, BrickFileReader, BrickPort};

/// What kind of client EVƎ C connects as. An `Mcp` client is an agent, which can never
/// confirm anything on a person's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Addon,
    Mcp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectOptions {
    /// Says where EVƎ C runs: other clients read it in `brick_busy` and `stoppedBy`.
    pub name: String,
    /// EVƎ C's version.
    pub version: String,
    pub kind: ClientKind,
    /// A development or test daemon (`EV3_COCKPIT_CHANNEL`); the user's own when `None`.
    pub channel: Option<String>,
}

/// A failure on the way to a brick or on it. `code` is cockpit's stable error code
/// (`brick_busy`, `not_connected`, `daemon_unavailable`, …); `failure` says why the daemon
/// could not be started (`missing_binary`, …) when that is the cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CockpitError {
    pub code: Option<String>,
    pub message: String,
    pub failure: Option<String>,
}

impl CockpitError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        CockpitError {
            code: Some(code.to_owned()),
            message: message.into(),
            failure: None,
        }
    }

    /// The daemon could not be started, for the reason `failure`.
    pub fn with_failure(mut self, failure: &str) -> Self {
        self.failure = Some(failure.to_owned());
        self
    }
}

impl fmt::Display for CockpitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CockpitError {}

impl From<BrickError> for CockpitError {
    fn from(error: BrickError) -> Self {
        CockpitError {
            code: error.code,
            message: error.message,
            failure: None,
        }
    }
}

/// One brick on a connection: its id, its port, and the reader result readers use (the port
/// itself, or one that spaces reads out).
#[derive(Debug, Clone)]
pub struct BrickHandle<P, R> {
    pub brick_id: String,
    pub port: P,
    pub reader: R,
}

/// Makes connections to EVƎ Cockpit's daemon, starting the daemon the installed cockpit
/// recorded when none runs.
pub trait Connector: Sync {
    type Connection: Connection;

    fn connect(
        &self,
        options: &ConnectOptions,
    ) -> impl Future<Output = Result<Self::Connection, CockpitError>> + Send;
}

/// A connection to the daemon; dropping it closes it.
pub trait Connection: Sync {
    type Port: BrickPort + Send;
    type Reader: BrickFileReader + Send;

    /// The brick `setting` names now ([`resolve_brick`]), held for EVƎ C on this connection.
    fn brick(
        &self,
        setting: &str,
    ) -> impl Future<Output = Result<BrickHandle<Self::Port, Self::Reader>, CockpitError>> + Send;
}

/// The brick the setting names. A name or id is passed on as written, or as the id of the
/// brick it names (cockpit matches names without regard to case and refuses an unknown one).
/// `active` means the brick EVƎ Cockpit's VS Code window shows; when no window shows a
/// connected one, the only connected brick. Windows showing different bricks leave the choice
/// to the setting.
pub fn resolve_brick(
    setting: &str,
    bricks: &[BrickInfo],
    shown: &[String],
) -> Result<String, CockpitError> {
    if setting != "active" {
        return Ok(find_brick(bricks, setting)
            .map_or_else(|| setting.to_owned(), |brick| brick.brick_id.clone()));
    }
    // A window may still name a brick it disconnected; only a connected one counts.
    let live: Vec<&String> = shown
        .iter()
        .filter(|id| bricks.iter().any(|b| &b.brick_id == *id && b.connected))
        .collect();
    match live.as_slice() {
        [one] => return Ok((*one).clone()),
        [] => {}
        several => {
            let names: Vec<&str> = several
                .iter()
                .map(|id| find_brick(bricks, id).map_or(id.as_str(), |b| b.name.as_str()))
                .collect();
            return Err(CockpitError::new(
                "brick_ambiguous",
                format!(
                    "EVƎ Cockpit's windows show different bricks ({}); set \"ev3-c.brick\" to the one EVƎ C should use.",
                    names.join(", ")
                ),
            ));
        }
    }
    let online: Vec<&BrickInfo> = bricks.iter().filter(|b| b.is_online()).collect();
    match online.as_slice() {
        [one] => Ok(one.brick_id.clone()),
        [] => Err(CockpitError::new("not_connected", "No brick is connected.")),
        several => {
            let names: Vec<&str> = several.iter().map(|b| b.name.as_str()).collect();
            Err(CockpitError::new(
                "brick_ambiguous",
                format!(
                    "Several bricks are connected ({}); set \"ev3-c.brick\" to the one EVƎ C should use.",
                    names.join(", ")
                ),
            ))
        }
    }
}

/// A brick in the daemon's list, as far as choosing one needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrickInfo {
    pub brick_id: String,
    pub name: String,
    pub connected: bool,
    pub reconnecting: bool,
    /// Whether one of its channels (USB, Bluetooth, Wi-Fi) is connected now.
    pub link_up: bool,
}

impl BrickInfo {
    /// Connected with a live link and not reconnecting.
    pub fn is_online(&self) -> bool {
        self.connected && !self.reconnecting && self.link_up
    }
}

/// The brick a reference names, as the cockpit's client finds it: by id, else by name
/// without regard to case.
pub fn find_brick<'b>(bricks: &'b [BrickInfo], reference: &str) -> Option<&'b BrickInfo> {
    let wanted = reference.trim().to_lowercase();
    bricks
        .iter()
        .find(|b| b.brick_id == reference)
        .or_else(|| bricks.iter().find(|b| b.name.to_lowercase() == wanted))
}

/// A client of the daemon, from `daemon/status` and `daemon/showsChanged`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientStatus {
    pub kind: String,
    /// The brick the client shows, if it shows one.
    pub shows: Option<String>,
}

/// The distinct bricks EVƎ Cockpit's VS Code windows (clients of kind `vscode`) show, in the
/// order listed. With several windows, several bricks may be shown.
pub fn shown_bricks(clients: &[ClientStatus]) -> Vec<String> {
    let mut shown: Vec<String> = Vec::new();
    for client in clients.iter().filter(|c| c.kind == "vscode") {
        if let Some(brick) = &client.shows
            && !shown.contains(brick)
        {
            shown.push(brick.clone());
        }
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    const AAA: &str = "usb-0016530aaaaa";
    const BBB: &str = "bt-001653000bbb";

    fn brick(brick_id: &str, name: &str, connected: bool) -> BrickInfo {
        BrickInfo {
            brick_id: brick_id.to_owned(),
            name: name.to_owned(),
            connected,
            reconnecting: false,
            link_up: connected,
        }
    }

    fn shown(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn active_is_the_brick_the_cockpits_window_shows_even_with_several_connected() {
        let bricks = [brick(AAA, "AAA", true), brick(BBB, "BBB", true)];
        assert_eq!(
            resolve_brick("active", &bricks, &shown(&[BBB])).unwrap(),
            BBB
        );
        assert_eq!(
            resolve_brick("AAA", &bricks, &shown(&[BBB])).unwrap(),
            AAA,
            "a named brick wins over the window"
        );
        let error = resolve_brick("active", &bricks, &shown(&[AAA, BBB])).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("brick_ambiguous"));
        assert!(
            error
                .message
                .contains("windows show different bricks (AAA, BBB)"),
            "{}",
            error.message
        );
    }

    #[test]
    fn a_window_that_still_names_a_disconnected_brick_does_not_count() {
        let bricks = [brick(AAA, "AAA", true), brick(BBB, "BBB", false)];
        assert_eq!(
            resolve_brick("active", &bricks, &shown(&[BBB])).unwrap(),
            AAA,
            "falls back to the only connected brick"
        );
        assert_eq!(
            resolve_brick("active", &bricks, &shown(&[AAA, BBB])).unwrap(),
            AAA,
            "no ambiguity with a disconnected one"
        );
    }

    #[test]
    fn active_without_a_window_is_the_only_connected_brick_and_a_name_or_id_is_taken_as_written() {
        let mut bricks = vec![brick(AAA, "AAA", true)];
        assert_eq!(resolve_brick("active", &bricks, &[]).unwrap(), AAA);
        assert_eq!(
            resolve_brick("aaa", &bricks, &[]).unwrap(),
            AAA,
            "a name resolves to its id"
        );
        assert_eq!(
            resolve_brick(BBB, &bricks, &[]).unwrap(),
            BBB,
            "an unknown id is left to cockpit"
        );

        bricks.push(brick(BBB, "BBB", true));
        let error = resolve_brick("active", &bricks, &[]).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("brick_ambiguous"));
        assert!(error.message.contains("(AAA, BBB)"), "{}", error.message);

        let error = resolve_brick("active", &[brick(AAA, "AAA", false)], &[]).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("not_connected"));
    }

    #[test]
    fn a_brick_reconnecting_or_without_a_live_link_is_not_online() {
        let mut reconnecting = brick(AAA, "AAA", true);
        reconnecting.reconnecting = true;
        let mut no_link = brick(BBB, "BBB", true);
        no_link.link_up = false;
        assert!(!reconnecting.is_online());
        assert!(!no_link.is_online());
        let error = resolve_brick("active", &[reconnecting, no_link], &[]).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("not_connected"));
    }

    #[test]
    fn the_shown_bricks_are_the_distinct_bricks_of_cockpit_windows_only() {
        let client = |kind: &str, shows: Option<&str>| ClientStatus {
            kind: kind.to_owned(),
            shows: shows.map(str::to_owned),
        };
        assert_eq!(
            shown_bricks(&[client("vscode", Some(AAA)), client("cli", Some(BBB))]),
            [AAA]
        );
        assert_eq!(
            shown_bricks(&[client("vscode", Some(BBB)), client("vscode", Some(BBB))]),
            [BBB],
            "two windows on the same brick are one brick"
        );
        assert!(shown_bricks(&[client("vscode", None)]).is_empty());
    }
}
