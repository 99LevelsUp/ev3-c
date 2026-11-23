//! The project settings a build embeds: the project name, which names the folder on the
//! brick and the launcher, and the rules the supervisor applies to the run folders.
//!
//! ev3c.json in the project folder, every key optional:
//!
//! ```json
//! {
//!     "name": "robot",
//!     "retention": { "keepRuns": 5 },     or  { "keepMiB": 2 }
//!     "outputCapKiB": 64
//! }
//! ```
//!
//! Without a name the folder name is used. Without rules the defaults hold: keep the last
//! 5 runs, and cap each captured output stream (stdout, stderr) at 64 KiB. The project
//! folder on the brick is a small RAM disk shared by every project, and these defaults
//! keep one project's runs well under a megabyte.

use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::json::{quote, stringify};
use crate::launcher::RetentionRule;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    pub name: String,
    pub retention: RetentionRule,
    /// Size limit in bytes for each captured output stream.
    pub output_cap_bytes: u32,
}

pub const PROJECT_FILE: &str = "ev3c.json";

pub const DEFAULT_RETENTION: RetentionRule = RetentionRule::Runs { count: 5 };
pub const DEFAULT_OUTPUT_CAP_BYTES: u32 = 64 * 1024;

// The brick refuses paths of 60 bytes or more, and the launcher's path repeats the name:
// "/home/root/lms2012/prjs/" (24) + name + "/" + name + ".rbf" stays below 60 at 15 bytes.
const NAME_MAX_BYTES: usize = 15;
const NAME_RULE: &str = r#"a project name is at most 15 bytes of lowercase ASCII letters, digits, "-" or "_", and not empty"#;

// The supervisor takes every number as a signed 32-bit integer.
const MAX_INT32: u32 = 0x7fff_ffff;
const KIB: u32 = 1024;
const MIB: u32 = 1024 * 1024;

/// Why a project's settings cannot be used. The message names the broken rule, so a build
/// stops before compiling with something the user can act on.
#[derive(Debug)]
pub enum ProjectError {
    /// The name or a rule is not valid, or ev3c.json is not a JSON object.
    Invalid(String),
    /// ev3c.json exists but could not be read.
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::Invalid(message) => f.write_str(message),
            ProjectError::Io { path, source } => {
                write!(f, "Reading {} failed: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ProjectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProjectError::Invalid(_) => None,
            ProjectError::Io { source, .. } => Some(source),
        }
    }
}

fn invalid(message: String) -> ProjectError {
    ProjectError::Invalid(message)
}

/// Returns why `name` cannot be a project name, or `None` when it can.
pub fn project_name_problem(name: &str) -> Option<String> {
    let valid = (1..=NAME_MAX_BYTES).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    (!valid).then(|| format!("Project name {} is not valid: {NAME_RULE}.", quote(name)))
}

/// Reads ev3c.json from `project_dir`, fills in the defaults, and validates the result.
/// Fails with the broken rule in the message, so a build stops before compiling.
pub fn load_project_config(project_dir: &Path) -> Result<ProjectConfig, ProjectError> {
    let file = project_dir.join(PROJECT_FILE);
    let raw = read_json(&file)?;
    let place = format!("{PROJECT_FILE} in {}", project_dir.display());

    // A key that is present but null is a broken setting, not a missing one.
    let name = match raw.get("name") {
        None => {
            let name = folder_name(project_dir);
            if let Some(problem) = project_name_problem(&name) {
                return Err(invalid(format!(
                    r#"{problem} The name comes from the project folder; set a valid "name" in {place}."#
                )));
            }
            name
        }
        Some(Value::String(name)) => {
            if let Some(problem) = project_name_problem(name) {
                return Err(invalid(format!(r#"{problem} Change "name" in {place}."#)));
            }
            name.clone()
        }
        Some(_) => {
            return Err(invalid(format!(
                r#""name" in {place} must be a string: {NAME_RULE}."#
            )));
        }
    };

    let cap_kib = match raw.get("outputCapKiB") {
        None => DEFAULT_OUTPUT_CAP_BYTES / KIB,
        Some(value) => whole_number(value, "outputCapKiB", MAX_INT32 / KIB, &place)?,
    };
    let output_cap_bytes = cap_kib * KIB;

    let retention = read_retention(raw.get("retention"), &place)?;
    if let RetentionRule::Megabytes { limit } = retention
        && u64::from(limit) * u64::from(MIB) < 2 * u64::from(output_cap_bytes)
    {
        return Err(invalid(format!(
            r#""keepMiB": {limit} in {place} is too small: it must hold at least two capped output streams (2 × {cap_kib} KiB), because a single run can write that much."#
        )));
    }

    Ok(ProjectConfig {
        name,
        retention,
        output_cap_bytes,
    })
}

fn read_retention(value: Option<&Value>, place: &str) -> Result<RetentionRule, ProjectError> {
    let Some(value) = value else {
        return Ok(DEFAULT_RETENTION);
    };
    let Value::Object(rule) = value else {
        return Err(invalid(format!(
            r#""retention" in {place} must be an object such as {{ "keepRuns": 5 }} or {{ "keepMiB": 2 }}."#
        )));
    };
    match (rule.get("keepRuns"), rule.get("keepMiB")) {
        (Some(runs), None) => Ok(RetentionRule::Runs {
            count: whole_number(runs, "keepRuns", MAX_INT32, place)?,
        }),
        (None, Some(megabytes)) => Ok(RetentionRule::Megabytes {
            limit: whole_number(megabytes, "keepMiB", MAX_INT32 / MIB, place)?,
        }),
        (runs, _) => Err(invalid(format!(
            r#""retention" in {place} must set either "keepRuns" or "keepMiB", not {}."#,
            if runs.is_some() { "both" } else { "neither" }
        ))),
    }
}

/// A JSON number that is a whole number from 1 to `max`. JavaScript read `5.0` as the
/// whole number 5, so a float without a fraction counts.
fn whole_number(value: &Value, key: &str, max: u32, place: &str) -> Result<u32, ProjectError> {
    match value.as_f64() {
        Some(number)
            if value.is_number()
                && number.fract() == 0.0
                && number >= 1.0
                && number <= f64::from(max) =>
        {
            Ok(number as u32)
        }
        _ => Err(invalid(format!(
            r#""{key}" in {place} must be a whole number from 1 to {max}, got {}."#,
            stringify(value)
        ))),
    }
}

fn read_json(file: &Path) -> Result<Map<String, Value>, ProjectError> {
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(source) => {
            return Err(ProjectError::Io {
                path: file.to_path_buf(),
                source,
            });
        }
    };
    let value: Value = serde_json::from_str(&String::from_utf8_lossy(&bytes))
        .map_err(|error| invalid(format!("{} is not valid JSON: {error}", file.display())))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(invalid(format!(
            "{} must hold a JSON object.",
            file.display()
        ))),
    }
}

/// The last component of the absolute form of `dir`, with `.` and `..` resolved by name as
/// Node's path.resolve does; empty for a root.
fn folder_name(dir: &Path) -> String {
    let absolute = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in absolute.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::ParentDir => {
                parts.pop();
            }
            Component::CurDir | Component::Prefix(_) | Component::RootDir => {}
        }
    }
    parts
        .last()
        .map(|part| part.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_dir;

    fn assert_name_rule(message: &str) {
        assert!(
            message.contains(r#"at most 15 bytes of lowercase ASCII letters, digits, "-" or "_""#),
            "{message}"
        );
    }

    // The temporary folder's name is not a valid project name, so every project lives in a
    // named folder inside it.
    fn project_dir(folder_name: &str, config: Option<&str>) -> PathBuf {
        let dir = temp_dir("ev3c-project-").join(folder_name);
        std::fs::create_dir(&dir).unwrap();
        if let Some(text) = config {
            std::fs::write(dir.join(PROJECT_FILE), text).unwrap();
        }
        dir
    }

    fn load(folder: &str, config: Option<&str>) -> Result<ProjectConfig, ProjectError> {
        load_project_config(&project_dir(folder, config))
    }

    fn error_of(result: Result<ProjectConfig, ProjectError>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn a_missing_ev3c_json_uses_the_folder_name_and_the_default_rules() {
        assert_eq!(
            load("myrobot", None).unwrap(),
            ProjectConfig {
                name: "myrobot".into(),
                retention: RetentionRule::Runs { count: 5 },
                output_cap_bytes: 64 * 1024,
            }
        );
    }

    #[test]
    fn an_ev3c_json_without_a_name_or_rules_uses_the_folder_name_and_the_default_rules() {
        assert_eq!(
            load("line_follow-2", Some("{}")).unwrap(),
            ProjectConfig {
                name: "line_follow-2".into(),
                retention: RetentionRule::Runs { count: 5 },
                output_cap_bytes: 64 * 1024,
            }
        );
    }

    #[test]
    fn ev3c_json_sets_the_name_a_run_count_rule_and_the_output_cap() {
        let config = r#"{ "name": "arm", "retention": { "keepRuns": 12 }, "outputCapKiB": 16 }"#;
        assert_eq!(
            load("folder", Some(config)).unwrap(),
            ProjectConfig {
                name: "arm".into(),
                retention: RetentionRule::Runs { count: 12 },
                output_cap_bytes: 16 * 1024,
            }
        );
    }

    #[test]
    fn ev3c_json_sets_a_megabyte_rule() {
        let config = r#"{ "name": "arm", "retention": { "keepMiB": 2 } }"#;
        assert_eq!(
            load("folder", Some(config)).unwrap(),
            ProjectConfig {
                name: "arm".into(),
                retention: RetentionRule::Megabytes { limit: 2 },
                output_cap_bytes: 64 * 1024,
            }
        );
    }

    #[test]
    fn a_name_that_breaks_the_rule_is_rejected_with_the_rule_in_the_message() {
        for (why, name) in [
            ("uppercase letters", "MyRobot"),
            ("diacritics", "robůtek"),
            ("spaces", "my robot"),
            ("more than 15 bytes", "a-sixteen-bytesx"),
            ("nothing", ""),
        ] {
            let config = format!(r#"{{ "name": {} }}"#, quote(name));
            let message = error_of(load("folder", Some(&config)));
            assert_name_rule(&message);
            assert!(message.contains(&quote(name)), "{why}: {message}");
        }
    }

    #[test]
    fn a_15_byte_name_is_accepted() {
        let config = load("folder", Some(r#"{ "name": "abcdefghij-12_4" }"#)).unwrap();
        assert_eq!(config.name, "abcdefghij-12_4");
    }

    #[test]
    fn an_invalid_folder_name_without_ev3c_json_is_rejected_and_points_at_the_name_setting() {
        let message = error_of(load("My Robot", None));
        assert_name_rule(&message);
        assert!(message.contains(r#""name""#), "{message}");
        assert!(message.contains("ev3c.json"), "{message}");
    }

    #[test]
    fn a_name_that_is_not_a_string_or_is_null_is_rejected() {
        for config in [r#"{ "name": 5 }"#, r#"{ "name": null }"#] {
            let message = error_of(load("folder", Some(config)));
            assert!(message.contains(r#""name""#), "{message}");
            assert!(message.contains("must be a string"), "{message}");
        }
    }

    #[test]
    fn a_megabyte_rule_below_two_capped_streams_is_rejected() {
        let config = r#"{ "retention": { "keepMiB": 1 }, "outputCapKiB": 1024 }"#;
        let message = error_of(load("folder", Some(config)));
        assert!(
            message.contains("at least two capped output streams"),
            "{message}"
        );
    }

    #[test]
    fn a_megabyte_rule_of_exactly_two_capped_streams_is_accepted() {
        let config = r#"{ "retention": { "keepMiB": 1 }, "outputCapKiB": 512 }"#;
        assert_eq!(
            load("folder", Some(config)).unwrap().retention,
            RetentionRule::Megabytes { limit: 1 }
        );
    }

    #[test]
    fn both_retention_rules_at_once_are_rejected() {
        let config = r#"{ "retention": { "keepRuns": 3, "keepMiB": 2 } }"#;
        let message = error_of(load("folder", Some(config)));
        assert!(
            message.contains(r#"either "keepRuns" or "keepMiB""#),
            "{message}"
        );
    }

    #[test]
    fn a_retention_that_is_not_an_object_or_sets_no_rule_is_rejected() {
        for config in [
            r#"{ "retention": null }"#,
            r#"{ "retention": [] }"#,
            r#"{ "retention": {} }"#,
        ] {
            let message = error_of(load("folder", Some(config)));
            assert!(message.contains(r#""retention""#), "{message}");
        }
    }

    #[test]
    fn numbers_outside_their_range_are_rejected_before_building() {
        for (setting, config) in [
            ("keepRuns", r#"{ "retention": { "keepRuns": 0 } }"#),
            ("keepRuns", r#"{ "retention": { "keepRuns": 2.5 } }"#),
            ("keepRuns", r#"{ "retention": { "keepRuns": "5" } }"#),
            ("keepRuns", r#"{ "retention": { "keepRuns": null } }"#),
            ("keepMiB", r#"{ "retention": { "keepMiB": -1 } }"#),
            ("outputCapKiB", r#"{ "outputCapKiB": 0 }"#),
            ("outputCapKiB", r#"{ "outputCapKiB": 2097152 }"#),
        ] {
            let message = error_of(load("folder", Some(config)));
            assert!(message.contains(&format!(r#""{setting}""#)), "{message}");
            assert!(message.contains("whole number from 1 to"), "{message}");
        }
    }

    #[test]
    fn the_rejected_value_is_quoted_as_json() {
        let message = error_of(load(
            "folder",
            Some(r#"{ "retention": { "keepRuns": "5" } }"#),
        ));
        assert!(message.ends_with(r#"got "5"."#), "{message}");
        let message = error_of(load("folder", Some(r#"{ "outputCapKiB": 2.5 }"#)));
        assert!(message.ends_with("got 2.5."), "{message}");
    }

    #[test]
    fn a_whole_number_written_as_a_float_is_accepted() {
        let config = r#"{ "retention": { "keepRuns": 3.0 }, "outputCapKiB": 1e1 }"#;
        let config = load("folder", Some(config)).unwrap();
        assert_eq!(config.retention, RetentionRule::Runs { count: 3 });
        assert_eq!(config.output_cap_bytes, 10 * 1024);
    }

    #[test]
    fn an_ev3c_json_that_is_not_valid_json_is_rejected_with_the_file_named() {
        let message = error_of(load("folder", Some("{ name: ")));
        assert!(message.contains("ev3c.json"), "{message}");
        let message = error_of(load("folder", Some("[1]")));
        assert!(message.contains("must hold a JSON object"), "{message}");
    }

    #[test]
    fn the_folder_name_comes_from_the_resolved_path() {
        let dir = project_dir("robot", None);
        let dotted = dir.join("sub").join("..").join(".");
        assert_eq!(folder_name(&dotted), "robot");
    }

    #[test]
    fn the_config_serializes_with_the_typescript_field_names() {
        let config = ProjectConfig {
            name: "arm".into(),
            retention: RetentionRule::Megabytes { limit: 2 },
            output_cap_bytes: 4096,
        };
        assert_eq!(
            serde_json::to_value(&config).unwrap(),
            serde_json::json!({
                "name": "arm",
                "retention": { "kind": "megabytes", "limit": 2 },
                "outputCapBytes": 4096,
            })
        );
    }
}
