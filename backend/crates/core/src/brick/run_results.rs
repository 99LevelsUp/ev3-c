//! How a run ended, read from its run folder on the brick: the completion record, both
//! captured outputs, and the list of files the run returned. Parsing the record lives in
//! [`crate::results`].

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use crate::brick::run_folders::{RESULT_FILE, list_run_numbers, run_dir};
use crate::port::{BrickError, BrickFileReader, BrickPort};
use crate::results::{RunRecord, RunRecordError, parse_run_record};
use crate::returned::{RETURN_LIST_FILE, parse_return_list};

pub(crate) const OUT_FILE: &str = "out";
pub(crate) const ERR_FILE: &str = "err";

/// Launch IDs of the runs EVƎ C started, with their start times by this computer's clock.
/// A Run adds its own while result readers look theirs up, so it is shared, not borrowed
/// mutably.
#[derive(Debug, Default)]
pub struct Launches(Mutex<HashMap<String, SystemTime>>);

impl Launches {
    pub fn new() -> Self {
        Launches::default()
    }

    pub fn insert(&self, launch_id: String, started_at: SystemTime) {
        self.map().insert(launch_id, started_at);
    }

    pub fn remove(&self, launch_id: &str) {
        self.map().remove(launch_id);
    }

    pub fn get(&self, launch_id: &str) -> Option<SystemTime> {
        self.map().get(launch_id).copied()
    }

    /// Every launch with its start time, in no particular order.
    pub fn entries(&self) -> Vec<(String, SystemTime)> {
        self.map()
            .iter()
            .map(|(launch_id, started_at)| (launch_id.clone(), *started_at))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.map().len()
    }

    pub fn is_empty(&self) -> bool {
        self.map().is_empty()
    }

    fn map(&self) -> MutexGuard<'_, HashMap<String, SystemTime>> {
        // A panic while holding the lock cannot leave the map half-changed: every change is
        // one insert or remove.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub run: i64,
    pub record: RunRecord,
    /// Captured stdout; `None` when it could not be read from the brick.
    pub out: Option<String>,
    /// Captured stderr; `None` when it could not be read from the brick.
    pub err: Option<String>,
    /// When EVƎ C started the run, by this computer's clock; `None` for a run EVƎ C did not
    /// start.
    pub started_at: Option<SystemTime>,
    /// Brick paths of the files the run marked for return, in the order marked; empty when
    /// none.
    pub returned: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub run: i64,
    /// `None` while the run has no completion record (in progress, or interrupted).
    pub record: Option<RunRecord>,
    pub started_at: Option<SystemTime>,
}

/// Reading a run folder failed: the brick call, or a completion record that does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunReadError {
    Brick(BrickError),
    Record(RunRecordError),
}

impl fmt::Display for RunReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunReadError::Brick(error) => error.fmt(f),
            RunReadError::Record(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RunReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunReadError::Brick(error) => Some(error),
            RunReadError::Record(error) => Some(error),
        }
    }
}

impl From<BrickError> for RunReadError {
    fn from(error: BrickError) -> Self {
        RunReadError::Brick(error)
    }
}

impl From<RunRecordError> for RunReadError {
    fn from(error: RunRecordError) -> Self {
        RunReadError::Record(error)
    }
}

/// The result of one run, or `None` while it has no completion record.
pub async fn read_run_result(
    port: &impl BrickPort,
    reader: &impl BrickFileReader,
    project: &str,
    run: i64,
    launches: &Launches,
) -> Result<Option<RunResult>, RunReadError> {
    let dir = run_dir(project, run);
    let names: HashSet<String> = port
        .list(&dir)
        .await?
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    if !names.contains(RESULT_FILE) {
        return Ok(None);
    }
    let record = parse_run_record(&text(
        &reader.read_file(&format!("{dir}/{RESULT_FILE}")).await?,
    ))?;
    Ok(Some(
        result_from_record(reader, project, run, record, launches, Some(&names)).await,
    ))
}

/// Completes a run's result from its already-read record by reading both outputs and the
/// return list. A file that is missing (per `present`, when given) or cannot be read is
/// left out; a missing return list means the run marked no file.
pub async fn result_from_record(
    reader: &impl BrickFileReader,
    project: &str,
    run: i64,
    record: RunRecord,
    launches: &Launches,
    present: Option<&HashSet<String>>,
) -> RunResult {
    result_with_raw_output(reader, project, run, record, launches, present)
        .await
        .0
}

/// The bytes of a run's stdout and stderr as the brick holds them; `None` when unread.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawOutput {
    pub out: Option<Vec<u8>>,
    pub err: Option<Vec<u8>>,
}

/// As [`result_from_record`], with the outputs' bytes too, for a caller that already showed
/// part of them and needs the rest by byte offset.
pub async fn result_with_raw_output(
    reader: &impl BrickFileReader,
    project: &str,
    run: i64,
    record: RunRecord,
    launches: &Launches,
    present: Option<&HashSet<String>>,
) -> (RunResult, RawOutput) {
    let dir = run_dir(project, run);
    let read_bytes = async |name: &str| -> Option<Vec<u8>> {
        if present.is_some_and(|present| !present.contains(name)) {
            return None;
        }
        reader.read_file(&format!("{dir}/{name}")).await.ok()
    };
    let out = read_bytes(OUT_FILE).await;
    let err = read_bytes(ERR_FILE).await;
    let list = read_bytes(RETURN_LIST_FILE).await.map(|data| text(&data));
    let result = RunResult {
        run,
        started_at: start_time(&record, launches),
        record,
        out: out.as_deref().map(text),
        err: err.as_deref().map(text),
        returned: list.map_or_else(Vec::new, |list| parse_return_list(&list)),
    };
    (result, RawOutput { out, err })
}

/// Every run folder of the project, oldest first, with its completion record and, for runs
/// EVƎ C started, this computer's start time. Reads one record per run.
pub async fn list_runs(
    port: &impl BrickPort,
    reader: &impl BrickFileReader,
    project: &str,
    launches: &Launches,
) -> Result<Vec<RunSummary>, RunReadError> {
    let mut summaries = Vec::new();
    for run in list_run_numbers(port, project).await? {
        let record = read_run_record(port, reader, project, run).await?;
        summaries.push(RunSummary {
            run,
            started_at: record
                .as_ref()
                .and_then(|record| start_time(record, launches)),
            record,
        });
    }
    Ok(summaries)
}

/// The completion record of a run, or `None` while it has none.
pub async fn read_run_record(
    port: &impl BrickPort,
    reader: &impl BrickFileReader,
    project: &str,
    run: i64,
) -> Result<Option<RunRecord>, RunReadError> {
    let dir = run_dir(project, run);
    if !port
        .list(&dir)
        .await?
        .iter()
        .any(|entry| entry.name == RESULT_FILE)
    {
        return Ok(None);
    }
    let data = reader.read_file(&format!("{dir}/{RESULT_FILE}")).await?;
    Ok(Some(parse_run_record(&text(&data))?))
}

fn start_time(record: &RunRecord, launches: &Launches) -> Option<SystemTime> {
    record
        .launch
        .as_deref()
        .and_then(|launch| launches.get(launch))
}

/// Brick text as UTF-8, with a replacement character for each invalid byte sequence.
fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::brick::fake::FakeBrickPort;
    use crate::results::RunEnd;

    const RUNS: &str = "/home/root/lms2012/prjs/robot/runs";

    fn setup() -> FakeBrickPort {
        let port = FakeBrickPort::new();
        port.with(|brick| brick.put_file(&format!("{RUNS}/next"), "5"));
        port
    }

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_result_joins_the_record_both_outputs_and_the_pc_start_time() {
        let port = setup();
        port.with(|brick| {
            brick.put_file(&format!("{RUNS}/4/out"), "hello\n");
            brick.put_file(&format!("{RUNS}/4/err"), "oops\n");
            brick.put_file(
                &format!("{RUNS}/4/res"),
                "run=4\nend=program\nsignal=11\nout_truncated=0\nerr_truncated=0\nlaunch=L4\n",
            );
        });
        let launches = Launches::new();
        launches.insert("L4".into(), at(1_795_000_000));

        let result = read_run_result(&port, &port, "robot", 4, &launches)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(result.run, 4);
        assert_eq!(result.record.signal, Some(11));
        assert_eq!(result.out.as_deref(), Some("hello\n"));
        assert_eq!(result.err.as_deref(), Some("oops\n"));
        assert_eq!(result.started_at, Some(at(1_795_000_000)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_without_a_completion_record_has_no_result_yet() {
        let port = setup();
        port.with(|brick| {
            brick.put_file(&format!("{RUNS}/4/out"), "partial");
            brick.put_file(&format!("{RUNS}/4/res.tmp"), "run=4\nend=program\n");
        });

        let result = read_run_result(&port, &port, "robot", 4, &Launches::new()).await;

        assert_eq!(result, Ok(None));
        assert!(
            port.with(|brick| brick.read_times.is_empty()),
            "nothing is read before the record exists"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_output_stream_that_is_not_on_the_brick_is_left_out_instead_of_shown_as_empty() {
        let port = setup();
        port.with(|brick| {
            brick.put_file(
                &format!("{RUNS}/4/res"),
                "run=4\nend=before-start\nout_truncated=0\nerr_truncated=0\n",
            );
        });

        let result = read_run_result(&port, &port, "robot", 4, &Launches::new())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(result.out, None);
        assert_eq!(result.err, None);
        assert_eq!(result.started_at, None);
        assert_eq!(
            port.with(|brick| brick.read_times.len()),
            1,
            "only the record is read; the listing says the outputs are missing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_record_that_does_not_parse_is_an_error() {
        let port = setup();
        port.with(|brick| brick.put_file(&format!("{RUNS}/4/res"), "run=4\n"));

        let error = read_run_result(&port, &port, "robot", 4, &Launches::new())
            .await
            .unwrap_err();

        assert!(matches!(error, RunReadError::Record(_)), "{error:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn listing_shows_a_run_started_on_the_brick_without_a_pc_time_next_to_runs_evec_started()
    {
        let port = setup();
        port.with(|brick| {
            brick.put_file(
                &format!("{RUNS}/1/res"),
                "run=1\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\nlaunch=A\n",
            );
            brick.put_file(
                &format!("{RUNS}/2/res"),
                "run=2\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n",
            );
            brick.put_file(
                &format!("{RUNS}/3/res"),
                "run=3\nend=slot\nsignal=15\nout_truncated=0\nerr_truncated=0\nlaunch=B\n",
            );
            brick.put_file(&format!("{RUNS}/4/out"), "still running");
        });
        let launches = Launches::new();
        launches.insert("A".into(), at(100));
        launches.insert("B".into(), at(400));

        let runs = list_runs(&port, &port, "robot", &launches).await.unwrap();

        let seen: Vec<_> = runs
            .iter()
            .map(|run| {
                (
                    run.run,
                    run.started_at,
                    run.record.as_ref().map(|record| record.end),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (1, Some(at(100)), Some(RunEnd::Program)),
                (2, None, Some(RunEnd::Program)),
                (3, Some(at(400)), Some(RunEnd::Slot)),
                (4, None, None),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn listing_reads_each_completion_record_once() {
        let port = setup();
        port.with(|brick| {
            for run in [1, 2, 3] {
                brick.put_file(
                    &format!("{RUNS}/{run}/res"),
                    format!("run={run}\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n"),
                );
            }
        });

        list_runs(&port, &port, "robot", &Launches::new())
            .await
            .unwrap();

        assert_eq!(port.with(|brick| brick.read_times.len()), 3);
    }
}
