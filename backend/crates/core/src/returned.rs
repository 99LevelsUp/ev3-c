//! Files a run hands back to its author. ev3lib appends each marked file's full brick path
//! to the run folder's list, `ret`; after the run, EVƎ C downloads those files into the
//! project's `returned/run-<n>/` folder. The list comes from the program, so every path is
//! checked before it names a file on this computer.
//!
//! Only the path rules live here; downloading comes with the BrickPort.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::build::brick_project_dir;

/// Name of the return list inside a run folder.
pub const RETURN_LIST_FILE: &str = "ret";

/// Where a run's returned files land, below the project folder on this computer.
pub const RETURNED_DIR: &str = "returned";

/// The brick reaches the projects folder through this link as well (`prjs` lives in RAM).
const PROJECTS_ALIAS: &str = "/mnt/ramdisk/prjs";

/// A returned brick path that cannot name a local file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedPathError(String);

impl fmt::Display for ReturnedPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ReturnedPathError {}

/// The paths a return list names: absolute ones only, each once, in the order marked.
pub fn parse_return_list(text: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let brick_path = js_trim(line);
        if brick_path.starts_with('/') {
            let normalized = posix_normalize(brick_path);
            if !paths.contains(&normalized) {
                paths.push(normalized);
            }
        }
    }
    paths
}

/// The local folder for a run's returned files.
pub fn returned_dir(project_dir: &Path, run: i64) -> PathBuf {
    project_dir.join(RETURNED_DIR).join(format!("run-{run}"))
}

/// Where a returned brick file lands below `local_dir`: its path inside the brick project
/// folder when it is there, otherwise its whole brick path below `brick/`. Characters this
/// computer cannot use in a name become `_`. A path that would leave `local_dir` is refused.
pub fn local_path_for(
    brick_path: &str,
    project: &str,
    local_dir: &Path,
) -> Result<PathBuf, ReturnedPathError> {
    let normalized = posix_normalize(brick_path);
    if !normalized.starts_with('/') {
        return Err(ReturnedPathError(format!(
            "\"{brick_path}\" is not an absolute brick path."
        )));
    }
    let project_dirs = [
        brick_project_dir(project),
        format!("{PROJECTS_ALIAS}/{project}"),
    ];
    let relative = project_dirs
        .iter()
        .find_map(|dir| {
            normalized
                .strip_prefix(dir.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
        })
        .map_or_else(|| format!("brick{normalized}"), str::to_owned);
    let segments: Vec<&str> = relative
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.is_empty() || segments.contains(&"..") {
        return Err(ReturnedPathError(format!(
            "\"{brick_path}\" does not name a file that can be saved."
        )));
    }
    let local = segments
        .iter()
        .fold(local_dir.to_path_buf(), |path, segment| {
            path.join(safe_name(segment))
        });
    let inside = local.strip_prefix(local_dir).is_ok_and(|rest| {
        !rest.as_os_str().is_empty()
            && rest
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
    });
    if !inside {
        return Err(ReturnedPathError(format!(
            "\"{brick_path}\" would be saved outside {}.",
            local_dir.display()
        )));
    }
    Ok(local)
}

fn safe_name(segment: &str) -> String {
    // Windows refuses these and control characters in names, and drops a trailing dot or
    // space silently.
    let kept: String = segment
        .chars()
        .map(|ch| {
            if u32::from(ch) < 0x20 || "<>:\"\\|?*".contains(ch) {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let body = kept.trim_end_matches(['.', ' ']);
    let trailing = kept[body.len()..].chars().count();
    format!("{body}{}", "_".repeat(trailing))
}

/// String.prototype.trim: Unicode white space and line ends, plus the byte-order mark,
/// but not U+0085, which Rust counts as white space and JavaScript does not.
fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}'))
}

/// Node's path.posix.normalize: collapses repeated slashes, drops `.`, resolves `..` (never
/// above the root of an absolute path), and keeps a trailing slash. Brick paths are POSIX
/// paths on every host, so this works on the string, not on `std::path`.
fn posix_normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let mut normalized = parts.join("/");
    if normalized.is_empty() {
        return match (absolute, trailing) {
            (true, _) => "/".to_owned(),
            (false, true) => "./".to_owned(),
            (false, false) => ".".to_owned(),
        };
    }
    if trailing {
        normalized.push('/');
    }
    if absolute {
        normalized.insert(0, '/');
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT: &str = "/home/root/lms2012/prjs/robot";

    fn local() -> PathBuf {
        std::env::temp_dir().join("ev3c-returned-fixed")
    }

    fn join(base: &Path, parts: &[&str]) -> PathBuf {
        parts
            .iter()
            .fold(base.to_path_buf(), |path, part| path.join(part))
    }

    #[test]
    fn a_return_list_keeps_absolute_paths_once_each_in_the_order_marked() {
        assert_eq!(
            parse_return_list("/a/b.txt\nrelative.txt\n\n/a/./b.txt\n/c/d\r\n/a/x/../e\n"),
            ["/a/b.txt", "/c/d", "/a/e"]
        );
    }

    #[test]
    fn a_file_in_the_project_lands_at_its_path_in_the_project_any_other_below_brick() {
        let local = local();
        assert_eq!(
            local_path_for(&format!("{PROJECT}/data/log.csv"), "robot", &local),
            Ok(join(&local, &["data", "log.csv"]))
        );
        assert_eq!(
            local_path_for("/mnt/ramdisk/prjs/robot/log.csv", "robot", &local),
            Ok(join(&local, &["log.csv"])),
            "prjs reached through RAM"
        );
        assert_eq!(
            local_path_for("/media/card/out.txt", "robot", &local),
            Ok(join(&local, &["brick", "media", "card", "out.txt"]))
        );
        assert_eq!(
            local_path_for("/home/root/lms2012/prjs/robotics/x", "robot", &local),
            Ok(join(
                &local,
                &["brick", "home", "root", "lms2012", "prjs", "robotics", "x"]
            )),
            "a sibling project is not this project"
        );
    }

    #[test]
    fn a_returned_path_never_leaves_the_local_folder_and_never_names_it() {
        let local = local();
        for bad in ["relative.txt".to_owned(), format!("{PROJECT}/")] {
            assert!(local_path_for(&bad, "robot", &local).is_err(), "{bad}");
        }
        // Normalized first, so climbing out of the project ends up below brick/, still inside.
        assert_eq!(
            local_path_for(
                &format!("{PROJECT}/../../../../../../etc/passwd"),
                "robot",
                &local
            ),
            Ok(join(&local, &["brick", "etc", "passwd"]))
        );
    }

    #[test]
    fn names_this_computer_cannot_hold_are_made_safe() {
        let local = local();
        assert_eq!(
            local_path_for(&format!("{PROJECT}/a:b*c?.txt"), "robot", &local),
            Ok(join(&local, &["a_b_c_.txt"]))
        );
        assert_eq!(
            local_path_for(&format!("{PROJECT}/end. "), "robot", &local),
            Ok(join(&local, &["end__"]))
        );
        assert_eq!(safe_name("..."), "___");
        assert_eq!(safe_name("tab\there"), "tab_here");
    }

    #[test]
    fn a_runs_returned_files_go_into_returned_run_n() {
        let project_dir = Path::new("project");
        assert_eq!(
            returned_dir(project_dir, 3),
            join(project_dir, &["returned", "run-3"])
        );
    }

    #[test]
    fn posix_normalize_matches_node() {
        for (path, expected) in [
            ("/a//b/./c/", "/a/b/c/"),
            ("/a/b/../../..", "/"),
            ("/..", "/"),
            ("//a", "/a"),
            ("a/../..", ".."),
            ("a/..", "."),
            ("a/../", "./"),
            ("", "."),
            (
                "/home/root/lms2012/prjs/robot/",
                "/home/root/lms2012/prjs/robot/",
            ),
        ] {
            assert_eq!(posix_normalize(path), expected, "{path}");
        }
    }

    #[test]
    fn trimming_matches_javascript() {
        assert_eq!(js_trim("\u{feff} /a\r\n"), "/a");
        assert_eq!(js_trim("\u{85}/a"), "\u{85}/a");
    }
}
