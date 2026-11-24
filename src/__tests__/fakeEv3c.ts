import { ChildProcess, SpawnOptions } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { Spawn } from '../backend/ev3c';

// A stand-in for the `ev3-c` program: each start is a FakeProcess the test answers through,
// with what `ev3-c --events` would print. Nothing is really started.

export class FakeProcess extends EventEmitter {
    readonly stdin = new PassThrough();
    readonly stdout = new PassThrough();
    readonly stderr = new PassThrough();
    /** What was written to standard input. */
    input = '';
    /** Whether standard input was ended. */
    inputEnded = false;
    closed = false;

    constructor(readonly program: string, readonly args: readonly string[], readonly env: Record<string, string | undefined>) {
        super();
        this.stdin.on('data', (chunk: Buffer) => {
            this.input += chunk.toString('utf8');
        });
        this.stdin.on('finish', () => {
            this.inputEnded = true;
            this.emit('input-ended');
        });
    }

    /** Prints one event line. */
    send(event: object): void {
        this.stdout.write(`${JSON.stringify(event)}\n`);
    }

    /** Prints the terminal event and ends with its exit code. */
    finish(terminal: { event: string; exitCode: number; [field: string]: unknown }): void {
        this.send(terminal);
        this.exit(terminal.exitCode);
    }

    /** Ends without printing more. */
    exit(code: number, stderr = ''): void {
        if (stderr !== '') {
            this.stderr.write(stderr);
        }
        this.stdout.end();
        this.stderr.end();
        setImmediate(() => {
            this.closed = true;
            this.emit('close', code, null);
        });
    }
}

/** Starts FakeProcesses and hands each to `answer`, which a test sets. */
export class FakeEv3c {
    readonly started: FakeProcess[] = [];
    /** Answers a start; by default every command is done at once with exit code 0. */
    answer: (process: FakeProcess) => void = process => process.finish({ event: 'done', exitCode: 0 });

    readonly spawn: Spawn = (program: string, args: readonly string[], options: SpawnOptions) => {
        const process = new FakeProcess(program, args, (options.env ?? {}) as Record<string, string | undefined>);
        this.started.push(process);
        // The caller listens once spawn has returned, as with a real process.
        setImmediate(() => this.answer(process));
        return process as unknown as ChildProcess;
    };

    /** The starts whose first argument is `command`. */
    of(command: string): FakeProcess[] {
        return this.started.filter(process => process.args[0] === command);
    }
}
