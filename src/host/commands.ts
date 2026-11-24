import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { Ev3c, Ev3cError, Ev3cEvent, TerminalEvent } from '../backend/ev3c';
import { userMessage } from './errorMessages';
import {
    formatNoResult, formatReturned, formatRunResult, ReturnedFilesJson, RunEntryJson, RunResultJson, summaryLabel,
} from './resultsView';

// The "EV3 C" commands. Each one runs `ev3-c` once and shows what it reports: the extension
// keeps only the UI. Build, Upload, and Run each build first, so what goes onto the brick always
// matches the sources: Upload builds then uploads, Run builds, uploads, and runs. Upload skips
// files that are already on the brick unchanged.

export const COMMAND_IDS = {
    build: 'ev3-c.build',
    upload: 'ev3-c.upload',
    run: 'ev3-c.run',
    stop: 'ev3-c.stop',
    emergencyStop: 'ev3-c.emergencyStop',
    showRuns: 'ev3-c.showRuns',
    deleteRuns: 'ev3-c.deleteRuns',
    checkToolchain: 'ev3-c.checkToolchain',
    downloadFile: 'ev3-c.downloadFile',
} as const;

/** The file that makes a folder an EVƎ C project. */
export const PROJECT_FILE = 'ev3c.json';

export interface HostEnvironment {
    /** A log channel: every line carries a timestamp and a level, like EVƎ Cockpit's log. */
    output: vscode.LogOutputChannel;
    ev3c: Ev3c;
    /**
     * Launch IDs of the runs EVƎ C started in this session, with their start times (ISO), as
     * the `started` events told them; handed back to `ev3-c` in `EV3C_LAUNCHES`.
     */
    launches: Map<string, string>;
}

export type NoticeLevel = 'info' | 'warning' | 'error';

/** Shows a notification and records it in EVƎ C's log as "Shown: …" at its level. */
export function show(env: HostEnvironment, level: NoticeLevel, text: string): void {
    env.output[level === 'warning' ? 'warn' : level](`Shown: ${text}`);
    const window = vscode.window;
    void (level === 'error' ? window.showErrorMessage(text) : level === 'warning' ? window.showWarningMessage(text) : window.showInformationMessage(text));
}

/** `EV3C_LAUNCHES` for the launches of this session. */
export function launchesVariable(launches: ReadonlyMap<string, string>): string {
    const object: Record<string, number> = {};
    for (const [launchId, startedAt] of launches) {
        const millis = Date.parse(startedAt);
        if (!Number.isNaN(millis)) {
            object[launchId] = millis;
        }
    }
    return JSON.stringify(object);
}

const UNTRUSTED = 'EVƎ C does not build in an untrusted workspace, because building runs a compiler on the workspace\'s files. '
    + 'Trust the workspace to build, upload, and run.';

export function registerCommands(env: HostEnvironment): vscode.Disposable[] {
    return [
        register(env, COMMAND_IDS.build, async () => {
            const dir = await trustedProjectFolder();
            if (dir === undefined) {
                return;
            }
            const done = await withProgress(`EVƎ C: building ${path.basename(dir)}`,
                () => env.ev3c.run(['build', dir], { onEvent: event => logEvent(env, event) }));
            show(env, 'info', `EVƎ C built "${String(done.project)}".`);
        }),
        register(env, COMMAND_IDS.upload, async () => {
            const dir = await trustedProjectFolder();
            if (dir === undefined) {
                return;
            }
            const done = await withProgress(`EVƎ C: building and uploading ${path.basename(dir)}`,
                () => env.ev3c.run(['upload', dir], { onEvent: event => logEvent(env, event) }));
            const uploaded = strings(done.uploaded);
            const unchanged = strings(done.unchanged);
            env.output.info(`Uploaded [${uploaded.join(', ')}], unchanged [${unchanged.join(', ')}]`);
            show(env, 'info', `EVƎ C uploaded "${String(done.project)}": ${uploaded.length} file(s) uploaded, ${unchanged.length} unchanged.`);
        }),
        register(env, COMMAND_IDS.run, async () => {
            const dir = await trustedProjectFolder();
            if (dir === undefined) {
                return;
            }
            await run(env, dir);
        }),
        register(env, COMMAND_IDS.stop, async () => {
            reportStopped(env, 'Stop', await env.ev3c.run(['stop'], { onEvent: event => logEvent(env, event) }));
        }),
        register(env, COMMAND_IDS.emergencyStop, async () => {
            reportStopped(env, 'Emergency stop', await env.ev3c.run(['emergency-stop'], { onEvent: event => logEvent(env, event) }));
        }),
        register(env, COMMAND_IDS.downloadFile, async () => {
            const brickPath = (await vscode.window.showInputBox({
                title: 'Download a file from the brick',
                prompt: 'The file\'s full path on the brick',
                placeHolder: '/home/root/lms2012/prjs/<project>/<file>',
                validateInput: value => (value.trim().startsWith('/') && !value.trim().endsWith('/') ? undefined : 'A full path to a file, starting with /'),
            }))?.trim();
            if (brickPath === undefined) {
                return;
            }
            const folder = projectFolderOrNone() ?? vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
            const name = path.posix.basename(brickPath);
            const target = await vscode.window.showSaveDialog({
                title: `Save ${name} from the brick`,
                ...(folder !== undefined ? { defaultUri: vscode.Uri.file(path.join(folder, name)) } : {}),
            });
            if (target === undefined) {
                return;
            }
            const done = await env.ev3c.run(['get', brickPath, target.fsPath], { onEvent: event => logEvent(env, event) });
            env.output.info(`Downloaded ${brickPath} (${String(done.bytes)} bytes) → ${target.fsPath}`);
            show(env, 'info', `Downloaded ${name} from the brick.`);
        }),
        register(env, COMMAND_IDS.showRuns, async () => {
            const dir = await projectFolder();
            if (dir === undefined) {
                return;
            }
            const project = projectName(dir);
            const listed = await withProgress(`EVƎ C: reading the runs of "${project}"`,
                () => env.ev3c.run(['runs', dir], { onEvent: event => logEvent(env, event) }));
            const runs = (listed.runs ?? []) as RunEntryJson[];
            if (runs.length === 0) {
                show(env, 'info', `"${project}" has no runs on the brick.`);
                return;
            }
            const picked = await vscode.window.showQuickPick(
                [...runs].reverse().map(entry => ({ ...summaryLabel(entry), run: entry.run })),
                { title: `Runs of "${project}" on the brick`, placeHolder: 'Pick a run to show its results' },
            );
            if (picked === undefined) {
                return;
            }
            const shown = await env.ev3c.run(['runs', 'show', String(picked.run), dir], { onEvent: event => logEvent(env, event) });
            const result = shown.result as RunResultJson | null | undefined;
            if (result === null || result === undefined) {
                show(env, 'info', `Run ${picked.run} of "${project}" has no completion record yet.`);
                return;
            }
            writeLines(env, formatRunResult(project, result));
            writeReturned(env, shown.returnedFiles as ReturnedFilesJson | undefined);
            env.output.show(true);
        }),
        register(env, COMMAND_IDS.deleteRuns, async () => {
            const dir = await projectFolder();
            if (dir === undefined) {
                return;
            }
            const project = projectName(dir);
            const keepLast = await askKeepLast(project);
            if (keepLast === undefined) {
                return;
            }
            const done = await env.ev3c.run(
                keepLast === 0 ? ['runs', 'delete', dir] : ['runs', 'delete', '--keep', String(keepLast), dir],
                { onEvent: event => logEvent(env, event) },
            );
            const deleted = numbers(done.deleted);
            const kept = numbers(done.kept);
            const stillPresent = numbers(done.stillPresent);
            const parts = [`deleted ${deleted.length}`, `kept ${kept.length}`];
            env.output[stillPresent.length > 0 ? 'warn' : 'info'](`Run folders of "${project}": deleted [${deleted.join(', ')}], kept [${kept.join(', ')}]`
                + (stillPresent.length > 0 ? `, could not delete [${stillPresent.join(', ')}]` : ''));
            if (stillPresent.length > 0) {
                show(env, 'warning',
                    `Run folders of "${project}": ${parts.join(', ')}; ${stillPresent.length} could not be deleted (see the "EVƎ C" output).`);
            } else {
                show(env, 'info', `Run folders of "${project}": ${parts.join(', ')}.`);
            }
        }),
        register(env, COMMAND_IDS.checkToolchain, async () => {
            const compiler = await withProgress('EVƎ C: checking the compiler', () => env.ev3c.run(['toolchain']));
            const version = typeof compiler.version === 'string' ? compiler.version : '?';
            const where = compiler.origin === 'override' ? 'set in "ev3-c.toolchain.path"'
                : compiler.origin === 'download' ? `version ${version}, downloaded now` : `version ${version}`;
            env.output.info(`Compiler: ${String(compiler.path)} (${where})`);
            show(env, 'info', `EVƎ C's compiler is ready (${where}).`);
        }),
    ];
}

/**
 * Builds, uploads, and runs, and shows how the run ended. The progress notification lasts until
 * the program has started; the result comes when it ends, by Stop, the Back button, or itself.
 */
async function run(env: HostEnvironment, dir: string): Promise<void> {
    let project = projectName(dir);
    let markStarted = () => undefined as void;
    const started = new Promise<void>(resolve => {
        markStarted = resolve;
    });
    env.output.info(`Uploading and running "${project}". Stop it with "EV3 C: Stop" or the brick's Back button.`);
    const running = env.ev3c.run(['run', dir], {
        onEvent: event => {
            logEvent(env, event);
            if (event.event === 'built' && typeof event.project === 'string') {
                project = event.project;
            }
            if (event.event === 'started') {
                if (typeof event.launchId === 'string' && typeof event.startedAt === 'string') {
                    env.launches.set(event.launchId, event.startedAt);
                }
                const start = (event.start ?? {}) as { outcome?: unknown; runSeq?: unknown; uploaded?: unknown; skipped?: unknown };
                env.output.info(`Started "${project}" (${String(start.outcome)}, start ${String(start.runSeq)}): `
                    + `uploaded [${strings(start.uploaded).join(', ')}], unchanged [${strings(start.skipped).join(', ')}]`);
                env.output.show(true);
                markStarted();
            }
        },
    });
    void withProgress(`EVƎ C: building and starting ${path.basename(dir)}`, () => Promise.race([started, running.then(() => undefined, () => undefined)]));
    const end = await running;
    if (end.event === 'result') {
        const result = end.result as RunResultJson;
        writeLines(env, formatRunResult(project, result));
        const returned = writeReturned(env, end.returnedFiles as ReturnedFilesJson | undefined);
        const where = returned === undefined || returned.downloaded.length + returned.failed.length === 0 ? ''
            : `; ${returned.downloaded.length} file(s) returned to ${path.join('returned', `run-${result.run}`)}`
                + (returned.failed.length > 0 ? `, ${returned.failed.length} could not be returned` : '');
        show(env, 'info', `"${project}" run ${result.run}: ${result.record.ended}${where}.`);
    } else if (end.event === 'noResult') {
        const info = (end.noResult ?? {}) as { launchId?: unknown; startedAt?: unknown };
        writeLines(env, formatNoResult(project, String(info.launchId), String(info.startedAt)), 'warn');
        show(env, 'warning', `"${project}" ended, but no result was recorded on the brick.`);
    } else {
        env.output.warn(`"${project}": the run was left (${end.event}).`);
    }
}

/** Logs a progress event of `ev3-c` in the output channel. */
function logEvent(env: HostEnvironment, event: Ev3cEvent): void {
    switch (event.event) {
    case 'building':
        env.output.info(`Building ${String(event.folder)}`);
        break;
    case 'diagnostics':
        env.output.warn(String(event.text).trimEnd());
        break;
    case 'built': {
        const artifacts = Array.isArray(event.artifacts) ? event.artifacts as { brickPath?: unknown }[] : [];
        env.output.info(`Built "${String(event.project)}": ${artifacts.map(a => String(a.brickPath)).join(', ')}`);
        break;
    }
    case 'connected':
        env.output.info(`Using the brick ${String(event.brickId)}`);
        break;
    case 'stopping':
        env.output.info('Stopping the program…');
        break;
    case 'notice':
        env.output.warn(`EVƎ Cockpit: ${String(event.text)}`);
        void vscode.window.showWarningMessage(String(event.text));
        break;
    }
}

/** Registers a command whose failures are logged and shown, never thrown at VS Code. */
function register(env: HostEnvironment, id: string, action: () => Promise<void>): vscode.Disposable {
    return vscode.commands.registerCommand(id, async () => {
        try {
            await action();
        } catch (error) {
            const code = error instanceof Ev3cError && error.code !== undefined ? ` [${error.code}]` : '';
            env.output.error(`${id} failed${code}: ${error instanceof Error ? error.message : String(error)}`);
            if (error instanceof Ev3cError && error.code === undefined && error.failure === undefined) {
                // A failure of EVƎ C's own, such as a build's compiler errors: the details are in the output.
                env.output.show(true);
            }
            show(env, 'error', userMessage(error));
        }
    });
}

function withProgress<T>(title: string, task: () => Promise<T>): Promise<T> {
    return Promise.resolve(vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title }, task));
}

function reportStopped(env: HostEnvironment, what: string, done: TerminalEvent): void {
    const running = done.running === true;
    env.output[running ? 'warn' : 'info'](`${what}: the brick reports ${running ? 'a program still running' : 'no program running'}.`);
    if (running) {
        show(env, 'warning', `${what} was sent, but the brick still reports a running program.`);
    }
}

function writeReturned(env: HostEnvironment, report: ReturnedFilesJson | undefined): ReturnedFilesJson | undefined {
    if (report !== undefined) {
        for (const line of formatReturned(report)) {
            env.output[line.startsWith('Could not') ? 'warn' : 'info'](line);
        }
    }
    return report;
}

function writeLines(env: HostEnvironment, lines: readonly string[], level: 'info' | 'warn' = 'info'): void {
    for (const line of lines) {
        env.output[level](line);
    }
}

function strings(value: unknown): string[] {
    return Array.isArray(value) ? value.map(String) : [];
}

function numbers(value: unknown): number[] {
    return Array.isArray(value) ? value.filter((n): n is number => typeof n === 'number') : [];
}

/** The project's name for messages: `name` in ev3c.json, else the folder's name, as `ev3-c` takes it. */
export function projectName(dir: string): string {
    try {
        const name = (JSON.parse(fs.readFileSync(path.join(dir, PROJECT_FILE), 'utf8')) as { name?: unknown }).name;
        if (typeof name === 'string' && name.trim() !== '') {
            return name;
        }
    } catch {
        // `ev3-c` reports a broken ev3c.json itself.
    }
    return path.basename(dir);
}

/** The single project folder, or undefined when there is none or several. */
function projectFolderOrNone(): string | undefined {
    const folders = (vscode.workspace.workspaceFolders ?? []).filter(folder => fs.existsSync(path.join(folder.uri.fsPath, PROJECT_FILE)));
    return folders.length === 1 ? folders[0].uri.fsPath : undefined;
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
