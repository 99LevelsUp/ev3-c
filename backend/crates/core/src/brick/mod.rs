//! What EVƎ C does on a brick through the [`BrickPort`](crate::port::BrickPort): upload a
//! project, run it and wait for its result, read and delete run folders, and download the
//! files a run returned.

pub mod deploy;
pub mod returned_files;
pub mod run_control;
pub mod run_folders;
pub mod run_results;

#[cfg(test)]
pub(crate) mod fake;
