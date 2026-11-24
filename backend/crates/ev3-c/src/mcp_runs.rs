//! Runs an agent starts and reads without waiting for their end: `start_project_run`,
//! `read_project_run` and `stop_project_run` of `ev3 c mcp`.
//!
//! Each run is `ev3 c run --events` going on in the background on the server's thread. Its
//! events fill a [`LiveRun`]: the program's stdout and stderr as they come, the start, what EVƎ C
//! said along the way, and the end. A read takes the output from offsets the previous read
//! gave, so nothing is repeated, and may wait a while for something new.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Map, Value};
use tokio::sync::{Notify, mpsc};

use crate::cli::Io;

/// The most text of one stream a read returns; the rest comes with the next read.
pub const READ_LIMIT: usize = 32 * 1024;

/// Finished runs kept for reading, besides the runs still going.
pub const KEEP_ENDED: usize = 8;

/// One background run.
pub struct LiveRun {
    pub out: String,
    pub err: String,
    /// What EVƎ C said along the way: cockpit notices, the output coming at the end only, the
    /// stop.
    pub messages: Vec<String>,
    /// The `started` event, without its name.
    pub started: Option<Map<String, Value>>,
    /// The terminal object `--json` would print, once the command has ended.
    pub ended: Option<Map<String, Value>>,
    /// Ctrl+C for this run; kept here so the run can always hear it.
    pub stop: mpsc::UnboundedSender<()>,
    /// Woken on every change.
    pub changed: Rc<Notify>,
}

impl LiveRun {
    pub fn new(stop: mpsc::UnboundedSender<()>) -> Self {
        LiveRun {
            out: String::new(),
            err: String::new(),
            messages: Vec::new(),
            started: None,
            ended: None,
            stop,
            changed: Rc::new(Notify::new()),
        }
    }

    pub fn running(&self) -> bool {
        self.ended.is_none()
    }
}

/// The [`Io`] of a background run: takes its event lines into the [`LiveRun`].
pub struct RunSink(pub Rc<RefCell<LiveRun>>);

impl Io for RunSink {
    fn out(&mut self, text: &str) {
        let Ok(Value::Object(mut event)) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let name = event
            .remove("event")
            .and_then(|name| name.as_str().map(str::to_owned))
            .unwrap_or_default();
        let mut run = self.0.borrow_mut();
        if event.contains_key("exitCode") {
            run.ended = Some(event);
        } else {
            let text = |key: &str| {
                event
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            match name.as_str() {
                "output" if text("stream") == "err" => run.err.push_str(&text("text")),
                "output" => run.out.push_str(&text("text")),
                "started" => run.started = Some(event),
                "notice" => run.messages.push(text("text")),
                "outputAtEnd" => run.messages.push(format!(
                    "The program's output comes when it ends: {}.",
                    text("reason")
                )),
                "stopping" => run.messages.push("Stopping the program.".to_owned()),
                // building, built, diagnostics, connected: the start tells what matters.
                _ => {}
            }
        }
        run.changed.notify_waiters();
    }

    fn err(&mut self, text: &str) {
        let mut run = self.0.borrow_mut();
        if !text.trim().is_empty() {
            run.messages.push(text.to_owned());
        }
        run.changed.notify_waiters();
    }
}

/// The text of `all` from byte `offset` on, at most [`READ_LIMIT`] bytes cut at a character,
/// and the offset after it; `None` when `offset` is not one a read gave.
pub fn text_from(all: &str, offset: usize) -> Option<(&str, usize)> {
    if offset > all.len() || !all.is_char_boundary(offset) {
        return None;
    }
    let mut end = all.len().min(offset + READ_LIMIT);
    while !all.is_char_boundary(end) {
        end -= 1;
    }
    Some((&all[offset..end], end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_from_cuts_at_a_character_and_refuses_an_offset_inside_one() {
        let all = format!("{}ž", "a".repeat(READ_LIMIT - 1));
        let (first, next) = text_from(&all, 0).unwrap();
        assert_eq!((first.len(), next), (READ_LIMIT - 1, READ_LIMIT - 1));
        assert_eq!(text_from(&all, next), Some(("ž", all.len())));
        assert_eq!(text_from(&all, READ_LIMIT), None, "inside ž");
        assert_eq!(text_from(&all, all.len() + 1), None);
    }

    #[test]
    fn the_sink_takes_output_the_start_messages_and_the_end_from_the_event_lines() {
        let (stop, _) = mpsc::unbounded_channel();
        let run = Rc::new(RefCell::new(LiveRun::new(stop)));
        let mut sink = RunSink(run.clone());
        sink.out(r#"{"event":"started","start":{"runSeq":3}}"#);
        sink.out(r#"{"event":"output","stream":"out","text":"a\n"}"#);
        sink.out(r#"{"event":"output","stream":"err","text":"b\n"}"#);
        sink.out(r#"{"event":"outputAtEnd","reason":"no follow free"}"#);
        assert!(run.borrow().running());
        sink.out(r#"{"event":"result","exitCode":0,"result":{}}"#);
        let run = run.borrow();
        assert_eq!((run.out.as_str(), run.err.as_str()), ("a\n", "b\n"));
        assert_eq!(run.started.as_ref().unwrap()["start"]["runSeq"], 3);
        assert_eq!(
            run.messages,
            ["The program's output comes when it ends: no follow free."]
        );
        assert!(!run.running());
        assert_eq!(run.ended.as_ref().unwrap()["exitCode"], 0);
    }
}
