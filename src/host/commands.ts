import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import {
    BrickFileReader, BrickPort, BuildResult, buildProject, deleteRuns, deploy, emergencyStop, listRuns, loadProjectConfig,
    PROJECT_FILE, readRunResult, runProject, stopProgram,
} from '../core';
import { ensureCompiler } from '../core/toolchain/toolchainManager';
import { userMessage } from './errorMessages';
import { formatNoResult, formatRunResult, howItEnded, summaryLabel } from './resultsView';
import { toolchainOverride } from './settings';

// The "EV3 C" commands. Build, Upload, and Run each build first, so what goes onto the
// brick always matches the sources: Upload builds then uploads, Run builds, uploads, and
// runs. Upload skips files that are already on the brick unchanged.

export const COMMAND_IDS = {
    build: 'ev3-c.build',
    upload: 'ev3-c.upload',
    run: 'ev3-c.run',
    stop: 'ev3-c.stop',
    emergencyStop: 'ev3-c.emergencyStop',
    showRuns: 'ev3-c.showRuns',
    deleteRuns: 'ev3-c.deleteRuns',
    checkToolchain: 'ev3-c.checkToolchain',
} as const;

/** The core functions the commands call; tests replace some of them. */
export const defaultCore = {
    buildProject, deploy, runProject, stopProgram, emergencyStop, listRuns, readRunResult, deleteRuns, loadProjectConfig, ensureCompiler,
};
export type CoreFunctions = typeof defaultCore;

export interface HostEnvironment {
    output: vscode.OutputChannel;
    /** The extension's install folder; ev3lib's headers and the runtime are below it. */
    packageRoot: string;
    /** Where the downloaded compiler is kept. */
    toolchainCacheDir: string;
    core: CoreFunctions;
}

/** How the brick commands reach the brick the "ev3-c.brick" setting names. */
export interface BrickAccess {
    /** The port and shared reader of the brick the setting names now; throws when there is none. */
    current(): { port: BrickPort; reader: BrickFileReader };
    /** Launch IDs of the runs EVƎ C started in this session, with their PC start times. */
    readonly launches: Map<string, Date>;
}

const UNTRUSTED = 'EVƎ C does not build in an untrusted workspace, because building runs a compiler on the workspace\'s files. '
    + 'Trust the workspace to build, upload, and run.';

/** Commands that need no brick: Build and Check Toolchain. */
export function registerProjectCommands(env: HostEnvironment): vscode.Disposable[] {
    return [
        register(env, COMMAND_IDS.build, async () => {
            const dir = await trustedProjectFolder();
            if (dir === undefined) {
                return;
            }
            const built = await build(env, dir);
            void vscode.window.showInformationMessage(`EVƎ C built "${built.project.name}".`);
        }),
        register(env, COMMAND_IDS.checkToolchain, async () => {
            const compiler = await vscode.window.withProgress(
                { location: vscode.ProgressLocation.Notification, title: 'EVƎ C: checking the compiler' },
                () => env.core.ensureCompiler({ cacheDir: env.toolchainCacheDir, override: toolchainOverride() }),
            );
            const where = compiler.origin === 'override' ? 'set in "ev3-c.toolchain.path"'
                : compiler.origin === 'download' ? `version ${compiler.version ?? '?'}, downloaded now` : `version ${compiler.version ?? '?'}`;
            env.output.appendLine(`Compiler: ${compiler.path} (${where})`);
            void vscode.window.showInformationMessage(`EVƎ C's compiler is ready (${where}).`);
        }),
    ];
}

/** Commands that reach a brick through EVƎ Cockpit. */
export function registerBrickCommands(env: HostEnvironment, bricks: BrickAccess): vscode.Disposable[] {
    return [
        register(env, COMMAND_IDS.upload, async () => {
            const dir = await trustedProjectFolder();
            if (dir === undefined) {
                return;
            }
            const built = await build(env, dir);
            await upload(env, bricks.current().port, built);
        }),
        register(env, COMMAND_IDS.run, async () => {
            const dir = await trustedProjectFolder();
            if (dir === undefined) {
                return;
            }
            const built = await build(env, dir);
            const { port, reader } = bricks.current();
            await upload(env, port, built);
            const project = built.project.name;
            env.output.appendLine(`Running "${project}". Stop it with "EV3 C: Stop" or the brick's Back button.`);
            const outcome = await env.core.runProject({ port, reader, project, launches: bricks.launches });
            env.output.show(true);
            if (outcome.kind === 'result') {
                writeLines(env, formatRunResult(project, outcome.result));
                void vscode.window.showInformationMessage(`"${project}" run ${outcome.result.run}: ${howItEnded(outcome.result.record)}.`);
            } else {
                writeLines(env, formatNoResult(project, outcome.launchId, outcome.startedAt));
                void vscode.window.showWarningMessage(`"${project}" ended, but no result was recorded on the brick.`);
            }
        }),
        register(env, COMMAND_IDS.stop, async () => {
            const state = await env.core.stopProgram(bricks.current().port);
            reportStopped(env, 'Stop', state.running);
        }),
        register(env, COMMAND_IDS.emergencyStop, async () => {
            const state = await env.core.emergencyStop(bricks.current().port);
            reportStopped(env, 'Emergency stop', state.running);
        }),
        register(env, COMMAND_IDS.showRuns, async () => {
            const dir = await projectFolder();
            if (dir === undefined) {
                return;
            }
            const project = (await env.core.loadProjectConfig(dir)).name;
            const { port, reader } = bricks.current();
            const runs = await vscode.window.withProgress(
                { location: vscode.ProgressLocation.Notification, title: `EVƎ C: reading the runs of "${project}"` },
                () => env.core.listRuns(port, reader, project, bricks.launches),
            );
            if (runs.length === 0) {
                void vscode.window.showInformationMessage(`"${project}" has no runs on the brick.`);
                return;
            }
            const picked = await vscode.window.showQuickPick(
                [...runs].reverse().map(summary => ({ ...summaryLabel(summary), run: summary.run })),
                { title: `Runs of "${project}" on the brick`, placeHolder: 'Pick a run to show its results' },
            );
            if (picked === undefined) {
                return;
            }
            const result = await env.core.readRunResult(port, reader, project, picked.run, bricks.launches);
            if (result === undefined) {
                void vscode.window.showInformationMessage(`Run ${picked.run} of "${project}" has no completion record yet.`);
                return;
            }
            writeLines(env, formatRunResult(project, result));
            env.output.show(true);
        }),
        register(env, COMMAND_IDS.deleteRuns, async () => {
            const dir = await projectFolder();
            if (dir === undefined) {
                return;
            }
            const project = (await env.core.loadProjectConfig(dir)).name;
            const keepLast = await askKeepLast(project);
            if (keepLast === undefined) {
                return;
            }
            const report = await env.core.deleteRuns(bricks.current().port, project, { keepLast });
            const parts = [`deleted ${report.deleted.length}`, `kept ${report.kept.length}`];
            env.output.appendLine(`Run folders of "${project}": deleted [${report.deleted.join(', ')}], kept [${report.kept.join(', ')}]`
                + (report.stillPresent.length > 0 ? `, could not delete [${report.stillPresent.join(', ')}]` : ''));
            if (report.stillPresent.length > 0) {
                void vscode.window.showWarningMessage(
                    `Run folders of "${project}": ${parts.join(', ')}; ${report.stillPresent.length} could not be deleted (see the "EVƎ C" output).`);
            } else {
                void vscode.window.showInformationMessage(`Run folders of "${project}": ${parts.join(', ')}.`);
            }
        }),
    ];
}

/** Registers a command whose failures are logged and shown, never thrown at VS Code. */
function register(env: HostEnvironment, id: string, action: () => Promise<void>): vscode.Disposable {
    return vscode.commands.registerCommand(id, async () => {
        try {
            await action();
        } catch (error) {
            const code = typeof error === 'object' && error !== null && 'code' in error ? ` [${String((error as { code: unknown }).code)}]` : '';
            env.output.appendLine(`${id} failed${code}: ${error instanceof Error ? error.message : String(error)}`);
            void vscode.window.showErrorMessage(userMessage(error));
        }
    });
}

async function trustedProjectFolder(): Promise<string | undefined> {
    if (!vscode.workspace.isTrusted) {
        throw new Error(UNTRUSTED);
    }
    return projectFolder();
}

/**
 * The workspace folder that holds ev3c.json; with several, the author picks one.
 * Undefined when the pick is dismissed.
 */
async function projectFolder(): Promise<string | undefined> {
    const folders = (vscode.workspace.workspaceFolders ?? [])
        .filter(folder => fs.existsSync(path.join(folder.uri.fsPath, PROJECT_FILE)));
    if (folders.length === 0) {
        throw new Error(`Open the folder of an EVƎ C project: no workspace folder contains ${PROJECT_FILE}.`);
    }
    if (folders.length === 1) {
        return folders[0].uri.fsPath;
    }
    const picked = await vscode.window.showQuickPick(
        folders.map(folder => ({ label: folder.name, description: folder.uri.fsPath, dir: folder.uri.fsPath })),
        { placeHolder: 'Which EVƎ C project?' },
    );
    return picked?.dir;
}

async function build(env: HostEnvironment, dir: string): Promise<Extract<BuildResult, { ok: true }>> {
    env.output.appendLine(`Building ${dir}`);
    const result = await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: `EVƎ C: building ${path.basename(dir)}` },
        () => env.core.buildProject({
            projectDir: dir,
            packageRoot: env.packageRoot,
            toolchain: { cacheDir: env.toolchainCacheDir, override: toolchainOverride() },
        }),
    );
    if (result.diagnostics.trim() !== '') {
        env.output.appendLine(result.diagnostics.trimEnd());
    }
    if (!result.ok) {
        env.output.show(true);
        throw new Error('The build failed: the compiler reported errors. See the "EVƎ C" output.');
    }
    env.output.appendLine(`Built "${result.project.name}": ${result.artifacts.map(a => a.brickPath).join(', ')}`);
    return result;
}

async function upload(env: HostEnvironment, port: BrickPort, built: Extract<BuildResult, { ok: true }>): Promise<void> {
    const report = await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: `EVƎ C: uploading "${built.project.name}"` },
        () => env.core.deploy(port, built.artifacts),
    );
    env.output.appendLine(`Uploaded [${report.uploaded.map(a => a.brickPath).join(', ')}], unchanged [${report.unchanged.map(a => a.brickPath).join(', ')}]`);
    void vscode.window.showInformationMessage(
        `EVƎ C uploaded "${built.project.name}": ${report.uploaded.length} file(s) uploaded, ${report.unchanged.length} unchanged.`);
}

function reportStopped(env: HostEnvironment, what: string, running: boolean): void {
    env.output.appendLine(`${what}: the brick reports ${running ? 'a program still running' : 'no program running'}.`);
    if (running) {
        void vscode.window.showWarningMessage(`${what} was sent, but the brick still reports a running program.`);
    }
}

/** All folders (0), or all but the newest N; undefined when dismissed. */
async function askKeepLast(project: string): Promise<number | undefined> {
    const choice = await vscode.window.showQuickPick(
        [
            { label: 'Delete all run folders', keep: false },
            { label: 'Keep the newest run folders', description: 'and delete the older ones', keep: true },
        ],
        { title: `Delete the run folders of "${project}" on the brick` },
    );
    if (choice === undefined) {
        return undefined;
    }
    if (!choice.keep) {
        return 0;
    }
    const answer = await vscode.window.showInputBox({
        title: 'How many of the newest run folders to keep?',
        value: '1',
        validateInput: value => (/^[1-9][0-9]{0,5}$/.test(value.trim()) ? undefined : 'A whole number from 1 up'),
    });
    return answer === undefined ? undefined : Number(answer.trim());
}

function writeLines(env: HostEnvironment, lines: readonly string[]): void {
    for (const line of lines) {
        env.output.appendLine(line);
    }
}
