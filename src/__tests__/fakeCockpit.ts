import * as fs from 'node:fs';
import type { CockpitClient } from '../cockpit/nativeCockpit';

// A stand-in for EVƎ Cockpit's native client, as far as EVƎ C uses it: the cached brick
// list, `request` for the contract methods EVƎ C sends, program-state subscriptions, and
// the notifications EVƎ C listens to. Like the library, a program subscription ends
// silently when the brick is signed out: it stays open but hears nothing more.

export const BRICK_ID = 'usb-001653000aaa';

export class FakeCockpitError extends Error {
    constructor(readonly code: string, message = code, readonly data: Record<string, unknown> = {}) {
        super(message);
    }
}

interface FakeBrick {
    brickId: string;
    name: string;
    nameConfirmed: boolean;
    connected: boolean;
    paired: boolean;
    channels: { transport: 'usb' | 'bt' | 'tcp'; state: 'available' | 'connected' | 'lost' }[];
    holders: { kind: 'client' | 'hold' | 'remembered'; client?: string }[];
    reconnecting?: boolean;
}

export function fakeBrick(brickId: string, name: string, connected = true): FakeBrick {
    return {
        brickId, name, nameConfirmed: true, connected, paired: false, holders: [],
        channels: [{ transport: 'usb', state: connected ? 'connected' : 'available' }],
    };
}

type State = { brickId: string; running: boolean; runSeq: number; stoppedBy?: string };
type Listener<T> = (value: T) => void;

interface ProgramSubscription {
    brick: string;
    listener: Listener<State>;
    alive: boolean;
    open: boolean;
}

export class FakeCockpitClient {
    brickList: FakeBrick[] = [fakeBrick(BRICK_ID, 'AAA')];
    /** Every request as [method, params]. */
    readonly calls: [string, Record<string, unknown>][] = [];
    /** Errors the next request of a method fails with. */
    readonly failures = new Map<string, Error>();
    /** File contents `files/read` answers with, by path. */
    readonly files = new Map<string, Uint8Array>();
    running = false;
    runSeq = 0;
    disposed = false;
    questionHandler: unknown;
    readonly daemon = { version: '0.0.0-test' };
    private readonly subscriptions: ProgramSubscription[] = [];
    private readonly bricksListeners = new Set<Listener<readonly FakeBrick[]>>();
    private readonly reconnectedListeners = new Set<Listener<unknown>>();
    private readonly lostListeners = new Set<Listener<Error>>();
    private readonly handlers = new Map<string, Set<Listener<unknown>>>();

    /** This fake typed as the client EVƎ C takes. */
    asClient(): CockpitClient {
        return this as unknown as CockpitClient;
    }

    /** Subscriptions that are open and still hear changes. */
    get liveSubscriptions(): number {
        return this.subscriptions.filter(s => s.open && s.alive).length;
    }

    /** Subscriptions EVƎ C has not disposed. */
    get openSubscriptions(): number {
        return this.subscriptions.filter(s => s.open).length;
    }

    // ----- scripting -----

    setRunning(running: boolean, runSeq = this.runSeq): void {
        this.running = running;
        this.runSeq = runSeq;
        for (const s of this.subscriptions.filter(x => x.open && x.alive)) {
            s.listener(this.state(s.brick));
        }
    }

    /** The brick is signed out: its subscriptions hear nothing more. */
    signOut(brickId = BRICK_ID): void {
        for (const s of this.subscriptions.filter(x => x.brick === brickId)) {
            s.alive = false;
        }
        this.setConnected(brickId, false);
    }

    signIn(brickId = BRICK_ID): void {
        this.setConnected(brickId, true);
    }

    notifyBricks(): void {
        for (const listener of [...this.bricksListeners]) {
            listener(this.brickList);
        }
    }

    notify(name: string, params: unknown): void {
        for (const handler of [...(this.handlers.get(name) ?? [])]) {
            handler(params);
        }
    }

    daemonReplaced(): void {
        for (const listener of [...this.reconnectedListeners]) {
            listener(this.daemon);
        }
    }

    lose(error: Error): void {
        for (const listener of [...this.lostListeners]) {
            listener(error);
        }
    }

    // ----- the client -----

    bricks(): readonly FakeBrick[] {
        return this.brickList;
    }

    brick(ref: string): FakeBrick | undefined {
        const wanted = ref.trim().toLowerCase();
        return this.brickList.find(b => b.brickId === ref) ?? this.brickList.find(b => b.name.toLowerCase() === wanted);
    }

    onBricksChanged(listener: Listener<readonly FakeBrick[]>): { dispose(): void } {
        this.bricksListeners.add(listener);
        return { dispose: () => this.bricksListeners.delete(listener) };
    }

    onReconnected(listener: Listener<unknown>): { dispose(): void } {
        this.reconnectedListeners.add(listener);
        return { dispose: () => this.reconnectedListeners.delete(listener) };
    }

    onLost(listener: Listener<Error>): { dispose(): void } {
        this.lostListeners.add(listener);
        return { dispose: () => this.lostListeners.delete(listener) };
    }

    onNotice(): { dispose(): void } {
        return { dispose: () => undefined };
    }

    onQuestion(handler: unknown): void {
        this.questionHandler = handler;
    }

    on(name: string, handler: Listener<unknown>): { dispose(): void } {
        let set = this.handlers.get(name);
        if (!set) {
            set = new Set();
            this.handlers.set(name, set);
        }
        const handlers = set;
        handlers.add(handler);
        return { dispose: () => handlers.delete(handler) };
    }

    async request(method: string, params: Record<string, unknown>): Promise<unknown> {
        await Promise.resolve();
        this.calls.push([method, params]);
        const failure = this.failures.get(method);
        if (failure !== undefined) {
            this.failures.delete(method);
            throw failure;
        }
        const brickId = String(params.brick ?? '');
        switch (method) {
        case 'files/list': return { entries: [{ name: 'prog', kind: 'file', bytes: 3, md5: 'abc' }] };
        case 'files/exists': return { exists: true };
        case 'files/read': {
            const data = this.files.get(String(params.path)) ?? new Uint8Array();
            if (typeof params.toLocalPath === 'string') {
                fs.writeFileSync(params.toLocalPath, data);
                return { bytes: data.length };
            }
            return { bytes: data.length, content: Buffer.from(data).toString('base64') };
        }
        case 'files/write': return { bytes: 0 };
        case 'files/removePaths': return { leftBehind: [] };
        case 'programs/deployAndRun': {
            this.runSeq++;
            this.running = true;
            const files = params.files as { remotePath: string }[];
            return { brickId, outcome: 'started', runSeq: this.runSeq, uploaded: files.map(f => f.remotePath), skipped: [] };
        }
        case 'programs/stop':
        case 'programs/emergencyStop':
            this.running = false;
            return this.state(brickId);
        case 'programs/status': return this.state(brickId);
        default: throw new FakeCockpitError('invalid_request', `the fake does not know ${method}`);
        }
    }

    async subscribePrograms(brick: string, listener: Listener<State>): Promise<{ readonly state: State; dispose(): void }> {
        await Promise.resolve();
        this.calls.push(['programs/subscribe', { brick }]);
        const failure = this.failures.get('programs/subscribe');
        if (failure !== undefined) {
            this.failures.delete('programs/subscribe');
            throw failure;
        }
        const subscription: ProgramSubscription = { brick, listener, alive: true, open: true };
        this.subscriptions.push(subscription);
        return { state: this.state(brick), dispose: () => { subscription.open = false; } };
    }

    dispose(): void {
        this.disposed = true;
    }

    private state(brickId: string): State {
        return { brickId, running: this.running, runSeq: this.runSeq };
    }

    private setConnected(brickId: string, connected: boolean): void {
        this.brickList = this.brickList.map(b => (b.brickId === brickId
            ? { ...b, connected, channels: b.channels.map(c => ({ ...c, state: connected ? 'connected' : 'available' })) }
            : b));
        this.notifyBricks();
    }
}
