// The only way the core reaches a brick. The host adapts EVƎ Cockpit's client to this
// interface for the brick the author picked, so nothing in the core depends on VS Code or
// on cockpit's own types.

/** One entry of a brick folder listing. */
export interface BrickEntry {
    readonly name: string;
    readonly kind: 'file' | 'directory' | 'unknown';
    readonly bytes?: number;
    /** Hex MD5 of a file's content, when cockpit reports it. */
    readonly md5?: string;
}

/**
 * The user-slot state cockpit reports. `runSeq` numbers the starts cockpit sent to the
 * brick (0 before any): a state with a `runSeq` at least a start's own is about that run
 * or a later one, never an earlier.
 */
export interface ProgramState {
    readonly running: boolean;
    readonly runSeq: number;
}

/**
 * What the state subscription delivers: a change of the user-slot state, or the brick
 * coming back after its connection was lost, when the state may have changed unseen.
 */
export type BrickEvent =
    | { readonly kind: 'program'; readonly state: ProgramState }
    | { readonly kind: 'reconnected' };

export interface Subscription {
    dispose(): void;
}

/** File content to put on the brick: a file on this computer, or a few bytes. */
export type FileSource =
    | { readonly localPath: string }
    | { readonly data: Uint8Array };

/** One file of a deployment. */
export interface DeployItem {
    readonly brickPath: string;
    readonly source: FileSource;
    /** Leave the brick's copy when its MD5 matches. */
    readonly skipIfSame?: boolean;
}

/**
 * How a start ended: `started` (seen running), `not_seen_running` (it may have ended at
 * once), or `uncertain` (the reply was lost; read the state, never start again).
 */
export type StartOutcome = 'started' | 'not_seen_running' | 'uncertain';

export interface StartReport {
    readonly outcome: StartOutcome;
    /** This start's number; see `ProgramState.runSeq`. */
    readonly runSeq: number;
    readonly uploaded: readonly string[];
    readonly skipped: readonly string[];
}

/**
 * Brick operations the core needs. Paths are absolute brick paths shorter than 60 bytes.
 * Failures are thrown as errors carrying cockpit's stable `code` (see `brickErrorCode`).
 */
export interface BrickPort {
    list(path: string): Promise<BrickEntry[]>;
    exists(path: string): Promise<boolean>;
    readFile(path: string): Promise<Uint8Array>;
    /** Writes one file; missing folders are created. */
    writeFile(path: string, source: FileSource): Promise<void>;
    /** Removes several paths at once; returns what a recursive removal left behind. */
    removePaths(paths: readonly string[], options?: { recursive?: boolean }): Promise<{ leftBehind: readonly string[] }>;
    /**
     * Checks that no program runs, uploads the files, and starts `runPath`, as one operation
     * no other client can interleave with. Refused with `program_running` while one runs.
     */
    deployAndRun(files: readonly DeployItem[], runPath: string): Promise<StartReport>;
    stopProgram(): Promise<ProgramState>;
    emergencyStop(): Promise<ProgramState>;
    programState(): Promise<ProgramState>;
    /** Follows the brick's program state; resolves once the subscription is live. */
    onBrickEvent(listener: (event: BrickEvent) => void): Promise<Subscription>;
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

/** Reads brick files; a port is one. Kept apart so result readers need nothing else of the port. */
export interface BrickFileReader {
    readFile(path: string): Promise<Uint8Array>;
}
