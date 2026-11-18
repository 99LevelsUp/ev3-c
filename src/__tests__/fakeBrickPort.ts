import * as crypto from 'node:crypto';
import { BrickEntry, BrickEvent, BrickPort, Clock, ProgramState, Subscription } from '../core/brickPort';

// An in-memory brick behind the BrickPort interface, with a virtual clock, for the
// brick-operation tests. It behaves like the real brick where the core depends on it:
// a recursive remove deletes only one folder level and reports no failure, listings
// carry each file's MD5, and every call fails with "not_connected" while disconnected.

export class FakeCockpitError extends Error {
    constructor(readonly code: string, message = code) {
        super(message);
    }
}

/** Virtual time: `sleep` moves the time forward at once and runs what was scheduled up to then. */
export class FakeClock implements Clock {
    private time = 1_700_000_000_000;
    private scheduled: { at: number; action: () => void }[] = [];

    now(): number {
        return this.time;
    }

    async sleep(ms: number): Promise<void> {
        this.advance(ms);
        await Promise.resolve();
    }

    advance(ms: number): void {
        const end = this.time + ms;
        for (;;) {
            this.scheduled.sort((a, b) => a.at - b.at);
            const next = this.scheduled[0];
            if (next === undefined || next.at > end) {
                break;
            }
            this.scheduled.shift();
            this.time = Math.max(this.time, next.at);
            next.action();
        }
        this.time = end;
    }

    /** Runs `action` once the virtual time reaches `at`. */
    schedule(at: number, action: () => void): void {
        this.scheduled.push({ at, action });
    }
}

export class FakeBrickPort implements BrickPort {
    readonly files = new Map<string, Uint8Array>();
    readonly dirs = new Set<string>(['/']);
    /** Every call in order, as "<method> <path>". */
    readonly calls: string[] = [];
    /** Virtual times at which readFile was called. */
    readonly readTimes: number[] = [];
    /** Error codes the next writes of a path fail with, one per write. */
    readonly writeFailures = new Map<string, string[]>();
    running = false;
    /** The state runProgram reports; defaults to running. */
    runProgramResult: ProgramState = { running: true };
    /** Error code runProgram fails with, if set. */
    runProgramError: string | undefined;
    disconnected = false;
    /** Called inside runProgram, before it returns. */
    onRun: ((path: string) => void) | undefined;
    /** An event sent to each new subscriber at once, like a replay of an earlier state. */
    replayOnSubscribe: BrickEvent | undefined;
    /** Called inside remove, after the removal. */
    onRemove: ((path: string) => void) | undefined;
    private readonly listeners = new Set<(event: BrickEvent) => void>();

    constructor(readonly clock: FakeClock) {}

    get listenerCount(): number {
        return this.listeners.size;
    }

    // ----- scripting helpers -----

    putFile(path: string, content: string | Uint8Array): void {
        this.mkdirs(parentOf(path));
        this.files.set(path, typeof content === 'string' ? Buffer.from(content, 'utf8') : content);
    }

    mkdirs(path: string): void {
        for (let dir = path; dir !== '/' && !this.dirs.has(dir); dir = parentOf(dir)) {
            this.dirs.add(dir);
        }
    }

    text(path: string): string | undefined {
        const data = this.files.get(path);
        return data === undefined ? undefined : Buffer.from(data).toString('utf8');
    }

    has(path: string): boolean {
        return this.files.has(path) || this.dirs.has(path);
    }

    emit(event: BrickEvent): void {
        for (const listener of [...this.listeners]) {
            listener(event);
        }
    }

    // ----- BrickPort -----

    list(path: string): Promise<BrickEntry[]> {
        return this.call('list', path, () => {
            if (!this.dirs.has(path)) {
                throw new FakeCockpitError('protocol_failed', `no folder ${path}`);
            }
            const entries: BrickEntry[] = [];
            for (const dir of this.dirs) {
                if (dir !== '/' && parentOf(dir) === path) {
                    entries.push({ name: nameOf(dir), kind: 'directory' });
                }
            }
            for (const [file, data] of this.files) {
                if (parentOf(file) === path) {
                    entries.push({ name: nameOf(file), kind: 'file', bytes: data.length, md5: md5(data) });
                }
            }
            return entries;
        });
    }

    exists(path: string): Promise<boolean> {
        return this.call('exists', path, () => this.has(path));
    }

    makeDirectory(path: string): Promise<void> {
        return this.call('makeDirectory', path, () => {
            this.requireFolder(parentOf(path));
            this.dirs.add(path);
        });
    }

    readFile(path: string): Promise<Uint8Array> {
        return this.call('readFile', path, () => {
            this.readTimes.push(this.clock.now());
            const data = this.files.get(path);
            if (data === undefined) {
                throw new FakeCockpitError('protocol_failed', `no file ${path}`);
            }
            return data;
        });
    }

    writeFile(path: string, data: Uint8Array): Promise<void> {
        return this.call('writeFile', path, () => {
            const failure = this.writeFailures.get(path)?.shift();
            if (failure !== undefined) {
                throw new FakeCockpitError(failure);
            }
            this.requireFolder(parentOf(path));
            this.files.set(path, data);
        });
    }

    remove(path: string, options?: { recursive?: boolean }): Promise<void> {
        return this.call('remove' + (options?.recursive === true ? ' -r' : ''), path, () => {
            if (this.files.delete(path)) {
                return;
            }
            if (!this.dirs.has(path)) {
                throw new FakeCockpitError('protocol_failed', `no ${path}`);
            }
            for (const file of [...this.files.keys()]) {
                if (parentOf(file) === path && options?.recursive === true) {
                    this.files.delete(file);
                }
            }
            const hasChildren = [...this.files.keys(), ...this.dirs].some(child => child !== path && parentOf(child) === path);
            if (!hasChildren) {
                this.dirs.delete(path);
            } else if (options?.recursive !== true) {
                throw new FakeCockpitError('protocol_failed', `${path} is not empty`);
            }
            // Recursive and a subfolder is left: the real brick keeps the folder and reports nothing.
            this.onRemove?.(path);
        });
    }

    runProgram(path: string): Promise<ProgramState> {
        return this.call('runProgram', path, () => {
            if (this.runProgramError !== undefined) {
                throw new FakeCockpitError(this.runProgramError);
            }
            this.running = this.runProgramResult.running;
            this.onRun?.(path);
            return this.runProgramResult;
        });
    }

    stopProgram(): Promise<ProgramState> {
        return this.call('stopProgram', '', () => {
            this.running = false;
            return { running: false };
        });
    }

    emergencyStop(): Promise<ProgramState> {
        return this.call('emergencyStop', '', () => {
            this.running = false;
            return { running: false };
        });
    }

    isProgramRunning(): Promise<ProgramState> {
        return this.call('isProgramRunning', '', () => ({ running: this.running }));
    }

    onBrickEvent(listener: (event: BrickEvent) => void): Subscription {
        this.listeners.add(listener);
        if (this.replayOnSubscribe !== undefined) {
            listener(this.replayOnSubscribe);
        }
        return { dispose: () => this.listeners.delete(listener) };
    }

    private requireFolder(path: string): void {
        if (!this.dirs.has(path)) {
            throw new FakeCockpitError('protocol_failed', `no folder ${path}`);
        }
    }

    private call<T>(method: string, path: string, body: () => T): Promise<T> {
        this.calls.push(path === '' ? method : `${method} ${path}`);
        if (this.disconnected) {
            return Promise.reject(new FakeCockpitError('not_connected'));
        }
        try {
            return Promise.resolve(body());
        } catch (error) {
            return Promise.reject(error);
        }
    }
}

export function md5(data: Uint8Array): string {
    return crypto.createHash('md5').update(data).digest('hex');
}

function parentOf(path: string): string {
    const slash = path.lastIndexOf('/');
    return slash <= 0 ? '/' : path.slice(0, slash);
}

function nameOf(path: string): string {
    return path.slice(path.lastIndexOf('/') + 1);
}
