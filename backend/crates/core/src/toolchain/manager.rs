//! Obtains the pinned cross-compiler. The compiler is not bundled with EVƎ C: it is
//! downloaded on first use from the URL in manifest.json, verified against the pinned SHA-256
//! before it is extracted, and kept in a cache folder the host supplies. A host setting can
//! point at a compiler instead, which skips the download.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// manifest.json of the TypeScript backend, the single place that pins the compiler and
/// defines what a brick binary is. It is read from there, not copied.
const MANIFEST_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../src/core/toolchain/manifest.json"
));

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HostToolchain {
    /// Archive file name, appended to the manifest's base URL.
    pub file: String,
    pub sha256: String,
    /// Path of the compiler executable inside the extracted archive, with `/` separators.
    pub entry: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainManifest {
    pub compiler: String,
    pub version: String,
    pub base_url: String,
    /// Keyed by Node's `<platform>-<arch>` names, such as `win32-x64`; see [`host_key`].
    pub hosts: BTreeMap<String, HostToolchain>,
    pub target_flags: Vec<String>,
}

impl ToolchainManifest {
    /// The manifest built into this program.
    pub fn bundled() -> Result<&'static ToolchainManifest, ToolchainError> {
        static BUNDLED: OnceLock<Result<ToolchainManifest, String>> = OnceLock::new();
        BUNDLED
            .get_or_init(|| serde_json::from_str(MANIFEST_JSON).map_err(|error| error.to_string()))
            .as_ref()
            .map_err(|error| ToolchainError::Manifest(error.clone()))
    }
}

/// Fetches a URL into a file.
pub type Downloader<'a> = dyn Fn(&str, &Path) -> Result<(), ToolchainError> + 'a;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerOverride {
    pub path: PathBuf,
    /// Name of the host setting the path came from, quoted in errors.
    pub setting: String,
}

#[derive(Default)]
pub struct EnsureCompilerOptions<'a> {
    pub cache_dir: PathBuf,
    pub compiler_override: Option<CompilerOverride>,
    /// Replaces the HTTPS download, for tests.
    pub download: Option<&'a Downloader<'a>>,
    /// Replaces the bundled manifest, for tests.
    pub manifest: Option<&'a ToolchainManifest>,
    /// Manifest host key such as `win32-x64`; defaults to the running host.
    pub host: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompilerOrigin {
    Override,
    Cache,
    Download,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompilerInfo {
    pub path: PathBuf,
    /// The pinned version; unknown for a compiler set by the host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub origin: CompilerOrigin,
}

/// Why the compiler could not be obtained or run.
#[derive(Debug)]
pub enum ToolchainError {
    OverrideMissing {
        setting: String,
        path: PathBuf,
    },
    NotPinned {
        compiler: String,
        version: String,
        host: String,
    },
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    MissingEntry {
        file: String,
        entry: String,
    },
    /// The download failed; the message names the URL.
    Download(String),
    Extract {
        archive: PathBuf,
        detail: String,
    },
    /// A file operation failed; `context` says which and on what.
    Io {
        context: String,
        source: io::Error,
    },
    Manifest(String),
}

impl fmt::Display for ToolchainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolchainError::OverrideMissing { setting, path } => write!(
                f,
                "The compiler set in \"{setting}\" does not exist: {}",
                path.display()
            ),
            ToolchainError::NotPinned {
                compiler,
                version,
                host,
            } => write!(
                f,
                "No {compiler} {version} build is pinned for this host ({host})."
            ),
            ToolchainError::ChecksumMismatch {
                file,
                expected,
                actual,
            } => write!(
                f,
                "Downloaded {file} does not match the pinned checksum and was deleted. Expected SHA-256 {expected}, got {actual}."
            ),
            ToolchainError::MissingEntry { file, entry } => {
                write!(f, "{file} does not contain {entry}.")
            }
            ToolchainError::Download(message) => f.write_str(message),
            ToolchainError::Extract { archive, detail } => {
                write!(f, "Extracting {} failed: {detail}", archive.display())
            }
            ToolchainError::Io { context, source } => write!(f, "{context}: {source}"),
            ToolchainError::Manifest(message) => {
                write!(f, "The bundled toolchain manifest is not valid: {message}")
            }
        }
    }
}

impl std::error::Error for ToolchainError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ToolchainError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) fn io_error(context: String) -> impl FnOnce(io::Error) -> ToolchainError {
    move |source| ToolchainError::Io { context, source }
}

/// The manifest key of the running host, in Node's `<process.platform>-<process.arch>` form
/// that the manifest and the TypeScript backend use.
pub fn host_key() -> String {
    let platform = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    format!("{platform}-{arch}")
}

pub fn ensure_compiler(options: &EnsureCompilerOptions) -> Result<CompilerInfo, ToolchainError> {
    if let Some(CompilerOverride { path, setting }) = &options.compiler_override {
        if !exists(path) {
            return Err(ToolchainError::OverrideMissing {
                setting: setting.clone(),
                path: path.clone(),
            });
        }
        return Ok(CompilerInfo {
            path: path.clone(),
            version: None,
            origin: CompilerOrigin::Override,
        });
    }

    let pinned = match options.manifest {
        Some(manifest) => manifest,
        None => ToolchainManifest::bundled()?,
    };
    let host = options.host.clone().unwrap_or_else(host_key);
    let Some(entry) = pinned.hosts.get(&host) else {
        return Err(ToolchainError::NotPinned {
            compiler: pinned.compiler.clone(),
            version: pinned.version.clone(),
            host,
        });
    };

    let install_dir = options
        .cache_dir
        .join(format!("{}-{}-{host}", pinned.compiler, pinned.version));
    let compiler_path = entry
        .entry
        .split('/')
        .fold(install_dir.clone(), |path, part| path.join(part));
    if exists(&compiler_path) {
        return Ok(CompilerInfo {
            path: compiler_path,
            version: Some(pinned.version.clone()),
            origin: CompilerOrigin::Cache,
        });
    }

    fs::create_dir_all(&options.cache_dir).map_err(io_error(format!(
        "Creating {} failed",
        options.cache_dir.display()
    )))?;
    let archive = options.cache_dir.join(&entry.file);
    let mut origin = CompilerOrigin::Cache;
    if !exists(&archive) || sha256_of(&archive)? != entry.sha256 {
        origin = CompilerOrigin::Download;
        let url = format!("{}{}", pinned.base_url, entry.file);
        match options.download {
            Some(download) => download(&url, &archive)?,
            None => fetch_to_file(&url, &archive)?,
        }
        let actual = sha256_of(&archive)?;
        if actual != entry.sha256 {
            remove_file_if_present(&archive)?;
            return Err(ToolchainError::ChecksumMismatch {
                file: entry.file.clone(),
                expected: entry.sha256.clone(),
                actual,
            });
        }
    }

    // Extract next to the final folder and rename it into place, so an interrupted
    // extraction is never mistaken for a usable compiler.
    let mut partial = OsString::from(install_dir.as_os_str());
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    remove_dir_if_present(&partial)?;
    fs::create_dir_all(&partial)
        .map_err(io_error(format!("Creating {} failed", partial.display())))?;
    extract(&archive, &partial)?;
    remove_dir_if_present(&install_dir)?;
    fs::rename(&partial, &install_dir).map_err(io_error(format!(
        "Renaming {} to {} failed",
        partial.display(),
        install_dir.display()
    )))?;
    if !exists(&compiler_path) {
        return Err(ToolchainError::MissingEntry {
            file: entry.file.clone(),
            entry: entry.entry.clone(),
        });
    }
    Ok(CompilerInfo {
        path: compiler_path,
        version: Some(pinned.version.clone()),
        origin,
    })
}

/// The tar that extracts the archive. Windows ships bsdtar in System32, which extracts both
/// .zip and .tar.xz. Name it explicitly: a GNU tar earlier on PATH (Git for Windows) cannot
/// read .zip.
pub(crate) fn system_tar() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
        Path::new(&root).join("System32").join("tar.exe")
    } else {
        PathBuf::from("tar")
    }
}

fn extract(archive: &Path, into: &Path) -> Result<(), ToolchainError> {
    let tar = system_tar();
    let mut command = Command::new(&tar);
    command.arg("-xf").arg(archive).arg("-C").arg(into);
    super::hide_window(&mut command);
    let output = command
        .output()
        .map_err(io_error(format!("Starting {} failed", tar.display())))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolchainError::Extract {
            archive: archive.to_path_buf(),
            detail: format!(
                "{} exited with {}: {}",
                tar.display(),
                output.status,
                stderr.trim()
            ),
        });
    }
    Ok(())
}

fn fetch_to_file(url: &str, destination: &Path) -> Result<(), ToolchainError> {
    let failed =
        |detail: String| ToolchainError::Download(format!("Downloading {url} failed: {detail}"));
    let response = ureq::get(url).call().map_err(|error| match error {
        ureq::Error::StatusCode(status) => failed(format!("HTTP {status}.")),
        other => failed(format!("{other}.")),
    })?;
    // The archive is far larger than ureq's in-memory body limit, so it is streamed.
    let mut reader = response.into_body().into_reader();
    let mut file = fs::File::create(destination).map_err(io_error(format!(
        "Creating {} failed",
        destination.display()
    )))?;
    if let Err(error) = io::copy(&mut reader, &mut file) {
        drop(file);
        // A part of the archive is not worth keeping; its checksum would not match anyway.
        let _ = fs::remove_file(destination);
        return Err(failed(format!("{error}.")));
    }
    Ok(())
}

fn sha256_of(file: &Path) -> Result<String, ToolchainError> {
    let mut input =
        fs::File::open(file).map_err(io_error(format!("Opening {} failed", file.display())))?;
    let mut hasher = Sha256::new();
    io::copy(&mut input, &mut hasher)
        .map_err(io_error(format!("Reading {} failed", file.display())))?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn exists(path: &Path) -> bool {
    fs::metadata(path).is_ok()
}

fn remove_file_if_present(path: &Path) -> Result<(), ToolchainError> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(ToolchainError::Io {
            context: format!("Deleting {} failed", path.display()),
            source: error,
        }),
        _ => Ok(()),
    }
}

fn remove_dir_if_present(path: &Path) -> Result<(), ToolchainError> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(ToolchainError::Io {
            context: format!("Deleting {} failed", path.display()),
            source: error,
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::testutil::temp_dir;

    const HOST: &str = "test-host";
    const ARCHIVE: &str = "fake-zig.tar.xz";
    const ENTRY: &str = "fake-zig/zig";

    // A local stand-in for the Zig archive: one folder holding a fake compiler, packed with
    // the same tar the manager uses.
    fn make_fake_archive() -> (PathBuf, String) {
        let work = temp_dir("ev3c-toolchain-");
        fs::create_dir(work.join("fake-zig")).unwrap();
        fs::write(work.join("fake-zig").join("zig"), "fake compiler\n").unwrap();
        let file = work.join(ARCHIVE);
        let status = Command::new(system_tar())
            .arg("-cJf")
            .arg(&file)
            .arg("-C")
            .arg(&work)
            .arg("fake-zig")
            .status()
            .unwrap();
        assert!(status.success());
        let sha256 = sha256_of(&file).unwrap();
        (file, sha256)
    }

    fn manifest_for(sha256: &str) -> ToolchainManifest {
        ToolchainManifest {
            compiler: "zig".into(),
            version: "9.9.9".into(),
            base_url: "https://example.invalid/zig/".into(),
            hosts: BTreeMap::from([(
                HOST.to_owned(),
                HostToolchain {
                    file: ARCHIVE.into(),
                    sha256: sha256.into(),
                    entry: ENTRY.into(),
                },
            )]),
            target_flags: Vec::new(),
        }
    }

    /// A downloader that records each URL and copies `source`, or fails without one.
    struct Recording {
        source: Option<PathBuf>,
        urls: RefCell<Vec<String>>,
    }

    impl Recording {
        fn new(source: Option<&Path>) -> Self {
            Self {
                source: source.map(Path::to_path_buf),
                urls: RefCell::new(Vec::new()),
            }
        }

        fn download(&self, url: &str, destination: &Path) -> Result<(), ToolchainError> {
            self.urls.borrow_mut().push(url.to_owned());
            match &self.source {
                Some(source) => {
                    fs::copy(source, destination).unwrap();
                    Ok(())
                }
                None => Err(ToolchainError::Download(
                    "network must not be used".to_owned(),
                )),
            }
        }
    }

    fn options<'a>(
        cache_dir: &Path,
        download: &'a Downloader<'a>,
        manifest: &'a ToolchainManifest,
    ) -> EnsureCompilerOptions<'a> {
        EnsureCompilerOptions {
            cache_dir: cache_dir.to_path_buf(),
            download: Some(download),
            manifest: Some(manifest),
            host: Some(HOST.to_owned()),
            ..EnsureCompilerOptions::default()
        }
    }

    #[test]
    fn a_cached_archive_with_a_matching_hash_is_reused_without_a_download() {
        let (archive, sha256) = make_fake_archive();
        let cache_dir = temp_dir("ev3c-toolchain-");
        fs::copy(&archive, cache_dir.join(ARCHIVE)).unwrap();
        let recording = Recording::new(None);
        let download = |url: &str, to: &Path| recording.download(url, to);
        let manifest = manifest_for(&sha256);

        let compiler = ensure_compiler(&options(&cache_dir, &download, &manifest)).unwrap();

        assert!(recording.urls.borrow().is_empty());
        assert_eq!(compiler.origin, CompilerOrigin::Cache);
        assert_eq!(
            fs::read_to_string(&compiler.path).unwrap(),
            "fake compiler\n"
        );
    }

    #[test]
    fn a_missing_archive_is_downloaded_from_the_manifest_url_extracted_and_then_reused() {
        let (archive, sha256) = make_fake_archive();
        let cache_dir = temp_dir("ev3c-toolchain-");
        let recording = Recording::new(Some(&archive));
        let download = |url: &str, to: &Path| recording.download(url, to);
        let manifest = manifest_for(&sha256);
        let options = options(&cache_dir, &download, &manifest);

        let first = ensure_compiler(&options).unwrap();
        let second = ensure_compiler(&options).unwrap();

        assert_eq!(
            *recording.urls.borrow(),
            ["https://example.invalid/zig/fake-zig.tar.xz"]
        );
        assert_eq!(first.origin, CompilerOrigin::Download);
        assert_eq!(first.version.as_deref(), Some("9.9.9"));
        assert_eq!(second.origin, CompilerOrigin::Cache);
        assert_eq!(second.path, first.path);
        assert_eq!(fs::read_to_string(&first.path).unwrap(), "fake compiler\n");
        assert_eq!(
            first.path,
            cache_dir
                .join("zig-9.9.9-test-host")
                .join("fake-zig")
                .join("zig")
        );
    }

    #[test]
    fn a_cached_archive_with_a_wrong_hash_is_replaced_by_a_fresh_download() {
        let (archive, sha256) = make_fake_archive();
        let cache_dir = temp_dir("ev3c-toolchain-");
        fs::write(cache_dir.join(ARCHIVE), "partial download").unwrap();
        let recording = Recording::new(Some(&archive));
        let download = |url: &str, to: &Path| recording.download(url, to);
        let manifest = manifest_for(&sha256);

        let compiler = ensure_compiler(&options(&cache_dir, &download, &manifest)).unwrap();

        assert_eq!(recording.urls.borrow().len(), 1);
        assert_eq!(compiler.origin, CompilerOrigin::Download);
        assert_eq!(
            fs::read_to_string(&compiler.path).unwrap(),
            "fake compiler\n"
        );
    }

    #[test]
    fn a_downloaded_archive_with_a_wrong_hash_is_deleted_and_the_error_names_both_hashes() {
        let (archive, sha256) = make_fake_archive();
        let cache_dir = temp_dir("ev3c-toolchain-");
        let expected = "0".repeat(64);
        let recording = Recording::new(Some(&archive));
        let download = |url: &str, to: &Path| recording.download(url, to);
        let manifest = manifest_for(&expected);

        let message = ensure_compiler(&options(&cache_dir, &download, &manifest))
            .unwrap_err()
            .to_string();

        assert!(message.contains(&expected), "{message}");
        assert!(message.contains(&sha256), "{message}");
        assert!(!cache_dir.join(ARCHIVE).exists());
    }

    #[test]
    fn a_failed_download_is_reported() {
        let cache_dir = temp_dir("ev3c-toolchain-");
        let recording = Recording::new(None);
        let download = |url: &str, to: &Path| recording.download(url, to);
        let manifest = manifest_for(&"0".repeat(64));

        let message = ensure_compiler(&options(&cache_dir, &download, &manifest))
            .unwrap_err()
            .to_string();

        assert_eq!(message, "network must not be used");
    }

    #[test]
    fn an_override_path_that_exists_is_used_without_a_download() {
        let cache_dir = temp_dir("ev3c-toolchain-");
        let compiler_path = temp_dir("ev3c-toolchain-").join("zig");
        fs::write(&compiler_path, "").unwrap();
        let recording = Recording::new(None);
        let download = |url: &str, to: &Path| recording.download(url, to);
        let manifest = manifest_for(&"0".repeat(64));

        let compiler = ensure_compiler(&EnsureCompilerOptions {
            compiler_override: Some(CompilerOverride {
                path: compiler_path.clone(),
                setting: "ev3c.compilerPath".into(),
            }),
            ..options(&cache_dir, &download, &manifest)
        })
        .unwrap();

        assert!(recording.urls.borrow().is_empty());
        assert_eq!(compiler.origin, CompilerOrigin::Override);
        assert_eq!(compiler.path, compiler_path);
        assert_eq!(compiler.version, None);
    }

    #[test]
    fn an_override_path_that_does_not_exist_fails_with_a_message_naming_the_setting() {
        let missing = temp_dir("ev3c-toolchain-").join("no-such-zig");
        let recording = Recording::new(None);
        let download = |url: &str, to: &Path| recording.download(url, to);
        let manifest = manifest_for(&"0".repeat(64));

        let message = ensure_compiler(&EnsureCompilerOptions {
            compiler_override: Some(CompilerOverride {
                path: missing.clone(),
                setting: "ev3c.compilerPath".into(),
            }),
            ..options(&temp_dir("ev3c-toolchain-"), &download, &manifest)
        })
        .unwrap_err()
        .to_string();

        assert!(message.contains("ev3c.compilerPath"), "{message}");
        assert!(
            message.contains(&missing.display().to_string()),
            "{message}"
        );
    }

    #[test]
    fn a_host_without_a_pinned_build_is_refused() {
        let recording = Recording::new(None);
        let download = |url: &str, to: &Path| recording.download(url, to);
        let manifest = manifest_for(&"0".repeat(64));

        let message = ensure_compiler(&EnsureCompilerOptions {
            host: Some("plan9-mips".into()),
            ..options(&temp_dir("ev3c-toolchain-"), &download, &manifest)
        })
        .unwrap_err()
        .to_string();

        assert_eq!(
            message,
            "No zig 9.9.9 build is pinned for this host (plan9-mips)."
        );
    }

    #[test]
    fn the_bundled_manifest_pins_zig_0_14_1_for_every_supported_host() {
        let manifest = ToolchainManifest::bundled().unwrap();
        assert_eq!(manifest.compiler, "zig");
        assert_eq!(manifest.version, "0.14.1");
        let hosts: Vec<&str> = manifest.hosts.keys().map(String::as_str).collect();
        assert_eq!(
            hosts,
            ["darwin-arm64", "darwin-x64", "linux-x64", "win32-x64"]
        );
        for entry in manifest.hosts.values() {
            assert_eq!(entry.sha256.len(), 64);
            assert!(entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert!(
            manifest
                .target_flags
                .contains(&"arm-linux-musleabi".to_owned())
        );
    }

    #[test]
    fn the_host_key_uses_the_names_node_gives_the_platform_and_architecture() {
        let key = host_key();
        if cfg!(all(windows, target_arch = "x86_64")) {
            assert_eq!(key, "win32-x64");
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            assert_eq!(key, "linux-x64");
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(key, "darwin-arm64");
        }
        assert!(!key.contains("windows") && !key.contains("x86_64"), "{key}");
    }
}
