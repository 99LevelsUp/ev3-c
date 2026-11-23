//! Downloads the files a run returned into the project's `returned/run-<n>/` folder. The
//! paths come from the program, so each one goes through
//! [`local_path_for`] before it names a file on this computer.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::port::{BrickError, BrickFileReader};
use crate::returned::local_path_for;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedFile {
    pub brick_path: String,
    pub local_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedFailure {
    pub brick_path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReturnedReport {
    pub downloaded: Vec<ReturnedFile>,
    pub failed: Vec<ReturnedFailure>,
}

/// A download that failed: reading the brick file, or saving it on this computer.
#[derive(Debug)]
pub enum DownloadError {
    Brick(BrickError),
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::Brick(error) => error.fmt(f),
            DownloadError::Io { path, source } => {
                write!(f, "Could not save {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for DownloadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DownloadError::Brick(error) => Some(error),
            DownloadError::Io { source, .. } => Some(source),
        }
    }
}

/// Downloads every listed file into `local_dir`; a file that fails does not stop the rest.
pub async fn download_returned_files(
    reader: &impl BrickFileReader,
    project: &str,
    brick_paths: &[String],
    local_dir: &Path,
) -> ReturnedReport {
    let mut report = ReturnedReport::default();
    for brick_path in brick_paths {
        let downloaded = match local_path_for(brick_path, project, local_dir) {
            Ok(local_path) => download_file(reader, brick_path, &local_path)
                .await
                .map(|_| local_path)
                .map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        match downloaded {
            Ok(local_path) => report.downloaded.push(ReturnedFile {
                brick_path: brick_path.clone(),
                local_path,
            }),
            Err(reason) => report.failed.push(ReturnedFailure {
                brick_path: brick_path.clone(),
                reason,
            }),
        }
    }
    report
}

/// Copies one brick file to a local path, creating its folders; returns its size.
pub async fn download_file(
    reader: &impl BrickFileReader,
    brick_path: &str,
    local_path: &Path,
) -> Result<u64, DownloadError> {
    let data = reader
        .read_file(brick_path)
        .await
        .map_err(DownloadError::Brick)?;
    let io_error = |source| DownloadError::Io {
        path: local_path.to_path_buf(),
        source,
    };
    if let Some(parent) = local_path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    std::fs::write(local_path, &data).map_err(io_error)?;
    Ok(data.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick::fake::FakeBrickPort;
    use crate::brick::run_results::{Launches, read_run_result};
    use crate::returned::returned_dir;
    use crate::testutil::temp_dir;

    const PROJECT: &str = "/home/root/lms2012/prjs/robot";

    #[tokio::test(start_paused = true)]
    async fn a_runs_result_lists_its_returned_files_and_they_download_into_returned_run_n() {
        let port = FakeBrickPort::new();
        port.with(|brick| {
            brick.put_file(
                &format!("{PROJECT}/runs/3/res"),
                "run=3\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n",
            );
            brick.put_file(&format!("{PROJECT}/runs/3/out"), "");
            brick.put_file(
                &format!("{PROJECT}/runs/3/ret"),
                format!("{PROJECT}/log.csv\n/media/card/missing.txt\n"),
            );
            brick.put_file(&format!("{PROJECT}/log.csv"), "1,2\n");
        });

        let result = read_run_result(&port, &port, "robot", 3, &Launches::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            result.returned,
            [
                format!("{PROJECT}/log.csv"),
                "/media/card/missing.txt".into()
            ]
        );

        let project_dir = temp_dir("ev3c-returned-");
        let local = returned_dir(&project_dir, 3);
        let report = download_returned_files(&port, "robot", &result.returned, &local).await;
        let saved = project_dir.join("returned").join("run-3").join("log.csv");
        assert_eq!(
            report.downloaded,
            [ReturnedFile {
                brick_path: format!("{PROJECT}/log.csv"),
                local_path: saved.clone(),
            }]
        );
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "1,2\n");
        assert_eq!(
            report.failed.len(),
            1,
            "a missing file is reported and does not stop the rest"
        );
        assert_eq!(report.failed[0].brick_path, "/media/card/missing.txt");
    }

    #[tokio::test(start_paused = true)]
    async fn a_path_that_cannot_be_saved_is_reported_with_its_reason() {
        let port = FakeBrickPort::new();
        let local = temp_dir("ev3c-returned-");

        let report =
            download_returned_files(&port, "robot", &[format!("{PROJECT}/")], &local).await;

        assert!(report.downloaded.is_empty());
        assert_eq!(
            report.failed,
            [ReturnedFailure {
                brick_path: format!("{PROJECT}/"),
                reason: format!("\"{PROJECT}/\" does not name a file that can be saved."),
            }]
        );
        assert!(port.calls().is_empty(), "nothing is read from the brick");
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_without_a_return_list_returned_nothing() {
        let port = FakeBrickPort::new();
        port.with(|brick| {
            brick.put_file(
                &format!("{PROJECT}/runs/1/res"),
                "run=1\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n",
            );
        });

        let result = read_run_result(&port, &port, "robot", 1, &Launches::new())
            .await
            .unwrap()
            .unwrap();

        assert!(result.returned.is_empty());
    }
}
