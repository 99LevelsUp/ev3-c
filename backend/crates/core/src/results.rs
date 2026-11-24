//! How a run ended, read from its run folder. The completion record `res` is `key=value`
//! text, one pair per line:
//!
//! ```text
//! run=<n>                          always
//! end=program|slot|before-start    always
//! exit=<code>                      when the program exited
//! signal=<number>                  when a signal ended the program
//! out_truncated=0|1, err_truncated=0|1
//! launch=<id>                      only when EVƎ C started the run
//! ```
//!
//! Only the parsing lives here; reading run folders from a brick is in
//! [`crate::brick::run_results`].

use std::collections::HashMap;
use std::fmt;

use serde::Serialize;

use crate::json::quote;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunEnd {
    /// The program ended on its own.
    Program,
    /// The slot ended and stopped the program.
    Slot,
    /// The slot ended before the program started.
    BeforeStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub run: i64,
    pub end: RunEnd,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i64>,
    pub out_truncated: bool,
    pub err_truncated: bool,
    /// The launch ID EVƎ C wrote before starting the run; absent for a run started on the
    /// brick.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<String>,
}

/// A completion record without a valid run number or end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecordError(String);

impl fmt::Display for RunRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RunRecordError {}

/// Parses a completion record. Unknown keys are ignored, a later line overrides an earlier
/// one with the same key, and a trailing `\r` is dropped from each value.
pub fn parse_run_record(text: &str) -> Result<RunRecord, RunRecordError> {
    let mut values: HashMap<&str, &str> = HashMap::new();
    for line in text.split('\n') {
        if let Some(eq) = line.find('=')
            && eq > 0
        {
            let value = &line[eq + 1..];
            values.insert(&line[..eq], value.strip_suffix('\r').unwrap_or(value));
        }
    }
    let run = integer(values.get("run").copied());
    let end = match values.get("end").copied() {
        Some("program") => Some(RunEnd::Program),
        Some("slot") => Some(RunEnd::Slot),
        Some("before-start") => Some(RunEnd::BeforeStart),
        _ => None,
    };
    let (Some(run), Some(end)) = (run, end) else {
        return Err(RunRecordError(format!(
            "The run's completion record is not valid: {}",
            quote(text)
        )));
    };
    Ok(RunRecord {
        run,
        end,
        exit: integer(values.get("exit").copied()),
        signal: integer(values.get("signal").copied()),
        out_truncated: values.get("out_truncated") == Some(&"1"),
        err_truncated: values.get("err_truncated") == Some(&"1"),
        launch: values
            .get("launch")
            .filter(|launch| !launch.is_empty())
            .map(|launch| (*launch).to_owned()),
    })
}

/// A decimal integer with an optional minus sign and nothing else, not even a plus sign.
fn integer(value: Option<&str>) -> Option<i64> {
    let value = value?;
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// The exit code ev3lib ends a program with when a plain call failed
/// (`EV3_EXIT_LIBRARY_ERROR` in ev3lib's `ev3_error.h`).
pub const LIBRARY_ERROR_EXIT: i64 = 70;

/// The line ev3lib writes to stderr after the error when it stops a program
/// (`EV3_LIBRARY_STOP_LINE`).
pub const LIBRARY_STOP_LINE: &str = "ev3lib stopped the program because of this error.";

/// The error that made ev3lib stop the program, such as
/// `main.c:12: ev3_motor_start: port 1 is a sensor port; motors are on ports A to D`: the line
/// before ev3lib's stop line at the end of stderr, when the program exited with ev3lib's code.
/// A program that only returns 70 itself, or whose stderr was cut at the cap, has none.
pub fn library_error(record: &RunRecord, err: Option<&str>) -> Option<String> {
    if record.end != RunEnd::Program
        || record.signal.is_some()
        || record.exit != Some(LIBRARY_ERROR_EXIT)
        || record.err_truncated
    {
        return None;
    }
    let mut lines = err?
        .lines()
        .rev()
        .map(str::trim_end)
        .skip_while(|line| line.is_empty());
    if lines.next()? != LIBRARY_STOP_LINE {
        return None;
    }
    lines
        .next()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ev3libs_stop_is_told_by_its_exit_code_and_its_last_line() {
        let record =
            parse_run_record("run=3\nend=program\nexit=70\nout_truncated=0\nerr_truncated=0\n")
                .unwrap();
        let err = "warming up\nmain.c:12: ev3_motor_start: port 1 is a sensor port; motors are on ports A to D\nev3lib stopped the program because of this error.\n";
        assert_eq!(
            library_error(&record, Some(err)).as_deref(),
            Some("main.c:12: ev3_motor_start: port 1 is a sensor port; motors are on ports A to D")
        );
        assert_eq!(
            library_error(&record, Some("exit 70 of its own\n")),
            None,
            "a program that returns 70 itself"
        );
        let other =
            parse_run_record("run=3\nend=program\nexit=1\nout_truncated=0\nerr_truncated=0\n")
                .unwrap();
        assert_eq!(library_error(&other, Some(err)), None, "another exit code");
        assert_eq!(library_error(&record, None), None, "no stderr read");
    }

    #[test]
    fn a_completion_record_with_an_exit_code_and_a_launch_id_is_parsed() {
        assert_eq!(
            parse_run_record(
                "run=7\nend=program\nexit=3\nout_truncated=0\nerr_truncated=1\nlaunch=abc-123\n"
            ),
            Ok(RunRecord {
                run: 7,
                end: RunEnd::Program,
                exit: Some(3),
                signal: None,
                out_truncated: false,
                err_truncated: true,
                launch: Some("abc-123".into()),
            })
        );
    }

    #[test]
    fn a_completion_record_of_a_run_killed_by_a_signal_has_no_exit_code_and_no_launch_id() {
        assert_eq!(
            parse_run_record("run=2\nend=slot\nsignal=9\nout_truncated=0\nerr_truncated=0\n"),
            Ok(RunRecord {
                run: 2,
                end: RunEnd::Slot,
                exit: None,
                signal: Some(9),
                out_truncated: false,
                err_truncated: false,
                launch: None,
            })
        );
    }

    #[test]
    fn unknown_keys_in_a_completion_record_are_ignored() {
        let record = parse_run_record(
            "run=3\nend=before-start\nret=later\nout_truncated=0\nerr_truncated=0\n",
        )
        .unwrap();
        assert_eq!(record.end, RunEnd::BeforeStart);
        assert_eq!(record.run, 3);
    }

    #[test]
    fn crlf_line_ends_and_an_empty_launch_id_are_handled() {
        let record = parse_run_record("run=4\r\nend=program\r\nexit=-1\r\nlaunch=\r\n").unwrap();
        assert_eq!(record.end, RunEnd::Program);
        assert_eq!(record.exit, Some(-1));
        assert_eq!(record.launch, None);
    }

    #[test]
    fn a_completion_record_without_a_valid_run_number_or_end_is_rejected() {
        for text in [
            "end=program\nout_truncated=0\nerr_truncated=0\n",
            "run=1\nout_truncated=0\n",
            "run=x\nend=program\n",
            "run=+1\nend=program\n",
            "run=\nend=program\n",
            "run=1\nend=crashed\n",
        ] {
            let error = parse_run_record(text).unwrap_err();
            assert!(error.to_string().contains("completion record"), "{error}");
            assert!(error.to_string().contains(&quote(text)), "{error}");
        }
    }

    #[test]
    fn a_record_serializes_with_the_typescript_field_names() {
        let record = parse_run_record("run=3\nend=before-start\nsignal=15\n").unwrap();
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            serde_json::json!({
                "run": 3,
                "end": "before-start",
                "signal": 15,
                "outTruncated": false,
                "errTruncated": false,
            })
        );
    }
}
