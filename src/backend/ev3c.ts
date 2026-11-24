import { ChildProcess, spawn as nodeSpawn, SpawnOptions } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { StringDecoder } from 'node:string_decoder';

// The extension's only way to its backend: it runs the Rust program `ev3-c` once per command
// with `--events` and reads one JSON event per line from its standard output (docs/cli.md).
// Standard input stays an open pipe while the command runs: a line `interrupt` stands for
// Ctrl+C, and its end, when VS Code goes away, stops what the command started.

/** How EVƎ C names itself to the cockpit: other clients read it in `brick_busy` and `stoppedBy`. */
export const CLIENT_NAME = 'EVƎ C (VS Code)';

/** One line of `--events`: `event` first, then the event's fields. */
export interface Ev3cEvent {
    readonly event: string;
    readonly [field: string]: unknown;
}

/** The last event of a command: it carries the exit code. */
export interface TerminalEvent extends Ev3cEvent {
    readonly exitCode: number;
}

/** A command that ended with the `error` event, or `ev3-c` that could not be run at all. */
export class Ev3cError extends Error {
    constructor(message: string, readonly code?: string, readonly failure?: string) {
        super(message);
        this.name = 'Ev3cError';
    }
}

/** Starts a process; tests replace it. */
export type Spawn = (program: string, args: readonly string[], options: SpawnOptions) => ChildProcess;

export interface RunOptions {
    /** Called for each progress event, in order. */
    onEvent?: (event: Ev3cEvent) => void;
    /** Aborting it sends one interrupt, as Ctrl+C would. */
    signal?: AbortSignal;
    /** More environment variables for this command. */
    env?: Readonly<Record<string, string>>;
}

/** Where `ev3-c` runs from and what it gets on every command. */
export class Ev3c {
    constructor(
        private readonly program: () => string,
        private readonly env: () => Readonly<Record<string, string>>,
        private readonly spawn: Spawn = nodeSpawn,
    ) {}

    /**
     * Runs `ev3-c <args> --events` to its terminal event. An `error` event rejects with an
     * {@link Ev3cError} that carries the cockpit's code; any other terminal event resolves,
     * whatever its exit code (a run's exit code is the program's own).
     */
    async run(args: readonly string[], options: RunOptions = {}): Promise<TerminalEvent> {
        const terminal = await this.events(args, options);
        if (terminal.event === 'error') {
            const error = terminal.error as { message?: unknown; code?: unknown; failure?: unknown } | undefined;
            throw new Ev3cError(
                typeof error?.message === 'string' ? error.message : 'ev3-c failed.',
                typeof error?.code === 'string' ? error.code : undefined,
                typeof error?.failure === 'string' ? error.failure : undefined,
            );
        }
        return terminal;
    }

    /**
     * Starts a command that runs until it is told to end, such as `watch`: `onEvent` hears its
     * events, `ended` settles with its terminal event, and `stop` ends its standard input.
     */
    follow(args: readonly string[], onEvent: (event: Ev3cEvent) => void): { ended: Promise<TerminalEvent>; stop(): void } {
        const controller = new AbortController();
        const ended = this.events(args, { onEvent, signal: controller.signal, closeOnAbort: true });
        return { ended, stop: () => controller.abort() };
    }

    private events(args: readonly string[], options: RunOptions & { closeOnAbort?: boolean }): Promise<TerminalEvent> {
        return new Promise<TerminalEvent>((resolve, reject) => {
            let child: ChildProcess;
            let program = 'ev3-c';
            try {
                program = this.program();
                child = this.spawn(program, [...args, '--events'], {
                    env: { ...process.env, ...this.env(), ...options.env },
                    stdio: ['pipe', 'pipe', 'pipe'],
                    windowsHide: true,
                    shell: false,
                });
            } catch (error) {
                reject(error instanceof Ev3cError ? error
                    : new Ev3cError(`EVƎ C's program ${program} could not be started: ${String(error)}`));
                return;
            }
            let terminal: TerminalEvent | undefined;
            let stderr = '';
            let settled = false;
            const finish = (outcome: () => void) => {
                if (!settled) {
                    settled = true;
                    options.signal?.removeEventListener('abort', onAbort);
                    outcome();
                }
            };
            const onAbort = () => {
                if (options.closeOnAbort) {
                    child.stdin?.end();
                } else {
                    child.stdin?.write('interrupt\n');
                }
            };
            if (options.signal?.aborted) {
                onAbort();
            } else {
                options.signal?.addEventListener('abort', onAbort);
            }
            // A pipe whose reader is gone fails a write; the exit tells what happened.
            child.stdin?.on('error', () => undefined);

            const lines = new LineReader(line => {
                let event: Ev3cEvent;
                try {
                    event = JSON.parse(line) as Ev3cEvent;
                } catch {
                    return;
                }
                if (typeof event !== 'object' || event === null || typeof event.event !== 'string') {
                    return;
                }
                if (typeof event.exitCode === 'number') {
                    terminal = event as TerminalEvent;
                } else {
                    options.onEvent?.(event);
                }
            });
            child.stdout?.on('data', (chunk: Buffer) => lines.push(chunk));
            child.stderr?.on('data', (chunk: Buffer) => {
                stderr += chunk.toString('utf8');
            });
            child.on('error', error => finish(() => reject(new Ev3cError(
                `EVƎ C's program ${program} could not be run: ${error.message}`))));
            child.on('close', (code, signal) => finish(() => {
                lines.end();
                child.stdin?.end();
                if (terminal !== undefined) {
                    resolve(terminal);
                    return;
                }
                const why = stderr.trim() !== '' ? `: ${stderr.trim()}` : '';
                reject(new Ev3cError(`EVƎ C's program ended without a result (${signal ?? `exit code ${code}`})${why}`));
            }));
        });
    }
}

/** Splits UTF-8 output into lines, also when a character is split across chunks. */
export class LineReader {
    private readonly decoder = new StringDecoder('utf8');
    private pending = '';

    constructor(private readonly onLine: (line: string) => void) {}

    push(chunk: Buffer): void {
        this.pending += this.decoder.write(chunk);
        let end: number;
        while ((end = this.pending.indexOf('\n')) >= 0) {
            const line = this.pending.slice(0, end).replace(/\r$/, '');
            this.pending = this.pending.slice(end + 1);
            if (line.trim() !== '') {
                this.onLine(line);
            }
        }
    }

    end(): void {
        this.pending += this.decoder.end();
        const rest = this.pending.trim();
        this.pending = '';
        if (rest !== '') {
            this.onLine(rest);
        }
    }
}

const EXE = process.platform === 'win32' ? 'ev3-c.exe' : 'ev3-c';

/** When a file was last written, in milliseconds; undefined when it does not exist. */
export type ModifiedAt = (file: string) => number | undefined;

const modifiedAt: ModifiedAt = file => {
    try {
        return fs.statSync(file).mtimeMs;
    } catch {
        return undefined;
    }
};

/**
 * The `ev3-c` program: the one packaged with the extension (`bin/`), else, in a working copy of
 * the repository, the one Cargo built last, release or debug, so a stale build never wins.
 */
export function findProgram(extensionPath: string, written: ModifiedAt = modifiedAt): string {
    const packaged = path.join(extensionPath, 'bin', EXE);
    if (written(packaged) !== undefined) {
        return packaged;
    }
    const built = ['release', 'debug']
        .map(profile => path.join(extensionPath, 'backend', 'target', profile, EXE))
        .map(file => ({ file, at: written(file) }))
        .filter((candidate): candidate is { file: string; at: number } => candidate.at !== undefined)
        .sort((a, b) => b.at - a.at);
    const found = built[0]?.file;
    if (found === undefined) {
        throw new Ev3cError(`EVƎ C's program ${EXE} is missing from ${path.join(extensionPath, 'bin')}. `
            + 'Reinstall EVƎ C; in a working copy of the repository, run "cargo build" in backend/.');
    }
    return found;
}
