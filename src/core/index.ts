// EVƎ C core: build, toolchain, launcher, and brick operations. Nothing here may
// depend on the VS Code API, so the same core can later serve a command-line tool.
export { buildProject, BRICK_PROJECTS_DIR, BUILD_DIR, PROGRAM_FILE } from './build/buildProject';
export type { BuildArtifact, BuildProjectOptions, BuildResult } from './build/buildProject';
export {
    loadProjectConfig,
    projectNameProblem,
    PROJECT_FILE,
    DEFAULT_RETENTION,
    DEFAULT_OUTPUT_CAP_BYTES,
} from './project/projectConfig';
export type { ProjectConfig } from './project/projectConfig';
export { buildLauncher, SUPERVISOR_FILE } from './launcher/launcherTemplate';
export type { LauncherOptions, RetentionRule } from './launcher/launcherTemplate';
export { brickErrorCode, PacedReader, READ_INTERVAL_MS, systemClock } from './brickPort';
export type { BrickEntry, BrickEvent, BrickFileReader, BrickPort, Clock, ProgramState, Subscription } from './brickPort';
export { BrickOperationError, deploy } from './brick/deploy';
export type { DeployReport } from './brick/deploy';
export { emergencyStop, RESULT_WAIT_MS, runProject, stopProgram } from './brick/runControl';
export type { RunOutcome, RunProjectOptions } from './brick/runControl';
export { listRuns, parseRunRecord, readRunRecord, readRunResult, resultFromRecord, RunRecordError } from './brick/runResults';
export type { RunEnd, RunRecord, RunResult, RunSummary } from './brick/runResults';
export { brickProjectDir, deleteRuns, listRunNumbers, runDir, runsDir } from './brick/runFolders';
export type { DeleteRunsOptions, DeleteRunsReport } from './brick/runFolders';
