// The part of EVƎ Cockpit's public API (version 1) that EVƎ C uses. Cockpit publishes no
// types package, so these are copied from its sources and trimmed to what EVƎ C calls.
// Everything that crosses the API is plain data and promises, and every error carries a
// stable string `code`.

/** The cockpit extension's identifier. */
export const COCKPIT_EXTENSION_ID = '99LevelsUp.ev3-cockpit';

/** The API version EVƎ C is written against. */
export const COCKPIT_API_VERSION = 1;

/** A brick by its id, or `'active'` for the active brick at the time of the call. */
export type BrickTarget = string;

export interface Subscription {
    dispose(): void;
}

/** One brick as the API shows it. */
export interface ApiBrick {
    readonly brickId: string;
    readonly name: string;
    readonly nameConfirmed: boolean;
    readonly signedIn: boolean;
    readonly active: boolean;
    readonly paired: boolean;
    readonly channels: readonly { readonly transport: string; readonly state: string }[];
}

export interface ProgramState {
    readonly brickId: string;
    readonly running: boolean;
}

/** One entry in a brick folder listing. */
export interface FsEntry {
    readonly name: string;
    readonly kind: 'file' | 'directory' | 'unknown';
    /** Byte size, for files only. */
    readonly bytes?: number;
    /** MD5 the brick computed, lowercase hex, for files only. */
    readonly md5?: string;
}

/** What one registered extension works with; only the members EVƎ C uses. */
export interface CockpitConsumer extends Subscription {
    readonly name: string;
    listBricks(): ApiBrick[];
    getActiveBrickId(): string | undefined;
    onBricksChanged(listener: (bricks: ApiBrick[]) => void): Subscription;
    onActiveBrickChanged(listener: (brickId: string | undefined) => void): Subscription;
    list(brick: BrickTarget, path: string): Promise<FsEntry[]>;
    exists(brick: BrickTarget, path: string): Promise<boolean>;
    readFile(brick: BrickTarget, path: string): Promise<Uint8Array>;
    writeFile(brick: BrickTarget, path: string, content: Uint8Array): Promise<void>;
    makeDirectory(brick: BrickTarget, path: string): Promise<void>;
    /** Removes a file or an empty folder; `recursive` removes a folder with what it holds. */
    remove(brick: BrickTarget, path: string, options?: { recursive?: boolean }): Promise<void>;
    runProgram(brick: BrickTarget, path: string, options?: { replace?: boolean }): Promise<ProgramState>;
    stopProgram(brick: BrickTarget): Promise<ProgramState>;
    emergencyStop(brick: BrickTarget): Promise<ProgramState>;
    isProgramRunning(brick: BrickTarget): Promise<ProgramState>;
    /**
     * Changes of the user-slot state, not a replay of the current one. Throws when the brick
     * cannot be followed. The subscription ends silently when the brick's session ends.
     */
    onProgramStateChanged(brick: BrickTarget, listener: (state: ProgramState) => void): Subscription;
}

/** What cockpit's `activate()` returns. `apiVersion` is a plain number here so the check means something. */
export interface CockpitApi {
    readonly apiVersion: number;
    registerConsumer(name: string): CockpitConsumer;
}
