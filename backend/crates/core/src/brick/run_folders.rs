//! A project's run folders on the brick: `<project>/runs/<n>/` with `out`, `err`, and the
//! completion record `res`, which the supervisor renames into place as its last write.
//! `runs/next` is the run counter; it is never deleted, so run numbers are never reused.

use std::collections::HashSet;

use serde::Serialize;

use crate::build::brick_project_dir;
use crate::port::{BrickError, BrickPort, EntryKind};

/// Name of the completion record inside a run folder.
pub const RESULT_FILE: &str = "res";

pub fn runs_dir(project: &str) -> String {
    format!("{}/runs", brick_project_dir(project))
}

pub fn run_dir(project: &str, run: i64) -> String {
    format!("{}/{run}", runs_dir(project))
}

/// Run numbers of the project's run folders, oldest first; none when the runs folder is
/// missing. Only folders named by a number without leading zeros count.
pub async fn list_run_numbers(
    port: &impl BrickPort,
    project: &str,
) -> Result<Vec<i64>, BrickError> {
    let dir = runs_dir(project);
    if !port.exists(&dir).await? {
        return Ok(Vec::new());
    }
    let mut runs: Vec<i64> = port
        .list(&dir)
        .await?
        .into_iter()
        .filter(|entry| entry.kind == EntryKind::Directory && is_run_number(&entry.name))
        // A number too large for i64 names no run the supervisor made.
        .filter_map(|entry| entry.name.parse().ok())
        .collect();
    runs.sort_unstable();
    Ok(runs)
}

/// `^[1-9][0-9]*$`
fn is_run_number(name: &str) -> bool {
    name.starts_with(|c: char| ('1'..='9').contains(&c)) && name.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRunsReport {
    /// Run folders that are gone.
    pub deleted: Vec<i64>,
    /// Run folders left on purpose: the newest ones asked to be kept and a running run's
    /// folder.
    pub kept: Vec<i64>,
    /// Run folders that were to be deleted but are still listed afterwards.
    pub still_present: Vec<i64>,
}

/// Deletes the project's run folders, all of them (`keep_last` 0) or all but the newest
/// `keep_last`, in one cockpit operation, and never the folder of the run in progress. The
/// runs folder is listed again afterwards, so what is reported as still there is what the
/// brick shows, whatever the removal said or however it failed.
pub async fn delete_runs(
    port: &impl BrickPort,
    project: &str,
    keep_last: usize,
) -> Result<DeleteRunsReport, BrickError> {
    // Targets come from this one listing, so a run folder the supervisor creates while
    // deleting is never touched.
    let runs = list_run_numbers(port, project).await?;
    if runs.is_empty() {
        return Ok(DeleteRunsReport::default());
    }
    let mut kept: HashSet<i64> = runs[runs.len().saturating_sub(keep_last)..]
        .iter()
        .copied()
        .collect();
    if let Some(running) = running_run(port, project, &runs).await? {
        kept.insert(running);
    }

    let targets: Vec<i64> = runs
        .iter()
        .copied()
        .filter(|run| !kept.contains(run))
        .collect();
    if !targets.is_empty() {
        let paths: Vec<String> = targets.iter().map(|run| run_dir(project, *run)).collect();
        // Whatever the removal says or however it fails, the listing below tells what is left.
        let _ = port.remove_paths(&paths, true).await;
    }

    let left: HashSet<i64> = list_run_numbers(port, project).await?.into_iter().collect();
    Ok(DeleteRunsReport {
        deleted: targets
            .iter()
            .copied()
            .filter(|run| !left.contains(run))
            .collect(),
        kept: runs
            .iter()
            .copied()
            .filter(|run| kept.contains(run))
            .collect(),
        still_present: targets
            .iter()
            .copied()
            .filter(|run| left.contains(run))
            .collect(),
    })
}

/// The run in progress: while the user slot runs, the newest run folder without a
/// completion record. A newest folder that has its record belongs to a run that has ended
/// (or the supervisor has not created the new folder yet), so nothing is protected.
async fn running_run(
    port: &impl BrickPort,
    project: &str,
    runs: &[i64],
) -> Result<Option<i64>, BrickError> {
    let Some(&newest) = runs.last() else {
        return Ok(None);
    };
    if !port.program_state().await?.running {
        return Ok(None);
    }
    let entries = port.list(&run_dir(project, newest)).await?;
    Ok((!entries.iter().any(|entry| entry.name == RESULT_FILE)).then_some(newest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::fake::FakeBrickPort;

    const RUNS: &str = "/home/root/lms2012/prjs/robot/runs";

    fn brick_with_runs(complete: &[i64], incomplete: &[i64]) -> FakeBrickPort {
        let port = FakeBrickPort::new();
        port.with(|brick| {
            let newest = complete
                .iter()
                .chain(incomplete)
                .copied()
                .max()
                .unwrap_or(0);
            brick.put_file(&format!("{RUNS}/next"), (newest + 1).to_string());
            for run in complete {
                brick.put_file(&format!("{RUNS}/{run}/out"), "out");
                brick.put_file(
                    &format!("{RUNS}/{run}/res"),
                    format!("run={run}\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n"),
                );
            }
            for run in incomplete {
                brick.put_file(&format!("{RUNS}/{run}/out"), "out");
            }
        });
        port
    }

    fn removals(port: &FakeBrickPort) -> Vec<String> {
        port.calls()
            .into_iter()
            .filter(|call| call.starts_with("remove"))
            .collect()
    }

    fn report(deleted: &[i64], kept: &[i64], still_present: &[i64]) -> DeleteRunsReport {
        DeleteRunsReport {
            deleted: deleted.to_vec(),
            kept: kept.to_vec(),
            still_present: still_present.to_vec(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn run_folders_are_listed_by_number_in_numeric_order_other_entries_ignored() {
        let port = brick_with_runs(&[10, 2, 9], &[]);
        port.with(|brick| {
            brick.mkdirs(&format!("{RUNS}/tmp"));
            brick.mkdirs(&format!("{RUNS}/3a"));
            brick.mkdirs(&format!("{RUNS}/04"));
            brick.mkdirs(&format!("{RUNS}/0"));
            brick.put_file(&format!("{RUNS}/next.tmp"), "11");
            brick.put_file(&format!("{RUNS}/5"), "a file, not a run folder");
        });

        assert_eq!(list_run_numbers(&port, "robot").await, Ok(vec![2, 9, 10]));
    }

    #[tokio::test(start_paused = true)]
    async fn a_project_without_a_runs_folder_has_no_runs() {
        let port = FakeBrickPort::new();
        port.with(|brick| brick.mkdirs("/home/root/lms2012/prjs/robot"));

        assert_eq!(list_run_numbers(&port, "robot").await, Ok(vec![]));
    }

    #[tokio::test(start_paused = true)]
    async fn deleting_all_but_the_last_2_removes_the_older_folders_in_one_call_and_reports_one_still_present()
     {
        let port = brick_with_runs(&[1, 2, 3, 4, 5], &[]);
        // A folder with a subfolder survives the fake's recursive removal, which deletes one
        // level only.
        port.with(|brick| brick.mkdirs(&format!("{RUNS}/2/data")));

        let result = delete_runs(&port, "robot", 2).await;

        assert_eq!(
            removals(&port),
            [format!("remove_paths -r {RUNS}/1 {RUNS}/2 {RUNS}/3")]
        );
        assert_eq!(
            port.calls().last(),
            Some(&format!("list {RUNS}")),
            "the runs folder is listed again after deleting"
        );
        assert_eq!(result, Ok(report(&[1, 3], &[4, 5], &[2])));
        assert_eq!(
            port.with(|brick| brick.text(&format!("{RUNS}/next"))),
            Some("6".into()),
            "the run counter is never touched"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn keeping_more_runs_than_there_are_keeps_them_all() {
        let port = brick_with_runs(&[1, 2, 3], &[]);

        assert_eq!(
            delete_runs(&port, "robot", 5).await,
            Ok(report(&[], &[1, 2, 3], &[]))
        );
        assert_eq!(removals(&port), Vec::<String>::new());
    }

    #[tokio::test(start_paused = true)]
    async fn deleting_all_during_a_run_keeps_the_running_programs_folder() {
        let port = brick_with_runs(&[1, 2], &[3]);
        port.with(|brick| brick.running = true);

        let result = delete_runs(&port, "robot", 0).await;

        assert_eq!(result, Ok(report(&[1, 2], &[3], &[])));
        assert!(port.with(|brick| brick.has(&format!("{RUNS}/3/out"))));
    }

    #[tokio::test(start_paused = true)]
    async fn deleting_all_with_no_program_running_also_removes_a_folder_left_without_a_result() {
        let port = brick_with_runs(&[1, 2], &[3]);

        assert_eq!(
            delete_runs(&port, "robot", 0).await,
            Ok(report(&[1, 2, 3], &[], &[]))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn while_the_slot_runs_but_the_newest_run_is_complete_no_folder_is_protected() {
        let port = brick_with_runs(&[1, 2], &[]);
        port.with(|brick| brick.running = true);

        assert_eq!(
            delete_runs(&port, "robot", 0).await,
            Ok(report(&[1, 2], &[], &[]))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_folder_that_appears_during_deletion_is_not_deleted() {
        let port = brick_with_runs(&[1, 2], &[]);
        port.with(|brick| {
            brick.on_remove = Some(Box::new(|brick| {
                brick.put_file(&format!("{RUNS}/3/out"), "new run");
            }));
        });

        let result = delete_runs(&port, "robot", 0).await.unwrap();

        assert_eq!(result.deleted, [1, 2]);
        assert!(port.with(|brick| brick.has(&format!("{RUNS}/3"))));
        assert!(!removals(&port).iter().any(|call| call.ends_with("/3")));
    }

    #[tokio::test(start_paused = true)]
    async fn deleting_from_a_project_without_runs_does_nothing() {
        let port = FakeBrickPort::new();
        port.with(|brick| brick.mkdirs("/home/root/lms2012/prjs/robot"));

        assert_eq!(
            delete_runs(&port, "robot", 0).await,
            Ok(DeleteRunsReport::default())
        );
        assert_eq!(removals(&port), Vec::<String>::new());
    }
}
