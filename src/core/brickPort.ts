// The only way the core reaches a brick. The VS Code layer adapts EVƎ Cockpit's
// consumer handle to this interface for the brick the author picked, so nothing in the
// core depends on VS Code or on cockpit's own types.

/** One entry of a brick folder listing. */
export interface BrickEntry {
    readonly name: string;
    readonly kind: 'file' | 'directory' | 'unknown';
    readonly bytes?: number;
    /** Hex MD5 of a file's content, when cockpit reports it. */
    readonly md5?: string;
}

/** The user-slot state cockpit reports. */
export interface ProgramState {
    readonly running: boolean;
}

/**
 * What the state subscription delivers: a change of the user-slot state, or the brick
 * coming back after its connection was lost. Cockpit has no reconnect event of its own;
 * the adapter sends `reconnected` once it has subscribed again after a lost connection.
 * The subscription delivers changes only, not a replay of the current state.
 */
export type BrickEvent =
    | { readonly kind: 'program'; readonly running: boolean }
    | { readonly kind: 'reconnected' };

export interface Subscription {
    dispose(): void;
}

/**
 * Brick operations the core needs. Paths are absolute brick paths shorter than 60 bytes.
 * Failures are thrown as errors carrying cockpit's stable `code` (see `brickErrorCode`).
 */
export interface BrickPort {
    list(path: string): Promise<BrickEntry[]>;
    exists(path: string): Promise<boolean>;
    makeDirectory(path: string): Promise<void>;
    readFile(path: string): Promise<Uint8Array>;
    writeFile(path: string, data: Uint8Array): Promise<void>;
    remove(path: string, options?: { recursive?: boolean }): Promise<void>;
    runProgram(path: string): Promise<ProgramState>;
    stopProgram(): Promise<ProgramState>;
    emergencyStop(): Promise<ProgramState>;
    isProgramRunning(): Promise<ProgramState>;
    onBrickEvent(listener: (event: BrickEvent) => void): Subscription;
}

/** Cockpit's error code of a failed brick call, if the error carries one. */
export function brickErrorCode(error: unknown): string | undefined {
    if (typeof error === 'object' && error !== null && 'code' in error) {
        const code = (error as { code: unknown }).code;
        return typeof code === 'string' ? code : undefined;
    }
    return undefined;
}

/** Time source, replaceable in tests so waits take no real time. */
export interface Clock {
    /** Milliseconds since the epoch. */
    now(): number;
    sleep(ms: number): Promise<void>;
}

export const systemClock: Clock = {
    now: () => Date.now(),
    sleep: ms => new Promise(resolve => setTimeout(resolve, ms)),
};

/** Shortest gap between two file reads; cockpit's readFile serves small files about once per second. */
export const READ_INTERVAL_MS = 1000;

/** Reads brick files; `PacedReader` is the implementation the host shares across operations. */
export interface BrickFileReader {
    readFile(path: string): Promise<Uint8Array>;
}

/**
 * Keeps file reads at most one per second, also across operations that share it.
 * The host keeps one per port.
 */
export class PacedReader implements BrickFileReader {
    private lastStart = Number.NEGATIVE_INFINITY;
    private queue: Promise<unknown> = Promise.resolve();

    constructor(private readonly port: BrickPort, private readonly clock: Clock = systemClock) {}

    readFile(path: string): Promise<Uint8Array> {
        const read = this.queue.then(async () => {
            const wait = this.lastStart + READ_INTERVAL_MS - this.clock.now();
            if (wait > 0) {
                await this.clock.sleep(wait);
            }
            this.lastStart = this.clock.now();
            return this.port.readFile(path);
        });
        this.queue = read.catch(() => undefined);
        return read;
    }
}
