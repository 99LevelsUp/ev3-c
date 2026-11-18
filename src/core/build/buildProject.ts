import * as fs from 'node:fs/promises';
import * as path from 'node:path';
import { compile as zigCompile, CompileRequest, CompileResult } from '../toolchain/compiler';
import { checkElf } from '../toolchain/elfCheck';
import { CompilerOverride, ensureCompiler } from '../toolchain/toolchainManager';
import { buildLauncher, SUPERVISOR_FILE } from '../launcher/launcherTemplate';
import { loadProjectConfig, ProjectConfig } from '../project/projectConfig';

// Turns a project folder into the files that go onto the brick: the student's program
// linked with libev3.a, the launcher with the project's rules in its supervisor command,
// and the prebuilt supervisor. Every step that can refuse the project runs before the
// compiler: name and rules, the runtime files, the presence of sources.

/** Folder on the brick that holds one folder per project. */
export const BRICK_PROJECTS_DIR = '/home/root/lms2012/prjs';

/** File name of the student's program inside the project folder; the supervisor runs it. */
export const PROGRAM_FILE = 'prog';

/** Build output folder inside the project folder. */
export const BUILD_DIR = 'build';

// Pulls ev3lib's start-up object into every program, also one that calls no ev3
// function: its constructor sets line-buffered output before main(), and it registers
// the motor stop at exit.
const FORCE_EV3LIB_STARTUP = '-Wl,-u,ev3_core_init';

export interface BuildArtifact {
    readonly kind: 'program' | 'launcher' | 'supervisor';
    /** The file on this computer. */
    readonly localPath: string;
    /** Where the file goes on the brick, always shorter than 60 bytes. */
    readonly brickPath: string;
}

export interface BuildProjectOptions {
    /** Folder with the student's .c files and an optional ev3c.json. */
    projectDir: string;
    /** Root of the installed package; ev3lib's headers are in `ev3lib/include` below it. */
    packageRoot: string;
    /** Folder with the prebuilt libev3.a and supervisor; defaults to `out/runtime` in the package. */
    runtimeDir?: string;
    /** Where the compiler is cached, and a compiler path set by the host. */
    toolchain: { cacheDir: string; override?: CompilerOverride };
    /** Replaces the Zig compile step, for tests. */
    compile?: (request: CompileRequest) => Promise<CompileResult>;
}

/**
 * `ok: false` means the student's sources did not compile; `diagnostics` holds what the
 * compiler said. Anything else that stops a build (invalid name or rules, missing runtime
 * files, no sources, a program the brick cannot run) is thrown as an Error.
 */
export type BuildResult =
    | {
        readonly ok: true;
        readonly project: ProjectConfig;
        /** Program, launcher, and supervisor, in that order. */
        readonly artifacts: BuildArtifact[];
        /** Compiler warnings, if any. */
        readonly diagnostics: string;
    }
    | { readonly ok: false; readonly diagnostics: string };

export async function buildProject(options: BuildProjectOptions): Promise<BuildResult> {
    const { projectDir, packageRoot } = options;
    const project = await loadProjectConfig(projectDir);

    const runtimeDir = options.runtimeDir ?? path.join(packageRoot, 'out', 'runtime');
    const library = path.join(runtimeDir, 'libev3.a');
    const supervisor = path.join(runtimeDir, 'supervisor');
    for (const file of [library, supervisor]) {
        if (!await exists(file)) {
            throw new Error(`The EVƎ C runtime is incomplete: ${path.basename(file)} is missing in ${runtimeDir}.`);
        }
    }

    const sources = (await fs.readdir(projectDir, { withFileTypes: true }))
        .filter(entry => entry.isFile() && entry.name.endsWith('.c'))
        .map(entry => entry.name)
        .sort();
    if (sources.length === 0) {
        throw new Error(`The project folder ${projectDir} has no .c files to build.`);
    }

    const compiler = await ensureCompiler(options.toolchain);

    // Remove this project's previous outputs first, so a failed build leaves nothing to deploy.
    const buildDir = path.join(projectDir, BUILD_DIR);
    const programPath = path.join(buildDir, PROGRAM_FILE);
    const launcherFile = `${project.name}.rbf`;
    const launcherPath = path.join(buildDir, launcherFile);
    await fs.rm(programPath, { force: true });
    await fs.rm(launcherPath, { force: true });
    await fs.mkdir(buildDir, { recursive: true });

    const compiled = await (options.compile ?? zigCompile)({
        compilerPath: compiler.path,
        cwd: projectDir,
        sources,
        includeDirs: [path.join(packageRoot, 'ev3lib', 'include')],
        // The request has no field for linker flags; libraries are passed as plain
        // arguments, so the flag goes in front of the archive it applies to.
        libraries: [FORCE_EV3LIB_STARTUP, library],
        output: `${BUILD_DIR}/${PROGRAM_FILE}`,
    });
    if (!compiled.ok) {
        await fs.rm(programPath, { force: true });
        return { ok: false, diagnostics: compiled.diagnostics };
    }

    const check = await checkElf(programPath);
    if (!check.ok) {
        await fs.rm(programPath, { force: true });
        throw new Error(`The compiled program cannot run on the brick:\n  ${check.problems.join('\n  ')}`);
    }

    await fs.writeFile(launcherPath, buildLauncher({ retention: project.retention, outputCapBytes: project.outputCapBytes }));

    const brickDir = `${BRICK_PROJECTS_DIR}/${project.name}`;
    return {
        ok: true,
        project,
        artifacts: [
            { kind: 'program', localPath: programPath, brickPath: `${brickDir}/${PROGRAM_FILE}` },
            { kind: 'launcher', localPath: launcherPath, brickPath: `${brickDir}/${launcherFile}` },
            { kind: 'supervisor', localPath: supervisor, brickPath: `${brickDir}/${SUPERVISOR_FILE}` },
        ],
        diagnostics: compiled.diagnostics,
    };
}

async function exists(file: string): Promise<boolean> {
    try {
        await fs.access(file);
        return true;
    } catch {
        return false;
    }
}
