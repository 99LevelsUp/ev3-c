//! Upload without a run: puts a built project onto the brick. Only files whose MD5 differs
//! from the brick's listing are uploaded, which skips the unchanged supervisor and doubles
//! as its version check. Nothing is uploaded while a program runs: overwriting a running
//! binary fails on the brick or leaves it truncated. A Run uploads inside its own start
//! (`run_project`), where cockpit makes the same checks under one lease.

use std::collections::HashMap;
use std::time::Duration;

use md5::{Digest, Md5};

use crate::build::BuildArtifact;
use crate::port::{BrickError, BrickPort, EntryKind, FileSource};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeployReport {
    pub uploaded: Vec<BuildArtifact>,
    pub unchanged: Vec<BuildArtifact>,
}

/// Failures where the link dropped before or during the write. Writing the whole file
/// again is safe, and a moment later the link is often back. Anything else, above all
/// `outcome_unknown`, `brick_busy` and `cancelled`, is not retried.
const RETRIED_CODES: [&str; 3] = ["not_connected", "link_lost", "transport_failed"];
const RETRY_DELAY: Duration = Duration::from_millis(250);

/// Uploads the artifacts of one build, which all go into the same project folder. A local
/// artifact that cannot be read fails the upload with an error without a code.
pub async fn deploy(
    port: &impl BrickPort,
    artifacts: &[BuildArtifact],
) -> Result<DeployReport, BrickError> {
    if port.program_state().await?.running {
        return Err(BrickError::new(
            Some("program_running"),
            "A program is running on the brick. Stop it first, then upload the project again.",
        ));
    }
    let mut report = DeployReport::default();
    let Some(first) = artifacts.first() else {
        return Ok(report);
    };

    let dir = folder_of(&first.brick_path);
    let mut on_brick: HashMap<String, Option<String>> = HashMap::new();
    if port.exists(dir).await? {
        for entry in port.list(dir).await? {
            if entry.kind == EntryKind::File {
                on_brick.insert(entry.name, entry.md5.map(|md5| md5.to_lowercase()));
            }
        }
    }

    for artifact in artifacts {
        let data = std::fs::read(&artifact.local_path).map_err(|error| {
            BrickError::new(
                None,
                format!("Could not read {}: {error}", artifact.local_path.display()),
            )
        })?;
        let brick_md5 = on_brick
            .get(name_of(&artifact.brick_path))
            .cloned()
            .flatten();
        if brick_md5.is_some_and(|md5| md5 == format!("{:x}", Md5::digest(&data))) {
            report.unchanged.push(artifact.clone());
            continue;
        }
        upload(port, artifact).await?;
        report.uploaded.push(artifact.clone());
    }
    Ok(report)
}

async fn upload(port: &impl BrickPort, artifact: &BuildArtifact) -> Result<(), BrickError> {
    let source = FileSource::LocalPath(artifact.local_path.clone());
    match port.write_file(&artifact.brick_path, &source).await {
        Ok(()) => return Ok(()),
        Err(error) if !RETRIED_CODES.contains(&error.code().unwrap_or_default()) => {
            return Err(failed(&artifact.brick_path, &error, "failed"));
        }
        Err(_) => {}
    }
    tokio::time::sleep(RETRY_DELAY).await;
    port.write_file(&artifact.brick_path, &source)
        .await
        .map_err(|error| failed(&artifact.brick_path, &error, "failed twice"))
}

fn failed(brick_path: &str, error: &BrickError, how: &str) -> BrickError {
    let code = error.code();
    BrickError::new(
        code,
        format!(
            "Uploading {brick_path} {how} ({}): {error}",
            code.unwrap_or("no error code")
        ),
    )
}

fn folder_of(brick_path: &str) -> &str {
    brick_path
        .rfind('/')
        .map_or("", |slash| &brick_path[..slash])
}

fn name_of(brick_path: &str) -> &str {
    brick_path
        .rfind('/')
        .map_or(brick_path, |slash| &brick_path[slash + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::fake::FakeBrickPort;
    use crate::build::ArtifactKind;
    use crate::testutil::temp_dir;

    const BRICK_DIR: &str = "/home/root/lms2012/prjs/robot";

    fn artifacts() -> Vec<BuildArtifact> {
        let dir = temp_dir("ev3c-deploy-");
        let make = |kind, name: &str, content: &str| {
            let local_path = dir.join(name);
            std::fs::write(&local_path, content).unwrap();
            BuildArtifact {
                kind,
                local_path,
                brick_path: format!("{BRICK_DIR}/{name}"),
            }
        };
        vec![
            make(ArtifactKind::Program, "prog", "program v2"),
            make(ArtifactKind::Launcher, "robot.rbf", "launcher v2"),
            make(ArtifactKind::Supervisor, "ev3c-sup", "supervisor v1"),
        ]
    }

    fn brick() -> FakeBrickPort {
        let port = FakeBrickPort::new();
        port.with(|brick| brick.mkdirs("/home/root/lms2012/prjs"));
        port
    }

    fn brick_with_all_but_the_program_current() -> FakeBrickPort {
        let port = brick();
        port.with(|brick| {
            brick.put_file(&format!("{BRICK_DIR}/prog"), "program v1");
            brick.put_file(&format!("{BRICK_DIR}/robot.rbf"), "launcher v2");
            brick.put_file(&format!("{BRICK_DIR}/ev3c-sup"), "supervisor v1");
        });
        port
    }

    fn writes(port: &FakeBrickPort) -> Vec<String> {
        port.calls()
            .into_iter()
            .filter(|call| call.starts_with("write_file "))
            .collect()
    }

    fn writes_of(port: &FakeBrickPort, name: &str) -> usize {
        writes(port)
            .iter()
            .filter(|call| call.ends_with(&format!("/{name}")))
            .count()
    }

    fn kinds(artifacts: &[BuildArtifact]) -> Vec<ArtifactKind> {
        artifacts.iter().map(|artifact| artifact.kind).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn deploy_uploads_only_the_artifacts_whose_md5_differs_from_the_brick_listing() {
        let port = brick_with_all_but_the_program_current();

        let report = deploy(&port, &artifacts()).await.unwrap();

        assert_eq!(kinds(&report.uploaded), [ArtifactKind::Program]);
        assert_eq!(
            kinds(&report.unchanged),
            [ArtifactKind::Launcher, ArtifactKind::Supervisor]
        );
        assert_eq!(writes(&port), [format!("write_file {BRICK_DIR}/prog")]);
        assert_eq!(
            port.with(|brick| brick.text(&format!("{BRICK_DIR}/prog"))),
            Some("program v2".into())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn deploy_uploads_a_file_whose_listing_has_no_md5() {
        let port = brick();
        port.with(|brick| {
            brick.put_file(&format!("{BRICK_DIR}/prog"), "program v2");
            brick.put_file(&format!("{BRICK_DIR}/robot.rbf"), "launcher v2");
            brick.put_file(&format!("{BRICK_DIR}/ev3c-sup"), "supervisor v1");
            brick.list_md5 = Some(Box::new(|_| None));
        });

        let report = deploy(&port, &artifacts()).await.unwrap();

        assert_eq!(report.uploaded.len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn deploy_compares_md5_without_regard_to_letter_case() {
        let port = brick();
        port.with(|brick| {
            brick.put_file(&format!("{BRICK_DIR}/prog"), "program v2");
            brick.put_file(&format!("{BRICK_DIR}/robot.rbf"), "launcher v2");
            brick.put_file(&format!("{BRICK_DIR}/ev3c-sup"), "supervisor v1");
            brick.list_md5 = Some(Box::new(|md5| md5.map(|md5| md5.to_uppercase())));
        });

        let report = deploy(&port, &artifacts()).await.unwrap();

        assert!(report.uploaded.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn deploy_into_a_missing_project_folder_uploads_everything() {
        let port = brick();

        let report = deploy(&port, &artifacts()).await.unwrap();

        assert_eq!(report.uploaded.len(), 3);
        assert_eq!(
            port.with(|brick| brick.text(&format!("{BRICK_DIR}/ev3c-sup"))),
            Some("supervisor v1".into())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn deploy_refuses_while_a_program_runs_and_touches_nothing_on_the_brick() {
        let port = brick();
        port.with(|brick| brick.running = true);

        let error = deploy(&port, &artifacts()).await.unwrap_err();

        assert_eq!(error.code(), Some("program_running"));
        let message = error.to_string().to_lowercase();
        assert!(message.contains("program is running"), "{message}");
        assert!(message.contains("stop it"), "{message}");
        assert_eq!(port.calls(), ["program_state"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_upload_is_retried_once() {
        let port = brick();
        port.with(|brick| brick.fail_writes(&format!("{BRICK_DIR}/prog"), &["transport_failed"]));

        let report = deploy(&port, &artifacts()).await.unwrap();

        assert_eq!(report.uploaded.len(), 3);
        assert_eq!(writes_of(&port, "prog"), 2);
        assert_eq!(
            port.with(|brick| brick.text(&format!("{BRICK_DIR}/prog"))),
            Some("program v2".into())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_upload_that_fails_twice_is_reported_with_the_cockpit_error_code() {
        let port = brick();
        port.with(|brick| {
            brick.fail_writes(
                &format!("{BRICK_DIR}/robot.rbf"),
                &["transport_failed", "timeout"],
            );
        });

        let error = deploy(&port, &artifacts()).await.unwrap_err();

        assert_eq!(error.code(), Some("timeout"));
        assert!(error.to_string().contains("robot.rbf"), "{error}");
        assert!(
            error.to_string().contains("failed twice (timeout)"),
            "{error}"
        );
        assert_eq!(writes_of(&port, "robot.rbf"), 2);
        assert!(
            !port.with(|brick| brick.has(&format!("{BRICK_DIR}/ev3c-sup"))),
            "nothing is uploaded after the failure"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_upload_whose_outcome_is_unknown_is_not_retried() {
        let port = brick();
        port.with(|brick| brick.fail_writes(&format!("{BRICK_DIR}/prog"), &["outcome_unknown"]));

        let error = deploy(&port, &artifacts()).await.unwrap_err();

        assert_eq!(error.code(), Some("outcome_unknown"));
        assert_eq!(writes_of(&port, "prog"), 1);
    }
}
