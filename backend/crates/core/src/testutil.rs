//! Helpers shared by the unit tests.

use std::path::{Path, PathBuf};

/// A file in the repository, for fixtures the TypeScript tests use as well.
pub(crate) fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(relative)
}
