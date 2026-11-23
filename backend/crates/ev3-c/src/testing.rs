//! Helpers shared by the unit tests.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A new, empty folder below the system's temporary folder, with links resolved so it reads
/// as the commands print it. Tests run in parallel threads, so the name carries the process
/// ID and a counter.
pub(crate) fn temp_dir(prefix: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "{prefix}{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // Left over from an earlier run with the same process ID.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    crate::paths::real_path(&dir)
}
