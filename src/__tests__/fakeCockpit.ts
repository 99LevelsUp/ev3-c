import { ApiBrick, CockpitConsumer, FsEntry, ProgramState, Subscription } from '../host/cockpitTypes';

// A stand-in for EVƎ Cockpit's consumer handle. It records every call with its arguments,
// fails a method with a coded error when told to, and behaves like cockpit where the host
// depends on it: program-state subscriptions deliver changes only, and a brick's session
// ending ends them without telling anyone.

export class FakeCockpitError extends Error {
    constructor(readonly code: string, message = `cockpit refused: ${code}`) {
        super(message);
    }
}

export const BRICK_ID = 'usb-0016533f1234';

export class FakeConsumer implements CockpitConsumer {
    readonly name = 'ev3-c';
    /** Every call in order, as [method, ...args]. */
    readonly calls: unknown[][] = [];
    /** Errors the next call of a method throws (or rejects with). */
    readonly failures = new Map<string, unknown>();
    bricks: ApiBrick[] = [brick(BRICK_ID, { signedIn: true, active: true })];
    running = false;
    disposed = false;
    private readonly programListeners = new Set<{ brick: string; listener: (state: ProgramState) => void }>();
    private readonly bricksListeners = new Set<(bricks: ApiBrick[]) => void>();
    private readonly activeListeners = new Set<(brickId: string | undefined) => void>();

    /** Number of program-state subscriptions that are live. */
    get programSubscriptions(): number {
        return this.programListeners.size;
    }

    listBricks(): ApiBrick[] {
        this.calls.push(['listBricks']);
        return this.bricks;
    }

    getActiveBrickId(): string | undefined {
        this.calls.push(['getActiveBrickId']);
        return this.bricks.find(b => b.active)?.brickId;
    }

    onBricksChanged(listener: (bricks: ApiBrick[]) => void): Subscription {
        this.calls.push(['onBricksChanged']);
        this.bricksListeners.add(listener);
        return { dispose: () => this.bricksListeners.delete(listener) };
    }

    onActiveBrickChanged(listener: (brickId: string | undefined) => void): Subscription {
        this.calls.push(['onActiveBrickChanged']);
        this.activeListeners.add(listener);
        return { dispose: () => this.activeListeners.delete(listener) };
    }

    list(brick: string, path: string): Promise<FsEntry[]> {
        return this.call('list', [brick, path], [{ name: 'prog', kind: 'file', bytes: 10, md5: 'abc' }]);
    }

    exists(brick: string, path: string): Promise<boolean> {
        return this.call('exists', [brick, path], true);
    }

    readFile(brick: string, path: string): Promise<Uint8Array> {
        return this.call('readFile', [brick, path], new Uint8Array([1, 2, 3]));
    }

    writeFile(brick: string, path: string, content: Uint8Array): Promise<void> {
        return this.call('writeFile', [brick, path, content], undefined);
    }

    makeDirectory(brick: string, path: string): Promise<void> {
        return this.call('makeDirectory', [brick, path], undefined);
    }

    remove(brick: string, path: string, options?: { recursive?: boolean }): Promise<void> {
        return this.call('remove', [brick, path, options], undefined);
    }

    runProgram(brick: string, path: string): Promise<ProgramState> {
        return this.call('runProgram', [brick, path], { brickId: BRICK_ID, running: true });
    }

    stopProgram(brick: string): Promise<ProgramState> {
        return this.call('stopProgram', [brick], { brickId: BRICK_ID, running: false });
    }

    emergencyStop(brick: string): Promise<ProgramState> {
        return this.call('emergencyStop', [brick], { brickId: BRICK_ID, running: false });
    }

    isProgramRunning(brick: string): Promise<ProgramState> {
        return this.call('isProgramRunning', [brick], { brickId: BRICK_ID, running: this.running });
    }

    onProgramStateChanged(brick: string, listener: (state: ProgramState) => void): Subscription {
        this.calls.push(['onProgramStateChanged', brick]);
        const failure = this.takeFailure('onProgramStateChanged');
        if (failure !== undefined) {
            throw failure;
        }
        const entry = { brick, listener };
        this.programListeners.add(entry);
        return { dispose: () => this.programListeners.delete(entry) };
    }

    dispose(): void {
        this.disposed = true;
    }

    // Test controls

    /** The user-slot state changes on the brick. */
    setRunning(running: boolean): void {
        this.running = running;
        for (const { brick: target, listener } of [...this.programListeners]) {
            listener({ brickId: target, running });
        }
    }

    /**
     * The brick's session ends: its program subscriptions end silently, then the list
     * changes. With `stillSignedIn` the brick stays signed in while its link is lost.
     */
    disconnect(brickId = BRICK_ID, stillSignedIn = false): void {
        for (const entry of [...this.programListeners]) {
            if (entry.brick === brickId) {
                this.programListeners.delete(entry);
            }
        }
        this.bricks = this.bricks.map(b => (b.brickId === brickId
            ? { ...b, signedIn: stillSignedIn, channels: [{ transport: 'usb', state: 'lost' }] }
            : b));
        this.notifyBricks();
    }

    /** The brick is signed in and connected again. */
    reconnect(brickId = BRICK_ID): void {
        this.bricks = this.bricks.map(b => (b.brickId === brickId
            ? { ...b, signedIn: true, channels: [{ transport: 'usb', state: 'connected' }] }
            : b));
        this.notifyBricks();
    }

    notifyBricks(): void {
        for (const listener of [...this.bricksListeners]) {
            listener(this.bricks);
        }
    }

    private takeFailure(method: string): unknown {
        const failure = this.failures.get(method);
        this.failures.delete(method);
        return failure;
    }

    private call<T>(method: string, args: unknown[], result: T): Promise<T> {
        this.calls.push([method, ...args]);
        const failure = this.takeFailure(method);
        return failure === undefined ? Promise.resolve(result) : Promise.reject(failure);
    }
}

export function brick(brickId: string, options: { signedIn: boolean; active: boolean }): ApiBrick {
    return {
        brickId,
        name: 'EV3',
        nameConfirmed: true,
        signedIn: options.signedIn,
        active: options.active,
        paired: false,
        channels: [{ transport: 'usb', state: options.signedIn ? 'connected' : 'available' }],
    };
}
