import * as crypto from 'node:crypto';
import * as fs from 'node:fs';
import {
    BrickEntry, BrickEvent, BrickPort, Clock, DeployItem, FileSource, ProgramState, StartOutcome, StartReport, Subscription,
} from '../core/brickPort';

// An in-memory brick behind the BrickPort interface, with a virtual clock, for the
// brick-operation tests. It behaves like the cockpit daemon where the core depends on it:
// writes create missing folders, listings carry each file's MD5, a start refuses while a
// program runs and numbers every start, and every call fails with "not_connected" while
// disconnected. Its recursive removal deletes one folder level only and reports what it
// left, so the core cannot rely on a removal being complete.

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
    /** The number of the last start; every start counts, a refused one too. */
    runSeq = 0;
    /** How the next starts end; `started` by default. */
    startOutcome: StartOutcome = 'started';
    /** Error code a start fails with after its files are uploaded, if set. */
    startError: string | undefined;
    disconnected = false;
    /** Called inside deployAndRun once the program "runs", before the call returns. */
    onRun: ((path: string) => void) | undefined;
    /** An event sent to each new subscriber at once, like a replay of an earlier state. */
    replayOnSubscribe: BrickEvent | undefined;
    /** Called inside removePaths, after the removal. */
    onRemove: ((paths: readonly string[]) => void) | undefined;
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

    state(): ProgramState {
        return { running: this.running, runSeq: this.runSeq };
    }

    /** Tells the subscribers the program state now. */
    emitState(): void {
        this.emit({ kind: 'program', state: this.state() });
    }

    /** The program stops and the subscribers hear it. */
    stop(): void {
        this.running = false;
        this.emitState();
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

    writeFile(path: string, source: FileSource): Promise<void> {
        return this.call('writeFile', path, () => this.write(path, source));
    }

    removePaths(paths: readonly string[], options?: { recursive?: boolean }): Promise<{ leftBehind: readonly string[] }> {
        return this.call('removePaths' + (options?.recursive === true ? ' -r' : ''), paths.join(' '), () => {
            const leftBehind: string[] = [];
            for (const path of paths) {
                if (this.files.delete(path) || !this.dirs.has(path)) {
                    continue;
                }
                for (const file of [...this.files.keys()]) {
                    if (parentOf(file) === path && options?.recursive === true) {
                        this.files.delete(file);
                    }
                }
                const hasChildren = [...this.files.keys(), ...this.dirs].some(child => child !== path && parentOf(child) === path);
                if (hasChildren) {
                    leftBehind.push(path);
                } else {
                    this.dirs.delete(path);
                }
            }
            this.onRemove?.(paths);
            return { leftBehind };
        });
    }

    deployAndRun(files: readonly DeployItem[], runPath: string): Promise<StartReport> {
        return this.call('deployAndRun', runPath, () => {
            if (this.running) {
                throw new FakeCockpitError('program_running');
            }
            const uploaded: string[] = [];
            const skipped: string[] = [];
            for (const item of files) {
                const data = content(item.source);
                const there = this.files.get(item.brickPath);
                if (item.skipIfSame === true && there !== undefined && md5(there) === md5(data)) {
                    skipped.push(item.brickPath);
                    continue;
                }
                this.write(item.brickPath, item.source);
                uploaded.push(item.brickPath);
            }
            this.runSeq++;
            if (this.startError !== undefined) {
                throw new FakeCockpitError(this.startError);
            }
            this.running = this.startOutcome !== 'not_seen_running';
            this.onRun?.(runPath);
            return { outcome: this.startOutcome, runSeq: this.runSeq, uploaded, skipped };
        });
    }

    stopProgram(): Promise<ProgramState> {
        return this.call('stopProgram', '', () => {
            this.running = false;
            return this.state();
        });
    }

    emergencyStop(): Promise<ProgramState> {
        return this.call('emergencyStop', '', () => {
            this.running = false;
            return this.state();
        });
    }

    programState(): Promise<ProgramState> {
        return this.call('programState', '', () => this.state());
    }

    onBrickEvent(listener: (event: BrickEvent) => void): Promise<Subscription> {
        return this.call('onBrickEvent', '', () => {
            this.listeners.add(listener);
            if (this.replayOnSubscribe !== undefined) {
                listener(this.replayOnSubscribe);
            }
            return { dispose: () => { this.listeners.delete(listener); } };
        });
    }

    private write(path: string, source: FileSource): void {
        const failure = this.writeFailures.get(path)?.shift();
        if (failure !== undefined) {
            throw new FakeCockpitError(failure);
        }
        this.putFile(path, content(source));
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

function content(source: FileSource): Uint8Array {
    return 'localPath' in source ? fs.readFileSync(source.localPath) : source.data;
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
