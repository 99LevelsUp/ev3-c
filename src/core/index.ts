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
