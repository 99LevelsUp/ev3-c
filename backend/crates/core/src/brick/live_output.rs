//! A run's stdout and stderr while the program runs, through the cockpit's file follow.
//!
//! Once the run has started, EVƎ C finds the run's folder by the launch ID the supervisor
//! writes to `runs/<n>/launch` before the program starts, then follows `out` and `err` as
//! they grow. Every byte reaches the front end once, as text cut at whole UTF-8 characters:
//! live while the program runs, and at the end whatever the follow had not delivered, taken
//! from the files read with the result. Live output is a bonus: when the brick has no follow
//! free (cockpit allows two per brick, every client together) or the folder is not found, the
//! front end hears so once and gets the output at the end, as before.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::brick::run_folders::{list_run_numbers, run_dir};
use crate::brick::run_results::{ERR_FILE, OUT_FILE, RawOutput};
use crate::port::{BrickFileReader, BrickPort, Clock, FollowEvent};

/// The file in a run folder that names the run's launch, written by the supervisor.
pub const LAUNCH_FILE: &str = "launch";

const FIND_INTERVAL: Duration = Duration::from_millis(250);
/// How long EVƎ C looks for the run's folder; the supervisor makes it within a second.
const FIND_LIMIT: Duration = Duration::from_secs(10);
/// How long a missing `out` or `err` is asked for again, while the supervisor creates it.
const CREATE_WAIT: Duration = Duration::from_secs(3);
const CREATE_INTERVAL: Duration = Duration::from_millis(200);
/// Cockpit refuses brick paths of 60 bytes or more.
const PATH_LIMIT: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Out,
    Err,
}

impl OutputStream {
    fn index(self) -> usize {
        match self {
            OutputStream::Out => 0,
            OutputStream::Err => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    /// The next piece of one of the program's outputs.
    Output { stream: OutputStream, text: String },
    /// The output cannot be followed while the program runs, and why; it comes whole at the
    /// end instead. Told at most once per run.
    Unavailable(String),
}

/// Hears a run's output while the program runs.
pub type OnOutput<'a> = dyn Fn(LiveEvent) + Sync + 'a;

/// One run's live output: what was delivered of each stream, and its unfinished characters.
pub(crate) struct LiveOutput<'a> {
    on_output: &'a OnOutput<'a>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Bytes of each stream taken in so far.
    received: [u64; 2],
    /// The start of a character whose other bytes have not come yet.
    pending: [Vec<u8>; 2],
    told_unavailable: bool,
}

impl<'a> LiveOutput<'a> {
    pub(crate) fn new(on_output: &'a OnOutput<'a>) -> Self {
        LiveOutput {
            on_output,
            state: Mutex::new(State::default()),
        }
    }

    /// Finds the run's folder and follows both outputs until their follows end. It never
    /// returns early on its own when nothing goes wrong; the caller drops it once the run has
    /// ended and its result is found, which cancels the follows.
    pub(crate) async fn follow(
        &self,
        port: &impl BrickPort,
        reader: &impl BrickFileReader,
        clock: &impl Clock,
        project: &str,
        newest_before: i64,
        launch_id: &str,
    ) {
        let Some(run) = find_run(port, reader, clock, project, newest_before, launch_id).await
        else {
            self.tell_unavailable("the run's folder was not found on the brick in time".into());
            return;
        };
        let dir = run_dir(project, run);
        let out = format!("{dir}/{OUT_FILE}");
        let err = format!("{dir}/{ERR_FILE}");
        if out.len().max(err.len()) >= PATH_LIMIT {
            self.tell_unavailable(format!(
                "{out} is {} bytes long, and the cockpit follows paths shorter than {PATH_LIMIT}",
                out.len()
            ));
            return;
        }
        tokio::join!(
            self.follow_stream(port, clock, OutputStream::Out, &out),
            self.follow_stream(port, clock, OutputStream::Err, &err),
        );
    }

    async fn follow_stream(
        &self,
        port: &impl BrickPort,
        clock: &impl Clock,
        stream: OutputStream,
        path: &str,
    ) {
        let give_up = clock.now() + CREATE_WAIT;
        let mut follow = loop {
            match port.follow_file(path).await {
                Ok(follow) => break follow,
                Err(error) if error.code() == Some("file_not_found") && clock.now() < give_up => {
                    clock.sleep(CREATE_INTERVAL).await;
                }
                Err(error) => {
                    self.tell_unavailable(format!("{path} cannot be followed: {error}"));
                    return;
                }
            }
        };
        while let Some(event) = follow.next().await {
            match event {
                FollowEvent::Appended { offset, data } => self.take(stream, offset, &data),
                FollowEvent::Ended(_) => break,
            }
        }
    }

    /// Takes in bytes from `offset` on. The follow delivers them in order and without gaps;
    /// bytes already taken in are skipped.
    fn take(&self, stream: OutputStream, offset: u64, data: &[u8]) {
        let mut state = self.lock();
        let index = stream.index();
        let end = offset + data.len() as u64;
        let received = state.received[index];
        if end <= received {
            return;
        }
        let skip = usize::try_from(received.saturating_sub(offset)).unwrap_or(data.len());
        state.received[index] = end;
        state.pending[index].extend_from_slice(&data[skip..]);
        let text = whole_characters(&mut state.pending[index], false);
        self.tell(stream, text);
    }

    /// The run has ended: delivers what the follows had not, from the output files read with
    /// the result, and the last unfinished characters. Call it after the follow is dropped, so
    /// nothing arrives after it.
    pub(crate) fn finish(&self, raw: Option<&RawOutput>) {
        for stream in [OutputStream::Out, OutputStream::Err] {
            let bytes = raw.and_then(|raw| match stream {
                OutputStream::Out => raw.out.as_deref(),
                OutputStream::Err => raw.err.as_deref(),
            });
            let mut state = self.lock();
            let index = stream.index();
            if let Some(bytes) = bytes {
                let from = usize::try_from(state.received[index])
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                state.pending[index].extend_from_slice(&bytes[from..]);
                state.received[index] = bytes.len() as u64;
            }
            let text = whole_characters(&mut state.pending[index], true);
            self.tell(stream, text);
        }
    }

    fn tell(&self, stream: OutputStream, text: String) {
        if !text.is_empty() {
            (self.on_output)(LiveEvent::Output { stream, text });
        }
    }

    fn tell_unavailable(&self, why: String) {
        let mut state = self.lock();
        if !state.told_unavailable {
            state.told_unavailable = true;
            (self.on_output)(LiveEvent::Unavailable(why));
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // Every change under the lock is complete before the next can see it.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The run folder newer than `newest_before` whose launch file names `launch_id`, looked for
/// every 250 ms for up to 10 s.
async fn find_run(
    port: &impl BrickPort,
    reader: &impl BrickFileReader,
    clock: &impl Clock,
    project: &str,
    newest_before: i64,
    launch_id: &str,
) -> Option<i64> {
    let give_up = clock.now() + FIND_LIMIT;
    // Folders whose launch file names another launch; their files do not change.
    let mut others = HashSet::new();
    loop {
        if let Ok(runs) = list_run_numbers(port, project).await {
            for run in runs.into_iter().rev().filter(|run| *run > newest_before) {
                if others.contains(&run) {
                    continue;
                }
                let path = format!("{}/{LAUNCH_FILE}", run_dir(project, run));
                // Missing or unreadable: not written yet, so asked again next time.
                if let Ok(data) = reader.read_file(&path).await {
                    if String::from_utf8_lossy(&data).trim() == launch_id {
                        return Some(run);
                    }
                    others.insert(run);
                }
            }
        }
        if clock.now() >= give_up {
            return None;
        }
        clock.sleep(FIND_INTERVAL).await;
    }
}

/// Takes the whole UTF-8 characters off the front of `pending` as text, leaving the start of
/// a character whose other bytes have not come yet. Invalid bytes become U+FFFD, and so does
/// an unfinished character at the `end`.
fn whole_characters(pending: &mut Vec<u8>, end: bool) -> String {
    let mut text = String::new();
    let mut rest: &[u8] = pending;
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                text.push_str(valid);
                rest = &[];
                break;
            }
            Err(error) => {
                let (valid, after) = rest.split_at(error.valid_up_to());
                // Checked just now by from_utf8.
                text.push_str(std::str::from_utf8(valid).unwrap_or_default());
                match error.error_len() {
                    Some(bad) => {
                        text.push(char::REPLACEMENT_CHARACTER);
                        rest = &after[bad..];
                    }
                    None if end => {
                        text.push(char::REPLACEMENT_CHARACTER);
                        rest = &[];
                        break;
                    }
                    None => {
                        rest = after;
                        break;
                    }
                }
            }
        }
    }
    *pending = rest.to_vec();
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_characters_keep_a_split_character_until_its_last_byte_comes() {
        // "ž" is C5 BE.
        let mut pending = b"a\xC5".to_vec();
        assert_eq!(whole_characters(&mut pending, false), "a");
        assert_eq!(pending, b"\xC5");
        pending.extend_from_slice(b"\xBEb");
        assert_eq!(whole_characters(&mut pending, false), "žb");
        assert!(pending.is_empty());
    }

    #[test]
    fn whole_characters_replace_invalid_bytes_and_an_unfinished_character_at_the_end() {
        let mut pending = b"x\xFFy\xE2\x82".to_vec();
        assert_eq!(whole_characters(&mut pending, false), "x\u{FFFD}y");
        assert_eq!(whole_characters(&mut pending, true), "\u{FFFD}");
        assert!(pending.is_empty());
    }
}
