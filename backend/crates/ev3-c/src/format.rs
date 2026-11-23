//! Run results as lines of text, as the TypeScript `resultsView` printed them, and times as
//! the commands print them.

use std::time::{SystemTime, UNIX_EPOCH};

use ev3c_core::brick::returned_files::ReturnedReport;
use ev3c_core::brick::run_results::RunResult;
use ev3c_core::results::{RunEnd, RunRecord};

/// How a run ended, in one phrase.
pub fn how_it_ended(record: &RunRecord) -> String {
    match record.end {
        RunEnd::Program => match (record.signal, record.exit) {
            (Some(signal), _) => format!("the program was ended by signal {signal}"),
            (None, Some(exit)) => format!("the program exited with code {exit}"),
            (None, None) => "the program ended".to_owned(),
        },
        RunEnd::Slot => {
            let how = match (record.signal, record.exit) {
                (Some(signal), _) => format!(" (signal {signal})"),
                (None, Some(exit)) => format!(" (exit code {exit})"),
                (None, None) => String::new(),
            };
            format!("the program was stopped when its slot ended{how}")
        }
        RunEnd::BeforeStart => "the slot ended before the program started".to_owned(),
    }
}

/// When EVƎ C started the run, or that it was not started by this command.
pub fn started_text(started_at: Option<SystemTime>) -> String {
    match started_at {
        Some(time) => format!("{} (PC time)", readable_time(time)),
        None => "not started by EVƎ C in this session (no PC start time)".to_owned(),
    }
}

pub fn format_run_result(project: &str, result: &RunResult) -> Vec<String> {
    let record = &result.record;
    let mut lines = vec![
        format!("--- {project}, run {} ---", result.run),
        format!("Started: {}", started_text(result.started_at)),
        format!("Ended: {}", how_it_ended(record)),
    ];
    lines.extend(stream(
        "Output (stdout)",
        result.out.as_deref(),
        record.out_truncated,
    ));
    lines.extend(stream(
        "Errors (stderr)",
        result.err.as_deref(),
        record.err_truncated,
    ));
    if !result.returned.is_empty() {
        lines.push("Returned files:".to_owned());
        lines.extend(result.returned.iter().map(|path| format!("  {path}")));
    }
    lines
}

/// What became of a run's returned files on this computer.
pub fn format_returned(report: &ReturnedReport) -> Vec<String> {
    let downloaded = report.downloaded.iter().map(|file| {
        format!(
            "Returned {} → {}",
            file.brick_path,
            file.local_path.display()
        )
    });
    let failed = report
        .failed
        .iter()
        .map(|file| format!("Could not return {}: {}", file.brick_path, file.reason));
    downloaded.chain(failed).collect()
}

/// `list_later` names what lists the run later: a command or a CLI call.
pub fn format_no_result(
    project: &str,
    launch_id: &str,
    started_at: SystemTime,
    list_later: &str,
) -> Vec<String> {
    vec![
        format!(
            "--- {project}, run started {} ---",
            started_text(Some(started_at))
        ),
        format!(
            "No result was recorded on the brick (launch {launch_id}). {list_later} lists the run if its record appears later."
        ),
    ]
}

fn stream(title: &str, text: Option<&str>, truncated: bool) -> Vec<String> {
    let note = if truncated {
        ", truncated at the size limit"
    } else {
        ""
    };
    match text {
        None => vec![format!("{title}{note}: could not be read from the brick")],
        Some("") => vec![format!("{title}{note}: (empty)")],
        Some(text) => {
            let mut lines = vec![format!("{title}{note}:")];
            let text = text.strip_suffix('\n').unwrap_or(text);
            lines.extend(text.split('\n').map(|line| format!("  {line}")));
            lines
        }
    }
}

/// A time as JavaScript's `toISOString()` writes it: `2026-11-23T10:00:00.000Z`.
pub fn iso_time(time: SystemTime) -> String {
    let (date, clock, millis) = utc_parts(time);
    format!("{date}T{clock}.{millis:03}Z")
}

/// A time for people: `2026-11-23 10:00:00 UTC`. The TypeScript backend printed local time
/// in the user's locale; Rust's standard library knows neither the time zone nor the locale,
/// so the time is UTC and says so.
pub fn readable_time(time: SystemTime) -> String {
    let (date, clock, _) = utc_parts(time);
    format!("{date} {clock} UTC")
}

fn utc_parts(time: SystemTime) -> (String, String, u32) {
    // A time before 1970 would be a broken clock; it prints as the epoch.
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let (year, month, day) = civil_from_days(seconds / 86_400);
    let of_day = seconds % 86_400;
    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!(
            "{:02}:{:02}:{:02}",
            of_day / 3600,
            of_day % 3600 / 60,
            of_day % 60
        ),
        since.subsec_millis(),
    )
}

/// The proleptic Gregorian date of a day count since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`, for days on or after the epoch).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use ev3c_core::brick::returned_files::{ReturnedFailure, ReturnedFile};

    use super::*;

    fn record(end: RunEnd, exit: Option<i64>, signal: Option<i64>) -> RunRecord {
        RunRecord {
            run: 4,
            end,
            exit,
            signal,
            out_truncated: false,
            err_truncated: false,
            launch: None,
        }
    }

    fn at(millis: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(millis)
    }

    #[test]
    fn how_a_run_ended_reads_as_one_phrase() {
        let cases = [
            (
                record(RunEnd::Program, Some(3), None),
                "the program exited with code 3",
            ),
            (
                record(RunEnd::Program, None, Some(11)),
                "the program was ended by signal 11",
            ),
            (record(RunEnd::Program, None, None), "the program ended"),
            (
                record(RunEnd::Slot, None, Some(15)),
                "the program was stopped when its slot ended (signal 15)",
            ),
            (
                record(RunEnd::Slot, Some(1), None),
                "the program was stopped when its slot ended (exit code 1)",
            ),
            (
                record(RunEnd::BeforeStart, None, None),
                "the slot ended before the program started",
            ),
        ];
        for (record, phrase) in cases {
            assert_eq!(how_it_ended(&record), phrase);
        }
    }

    #[test]
    fn a_result_shows_both_streams_indented_and_says_what_is_missing() {
        let mut result = RunResult {
            run: 4,
            record: record(RunEnd::Program, Some(0), None),
            out: Some("hello\nworld\n".to_owned()),
            err: Some(String::new()),
            started_at: Some(at(1_795_428_000_000)),
            returned: vec!["/home/root/lms2012/prjs/hello/log.csv".to_owned()],
        };
        result.record.err_truncated = true;
        assert_eq!(
            format_run_result("hello", &result),
            [
                "--- hello, run 4 ---",
                "Started: 2026-11-23 10:00:00 UTC (PC time)",
                "Ended: the program exited with code 0",
                "Output (stdout):",
                "  hello",
                "  world",
                "Errors (stderr), truncated at the size limit: (empty)",
                "Returned files:",
                "  /home/root/lms2012/prjs/hello/log.csv",
            ]
        );
        result.out = None;
        result.started_at = None;
        let lines = format_run_result("hello", &result);
        assert_eq!(
            lines[1],
            "Started: not started by EVƎ C in this session (no PC start time)"
        );
        assert_eq!(
            lines[3],
            "Output (stdout): could not be read from the brick"
        );
    }

    #[test]
    fn returned_files_and_a_missing_result_read_as_lines() {
        let report = ReturnedReport {
            downloaded: vec![ReturnedFile {
                brick_path: "/a/log.csv".to_owned(),
                local_path: PathBuf::from("log.csv"),
            }],
            failed: vec![ReturnedFailure {
                brick_path: "/a/b".to_owned(),
                reason: "gone".to_owned(),
            }],
        };
        assert_eq!(
            format_returned(&report),
            [
                "Returned /a/log.csv → log.csv",
                "Could not return /a/b: gone"
            ]
        );
        assert_eq!(
            format_no_result("hello", "abc", at(0), "\"ev3 c runs\""),
            [
                "--- hello, run started 1970-01-01 00:00:00 UTC (PC time) ---",
                "No result was recorded on the brick (launch abc). \"ev3 c runs\" lists the run if its record appears later.",
            ]
        );
    }

    #[test]
    fn times_print_as_javascripts_iso_strings() {
        assert_eq!(iso_time(at(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(at(1_795_428_000_123)), "2026-11-23T10:00:00.123Z");
        assert_eq!(iso_time(at(951_782_400_000)), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_time(at(4_107_542_399_999)), "2100-02-28T23:59:59.999Z");
    }
}
