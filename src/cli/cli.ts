import * as fs from 'node:fs';
import * as path from 'node:path';
import { parseArgs } from 'node:util';
import {
    BuildResult, buildProject, deleteRuns, deploy, emergencyStop, listRuns, loadProjectConfig, PROJECT_FILE, readRunResult, RunRecord,
    RunResult, runProject, stopProgram, brickErrorCode,
} from '../core';
import { ensureCompiler } from '../core/toolchain/toolchainManager';
import { CockpitClient, ConnectOptions, startFailure } from '../cockpit/cockpitLink';
import { CockpitSession } from '../shared/cockpitSession';
import { formatNoResult, formatRunResult, howItEnded } from '../shared/resultsView';

// `ev3 c …`: EVƎ C from a terminal or an agent. The cockpit's `ev3` command runs this as
// the add-on `ev3-c`. It does what the VS Code commands do, through the same core and the
// same cockpit link, and prints text or, with --json, one JSON object.

export const USAGE = `Usage: ev3 c <command> [project folder] [options]

Commands:
  build                 build the project
  upload                build, then upload what changed
  run                   build, upload and run; prints the run's output and exits with
                        the program's exit code (signal n: 128+n; stopped: 130)
  stop                  stop the program running on the brick
  emergency-stop        stop every program and motor on the brick
  runs                  list the project's runs on the brick
  runs show <n>         print run n's result
  runs delete [--keep <n>]
                        delete the run folders, all or all but the newest n
  toolchain             check the compiler, downloading it when needed

The project folder holds ${PROJECT_FILE}; it is the current folder when none is given.

Options:
  --brick <name|id>     the brick (default: EV3C_BRICK, else the brick EVƎ Cockpit's
                        window shows, else the only connected brick)
  --toolchain <path>    a compiler to use instead of the pinned one (or EV3C_TOOLCHAIN)
  --json                print one JSON object instead of text
  --help, --version`;

/** Exit codes besides a run's own. */
export const EXIT = { ok: 0, failed: 1, usage: 2, stopped: 130 } as const;

export interface CliIo {
    out(text: string): void;
    err(text: string): void;
}

/** What the command needs from its surroundings; tests replace all of it. */
export interface CliDeps {
    readonly connect: (options: ConnectOptions) => Promise<CockpitClient>;
    readonly core: typeof cliCore;
    /** EVƎ C's install folder: ev3lib's headers and the brick runtime are below it. */
    readonly packageRoot: string;
    readonly toolchainCacheDir: string;
    readonly version: string;
    readonly cwd: string;
    readonly env: Readonly<Record<string, string | undefined>>;
    /** Calls `handler` on each Ctrl+C until the returned function is called. */
    readonly onInterrupt: (handler: () => void) => () => void;
}

export const cliCore = {
    buildProject, deploy, runProject, stopProgram, emergencyStop, listRuns, readRunResult, deleteRuns, loadProjectConfig, ensureCompiler,
};

class UsageError extends Error {}

/** The process was asked to stop before the program started: nothing more to wait for. */
class Interrupted extends Error {}

interface Options {
    readonly json: boolean;
    readonly brick: string;
    readonly toolchain?: string;
    readonly keep?: number;
}

export async function runCli(argv: readonly string[], io: CliIo, deps: CliDeps): Promise<number> {
    let parsed;
    try {
        parsed = parseArgs({
            args: [...argv],
            allowPositionals: true,
            options: {
                brick: { type: 'string' },
                toolchain: { type: 'string' },
                keep: { type: 'string' },
                json: { type: 'boolean', default: false },
                help: { type: 'boolean', default: false },
                version: { type: 'boolean', default: false },
            },
        });
    } catch (error) {
        io.err(`ev3 c: ${messageOf(error)}\n\n${USAGE}`);
        return EXIT.usage;
    }
    const { values, positionals } = parsed;
    if (values.version) {
        io.out(deps.version);
        return EXIT.ok;
    }
    const [command, ...rest] = positionals;
    if (values.help || command === undefined || command === 'help') {
        io.out(USAGE);
        return command === undefined && !values.help ? EXIT.usage : EXIT.ok;
    }
    const toolchain = nonEmpty(values.toolchain) ?? nonEmpty(deps.env.EV3C_TOOLCHAIN);
    let keep: number | undefined;
    try {
        keep = values.keep === undefined ? undefined : wholeNumber(values.keep, '--keep');
    } catch (error) {
        io.err(`ev3 c: ${messageOf(error)}\n\n${USAGE}`);
        return EXIT.usage;
    }
    const options: Options = {
        json: values.json === true,
        brick: nonEmpty(values.brick) ?? nonEmpty(deps.env.EV3C_BRICK) ?? 'active',
        ...(toolchain !== undefined ? { toolchain } : {}),
        ...(keep !== undefined ? { keep } : {}),
    };

    const session = new CockpitSession(() => deps.connect({ name: `ev3 c ${command} (terminal)`, version: deps.version }));
    const cli = new Cli(io, deps, options, session);
    try {
        return await cli.dispatch(command, rest);
    } catch (error) {
        if (error instanceof UsageError) {
            io.err(`ev3 c: ${error.message}\n\n${USAGE}`);
            return EXIT.usage;
        }
        if (error instanceof Interrupted) {
            cli.report({ interrupted: true }, ['Interrupted; nothing was started.']);
            return EXIT.stopped;
        }
        cli.fail(error);
        return EXIT.failed;
    } finally {
        session.dispose();
    }
}

class Cli {
    constructor(
        private readonly io: CliIo,
        private readonly deps: CliDeps,
        private readonly options: Options,
        private readonly session: CockpitSession,
    ) {}

    async dispatch(command: string, rest: readonly string[]): Promise<number> {
        switch (command) {
        case 'build': return this.build(this.projectDir(rest, 0));
        case 'upload': return this.upload(this.projectDir(rest, 0));
        case 'run': return this.run(this.projectDir(rest, 0));
        case 'stop': return this.stop('stop', rest);
        case 'emergency-stop': return this.stop('emergency-stop', rest);
        case 'runs': return this.runs(rest);
        case 'toolchain': return this.toolchain(rest);
        default: throw new UsageError(`unknown command "${command}"`);
        }
    }

    /** Prints a result: the object with --json, the lines otherwise. */
    report(value: object, lines: readonly string[]): void {
        if (this.options.json) {
            this.io.out(JSON.stringify(value, null, 2));
        } else if (lines.length > 0) {
            this.io.out(lines.join('\n'));
        }
    }

    fail(error: unknown): void {
        const code = brickErrorCode(error);
        const failure = startFailure(error);
        const message = messageOf(error);
        if (this.options.json) {
            this.io.out(JSON.stringify({ error: { message, ...(code ? { code } : {}), ...(failure ? { failure } : {}) } }, null, 2));
        } else {
            this.io.err(`ev3 c: ${message}${code ? ` [${failure ?? code}]` : ''}`);
        }
    }

    private async build(dir: string): Promise<number> {
        const built = await this.compile(dir);
        this.report(
            { project: built.project.name, artifacts: built.artifacts.map(a => ({ kind: a.kind, localPath: a.localPath, brickPath: a.brickPath })), diagnostics: built.diagnostics },
            [`Built "${built.project.name}": ${built.artifacts.map(a => a.brickPath).join(', ')}`]);
        return EXIT.ok;
    }

    private async upload(dir: string): Promise<number> {
        const built = await this.compile(dir);
        const { port } = await this.session.brick(this.options.brick);
        const report = await this.deps.core.deploy(port, built.artifacts);
        this.report(
            { project: built.project.name, brickId: port.brickId, uploaded: report.uploaded.map(a => a.brickPath), unchanged: report.unchanged.map(a => a.brickPath) },
            [`Uploaded "${built.project.name}" to ${port.brickId}: ${report.uploaded.length} file(s) uploaded, ${report.unchanged.length} unchanged.`]);
        return EXIT.ok;
    }

    /**
     * Ctrl+C while building or connecting ends the command; closing the connection aborts its
     * requests. From the upload on it stops the program: a stop during the upload cancels the
     * start, a stop later ends the run, whose result is still printed. A second Ctrl+C leaves
     * without waiting.
     */
    private async run(dir: string): Promise<number> {
        let phase: 'preparing' | 'running' | 'stopping' = 'preparing';
        let port: Awaited<ReturnType<CockpitSession['brick']>>['port'] | undefined;
        let leave!: (error: Interrupted) => void;
        const left = new Promise<never>((_, reject) => { leave = reject; });
        left.catch(() => undefined);
        const release = this.deps.onInterrupt(() => {
            if (phase === 'running' && port !== undefined) {
                phase = 'stopping';
                this.io.err('Stopping the program (Ctrl+C again to leave without waiting for its result)…');
                void this.deps.core.stopProgram(port).catch(() => undefined);
            } else {
                leave(new Interrupted());
            }
        });
        try {
            const built = await Promise.race([this.compile(dir), left]);
            const handle = await Promise.race([this.session.brick(this.options.brick), left]);
            port = handle.port;
            const project = built.project.name;
            if (!this.options.json) {
                this.io.err(`Uploading and running "${project}" on ${port.brickId}. Ctrl+C stops it.`);
            }
            phase = 'running';
            const outcome = await Promise.race([
                this.deps.core.runProject({ port, reader: handle.reader, project, artifacts: built.artifacts, launches: new Map() }),
                left,
            ]);
            if (outcome.kind === 'no-result') {
                this.report(
                    { project, brickId: port.brickId, start: outcome.start, noResult: { launchId: outcome.launchId, startedAt: outcome.startedAt.toISOString() } },
                    formatNoResult(project, outcome.launchId, outcome.startedAt, '"ev3 c runs"'));
                return EXIT.failed;
            }
            this.report(
                { project, brickId: port.brickId, start: outcome.start, result: resultJson(outcome.result) },
                formatRunResult(project, outcome.result));
            return runExitCode(outcome.result.record);
        } catch (error) {
            if (error instanceof Interrupted && phase !== 'preparing') {
                this.report({ interrupted: true }, ['Left without waiting for the result; "ev3 c runs" lists the run.']);
                return EXIT.stopped;
            }
            // The Ctrl+C handler changes the phase, which TypeScript cannot see from here.
            if ((phase as string) === 'stopping' && brickErrorCode(error) === 'cancelled') {
                this.report({ interrupted: true }, ['Stopped before the program started.']);
                return EXIT.stopped;
            }
            throw error;
        } finally {
            release();
        }
    }

    private async stop(which: 'stop' | 'emergency-stop', rest: readonly string[]): Promise<number> {
        if (rest.length > 0) {
            throw new UsageError(`"${which}" takes no folder`);
        }
        const { port } = await this.session.brick(this.options.brick);
        const state = which === 'stop' ? await this.deps.core.stopProgram(port) : await this.deps.core.emergencyStop(port);
        const what = which === 'stop' ? 'Stop' : 'Emergency stop';
        this.report({ brickId: port.brickId, running: state.running, runSeq: state.runSeq },
            [`${what}: the brick reports ${state.running ? 'a program still running' : 'no program running'}.`]);
        return state.running ? EXIT.failed : EXIT.ok;
    }

    private async runs(rest: readonly string[]): Promise<number> {
        const [sub, ...args] = rest;
        if (sub === 'show') {
            const [n, ...more] = args;
            if (n === undefined) {
                throw new UsageError('"runs show" needs a run number');
            }
            const run = wholeNumber(n, 'the run number');
            const project = await this.projectName(this.projectDir(more, 0));
            const { port, reader } = await this.session.brick(this.options.brick);
            const result = await this.deps.core.readRunResult(port, reader, project, run, new Map());
            if (result === undefined) {
                this.report({ project, run, result: null }, [`Run ${run} of "${project}" has no completion record yet.`]);
                return EXIT.failed;
            }
            this.report({ project, result: resultJson(result) }, formatRunResult(project, result));
            return EXIT.ok;
        }
        if (sub === 'delete') {
            const project = await this.projectName(this.projectDir(args, 0));
            const { port } = await this.session.brick(this.options.brick);
            const report = await this.deps.core.deleteRuns(port, project, { keepLast: this.options.keep ?? 0 });
            this.report({ project, ...report }, [
                `Run folders of "${project}": deleted [${report.deleted.join(', ')}], kept [${report.kept.join(', ')}]`
                + (report.stillPresent.length > 0 ? `, could not delete [${report.stillPresent.join(', ')}]` : ''),
            ]);
            return report.stillPresent.length > 0 ? EXIT.failed : EXIT.ok;
        }
        const project = await this.projectName(this.projectDir(rest, 0));
        const { port, reader } = await this.session.brick(this.options.brick);
        const runs = await this.deps.core.listRuns(port, reader, project, new Map());
        this.report(
            { project, runs: runs.map(s => ({ run: s.run, ...(s.record ? { record: recordJson(s.record) } : {}) })) },
            runs.length === 0 ? [`"${project}" has no runs on the brick.`]
                : runs.map(s => `Run ${s.run}: ${s.record === undefined ? 'no completion record (running, or interrupted)' : howItEnded(s.record)}`));
        return EXIT.ok;
    }

    private async toolchain(rest: readonly string[]): Promise<number> {
        if (rest.length > 0) {
            throw new UsageError('"toolchain" takes no folder');
        }
        const compiler = await this.deps.core.ensureCompiler({ cacheDir: this.deps.toolchainCacheDir, override: this.override() });
        this.report({ path: compiler.path, origin: compiler.origin, ...(compiler.version ? { version: compiler.version } : {}) },
            [`Compiler: ${compiler.path} (${compiler.origin === 'override' ? 'given by --toolchain or EV3C_TOOLCHAIN' : `version ${compiler.version ?? '?'}`})`]);
        return EXIT.ok;
    }

    private async compile(dir: string): Promise<Extract<BuildResult, { ok: true }>> {
        const result = await this.deps.core.buildProject({
            projectDir: dir,
            packageRoot: this.deps.packageRoot,
            toolchain: { cacheDir: this.deps.toolchainCacheDir, override: this.override() },
        });
        if (result.diagnostics.trim() !== '') {
            this.io.err(result.diagnostics.trimEnd());
        }
        if (!result.ok) {
            throw new Error('The build failed: the compiler reported errors.');
        }
        return result;
    }

    private override() {
        return this.options.toolchain === undefined ? undefined : { path: this.options.toolchain, setting: '--toolchain' };
    }

    private async projectName(dir: string): Promise<string> {
        return (await this.deps.core.loadProjectConfig(dir)).name;
    }

    /** The project folder: the positional at `index`, else the current folder; it must hold ev3c.json. */
    private projectDir(rest: readonly string[], index: number): string {
        if (rest.length > index + 1) {
            throw new UsageError(`unexpected argument "${rest[index + 1]}"`);
        }
        const dir = path.resolve(this.deps.cwd, rest[index] ?? '.');
        if (!fs.existsSync(path.join(dir, PROJECT_FILE))) {
            throw new Error(`${dir} is not an EVƎ C project: it has no ${PROJECT_FILE}.`);
        }
        return dir;
    }
}

/** A run's exit code for the shell: the program's own, 128+n for signal n, 130 when stopped. */
export function runExitCode(record: RunRecord): number {
    if (record.end === 'program') {
        if (record.signal !== undefined) {
            return 128 + record.signal;
        }
        return Math.min(255, Math.max(0, record.exit ?? 0));
    }
    return record.end === 'slot' ? EXIT.stopped : EXIT.failed;
}

function resultJson(result: RunResult): object {
    return {
        run: result.run,
        record: recordJson(result.record),
        ...(result.out !== undefined ? { out: result.out } : {}),
        ...(result.err !== undefined ? { err: result.err } : {}),
        ...(result.startedAt !== undefined ? { startedAt: result.startedAt.toISOString() } : {}),
    };
}

function recordJson(record: RunRecord): object {
    return { ...record, ended: howItEnded(record) };
}

function wholeNumber(text: string, what: string): number {
    if (!/^[0-9]{1,9}$/.test(text)) {
        throw new UsageError(`${what} must be a whole number, got "${text}"`);
    }
    return Number(text);
}

function nonEmpty(value: string | undefined): string | undefined {
    return value === undefined || value.trim() === '' ? undefined : value.trim();
}

function messageOf(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

