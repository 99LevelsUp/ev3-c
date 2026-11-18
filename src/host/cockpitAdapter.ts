import { BrickEvent, BrickFileReader, BrickPort, PacedReader, ProgramState as PortState, Subscription } from '../core';
import {
    ApiBrick, COCKPIT_API_VERSION, COCKPIT_EXTENSION_ID, CockpitApi, CockpitConsumer, ProgramState, Subscription as CockpitSubscription,
} from './cockpitTypes';

// Connects EVƎ C to EVƎ Cockpit and turns cockpit's consumer handle into the core's
// BrickPort for one brick. Every call goes to cockpit unchanged, so a refusal keeps
// cockpit's error object and its `code`.

/** The name EVƎ C registers with cockpit. */
export const CONSUMER_NAME = 'ev3-c';

/** An error the host raises before reaching cockpit, coded like cockpit's own. */
export class HostBrickError extends Error {
    constructor(message: string, readonly code: string) {
        super(message);
        this.name = 'HostBrickError';
    }
}

export interface CockpitExtension {
    activate(): Thenable<unknown> | Promise<unknown>;
}

export type CockpitConnection =
    | { readonly ok: true; readonly api: CockpitApi; readonly consumer: CockpitConsumer }
    | { readonly ok: false; readonly problem: string };

const NO_BRICK = 'EVƎ C cannot reach a brick; building still works.';

/** Activates cockpit, checks its API version, and registers EVƎ C as a consumer. */
export async function connectCockpit(getExtension: (id: string) => CockpitExtension | undefined): Promise<CockpitConnection> {
    const extension = getExtension(COCKPIT_EXTENSION_ID);
    if (extension === undefined) {
        return { ok: false, problem: `EVƎ Cockpit (${COCKPIT_EXTENSION_ID}) is not installed or not enabled. Install or enable it, then reload the window. ${NO_BRICK}` };
    }
    let api: CockpitApi;
    try {
        api = await extension.activate() as CockpitApi;
    } catch (error) {
        return { ok: false, problem: `EVƎ Cockpit failed to start: ${messageOf(error)} ${NO_BRICK}` };
    }
    const version = (api as Partial<CockpitApi> | undefined)?.apiVersion;
    if (version !== COCKPIT_API_VERSION) {
        return {
            ok: false,
            problem: `EVƎ C needs EVƎ Cockpit API version ${COCKPIT_API_VERSION}, but the installed EVƎ Cockpit offers version ${String(version)}. `
                + `Install an EVƎ Cockpit that matches this EVƎ C. ${NO_BRICK}`,
        };
    }
    try {
        return { ok: true, api, consumer: api.registerConsumer(CONSUMER_NAME) };
    } catch (error) {
        return { ok: false, problem: `EVƎ Cockpit refused to register EVƎ C: ${messageOf(error)} ${NO_BRICK}` };
    }
}

/**
 * The brick id the `ev3-c.brick` setting names now: the active brick for `active`,
 * otherwise the id as written (cockpit refuses an id it does not know).
 */
export function resolveBrickId(consumer: CockpitConsumer, setting: string): string {
    if (setting !== 'active') {
        return setting;
    }
    const active = consumer.getActiveBrickId();
    if (active === undefined) {
        throw new HostBrickError('No brick is active in EVƎ Cockpit.', 'not_connected');
    }
    return active;
}

/** A BrickPort bound to one brick; `dispose` ends its cockpit subscriptions. */
export interface CockpitPort extends BrickPort {
    readonly brickId: string;
    dispose(): void;
}

export function createCockpitPort(consumer: CockpitConsumer, brickId: string): CockpitPort {
    const events = new BrickEvents(consumer, brickId);
    return {
        brickId,
        list: path => consumer.list(brickId, path),
        exists: path => consumer.exists(brickId, path),
        makeDirectory: path => consumer.makeDirectory(brickId, path),
        readFile: path => consumer.readFile(brickId, path),
        writeFile: (path, data) => consumer.writeFile(brickId, path, data),
        remove: (path, options) => consumer.remove(brickId, path, options),
        runProgram: async path => portState(await consumer.runProgram(brickId, path)),
        stopProgram: async () => portState(await consumer.stopProgram(brickId)),
        emergencyStop: async () => portState(await consumer.emergencyStop(brickId)),
        isProgramRunning: async () => portState(await consumer.isProgramRunning(brickId)),
        onBrickEvent: listener => events.add(listener),
        dispose: () => events.dispose(),
    };
}

/** A brick's port and the one paced reader every operation on that brick shares. */
export interface BrickHandle {
    readonly port: CockpitPort;
    readonly reader: BrickFileReader;
}

/**
 * One port and one reader per brick id for the whole session, so operations that overlap
 * (Show Runs while a Run waits for its result) still read at most one file per second.
 */
export class BrickPorts {
    private readonly handles = new Map<string, BrickHandle>();

    constructor(private readonly consumer: CockpitConsumer) {}

    get(brickId: string): BrickHandle {
        let handle = this.handles.get(brickId);
        if (handle === undefined) {
            const port = createCockpitPort(this.consumer, brickId);
            handle = { port, reader: new PacedReader(port) };
            this.handles.set(brickId, handle);
        }
        return handle;
    }

    dispose(): void {
        for (const handle of this.handles.values()) {
            handle.port.dispose();
        }
        this.handles.clear();
    }
}

/**
 * One cockpit program-state subscription shared by all listeners of a port. Cockpit ends
 * that subscription silently when the brick's session ends, so the brick list is watched:
 * when the brick is signed in again the subscription is made again, and the listeners
 * hear `reconnected`, because the state may have changed in between.
 */
class BrickEvents {
    private readonly listeners = new Set<(event: BrickEvent) => void>();
    private program: CockpitSubscription | undefined;
    private bricks: CockpitSubscription | undefined;
    private signedIn = false;

    constructor(private readonly consumer: CockpitConsumer, private readonly brickId: string) {}

    add(listener: (event: BrickEvent) => void): Subscription {
        if (this.program === undefined) {
            // Without a live cockpit subscription the failure goes to the caller, who would
            // otherwise wait for events that never come.
            this.program = this.subscribe();
            this.signedIn = true;
            // Listeners that were waiting for the brick to come back hear that it has.
            this.emit({ kind: 'reconnected' });
        }
        this.bricks ??= this.consumer.onBricksChanged(bricks => this.onBricks(bricks));
        this.listeners.add(listener);
        let open = true;
        return {
            dispose: () => {
                if (open) {
                    open = false;
                    this.listeners.delete(listener);
                    if (this.listeners.size === 0) {
                        this.dispose();
                    }
                }
            },
        };
    }

    dispose(): void {
        this.listeners.clear();
        this.program?.dispose();
        this.program = undefined;
        this.bricks?.dispose();
        this.bricks = undefined;
    }

    private subscribe(): CockpitSubscription {
        return this.consumer.onProgramStateChanged(this.brickId, (state: ProgramState) => {
            this.emit({ kind: 'program', running: state.running });
        });
    }

    private onBricks(bricks: readonly ApiBrick[]): void {
        const signedIn = isBrickOnline(bricks, this.brickId);
        if (!signedIn) {
            if (this.signedIn) {
                // The session ended, and cockpit's subscription with it.
                this.signedIn = false;
                this.program?.dispose();
                this.program = undefined;
            }
            return;
        }
        this.signedIn = true;
        if (this.program !== undefined) {
            return;
        }
        try {
            this.program = this.subscribe();
        } catch {
            // Not followable yet; the next change of the brick list tries again.
            return;
        }
        this.emit({ kind: 'reconnected' });
    }

    private emit(event: BrickEvent): void {
        for (const listener of [...this.listeners]) {
            listener(event);
        }
    }
}

/**
 * Whether the brick is signed in with a live link. Cockpit may keep a brick signed in while
 * its link is lost and coming back, so a connected channel is required too; a channel's
 * state change is what makes cockpit report the brick list again.
 */
export function isBrickOnline(bricks: readonly ApiBrick[], brickId: string): boolean {
    return bricks.some(b => b.brickId === brickId && b.signedIn && b.channels.some(c => c.state === 'connected'));
}

function portState(state: ProgramState): PortState {
    return { running: state.running };
}

function messageOf(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}
