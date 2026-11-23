//! Where things are on this computer: EVƎ C's install folder, the compiler cache, and paths
//! as the commands and the MCP server compare and print them.

use std::path::{Component, Path, PathBuf};

/// Overrides the search for EVƎ C's install folder.
pub const PACKAGE_ROOT_VARIABLE: &str = "EV3C_PACKAGE_ROOT";

/// EVƎ C's install folder: `EV3C_PACKAGE_ROOT` when set, else the nearest folder above the
/// program with ev3lib's headers (`ev3lib/include`) and the brick runtime (`out/runtime`).
pub fn package_root(variable: Option<&str>, program: &Path) -> Result<PathBuf, String> {
    match variable.map(str::trim) {
        Some(root) if !root.is_empty() => Ok(PathBuf::from(root)),
        _ => find_package_root(program),
    }
}

/// The nearest folder at or above `from` that holds `ev3lib/include` and `out/runtime`.
pub fn find_package_root(from: &Path) -> Result<PathBuf, String> {
    from.ancestors()
        .find(|dir| dir.join("ev3lib").join("include").is_dir() && dir.join("out").join("runtime").is_dir())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "EVƎ C's files (ev3lib/include and out/runtime) were not found above {}; set {PACKAGE_ROOT_VARIABLE} to the folder that holds them.",
                from.display()
            )
        })
}

/// Where the downloaded compiler is kept for this user, by the convention of `os` (Rust's
/// `std::env::consts::OS` names). `variable` reads an environment variable.
pub fn toolchain_cache_dir(
    variable: &dyn Fn(&str) -> Option<String>,
    os: &str,
    home: &Path,
) -> PathBuf {
    let set = |name: &str| {
        variable(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let base = match os {
        "windows" => set("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local")),
        "macos" => home.join("Library").join("Caches"),
        _ => set("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")),
    };
    base.join("ev3-c").join("toolchain")
}

/// `path` against `base` with `.` and `..` worked out from the text alone, like Node's
/// `path.resolve`: the commands print and compare these paths, so `./x/..` must not show.
pub fn resolve(base: &Path, path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in base.join(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // The root's parent is the root, as in Node.
                if resolved.file_name().is_some() {
                    resolved.pop();
                }
            }
            other => resolved.push(other),
        }
    }
    resolved
}

/// The path with links resolved, in the usual spelling: Windows' canonical form starts with
/// `\\?\`, which would leak into every path printed or sent to cockpit, so it is taken off
/// where a plain path means the same. A path that cannot be resolved is returned as given.
pub fn real_path(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(real) => plain(real),
        Err(_) => path.to_path_buf(),
    }
}

fn plain(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// The local path of a `file:` URI, as an MCP client declares its roots, or `None` when the
/// URI names none: another scheme, or a host this system cannot reach as a path. VS Code
/// sends `file:///d%3A/folder`, so the path is percent-decoded.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file:")?;
    let (host, path) = match rest.strip_prefix("//") {
        Some(after) => match after.find('/') {
            Some(slash) => (&after[..slash], &after[slash..]),
            None => (after, "/"),
        },
        None => ("", rest),
    };
    // A query or fragment is no part of the path.
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let host = if host.eq_ignore_ascii_case("localhost") {
        ""
    } else {
        host
    };
    let encoded_separator = |text: &str| {
        let lower = text.to_ascii_lowercase();
        lower.contains("%2f") || (cfg!(windows) && lower.contains("%5c"))
    };
    if encoded_separator(path) {
        return None;
    }
    let path = percent_decode(path)?;
    if cfg!(windows) {
        let path = path.replace('/', "\\");
        if !host.is_empty() {
            return Some(PathBuf::from(format!(r"\\{host}{path}")));
        }
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0] == b'\\'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':';
        drive.then(|| PathBuf::from(&path[1..]))
    } else {
        host.is_empty().then(|| PathBuf::from(path))
    }
}

/// `%xx` escapes decoded as UTF-8; `None` when they do not make valid UTF-8 text.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                i += 3;
            }
            None => {
                decoded.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::temp_dir;

    #[test]
    fn the_compiler_cache_follows_each_oss_convention() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        let home = Path::new("/home/u");
        assert_eq!(
            toolchain_cache_dir(&env(&[("LOCALAPPDATA", "/L")]), "windows", home),
            Path::new("/L").join("ev3-c").join("toolchain")
        );
        assert_eq!(
            toolchain_cache_dir(&env(&[]), "windows", home),
            home.join("AppData")
                .join("Local")
                .join("ev3-c")
                .join("toolchain")
        );
        assert_eq!(
            toolchain_cache_dir(&env(&[]), "macos", home),
            home.join("Library")
                .join("Caches")
                .join("ev3-c")
                .join("toolchain")
        );
        assert_eq!(
            toolchain_cache_dir(&env(&[("XDG_CACHE_HOME", "/x")]), "linux", home),
            Path::new("/x").join("ev3-c").join("toolchain")
        );
        assert_eq!(
            toolchain_cache_dir(&env(&[]), "linux", home),
            home.join(".cache").join("ev3-c").join("toolchain")
        );
    }

    #[test]
    fn the_install_folder_is_the_nearest_one_above_with_the_headers_and_the_runtime() {
        let root = temp_dir("ev3c-root-");
        let bin = root.join("backend").join("target").join("debug");
        std::fs::create_dir_all(&bin).unwrap();
        assert!(
            find_package_root(&bin)
                .unwrap_err()
                .contains("ev3lib/include")
        );

        std::fs::create_dir_all(root.join("ev3lib").join("include")).unwrap();
        assert!(
            find_package_root(&bin).is_err(),
            "the headers alone are not an install folder"
        );
        std::fs::create_dir_all(root.join("out").join("runtime")).unwrap();
        assert_eq!(find_package_root(&bin).unwrap(), root);
        assert_eq!(package_root(None, &bin).unwrap(), root);
        assert_eq!(
            package_root(Some(" /elsewhere "), &bin).unwrap(),
            Path::new("/elsewhere"),
            "EV3C_PACKAGE_ROOT wins"
        );
        assert_eq!(
            package_root(Some(""), &bin).unwrap(),
            root,
            "an empty one counts as unset"
        );
    }

    #[test]
    fn resolving_works_out_dot_and_dot_dot_like_node() {
        let base = Path::new("/work/projects");
        assert_eq!(resolve(base, Path::new(".")), Path::new("/work/projects"));
        assert_eq!(
            resolve(base, Path::new("../robot/./src")),
            Path::new("/work/robot/src")
        );
        assert_eq!(resolve(base, Path::new("/abs/x/..")), Path::new("/abs"));
        assert_eq!(resolve(Path::new("/"), Path::new("../..")), Path::new("/"));
    }

    #[test]
    fn a_real_path_has_no_verbatim_prefix() {
        let dir = temp_dir("ev3c-real-");
        let real = real_path(&dir.join("."));
        assert!(
            !real.to_string_lossy().starts_with(r"\\?\"),
            "{}",
            real.display()
        );
        assert_eq!(real, real_path(&real), "resolving again changes nothing");
        assert_eq!(
            real_path(Path::new("/no/such/folder/here")),
            Path::new("/no/such/folder/here")
        );
        assert_eq!(
            plain(PathBuf::from(r"\\?\D:\work")),
            PathBuf::from(r"D:\work")
        );
        assert_eq!(
            plain(PathBuf::from(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
    }

    #[test]
    fn file_uris_become_local_paths() {
        assert_eq!(file_uri_to_path("https://example.com/x"), None);
        assert_eq!(
            file_uri_to_path("file:///a%2Fb"),
            None,
            "an encoded separator"
        );
        if cfg!(windows) {
            assert_eq!(
                file_uri_to_path("file:///d%3A/My%20Robots/r%C5%AFj"),
                Some(PathBuf::from(r"d:\My Robots\růj"))
            );
            assert_eq!(
                file_uri_to_path("file:///C:/work"),
                Some(PathBuf::from(r"C:\work"))
            );
            assert_eq!(
                file_uri_to_path("file://server/share/x"),
                Some(PathBuf::from(r"\\server\share\x"))
            );
            assert_eq!(file_uri_to_path("file:///no-drive"), None);
        } else {
            assert_eq!(
                file_uri_to_path("file:///home/u/My%20Robots"),
                Some(PathBuf::from("/home/u/My Robots"))
            );
            assert_eq!(
                file_uri_to_path("file://localhost/tmp"),
                Some(PathBuf::from("/tmp"))
            );
            assert_eq!(file_uri_to_path("file://server/share"), None);
        }
    }
}
