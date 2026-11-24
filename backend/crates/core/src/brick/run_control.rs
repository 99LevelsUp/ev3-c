//! Run, Stop, and Emergency Stop. A Run is tied to its result by a launch ID, not by a
//! predicted run number: a run started from the brick menu, or one still finishing, could
//! take the predicted number first. EVƎ C puts the ID into `<project>/.req` in the same
//! cockpit operation that uploads the project and starts the launcher, so no other client
//! can start something in between; the supervisor moves the ID into the run's completion
//! record.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::hash::{BuildHasher, RandomState};
use std::pin::pin;
use std::time::{Duration, SystemTime};

use crate::brick::live_output::{LiveOutput, OnOutput};
use crate::brick::run_folders::list_run_numbers;
use crate::brick::run_results::{
    Launches, RunReadError, RunResult, read_run_record, result_with_raw_output,
};
use crate::build::{BuildArtifact, brick_project_dir};
use crate::port::{
    BrickError, BrickEvent, BrickFileReader, BrickPort, Clock, DeployItem, FileSource,
    ProgramState, StartOutcome, StartReport, Subscription,
};
use crate::results::RunRecord;

/// How long EVƎ C waits for the completion record after the slot stops.
pub const RESULT_WAIT: Duration = Duration::from_secs(10);

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const REQUEST_FILE: &str = ".req";

// Cockpit refused or cancelled the start, so no supervisor will take the launch ID. After any
// other failure, `outcome_unknown` above all, the launcher may already run, and its
// supervisor may still read the request.
const NOT_STARTED_CODES: [&str; 9] = [
    "program_running",
    "program_not_found",
    "access_denied",
    "not_connected",
    "brick_busy",
    "brick_not_found",
    "brick_ambiguous",
    "cancelled",
    "invalid_request",
];

/// Makes a launch ID.
pub type NewLaunchId = dyn Fn() -> String + Sync;

/// Hears that the start succeeded, while the run goes on.
pub type OnStarted<'a> = dyn Fn(&StartReport) + Sync + 'a;

pub struct RunProjectOptions<'a> {
    /// Project name; the launcher is `<project>/<project>.rbf` in the brick's projects
    /// folder.
    pub project: &'a str,
    /// The build's artifacts, uploaded as part of the start when they differ from the
    /// brick's.
    pub artifacts: &'a [BuildArtifact],
    /// Launch IDs of the runs EVƎ C started, with their start times; this run is added.
    pub launches: &'a Launches,
    /// Makes the launch ID; a fresh random one when `None`.
    pub new_launch_id: Option<&'a NewLaunchId>,
    /// Called once the start returned successfully, before the wait for the run's end, so a
    /// front end can show that the program runs.
    pub on_started: Option<&'a OnStarted<'a>>,
    /// Hears the program's output while it runs and the rest at the end (see
    /// [`live_output`](crate::brick::live_output)). `None` follows nothing: the brick's few
    /// follows are shared by every client, so a front end that does not show output live
    /// leaves them free.
    pub on_output: Option<&'a OnOutput<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Result {
        result: RunResult,
        start: StartReport,
    },
    /// The completion record did not appear in time; the run folder stays on the brick for
    /// later.
    NoResult {
        launch_id: String,
        started_at: SystemTime,
        start: StartReport,
    },
}

/// Uploads the project, starts its launcher, and waits until the run ends and its
/// completion record is on the brick. The run has ended when cockpit reports the slot
/// stopped under this start's number or a later one, or when the start was not seen
/// running. The wait for the record then polls at most once per second for up to 10 s, and
/// a reconnect during that time starts the 10 s again.
///
/// `reader` is the shared reader that keeps file reads apart; the port itself is one. A
/// failed start is returned as cockpit reported it.
pub async fn run_project(
    port: &impl BrickPort,
    reader: &impl BrickFileReader,
    clock: &impl Clock,
    options: RunProjectOptions<'_>,
) -> Result<RunOutcome, BrickError> {
    let RunProjectOptions {
        project,
        artifacts,
        launches,
        new_launch_id,
        on_started,
        on_output,
    } = options;
    let dir = brick_project_dir(project);
    let request_path = format!("{dir}/{REQUEST_FILE}");

    let newest_before = list_run_numbers(port, project)
        .await?
        .last()
        .copied()
        .unwrap_or(0);

    // Subscribed before the start, so the end of a short run is never missed. The
    // subscription ends when `wait` is dropped, on every way out of this function.
    let mut wait = ResultWait::new(port.subscribe().await?, clock);
    let launch_id = new_launch_id.map_or_else(random_launch_id, |make| make());
    let started_at = clock.now();
    launches.insert(launch_id.clone(), started_at);
    let mut files: Vec<DeployItem> = artifacts
        .iter()
        .map(|artifact| DeployItem {
            brick_path: artifact.brick_path.clone(),
            // Cockpit may resolve a relative path against another folder.
            source: FileSource::LocalPath(
                std::path::absolute(&artifact.local_path)
                    .unwrap_or_else(|_| artifact.local_path.clone()),
            ),
            skip_if_same: true,
        })
        .collect();
    files.push(DeployItem {
        brick_path: request_path.clone(),
        source: FileSource::Data(format!("{launch_id}\n").into_bytes()),
        skip_if_same: false,
    });

    let start = wait
        .hear_during(port.deploy_and_run(&files, &format!("{dir}/{project}.rbf")))
        .await;
    // Events already queued when the start returned were heard while it ran.
    wait.hear_queued();
    let start = match start {
        Ok(start) => start,
        Err(error) => {
            if error
                .code()
                .is_some_and(|code| NOT_STARTED_CODES.contains(&code))
            {
                launches.remove(&launch_id);
                // Best effort: a request left behind is ignored by the next run's supervisor,
                // whose launch ID replaces it.
                let _ = port.remove_paths(&[request_path], false).await;
            }
            return Err(error);
        }
    };
    if let Some(on_started) = on_started {
        on_started(&start);
    }
    wait.started(&start);

    let ended = async {
        wait.until_slot_stopped(port).await?;
        // Runs that ended with another launch ID, or none; their records are final.
        let mut others = HashSet::new();
        Ok::<_, BrickError>(loop {
            let attempt = find_record(
                port,
                reader,
                project,
                newest_before,
                &launch_id,
                &mut others,
            );
            // A brick error (disconnected, or a record not readable yet) means "not yet": the
            // next poll tries again.
            if let Ok(Some(found)) = wait.hear_during(attempt).await {
                break Some(found);
            }
            if wait.past_deadline(clock.now() + POLL_INTERVAL) {
                break None;
            }
            wait.hear_during(clock.sleep(POLL_INTERVAL)).await;
        })
    };
    let live = on_output.map(LiveOutput::new);
    let found = match &live {
        // The output is followed until the record is found; leaving the select drops the
        // follows, so nothing arrives after `finish` below.
        Some(live) => {
            let follow = async {
                live.follow(port, reader, clock, project, newest_before, &launch_id)
                    .await;
                std::future::pending::<()>().await;
            };
            tokio::select! {
                biased;
                found = ended => found?,
                () = follow => unreachable!("the follow future never ends"),
            }
        }
        None => ended.await?,
    };
    drop(wait);

    Ok(match found {
        Some((run, record)) => {
            let (result, raw) =
                result_with_raw_output(reader, project, run, record, launches, None).await;
            if let Some(live) = &live {
                live.finish(Some(&raw));
            }
            RunOutcome::Result { result, start }
        }
        None => {
            if let Some(live) = &live {
                live.finish(None);
            }
            RunOutcome::NoResult {
                launch_id,
                started_at,
                start,
            }
        }
    })
}

/// Stops the user slot through cockpit; EVƎ C adds no stop of its own.
pub async fn stop_program(port: &impl BrickPort) -> Result<ProgramState, BrickError> {
    port.stop_program().await
}

/// Cockpit's emergency stop, unchanged.
pub async fn emergency_stop(port: &impl BrickPort) -> Result<ProgramState, BrickError> {
    port.emergency_stop().await
}

/// 16 hex digits. Only uniqueness matters, not secrecy: std's `RandomState` is keyed from
/// the operating system's random source and changes its key for each new state, so no
/// random-number crate is needed.
fn random_launch_id() -> String {
    format!("{:016x}", RandomState::new().hash_one(()))
}

/// One poll: the newest run folders since the start, newest first, until one carries this
/// run's launch ID. Folders without a record yet are looked at again on the next poll.
async fn find_record(
    port: &impl BrickPort,
    reader: &impl BrickFileReader,
    project: &str,
    newest_before: i64,
    launch_id: &str,
    others: &mut HashSet<i64>,
) -> Result<Option<(i64, RunRecord)>, BrickError> {
    let candidates: Vec<i64> = list_run_numbers(port, project)
        .await?
        .into_iter()
        .rev()
        .filter(|run| *run > newest_before && !others.contains(run))
        .collect();
    for candidate in candidates {
        match read_run_record(port, reader, project, candidate).await {
            Ok(Some(record)) if record.launch.as_deref() == Some(launch_id) => {
                return Ok(Some((candidate, record)));
            }
            // A record that does not parse cannot carry this run's ID either.
            Ok(Some(_)) | Err(RunReadError::Record(_)) => {
                others.insert(candidate);
            }
            Ok(None) => {}
            Err(RunReadError::Brick(error)) => return Err(error),
        }
    }
    Ok(None)
}

/// Waits for the slot to stop, then keeps the deadline of the polls, which a reconnect
/// moves. The brick's events are heard during every brick call and every sleep of a Run.
/// States heard before the start's number is known are kept, so a run that ends while the
/// start is still returning is not missed.
struct ResultWait<'c, C> {
    events: Subscription,
    events_open: bool,
    clock: &'c C,
    /// This start's number, once the start returned.
    run_seq: Option<u64>,
    /// The highest start number heard together with a stopped slot or a later start.
    ended_seq: u64,
    stopped: bool,
    deadline: Option<SystemTime>,
    /// State checks to make, oldest first; `true` takes any stopped slot as the end.
    checks: VecDeque<bool>,
}

impl<'c, C: Clock> ResultWait<'c, C> {
    fn new(events: Subscription, clock: &'c C) -> Self {
        ResultWait {
            events,
            events_open: true,
            clock,
            run_seq: None,
            ended_seq: 0,
            stopped: false,
            deadline: None,
            checks: VecDeque::new(),
        }
    }

    /// Runs `future` to its end, hearing the brick's events meanwhile.
    async fn hear_during<F: Future>(&mut self, future: F) -> F::Output {
        let mut future = pin!(future);
        loop {
            // Events first: one the port sent before answering belongs before the answer.
            let event = tokio::select! {
                biased;
                event = self.events.next(), if self.events_open => event,
                output = &mut future => return output,
            };
            self.hear(event);
        }
    }

    /// Like `hear_during`, but gives up on `future` once the slot is heard stopped.
    async fn hear_until_stopped<F: Future>(&mut self, future: F) -> Option<F::Output> {
        let mut future = pin!(future);
        while !self.stopped {
            let event = tokio::select! {
                biased;
                event = self.events.next(), if self.events_open => event,
                output = &mut future => return Some(output),
            };
            self.hear(event);
        }
        None
    }

    fn hear_queued(&mut self) {
        while let Some(event) = self.events.try_next() {
            self.hear(Some(event));
        }
    }

    /// The start returned: from now on states are measured against its number.
    fn started(&mut self, start: &StartReport) {
        self.run_seq = Some(start.run_seq);
        match start.outcome {
            StartOutcome::NotSeenRunning => self.stop(),
            // The reply to the start was lost; the brick's state says whether anything runs.
            StartOutcome::Uncertain => self.check(false),
            StartOutcome::Started => self.judge(),
        }
    }

    /// Waits for the slot to stop. Fails when the port ends the subscription first, since
    /// the end of the run could then never be heard.
    async fn until_slot_stopped(&mut self, port: &impl BrickPort) -> Result<(), BrickError> {
        while !self.stopped {
            if let Some(any_stop) = self.checks.pop_front() {
                // A failed check tells nothing; the events still may.
                if let Some(Ok(state)) = self.hear_until_stopped(port.program_state()).await {
                    if any_stop && !state.running {
                        self.stop();
                    } else {
                        self.note(state);
                    }
                }
                continue;
            }
            if !self.events_open {
                return Err(BrickError::new(
                    None,
                    "The brick's program state is no longer reported, so the end of the run cannot be seen.",
                ));
            }
            let event = self.events.next().await;
            self.hear(event);
        }
        Ok(())
    }

    fn past_deadline(&self, time: SystemTime) -> bool {
        self.deadline.is_some_and(|deadline| time > deadline)
    }

    fn hear(&mut self, event: Option<BrickEvent>) {
        match event {
            None => self.events_open = false,
            Some(BrickEvent::Program(state)) => self.note(state),
            Some(BrickEvent::Reconnected) if self.stopped => {
                // The brick is back while polling: give the record the full wait again.
                self.deadline = Some(self.clock.now() + RESULT_WAIT);
            }
            Some(BrickEvent::Reconnected) => {
                // The end may have been missed while disconnected. Cockpit keeps its start
                // numbers across a daemon replacement, but starts again from 0 when its
                // record of them is damaged, so after a reconnect a stopped slot ends the
                // run whatever its number.
                self.check(true);
            }
        }
    }

    /// Asks the brick's state once the start's number is known.
    fn check(&mut self, any_stop: bool) {
        if self.run_seq.is_some() {
            self.checks.push_back(any_stop);
        }
    }

    fn note(&mut self, state: ProgramState) {
        // A stopped slot ends the start it is told with; any later start ends this one too.
        let ended = if state.running {
            state.run_seq.saturating_sub(1)
        } else {
            state.run_seq
        };
        self.ended_seq = self.ended_seq.max(ended);
        self.judge();
    }

    fn judge(&mut self) {
        if self
            .run_seq
            .is_some_and(|run_seq| self.ended_seq >= run_seq)
        {
            self.stop();
        }
    }

    fn stop(&mut self) {
        if !self.stopped {
            self.stopped = true;
            self.deadline = Some(self.clock.now() + RESULT_WAIT);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::brick::fake::{FakeBrick, FakeBrickPort, TestClock};
    use crate::brick::live_output::{LiveEvent, OutputStream};
    use crate::build::ArtifactKind;
    use crate::port::ProgramState;
    use crate::testutil::temp_dir;

    const PROJECT: &str = "/home/root/lms2012/prjs/robot";
    const RUNS: &str = "/home/root/lms2012/prjs/robot/runs";
    const REQ: &str = "/home/root/lms2012/prjs/robot/.req";
    const RBF: &str = "/home/root/lms2012/prjs/robot/robot.rbf";

    /// Long after a wait that started too early would have given up.
    const LATER: Duration = Duration::from_secs(20);

    fn res(run: i64, launch: Option<&str>) -> String {
        let launch = launch.map_or_else(String::new, |launch| format!("launch={launch}\n"));
        format!("run={run}\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n{launch}")
    }

    fn artifacts() -> Vec<BuildArtifact> {
        let dir = temp_dir("ev3c-run-");
        let make = |kind, name: &str, content: &str| {
            let local_path = dir.join(name);
            std::fs::write(&local_path, content).unwrap();
            BuildArtifact {
                kind,
                local_path,
                brick_path: format!("{PROJECT}/{name}"),
            }
        };
        vec![
            make(ArtifactKind::Program, "prog", "program"),
            make(ArtifactKind::Launcher, "robot.rbf", "launcher"),
            make(ArtifactKind::Supervisor, "sup", "supervisor"),
        ]
    }

    struct Setup {
        port: FakeBrickPort,
        clock: TestClock,
        launches: Launches,
        artifacts: Vec<BuildArtifact>,
    }

    impl Setup {
        async fn run(&self) -> Result<RunOutcome, BrickError> {
            let run = run_project(
                &self.port,
                &self.port,
                &self.clock,
                RunProjectOptions {
                    project: "robot",
                    artifacts: &self.artifacts,
                    launches: &self.launches,
                    new_launch_id: None,
                    on_started: None,
                    on_output: None,
                },
            );
            // Paused time jumps over the hour at once, so a run that never ends fails the
            // test instead of hanging it.
            tokio::time::timeout(Duration::from_secs(3600), run)
                .await
                .expect("the run never ended")
        }

        /// A run whose output `on_output` hears.
        async fn run_live(&self, on_output: &OnOutput<'_>) -> Result<RunOutcome, BrickError> {
            let run = run_project(
                &self.port,
                &self.port,
                &self.clock,
                RunProjectOptions {
                    project: "robot",
                    artifacts: &self.artifacts,
                    launches: &self.launches,
                    new_launch_id: None,
                    on_started: None,
                    on_output: Some(on_output),
                },
            );
            tokio::time::timeout(Duration::from_secs(3600), run)
                .await
                .expect("the run never ended")
        }

        /// Scripts what happens on the brick once the program "runs".
        fn on_run(&self, mut hook: impl FnMut(&mut FakeBrick) + Send + 'static) {
            self.port
                .with(|brick| brick.on_run = Some(Box::new(move |brick| hook(brick))));
        }

        fn calls(&self) -> Vec<String> {
            self.port.calls()
        }
    }

    fn setup() -> Setup {
        let port = FakeBrickPort::new();
        port.with(|brick| {
            brick.put_file(&format!("{PROJECT}/sup"), "supervisor");
            brick.put_file(&format!("{RUNS}/next"), "2");
            brick.put_file(&format!("{RUNS}/1/res"), res(1, Some("older")));
        });
        Setup {
            clock: port.clock(),
            port,
            launches: Launches::new(),
            artifacts: artifacts(),
        }
    }

    /// What the supervisor does with .req at start: takes the launch ID and deletes the file.
    fn take_launch_id(brick: &mut FakeBrick) -> Option<String> {
        let text = brick.text(REQ);
        brick.files.remove(REQ);
        text.and_then(|text| text.split('\n').next().map(str::to_owned))
    }

    fn put_record(brick: &mut FakeBrick, run: i64, launch: Option<&str>) {
        brick.put_file(&format!("{RUNS}/{run}/res"), res(run, launch));
    }

    fn result(outcome: RunOutcome) -> (RunResult, StartReport) {
        match outcome {
            RunOutcome::Result { result, start } => (result, start),
            RunOutcome::NoResult { .. } => panic!("expected a result, got {outcome:?}"),
        }
    }

    #[derive(Default)]
    struct Seen {
        run_at: Option<SystemTime>,
        request: Option<String>,
        launch_id: Option<String>,
    }

    /// What a supervisor does at start, with the launch file: takes the ID, makes run 2 with
    /// empty outputs.
    fn start_run_2(brick: &mut FakeBrick) -> Option<String> {
        let launch_id = take_launch_id(brick);
        brick.put_file(
            &format!("{RUNS}/2/launch"),
            format!("{}\n", launch_id.as_deref().unwrap_or_default()),
        );
        brick.put_file(&format!("{RUNS}/2/out"), "");
        brick.put_file(&format!("{RUNS}/2/err"), "");
        launch_id
    }

    /// The events heard, and each stream's text joined.
    fn heard(events: &Mutex<Vec<LiveEvent>>) -> (Vec<LiveEvent>, String, String) {
        let events = events.lock().unwrap().clone();
        let joined = |wanted: OutputStream| -> String {
            events
                .iter()
                .filter_map(|event| match event {
                    LiveEvent::Output { stream, text } if *stream == wanted => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        };
        let (out, err) = (joined(OutputStream::Out), joined(OutputStream::Err));
        (events, out, err)
    }

    #[tokio::test(start_paused = true)]
    async fn a_live_run_hears_its_output_as_it_grows_once_and_in_whole_characters() {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let id = start_run_2(brick);
            let out = format!("{RUNS}/2/out");
            let err = format!("{RUNS}/2/err");
            let (o1, o2, o3, o4, e1) = (out.clone(), out.clone(), out.clone(), out, err);
            handle.later(Duration::from_secs(1), move |b| b.append(&o1, "line 1\n"));
            // "ž" is C5 BE, split between two growths.
            handle.later(Duration::from_secs(2), move |b| {
                b.append(&o2, b"\xC5");
                b.append(&e1, "warn\n");
            });
            handle.later(Duration::from_secs(3), move |b| b.append(&o3, b"\xBE\n"));
            // The last line is written as the program ends: the follow may not deliver it.
            handle.later(Duration::from_secs(4), move |b| {
                b.files.get_mut(&o4).unwrap().extend_from_slice(b"last\n");
                put_record(b, 2, id.as_deref());
                b.stop();
            });
        });
        let events = Mutex::new(Vec::new());
        let on_output = |event: LiveEvent| events.lock().unwrap().push(event);

        let (result, _) = result(setup.run_live(&on_output).await.unwrap());

        let (events, out, err) = heard(&events);
        assert_eq!(
            out, "line 1\nž\nlast\n",
            "every byte once, in order: {events:?}"
        );
        assert_eq!(err, "warn\n");
        assert!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    LiveEvent::Output {
                        stream: OutputStream::Out,
                        ..
                    }
                ))
                .count()
                >= 3,
            "the output came in pieces while it grew: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, LiveEvent::Unavailable(_)))
        );
        assert_eq!(result.out.as_deref(), Some("line 1\nž\nlast\n"));
        assert!(setup.calls().contains(&format!("follow_file {RUNS}/2/out")));
        assert_eq!(
            setup.port.with(|brick| brick.follow_count()),
            0,
            "the follows end with the run"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn without_a_free_follow_the_output_comes_whole_at_the_end_and_is_said_once() {
        let setup = setup();
        setup.port.with(|brick| brick.max_follows = 0);
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let id = start_run_2(brick);
            brick.put_file(&format!("{RUNS}/2/out"), "hello\n");
            brick.put_file(&format!("{RUNS}/2/err"), "oops\n");
            handle.later(Duration::from_secs(2), move |b| {
                put_record(b, 2, id.as_deref());
                b.stop();
            });
        });
        let events = Mutex::new(Vec::new());
        let on_output = |event: LiveEvent| events.lock().unwrap().push(event);

        result(setup.run_live(&on_output).await.unwrap());

        let (events, out, err) = heard(&events);
        assert_eq!((out.as_str(), err.as_str()), ("hello\n", "oops\n"));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, LiveEvent::Unavailable(_)))
                .count(),
            1,
            "{events:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_nobody_hears_live_follows_nothing() {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let id = start_run_2(brick);
            handle.later(Duration::from_secs(1), move |b| {
                put_record(b, 2, id.as_deref());
                b.stop();
            });
        });
        result(setup.run().await.unwrap());
        assert!(
            !setup
                .calls()
                .iter()
                .any(|call| call.starts_with("follow_file")),
            "{:?}",
            setup.calls()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_live_run_stopped_from_outside_still_finds_its_result() {
        let setup = setup();
        setup.on_run(|brick| {
            let id = start_run_2(brick);
            put_record(brick, 2, id.as_deref());
        });
        let on_output = |_: LiveEvent| {};
        let port = setup.port.clone();
        let stopper = async {
            while !port.with(|brick| brick.running) {
                tokio::task::yield_now().await;
            }
            port.stop_program().await.unwrap();
        };
        let (outcome, ()) = tokio::join!(setup.run_live(&on_output), stopper);
        let (result, _) = result(outcome.unwrap());
        assert_eq!(result.run, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_live_run_given_up_lets_its_follows_go() {
        let setup = setup();
        setup.on_run(|brick| {
            start_run_2(brick);
        });
        let on_output = |_: LiveEvent| {};
        let run = setup.run_live(&on_output);
        // The program never ends; the caller gives up after a while, as Ctrl+C twice does.
        assert!(
            tokio::time::timeout(Duration::from_secs(5), run)
                .await
                .is_err()
        );
        assert!(setup.calls().contains(&format!("follow_file {RUNS}/2/err")));
        assert_eq!(setup.port.with(|brick| brick.follow_count()), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_uploads_the_changed_files_and_the_launch_request_with_the_start_and_a_stop_yields_the_result()
     {
        let setup = setup();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (clock, handle, hook_seen) = (setup.clock, setup.port.handle(), seen.clone());
        setup.on_run(move |brick| {
            let request = brick.text(REQ);
            let launch_id = take_launch_id(brick);
            brick.put_file(&format!("{RUNS}/2/out"), "hello\n");
            brick.put_file(&format!("{RUNS}/2/err"), "");
            let id = launch_id.clone();
            handle.later(Duration::from_millis(500), move |brick| {
                put_record(brick, 2, id.as_deref());
            });
            handle.later(Duration::ZERO, FakeBrick::stop);
            *hook_seen.lock().unwrap() = Seen {
                run_at: Some(clock.now()),
                request,
                launch_id,
            };
        });

        let (result, start) = result(setup.run().await.unwrap());

        let seen = seen.lock().unwrap();
        let run_at = seen.run_at.unwrap();
        assert_eq!(result.run, 2);
        assert_eq!(result.out.as_deref(), Some("hello\n"));
        assert_eq!(result.err.as_deref(), Some(""));
        assert_eq!(result.started_at, Some(run_at));
        assert_eq!(
            setup.launches.get(seen.launch_id.as_deref().unwrap()),
            result.started_at
        );
        let request = seen.request.as_deref().unwrap();
        assert!(
            request.len() == 17
                && request.ends_with('\n')
                && request[..16]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "a fresh launch ID on one line: {request:?}"
        );
        assert_eq!(
            start.uploaded,
            [format!("{PROJECT}/prog"), RBF.into(), REQ.into()]
        );
        assert_eq!(
            start.skipped,
            [format!("{PROJECT}/sup")],
            "an unchanged supervisor is left"
        );
        let calls = setup.calls();
        let at = |call: &str| calls.iter().position(|made| made == call).unwrap();
        assert!(
            at("subscribe") < at(&format!("deploy_and_run {RBF}")),
            "subscribed before the start"
        );
        let first_read = setup.port.with(|brick| brick.read_times[0]);
        assert!(
            first_read >= run_at + Duration::from_secs(1),
            "the record is read on the second poll"
        );
        assert_eq!(
            setup.port.with(|brick| brick.listener_count()),
            0,
            "the state subscription ends with the wait"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_stopped_state_from_before_the_start_does_not_start_the_result_wait() {
        let setup = setup();
        setup.port.with(|brick| {
            brick.replay_on_subscribe = Some(BrickEvent::Program(ProgramState {
                running: false,
                run_seq: 0,
            }));
        });
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let launch_id = take_launch_id(brick);
            brick.put_file(&format!("{RUNS}/2/out"), "out");
            handle.later(LATER, move |brick| {
                put_record(brick, 2, launch_id.as_deref());
                brick.stop();
            });
        });

        result(setup.run().await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn a_program_that_ends_before_the_start_returns_is_not_missed() {
        let setup = setup();
        setup.on_run(|brick| {
            let launch_id = take_launch_id(brick);
            put_record(brick, 2, launch_id.as_deref());
            // Heard while deploy_and_run is still returning: its number is not known yet.
            brick.stop();
        });

        let (result, _) = result(setup.run().await.unwrap());

        assert_eq!(result.run, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_later_start_by_someone_else_ends_the_wait_for_this_run() {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let launch_id = take_launch_id(brick);
            put_record(brick, 2, launch_id.as_deref());
            handle.later(Duration::ZERO, |brick| {
                let run_seq = brick.run_seq + 1;
                brick.emit(BrickEvent::Program(ProgramState {
                    running: true,
                    run_seq,
                }));
            });
        });

        result(setup.run().await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn a_newer_run_folder_from_a_brick_menu_start_without_a_launch_id_is_not_taken_as_the_authors_result()
     {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let launch_id = take_launch_id(brick);
            put_record(brick, 2, launch_id.as_deref());
            put_record(brick, 3, None);
            handle.later(Duration::ZERO, FakeBrick::stop);
        });

        let (result, _) = result(setup.run().await.unwrap());

        assert_eq!(result.run, 2);
        assert!(
            setup.calls().contains(&format!("read_file {RUNS}/3/res")),
            "the newer record was checked"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_record_that_never_appears_within_10_s_yields_no_result_and_the_run_stays_listed() {
        let setup = setup();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (clock, handle, hook_seen) = (setup.clock, setup.port.handle(), seen.clone());
        setup.on_run(move |brick| {
            hook_seen.lock().unwrap().launch_id = take_launch_id(brick);
            brick.put_file(&format!("{RUNS}/2/out"), "partial");
            let hook_seen = hook_seen.clone();
            handle.later(Duration::ZERO, move |brick| {
                hook_seen.lock().unwrap().run_at = Some(clock.now());
                brick.stop();
            });
        });

        let outcome = setup.run().await.unwrap();

        let RunOutcome::NoResult {
            launch_id,
            started_at,
            ..
        } = outcome
        else {
            panic!("expected no result, got {outcome:?}");
        };
        let (taken, stopped_at) = {
            let seen = seen.lock().unwrap();
            (seen.launch_id.clone(), seen.run_at.unwrap())
        };
        assert_eq!(Some(&launch_id), taken.as_ref());
        assert_eq!(setup.launches.get(&launch_id), Some(started_at));
        let waited = setup.clock.now().duration_since(stopped_at).unwrap();
        assert!(
            waited >= RESULT_WAIT && waited <= RESULT_WAIT + POLL_INTERVAL,
            "waited {waited:?}"
        );
        assert_eq!(list_run_numbers(&setup.port, "robot").await, Ok(vec![1, 2]));
    }

    #[tokio::test(start_paused = true)]
    async fn a_start_not_seen_running_starts_the_result_wait_at_once() {
        let setup = setup();
        setup
            .port
            .with(|brick| brick.start_outcome = StartOutcome::NotSeenRunning);
        setup.on_run(|brick| {
            let launch_id = take_launch_id(brick);
            put_record(brick, 2, launch_id.as_deref());
        });

        let (result, _) = result(setup.run().await.unwrap());

        assert_eq!(result.run, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn an_uncertain_start_reads_the_state_and_is_never_repeated() {
        let setup = setup();
        setup
            .port
            .with(|brick| brick.start_outcome = StartOutcome::Uncertain);
        setup.on_run(|brick| {
            let launch_id = take_launch_id(brick);
            put_record(brick, 2, launch_id.as_deref());
            brick.running = false;
        });

        result(setup.run().await.unwrap());

        let calls = setup.calls();
        assert!(calls.contains(&"program_state".to_owned()));
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with("deploy_and_run"))
                .count(),
            1
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_reconnect_during_the_wait_resumes_polling_instead_of_failing() {
        let setup = setup();
        let reconnect_at = Arc::new(Mutex::new(None));
        let (clock, handle, hook_reconnect_at) =
            (setup.clock, setup.port.handle(), reconnect_at.clone());
        setup.on_run(move |brick| {
            *hook_reconnect_at.lock().unwrap() = Some(clock.now() + Duration::from_secs(9));
            let launch_id = take_launch_id(brick);
            brick.put_file(&format!("{RUNS}/2/out"), "out");
            handle.later(Duration::from_millis(500), |brick| {
                brick.disconnected = true
            });
            handle.later(Duration::from_secs(9), |brick| {
                brick.disconnected = false;
                brick.emit(BrickEvent::Reconnected);
            });
            // After the first 10 s: only the wait restarted by the reconnect finds the record.
            handle.later(Duration::from_secs(12), move |brick| {
                put_record(brick, 2, launch_id.as_deref());
            });
            handle.later(Duration::ZERO, FakeBrick::stop);
        });

        let (result, _) = result(setup.run().await.unwrap());

        assert_eq!(result.run, 2);
        assert!(
            setup.calls().contains(&format!("list {RUNS}/2")),
            "polled the run folder"
        );
        let first_read = setup.port.with(|brick| brick.read_times[0]);
        assert!(
            first_read >= reconnect_at.lock().unwrap().unwrap(),
            "the record is read after the reconnect"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_reconnect_while_the_program_still_runs_keeps_waiting_for_it_to_stop() {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let launch_id = take_launch_id(brick);
            brick.put_file(&format!("{RUNS}/2/out"), "out");
            handle.later(Duration::ZERO, |brick| brick.emit(BrickEvent::Reconnected));
            handle.later(LATER, move |brick| {
                put_record(brick, 2, launch_id.as_deref());
                brick.stop();
            });
        });

        result(setup.run().await.unwrap());

        assert!(setup.calls().contains(&"program_state".to_owned()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_reconnect_after_the_program_ended_while_disconnected_starts_the_result_wait() {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let launch_id = take_launch_id(brick);
            put_record(brick, 2, launch_id.as_deref());
            handle.later(Duration::ZERO, |brick| {
                brick.running = false;
                brick.emit(BrickEvent::Reconnected);
            });
        });

        let (result, _) = result(setup.run().await.unwrap());

        assert_eq!(result.run, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_daemon_replaced_during_the_run_which_counts_starts_from_0_again_still_ends_the_wait()
    {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let launch_id = take_launch_id(brick);
            put_record(brick, 2, launch_id.as_deref());
            handle.later(Duration::ZERO, |brick| {
                // The new daemon knows no earlier start; the renewed subscription tells its
                // state.
                brick.run_seq = 0;
                brick.running = false;
                brick.emit_state();
                brick.emit(BrickEvent::Reconnected);
            });
        });

        let (result, _) = result(setup.run().await.unwrap());

        assert_eq!(result.run, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_reconnect_heard_before_the_start_returns_asks_nothing() {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let launch_id = take_launch_id(brick);
            put_record(brick, 2, launch_id.as_deref());
            brick.emit(BrickEvent::Reconnected);
            handle.later(Duration::ZERO, FakeBrick::stop);
        });

        result(setup.run().await.unwrap());

        assert!(!setup.calls().contains(&"program_state".to_owned()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscription_the_port_ends_before_the_slot_stops_fails_the_run() {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            take_launch_id(brick);
            handle.later(Duration::ZERO, FakeBrick::end_subscriptions);
        });

        let error = setup.run().await.unwrap_err();

        assert_eq!(error.code(), None);
        assert!(error.to_string().contains("no longer reported"), "{error}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_refused_because_a_program_runs_uploads_nothing_and_records_no_start() {
        let setup = setup();
        setup.port.with(|brick| brick.running = true);

        let error = setup.run().await.unwrap_err();

        assert_eq!(error.code(), Some("program_running"));
        setup.port.with(|brick| {
            assert!(!brick.has(REQ));
            assert!(!brick.has(&format!("{PROJECT}/prog")));
            assert_eq!(brick.listener_count(), 0);
        });
        assert!(setup.launches.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_a_stop_cancels_after_the_upload_removes_the_launch_request() {
        let setup = setup();
        setup
            .port
            .with(|brick| brick.start_error = Some("cancelled".into()));

        let error = setup.run().await.unwrap_err();

        assert_eq!(error.code(), Some("cancelled"));
        assert!(!setup.port.with(|brick| brick.has(REQ)));
        assert!(setup.launches.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_whose_outcome_is_unknown_keeps_the_launch_request_for_the_supervisor() {
        let setup = setup();
        setup
            .port
            .with(|brick| brick.start_error = Some("outcome_unknown".into()));

        let error = setup.run().await.unwrap_err();

        assert_eq!(error.code(), Some("outcome_unknown"));
        assert!(setup.port.with(|brick| brick.has(REQ)));
        assert_eq!(setup.launches.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn stop_and_emergency_stop_are_forwarded_unchanged() {
        let setup = setup();
        setup.port.with(|brick| brick.running = true);
        let stopped = ProgramState {
            running: false,
            run_seq: 0,
        };

        assert_eq!(stop_program(&setup.port).await, Ok(stopped));
        assert_eq!(emergency_stop(&setup.port).await, Ok(stopped));
        assert_eq!(setup.calls(), ["stop_program", "emergency_stop"]);
    }

    #[tokio::test(start_paused = true)]
    async fn an_injected_launch_id_is_the_one_written_and_recorded() {
        let setup = setup();
        let handle = setup.port.handle();
        setup.on_run(move |brick| {
            let launch_id = take_launch_id(brick);
            put_record(brick, 2, launch_id.as_deref());
            handle.later(Duration::ZERO, FakeBrick::stop);
        });
        let make = || "fixed-id".to_owned();
        let heard = Mutex::new(Vec::new());
        let on_started = |start: &StartReport| heard.lock().unwrap().push(start.clone());

        let outcome = run_project(
            &setup.port,
            &setup.port,
            &setup.clock,
            RunProjectOptions {
                project: "robot",
                artifacts: &setup.artifacts,
                launches: &setup.launches,
                new_launch_id: Some(&make),
                on_started: Some(&on_started),
                on_output: None,
            },
        )
        .await;

        let (result, start) = result(outcome.unwrap());
        assert_eq!(result.record.launch.as_deref(), Some("fixed-id"));
        assert!(setup.launches.get("fixed-id").is_some());
        assert_eq!(
            *heard.lock().unwrap(),
            [start],
            "the start is told once, as returned"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_start_is_not_told_as_started() {
        let setup = setup();
        setup.port.with(|brick| brick.running = true);
        let told = std::sync::atomic::AtomicBool::new(false);
        let on_started = |_: &StartReport| told.store(true, std::sync::atomic::Ordering::Relaxed);
        let outcome = run_project(
            &setup.port,
            &setup.port,
            &setup.clock,
            RunProjectOptions {
                project: "robot",
                artifacts: &setup.artifacts,
                launches: &setup.launches,
                new_launch_id: None,
                on_started: Some(&on_started),
                on_output: None,
            },
        )
        .await;
        assert_eq!(outcome.unwrap_err().code(), Some("program_running"));
        assert!(!told.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[tokio::test]
    async fn every_brick_operation_can_be_spawned_on_a_multi_threaded_runtime() {
        use crate::brick::deploy::deploy;
        use crate::brick::returned_files::{download_file, download_returned_files};
        use crate::brick::run_folders::delete_runs;
        use crate::brick::run_results::{list_runs, read_run_result};

        fn assert_send<T: Send>(_: &T) {}
        let port = FakeBrickPort::new();
        let launches = Launches::new();
        let clock = port.clock();
        let local = std::path::Path::new("returned");
        assert_send(&deploy(&port, &[]));
        assert_send(&delete_runs(&port, "robot", 0));
        assert_send(&list_runs(&port, &port, "robot", &launches));
        assert_send(&read_run_result(&port, &port, "robot", 1, &launches));
        assert_send(&download_returned_files(&port, "robot", &[], local));
        assert_send(&download_file(&port, "/a", local));
        assert_send(&stop_program(&port));
        assert_send(&emergency_stop(&port));
        let run = run_project(
            &port,
            &port,
            &clock,
            RunProjectOptions {
                project: "robot",
                artifacts: &[],
                launches: &launches,
                new_launch_id: None,
                on_started: None,
                on_output: None,
            },
        );
        assert_send(&run);
    }
}
