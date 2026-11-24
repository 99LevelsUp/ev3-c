//! The launcher: a small bytecode program (.rbf) that keeps the brick's user slot alive while
//! the native supervisor runs the student's program, and ends the slot when the program ends.
//!
//! What it does, in order:
//!   1. Build the project folder with FILENAME GET_FOLDERNAME, and every marker path as that
//!      folder plus a short dot-name: .tok (slot token), .run (run marker), .bak (Back request).
//!      The firmware resolves a bare dot-name against its own working directory, so the folder
//!      is always spelled out.
//!   2. Remove stale .run and .bak, delete .tok, then create .tok and keep it open for the whole
//!      run. The supervisor watches the VM's open descriptor on .tok to notice the slot ending.
//!   3. Start the supervisor in the background, once, through SYSTEM (see
//!      [`supervisor_arguments`]).
//!   4. Wait up to 6 s for .run, or end the slot. Read the supervisor's PID from .run once and
//!      build /proc/<pid>.
//!   5. Every 100 ms test .run and /proc/<pid>, and set or clear the Back block when .bak
//!      appears or disappears, so a program can take short presses of Back over.
//!   6. End when .run is gone or the supervisor's process has disappeared, so a crashed
//!      supervisor still ends the slot and the firmware stops the motors.
//!
//! No file is closed inside the loop, because every bytecode close runs a global sync(); the
//! PID read is the only close after start-up. The launcher changes no file permissions and
//! touches nothing outside its own project folder. It uses no project name: everything comes
//! from the folder it was started from.
//!
//! The golden source for these bytes is backend/crates/core/fixtures/launcher/launcher.lms; keep the
//! two in step (declaration order and sizes decide the local offsets).

use serde::Serialize;

use super::bytecode::{
    AssembledImage, LauncherError, Operand, ThreadBuilder, firmware_limits, label, local, op,
    string, sub,
};

/// How many run folders the supervisor keeps before it creates a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RetentionRule {
    /// Keep the last `count` runs.
    Runs { count: u32 },
    /// Keep runs up to `limit` megabytes.
    Megabytes { limit: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LauncherOptions {
    pub retention: RetentionRule,
    /// Size limit in bytes for each captured output stream (stdout, stderr).
    pub output_cap_bytes: u32,
}

/// File name of the supervisor executable inside the project folder.
pub const SUPERVISOR_FILE: &str = "sup";

const TICK_MS: i32 = 100;
const START_TIMEOUT_MS: i32 = 6000;

// The .run marker holds the supervisor's PID as decimal digits without a newline (written to
// a temporary name and renamed into place). The brick's kernel hands out PIDs of at most five
// digits; 15 is the read limit and leaves room.
const PID_READ_BYTES: u32 = 15;

// STRINGS ADD writes into a local with an unbounded sprintf, so every buffer must hold the
// longest value it can receive. A program's folder (with its trailing '/') is shorter than
// vmPATHSIZE, so GET_FOLDERNAME never truncates it into FOLDER_SIZE.
const FOLDER_SIZE: u32 = firmware_limits::PATH_SIZE;
const PATH_SIZE: u32 = firmware_limits::FILENAME_SIZE;
const COMMAND_SIZE: u32 = 256;

const MAX_ARGUMENT: u32 = 0x7fff_ffff;

fn positive_int32(value: u32, what: &str) -> Result<u32, LauncherError> {
    if value == 0 || value > MAX_ARGUMENT {
        return Err(LauncherError(format!(
            "{what} must be a whole number from 1 to {MAX_ARGUMENT}, got {value}"
        )));
    }
    Ok(value)
}

/// The supervisor's arguments after the project folder. The full command line is
///
/// ```text
/// <folder>/sup <folder> keep=<n> cap=<bytes> &
/// <folder>/sup <folder> keepmb=<m> cap=<bytes> &
/// ```
///
/// argv\[1\] is the project folder as the VM reports it (absolute when started through cockpit,
/// relative to the VM's working directory when started from the brick menu), argv\[2\] the
/// retention rule (keep the last n runs, or runs up to m megabytes), argv\[3\] the cap on each
/// captured output stream in bytes. Every number is a positive decimal that fits a signed
/// 32-bit integer.
pub fn supervisor_arguments(options: &LauncherOptions) -> Result<String, LauncherError> {
    let rule = match options.retention {
        RetentionRule::Runs { count } => format!("keep={}", positive_int32(count, "run count")?),
        RetentionRule::Megabytes { limit } => {
            format!("keepmb={}", positive_int32(limit, "megabyte limit")?)
        }
    };
    let cap = positive_int32(options.output_cap_bytes, "output cap")?;
    Ok(format!("{rule} cap={cap}"))
}

fn constant(value: u32) -> Operand {
    // Every size passed here is a small constant of this file.
    Operand::Constant(value as i32)
}

/// Assembles the launcher and keeps its layout, for inspection.
pub fn assemble_launcher(options: &LauncherOptions) -> Result<AssembledImage, LauncherError> {
    use sub::{file, filename, strings, ui_button};

    let tail = format!(" {} &", supervisor_arguments(options)?);
    let mut t = ThreadBuilder::new();

    t.data_string("Folder", FOLDER_SIZE)?;
    t.data_string("RunPath", PATH_SIZE)?;
    t.data_string("BakPath", PATH_SIZE)?;
    t.data_string("TokPath", PATH_SIZE)?;
    t.data_string("T1", COMMAND_SIZE)?;
    t.data_string("T2", COMMAND_SIZE)?;
    t.data_string("Cmd", COMMAND_SIZE)?;
    t.data_string("Pid", PID_READ_BYTES + 1)?;
    t.data_string("Proc", 32)?;
    t.data16("hTok")?;
    t.data16("hRun")?;
    t.data32("Size")?;
    t.data32("Status")?;
    t.data32("Timer")?;
    t.data8("Flag")?;
    t.data8("BackOn")?;
    t.data8("Count")?;

    // Marker paths from the project folder.
    t.emit(
        op::FILENAME,
        &[
            filename::GET_FOLDERNAME.into(),
            constant(FOLDER_SIZE),
            local("Folder"),
        ],
    );
    for (marker, path) in [
        ("/.run", "RunPath"),
        ("/.bak", "BakPath"),
        ("/.tok", "TokPath"),
    ] {
        t.emit(
            op::STRINGS,
            &[
                strings::ADD.into(),
                local("Folder"),
                string(marker),
                local(path),
            ],
        );
    }

    // Clear stale markers and hold a fresh slot token.
    t.emit(op::FILE, &[file::REMOVE.into(), local("RunPath")]);
    t.emit(op::FILE, &[file::REMOVE.into(), local("BakPath")]);
    t.emit(op::FILE, &[file::REMOVE.into(), local("TokPath")]);
    t.emit(
        op::FILE,
        &[file::OPEN_WRITE.into(), local("TokPath"), local("hTok")],
    );

    // Start the supervisor in the background.
    t.emit(
        op::STRINGS,
        &[
            strings::ADD.into(),
            local("Folder"),
            string(&format!("/{SUPERVISOR_FILE} ")),
            local("T1"),
        ],
    );
    t.emit(
        op::STRINGS,
        &[
            strings::ADD.into(),
            local("T1"),
            local("Folder"),
            local("T2"),
        ],
    );
    t.emit(
        op::STRINGS,
        &[
            strings::ADD.into(),
            local("T2"),
            string(&tail),
            local("Cmd"),
        ],
    );
    t.emit(op::SYSTEM, &[local("Cmd"), local("Status")]);

    // Wait for the run marker.
    t.emit(op::MOVE8_8, &[0.into(), local("Count")]);
    t.label("WaitStart")?;
    t.emit(op::TIMER_WAIT, &[TICK_MS.into(), local("Timer")]);
    t.emit(op::TIMER_READY, &[local("Timer")]);
    t.emit(
        op::FILENAME,
        &[filename::EXIST.into(), local("RunPath"), local("Flag")],
    );
    t.emit(op::JR_TRUE, &[local("Flag"), label("Started")]);
    t.emit(op::ADD8, &[local("Count"), 1.into(), local("Count")]);
    t.emit(
        op::JR_LT8,
        &[
            local("Count"),
            (START_TIMEOUT_MS / TICK_MS).into(),
            label("WaitStart"),
        ],
    );
    t.emit(op::JR, &[label("Done")]);

    // Read the supervisor's PID once.
    t.label("Started")?;
    t.emit(
        op::FILE,
        &[
            file::OPEN_READ.into(),
            local("RunPath"),
            local("hRun"),
            local("Size"),
        ],
    );
    t.emit(
        op::FILE,
        &[
            file::READ_TEXT.into(),
            local("hRun"),
            0.into(),
            constant(PID_READ_BYTES),
            local("Pid"),
        ],
    );
    t.emit(op::FILE, &[file::CLOSE.into(), local("hRun")]);
    t.emit(
        op::STRINGS,
        &[
            strings::ADD.into(),
            string("/proc/"),
            local("Pid"),
            local("Proc"),
        ],
    );
    t.emit(op::MOVE8_8, &[0.into(), local("BackOn")]);

    // Poll until the run marker or the supervisor is gone.
    t.label("Poll")?;
    t.emit(op::TIMER_WAIT, &[TICK_MS.into(), local("Timer")]);
    t.emit(op::TIMER_READY, &[local("Timer")]);
    t.emit(
        op::FILENAME,
        &[filename::EXIST.into(), local("RunPath"), local("Flag")],
    );
    t.emit(op::JR_FALSE, &[local("Flag"), label("Done")]);
    t.emit(
        op::FILENAME,
        &[filename::EXIST.into(), local("Proc"), local("Flag")],
    );
    t.emit(op::JR_FALSE, &[local("Flag"), label("Done")]);
    t.emit(
        op::FILENAME,
        &[filename::EXIST.into(), local("BakPath"), local("Flag")],
    );
    t.emit(op::JR_EQ8, &[local("Flag"), local("BackOn"), label("Poll")]);
    t.emit(op::MOVE8_8, &[local("Flag"), local("BackOn")]);
    t.emit(
        op::UI_BUTTON,
        &[ui_button::SET_BACK_BLOCK.into(), local("BackOn")],
    );
    t.emit(op::JR, &[label("Poll")]);

    // Ending the thread ends the slot.
    t.label("Done")?;
    t.assemble()
}

/// The launcher .rbf for a project, ready to upload as `<project>/<project>.rbf`.
pub fn build_launcher(options: &LauncherOptions) -> Result<Vec<u8>, LauncherError> {
    Ok(assemble_launcher(options)?.bytes)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::launcher::bytecode::{Instruction, encode_constant, encode_local};
    use crate::testutil::repo_path;

    // launcher.rbf.hex is assembled from launcher.lms by scripts/regen-launcher-golden.cjs with
    // LEGO's lmsasm.
    const GOLDEN: LauncherOptions = LauncherOptions {
        retention: RetentionRule::Runs { count: 5 },
        output_cap_bytes: 65536,
    };
    const WIDEST: LauncherOptions = LauncherOptions {
        retention: RetentionRule::Megabytes { limit: 0x7fff_ffff },
        output_cap_bytes: 0x7fff_ffff,
    };

    // Limits from the firmware's bytecodes.h; the checks below mirror cMemoryCheckFilename in
    // c_memory.c, which splits a path at its last '/' and its name at the first '.'.
    const VM_PATHSIZE: usize = 84;
    const VM_NAMESIZE: usize = 32;
    const VM_EXTSIZE: usize = 5;
    const VM_FILENAMESIZE: usize = 120;

    // Cockpit refuses brick paths of 60 bytes or more.
    const COCKPIT_PATH_LIMIT: usize = 60;
    const PRJS: &str = "/home/root/lms2012/prjs";
    const LONGEST_NAME: &str = "abcdefghijklmno";

    fn firmware_filename_problems(file: &str) -> Vec<String> {
        let slash = file.rfind('/').map_or(0, |at| at + 1);
        let (dir, rest) = file.split_at(slash);
        let (name, ext) = rest.split_at(rest.find('.').unwrap_or(rest.len()));
        let mut problems = Vec::new();
        if file.len() >= VM_FILENAMESIZE {
            problems.push(format!("path {} >= {VM_FILENAMESIZE}", file.len()));
        }
        if dir.len() >= VM_PATHSIZE {
            problems.push(format!("folder {} >= {VM_PATHSIZE}", dir.len()));
        }
        if name.len() >= VM_NAMESIZE {
            problems.push(format!("name {} >= {VM_NAMESIZE}", name.len()));
        }
        if ext.len() >= VM_EXTSIZE {
            problems.push(format!("extension {} >= {VM_EXTSIZE}", ext.len()));
        }
        if !file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " _.-/~".contains(c))
        {
            problems.push("character outside the firmware file-name set".to_owned());
        }
        problems
    }

    fn golden_bytes() -> Vec<u8> {
        let hex = std::fs::read_to_string(repo_path(
            "backend/crates/core/fixtures/launcher/launcher.rbf.hex",
        ))
        .unwrap();
        hex.split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect()
    }

    fn is_local(operand: Option<&Operand>, name: &str) -> bool {
        matches!(operand, Some(Operand::Local(local)) if local == name)
    }

    fn local_name(operand: Option<&Operand>) -> &str {
        match operand {
            Some(Operand::Local(name)) => name,
            other => panic!("expected a local, got {other:?}"),
        }
    }

    fn number(operand: Option<&Operand>) -> i32 {
        match operand {
            Some(Operand::Constant(value)) => *value,
            other => panic!("expected a constant, got {other:?}"),
        }
    }

    #[derive(Default)]
    struct Evaluation {
        /// Final string value of each string local.
        values: HashMap<String, String>,
        /// Every path handed to FILE or FILENAME, in program order.
        paths: Vec<String>,
        /// The shell command handed to SYSTEM.
        commands: Vec<String>,
        /// Destinations whose STRINGS ADD result would overflow the local.
        overflows: Vec<String>,
    }

    /// Follows the launcher's string building for a given folder, as the VM would. The PID read
    /// out of .run is taken to be `pid`; the longest the brick's kernel hands out has 5 digits.
    fn evaluate_strings(image: &AssembledImage, folder: &str, pid: &str) -> Evaluation {
        let mut eval = Evaluation::default();
        let text = |eval: &Evaluation, operand: Option<&Operand>| -> String {
            if let Some(Operand::String(text)) = operand {
                return text.clone();
            }
            eval.values
                .get(local_name(operand))
                .cloned()
                .unwrap_or_default()
        };
        let store = |eval: &mut Evaluation, name: &str, value: String| {
            let size = image.locals.get(name).map_or(0, |slot| slot.size as usize);
            if value.len() + 1 > size {
                eval.overflows
                    .push(format!("{name}: {} > {size}", value.len() + 1));
            }
            eval.values.insert(name.to_owned(), value);
        };
        for Instruction { opcode, operands } in &image.instructions {
            let cmd = match operands.first() {
                Some(Operand::Constant(cmd)) => Some(*cmd),
                _ => None,
            };
            let at = |i: usize| operands.get(i);
            if *opcode == op::FILENAME && cmd == Some(sub::filename::GET_FOLDERNAME) {
                let length = number(at(1)) as usize;
                let value = folder[..folder.len().min(length - 1)].to_owned();
                store(&mut eval, local_name(at(2)), value);
            } else if *opcode == op::FILENAME && cmd == Some(sub::filename::EXIST) {
                let path = text(&eval, at(1));
                eval.paths.push(path);
            } else if *opcode == op::STRINGS && cmd == Some(sub::strings::ADD) {
                let value = text(&eval, at(1)) + &text(&eval, at(2));
                store(&mut eval, local_name(at(3)), value);
            } else if *opcode == op::FILE
                && matches!(
                    cmd,
                    Some(sub::file::REMOVE | sub::file::OPEN_WRITE | sub::file::OPEN_READ)
                )
            {
                let path = text(&eval, at(1));
                eval.paths.push(path);
            } else if *opcode == op::FILE && cmd == Some(sub::file::READ_TEXT) {
                let length = number(at(3)) as usize;
                store(
                    &mut eval,
                    local_name(at(4)),
                    pid[..pid.len().min(length)].to_owned(),
                );
            } else if *opcode == op::SYSTEM {
                let command = text(&eval, at(0));
                eval.commands.push(command);
            }
        }
        eval
    }

    fn index_of_sequence(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        haystack
            .get(from..)?
            .windows(needle.len())
            .position(|window| window == needle)
            .map(|at| at + from)
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    #[test]
    fn the_launcher_matches_the_bytes_lmsasm_assembles_from_launcher_lms() {
        assert_eq!(build_launcher(&GOLDEN).unwrap(), golden_bytes());
    }

    #[test]
    fn the_image_size_field_equals_the_file_length_and_the_code_starts_after_the_headers() {
        let bytes = build_launcher(&GOLDEN).unwrap();
        assert_eq!(u32_at(&bytes, 4) as usize, bytes.len());
        assert_eq!(u16::from_le_bytes([bytes[10], bytes[11]]), 1, "one thread");
        // 16-byte image header plus one 12-byte object header.
        assert_eq!(u32_at(&bytes, 16), 28);
        assert_eq!(
            bytes[28],
            op::FILENAME,
            "first instruction builds the folder name"
        );
        assert_eq!(bytes[bytes.len() - 1], op::OBJECT_END);
    }

    #[test]
    fn the_supervisor_command_carries_the_folder_the_retention_rule_and_the_output_cap_in_order() {
        let folder = format!("{PRJS}/demo");
        let runs = evaluate_strings(&assemble_launcher(&GOLDEN).unwrap(), &folder, "32768");
        assert_eq!(
            runs.commands,
            [format!("{folder}/sup {folder} keep=5 cap=65536 &")]
        );

        let options = LauncherOptions {
            retention: RetentionRule::Megabytes { limit: 2 },
            output_cap_bytes: 4096,
        };
        let megabytes = evaluate_strings(&assemble_launcher(&options).unwrap(), &folder, "32768");
        assert_eq!(
            megabytes.commands,
            [format!("{folder}/sup {folder} keepmb=2 cap=4096 &")]
        );
    }

    #[test]
    fn supervisor_arguments_refuse_values_the_supervisor_cannot_read_as_a_positive_32_bit_number() {
        assert_eq!(supervisor_arguments(&GOLDEN).unwrap(), "keep=5 cap=65536");
        // Negative and fractional values do not type-check as u32.
        for bad in [0, 0x8000_0000, u32::MAX] {
            let runs = LauncherOptions {
                retention: RetentionRule::Runs { count: bad },
                ..GOLDEN
            };
            let megabytes = LauncherOptions {
                retention: RetentionRule::Megabytes { limit: bad },
                ..GOLDEN
            };
            let cap = LauncherOptions {
                output_cap_bytes: bad,
                ..GOLDEN
            };
            assert!(supervisor_arguments(&runs).is_err());
            assert!(supervisor_arguments(&megabytes).is_err());
            assert!(supervisor_arguments(&cap).is_err());
        }
        assert_eq!(
            supervisor_arguments(&WIDEST).unwrap(),
            "keepmb=2147483647 cap=2147483647"
        );
    }

    #[test]
    fn a_15_byte_project_name_gives_a_launcher_path_under_the_cockpit_limit() {
        assert_eq!(LONGEST_NAME.len(), 15);
        let launcher_path = format!("{PRJS}/{LONGEST_NAME}/{LONGEST_NAME}.rbf");
        assert!(
            launcher_path.len() < COCKPIT_PATH_LIMIT,
            "{} bytes",
            launcher_path.len()
        );
        assert_eq!(
            firmware_filename_problems(&launcher_path),
            Vec::<String>::new()
        );
    }

    #[test]
    fn with_a_15_byte_project_name_every_path_and_the_command_fit_their_buffers_and_the_firmware_rules()
     {
        // The VM reports the folder in the form the program was started with: absolute from
        // cockpit, relative to the VM's working directory from the brick menu.
        for folder in [
            format!("{PRJS}/{LONGEST_NAME}"),
            format!("../prjs/{LONGEST_NAME}"),
        ] {
            for options in [GOLDEN, WIDEST] {
                let result =
                    evaluate_strings(&assemble_launcher(&options).unwrap(), &folder, "32768");
                assert_eq!(
                    result.values.get("Folder"),
                    Some(&folder),
                    "the folder name is not truncated"
                );
                assert_eq!(result.overflows, Vec::<String>::new(), "{folder}");
                assert!(!result.paths.is_empty());
                for file in &result.paths {
                    assert_eq!(
                        firmware_filename_problems(file),
                        Vec::<String>::new(),
                        "{file}"
                    );
                }
                assert_eq!(result.commands.len(), 1);
                assert_eq!(
                    firmware_filename_problems(&format!("{folder}/{SUPERVISOR_FILE}")),
                    Vec::<String>::new()
                );
            }
        }
        // The supervisor is uploaded through cockpit next to the launcher.
        let supervisor_path = format!("{PRJS}/{LONGEST_NAME}/{SUPERVISOR_FILE}");
        assert!(
            supervisor_path.len() < COCKPIT_PATH_LIMIT,
            "{} bytes",
            supervisor_path.len()
        );
    }

    #[test]
    fn no_buffer_overflows_even_for_the_longest_folder_the_firmware_can_run_a_program_from() {
        // A program's folder plus its trailing '/' is shorter than vmPATHSIZE.
        let folder = format!("/{}", "x".repeat(VM_PATHSIZE - 3));
        let result = evaluate_strings(&assemble_launcher(&WIDEST).unwrap(), &folder, "32768");
        assert_eq!(result.values.get("Folder"), Some(&folder));
        assert_eq!(result.overflows, Vec::<String>::new());
    }

    #[test]
    fn every_marker_path_is_built_from_the_folder_name_never_a_bare_dot_name() {
        let image = assemble_launcher(&GOLDEN).unwrap();
        let mut markers = Vec::new();
        for Instruction { opcode, operands } in &image.instructions {
            if *opcode == op::FILE || *opcode == op::FILENAME {
                assert!(
                    !operands.iter().any(|o| matches!(o, Operand::String(_))),
                    "file instruction with a literal path: {operands:?}"
                );
            }
            for (i, operand) in operands.iter().enumerate() {
                let Operand::String(text) = operand else {
                    continue;
                };
                if !(text.contains("/.") || text.starts_with('.')) {
                    continue;
                }
                assert_eq!(*opcode, op::STRINGS);
                assert_eq!(operands[0], Operand::Constant(sub::strings::ADD));
                assert!(
                    is_local(operands.get(1), "Folder"),
                    "{text} is appended to the folder"
                );
                assert_eq!(i, 2);
                markers.push(text.clone());
            }
        }
        markers.sort();
        assert_eq!(markers, ["/.bak", "/.run", "/.tok"]);

        let folder = format!("{PRJS}/demo");
        let paths = evaluate_strings(&image, &folder, "32768").paths;
        for marker in [".run", ".bak", ".tok"] {
            assert!(
                paths.contains(&format!("{folder}/{marker}")),
                "{marker} is used as {folder}/{marker}"
            );
        }
    }

    #[test]
    fn the_slot_token_is_created_before_the_supervisor_starts_and_never_closed() {
        let instructions = assemble_launcher(&GOLDEN).unwrap().instructions;
        let is_file = |i: &Instruction, cmd: i32| {
            i.opcode == op::FILE && i.operands.first() == Some(&Operand::Constant(cmd))
        };
        let open_token = instructions
            .iter()
            .position(|i| is_file(i, sub::file::OPEN_WRITE))
            .unwrap();
        let start = instructions
            .iter()
            .position(|i| i.opcode == op::SYSTEM)
            .unwrap();
        assert!(
            open_token < start,
            "token opened before the supervisor starts"
        );
        assert!(is_local(
            instructions[open_token].operands.get(1),
            "TokPath"
        ));
        let closes: Vec<_> = instructions
            .iter()
            .filter(|i| is_file(i, sub::file::CLOSE))
            .collect();
        assert_eq!(closes.len(), 1, "only the single PID read is closed");
        assert!(is_local(closes[0].operands.get(1), "hRun"));
        assert_eq!(
            instructions
                .iter()
                .filter(|i| i.opcode == op::SYSTEM)
                .count(),
            1,
            "the supervisor starts once"
        );
    }

    #[test]
    fn the_launcher_waits_at_most_6_s_for_the_run_marker_in_100_ms_ticks() {
        let instructions = assemble_launcher(&GOLDEN).unwrap().instructions;
        let ticks: Vec<_> = instructions
            .iter()
            .filter(|i| i.opcode == op::TIMER_WAIT)
            .collect();
        assert!(!ticks.is_empty());
        assert!(
            ticks
                .iter()
                .all(|i| i.operands.first() == Some(&Operand::Constant(100)))
        );
        let limit = instructions
            .iter()
            .find(|i| i.opcode == op::JR_LT8)
            .unwrap();
        assert!(is_local(limit.operands.first(), "Count"));
        assert_eq!(number(limit.operands.get(1)) * 100, 6000);
    }

    #[test]
    fn the_poll_loop_ends_when_the_run_marker_or_proc_pid_of_the_pid_read_from_it_is_gone() {
        let image = assemble_launcher(&GOLDEN).unwrap();
        let slot = |name: &str| image.locals[name].offset;

        // The PID comes out of .run, and /proc/<pid> is built from it.
        let values = evaluate_strings(&image, &format!("{PRJS}/demo"), "1234").values;
        assert_eq!(values.get("Proc").map(String::as_str), Some("/proc/1234"));
        let read = image
            .instructions
            .iter()
            .find(|i| {
                i.opcode == op::FILE
                    && i.operands.first() == Some(&Operand::Constant(sub::file::OPEN_READ))
            })
            .unwrap();
        assert!(
            is_local(read.operands.get(1), "RunPath") && is_local(read.operands.get(2), "hRun")
        );

        // In the bytes, both checks after Poll jump to Done when the file is missing.
        let done = image.labels["Done"];
        let poll = image.labels["Poll"];
        let code = &image.code;
        for target in ["RunPath", "Proc"] {
            let mut check = vec![op::FILENAME];
            check.extend(encode_constant(sub::filename::EXIST));
            check.extend(encode_local(slot(target)).unwrap());
            check.extend(encode_local(slot("Flag")).unwrap());
            check.push(op::JR_FALSE);
            check.extend(encode_local(slot("Flag")).unwrap());
            check.push(0x82);
            let at = index_of_sequence(code, &check, poll)
                .unwrap_or_else(|| panic!("{target} check in the poll loop"));
            let offset_at = at + check.len();
            let offset = i16::from_le_bytes([code[offset_at], code[offset_at + 1]]);
            assert_eq!(
                (offset_at + 2) as isize + offset as isize,
                done as isize,
                "{target} missing jumps to the end"
            );
        }
        assert_eq!(code[done], op::OBJECT_END);
    }
}
