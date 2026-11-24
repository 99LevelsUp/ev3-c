//! EVƎ C's link to the cockpit against a real cockpit daemon in replay mode: the cockpit's
//! recording "usb-deploy-and-run-ccc" played through `CockpitConnector`, its brick hold and its
//! port. No brick is needed, but a daemon binary and the recording are, so this runs only where
//! both are named; elsewhere it says it was skipped and passes.
//!
//!   EV3C_COCKPIT_DAEMON    the cockpit's ev3-cockpitd (built with `npm run daemon:build`)
//!   EV3C_COCKPIT_FIXTURES  the cockpit's daemon/fixtures folder
//!
//! The daemon is started from an install.json in a state root of the test's own, as the
//! installed cockpit records it for its add-ons, so the user's own daemon is never touched.

use std::path::PathBuf;
use std::time::Duration;

use ev3_c::cockpit::{ClientKind, ConnectOptions, Connection, Connector};
use ev3_c::daemon::CockpitConnector;
use ev3_client::Paths;
use ev3_client::launch::{LaunchSpec, hash_file, record_install};
use ev3c_core::port::{BrickEvent, BrickPort, DeployItem, FileSource, StartOutcome};

const FOLDER: &str = "/home/root/lms2012/prjs/CockpitFixture";
const PROGRAM: &str = "/home/root/lms2012/prjs/CockpitFixture/Wait.rbf";
/// The recording's program: waits a moment and ends.
const WAIT_RBF: [u8; 36] = [
    0x4c, 0x45, 0x47, 0x4f, 0x24, 0x00, 0x00, 0x00, 0x6d, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x85, 0x82, 0x40, 0x1f,
    0x40, 0x86, 0x40, 0x0a,
];

fn named(variable: &str) -> Option<PathBuf> {
    let path = std::env::var_os(variable).map(PathBuf::from)?;
    path.exists().then_some(path)
}

#[tokio::test]
async fn a_program_is_deployed_run_and_followed_to_its_end_through_the_cockpit() {
    let (Some(daemon), Some(fixtures)) =
        (named("EV3C_COCKPIT_DAEMON"), named("EV3C_COCKPIT_FIXTURES"))
    else {
        eprintln!(
            "skipped: set EV3C_COCKPIT_DAEMON and EV3C_COCKPIT_FIXTURES to the cockpit's daemon and fixtures"
        );
        return;
    };
    let root = std::env::temp_dir().join(format!(
        "ev3c-replay-{}",
        ev3_channel::hex(&ev3_channel::random_bytes::<6>())
    ));
    let channel = "replay";
    let paths = Paths::new(&root, Some(channel));
    ev3_channel::fsutil::create_private_dir_all(&paths.dir).expect("channel folder");
    record_install(
        &paths.dir,
        &LaunchSpec {
            sha256: hash_file(&daemon).expect("hash").expect("the daemon"),
            binary: daemon,
            version: "0.0.0".to_owned(),
            build_kind: "development".to_owned(),
        },
    );
    let connector = CockpitConnector {
        daemon_args: vec![
            "--mode".to_owned(),
            "replay".to_owned(),
            "--fixtures".to_owned(),
            fixtures
                .join("service/usb-deploy-and-run-ccc")
                .display()
                .to_string(),
        ],
        state_root: Some(root.clone()),
        notices_as_events: false,
    };

    let connection = connector
        .connect(&ConnectOptions {
            name: "EVƎ C replay test".to_owned(),
            version: "0.0.1".to_owned(),
            kind: ClientKind::Addon,
            channel: Some(channel.to_owned()),
        })
        .await
        .expect("starts the recorded daemon and connects");
    let handle = connection.brick("CCC").await.expect("holds CCC");
    let port = handle.port;
    // Asked as the recording has it; whichever the answer, the deployment makes the folder.
    port.exists(FOLDER).await.expect("files/exists");

    let mut events = port.subscribe().await.expect("programs/subscribe");
    let start = port
        .deploy_and_run(
            &[DeployItem {
                brick_path: PROGRAM.to_owned(),
                source: FileSource::Data(WAIT_RBF.to_vec()),
                skip_if_same: false,
            }],
            PROGRAM,
        )
        .await
        .expect("programs/deployAndRun");
    assert_eq!(start.outcome, StartOutcome::Started, "{start:?}");
    assert_eq!(start.uploaded.len(), 1, "{start:?}");
    assert!(start.run_seq >= 1, "{start:?}");

    // The end: a stopped state of this start or a later one. States heard before the start
    // returned are judged now.
    let ended = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = events.next().await {
            if let BrickEvent::Program(state) = event
                && !state.running
                && state.run_seq >= start.run_seq
            {
                return Some(state);
            }
        }
        None
    })
    .await
    .expect("the program ends within 30 s")
    .expect("the subscription lasts until the end");
    assert_eq!(ended.run_seq, start.run_seq);
    drop(events);

    let removed = port
        .remove_paths(&[FOLDER.to_owned()], true)
        .await
        .expect("files/removePaths");
    assert!(removed.left_behind.is_empty(), "{removed:?}");

    drop(port);
    drop(handle.reader);
    drop(connection);
    // The shutdown the dropped connection started, and the daemon's own end.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = std::fs::remove_dir_all(&root);
}
