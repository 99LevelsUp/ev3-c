import * as fs from 'node:fs/promises';
import * as os from 'node:os';
import * as path from 'node:path';
import { CockpitClient } from '@ev3-cockpit/client';
import type {
    ClientStatus, ContractBrick, DeployFile, DeployResult, ErrorCode, ProgramState as ContractState,
} from '@ev3-cockpit/client';
import type { BrickEvent, BrickPort, DeployItem, FileSource, ProgramState, StartReport, Subscription } from '../core';

// EVƎ C's link to EVƎ Cockpit's daemon. This is the only module that imports the cockpit's
// client library, so a change of the library touches this file only. It does not import
// VS Code: the `ev3 c` command can use it as it is.

export { CockpitClient };
/** The error codes of cockpit's contract. */
export type CockpitErrorCode = ErrorCode;

/** Inline content of a file write is limited to this many bytes. */
export const INLINE_WRITE_LIMIT = 4096;

/** An error raised before reaching cockpit, coded like cockpit's own. */
export class CockpitLinkError extends Error {
    constructor(message: string, readonly code: string) {
        super(message);
        this.name = 'CockpitLinkError';
    }
}

export interface ConnectOptions {
    /** Says where EVƎ C runs: other clients read it in `brick_busy` and `stoppedBy`. */
    readonly name: string;
    /** EVƎ C's version. */
    readonly version: string;
    /** A test channel; by default EV3_COCKPIT_CHANNEL or the user's own daemon. */
    readonly channel?: string;
}

/**
 * Connects to the user's cockpit daemon, starting the one the installed cockpit recorded
 * when none runs. A start failure is the library's `DaemonStartError` (code
 * `daemon_unavailable`, with `failure`).
 */
export function connectCockpit(options: ConnectOptions): Promise<CockpitClient> {
    const channel = options.channel ?? process.env.EV3_COCKPIT_CHANNEL;
    return CockpitClient.connect({
        name: options.name,
        kind: 'addon',
        version: options.version,
        ...(channel ? { channel } : {}),
    });
}

/** Why the daemon could not start, when the error is a start failure. */
export function startFailure(error: unknown): string | undefined {
    if (typeof error === 'object' && error !== null && 'failure' in error) {
        const failure = (error as { failure: unknown }).failure;
        return typeof failure === 'string' ? failure : undefined;
    }
    return undefined;
}

/** Whether the brick is connected with a live link and not reconnecting. */
export function isBrickOnline(bricks: readonly ContractBrick[], brickId: string): boolean {
    return bricks.some(b => b.brickId === brickId && b.connected && b.reconnecting !== true
        && b.channels.some(c => c.state === 'connected'));
}

/**
 * The bricks the cockpit's VS Code windows show, kept current from `daemon/status` and
 * `daemon/showsChanged`. Each window is a client of kind `vscode`; with several windows,
 * several bricks may be shown.
 */
export class ShownBricks {
    private clients: readonly ClientStatus[] = [];
    private readonly listeners = new Set<() => void>();
    private readonly watches: { dispose(): void }[];

    private constructor(private readonly client: CockpitClient) {
        this.watches = [
            client.on('daemon/showsChanged', p => this.set(p.clients)),
            // A new daemon knows the windows anew; ask it.
            client.onReconnected(() => { void this.refresh(); }),
        ];
    }

    /** Follows what the windows show; a daemon that cannot say counts as showing nothing. */
    static async follow(client: CockpitClient): Promise<ShownBricks> {
        const shown = new ShownBricks(client);
        await shown.refresh();
        return shown;
    }

    /** The distinct brick ids the cockpit's windows show. */
    bricks(): string[] {
        return [...new Set(this.clients.filter(c => c.kind === 'vscode' && c.shows !== undefined).map(c => c.shows as string))];
    }

    onChanged(listener: () => void): { dispose(): void } {
        this.listeners.add(listener);
        return { dispose: () => this.listeners.delete(listener) };
    }

    dispose(): void {
        this.listeners.clear();
        for (const watch of this.watches) {
            watch.dispose();
        }
    }

    private async refresh(): Promise<void> {
        try {
            this.set((await this.client.request('daemon/status', {})).clients);
        } catch {
            this.set([]);
        }
    }

    private set(clients: readonly ClientStatus[]): void {
        this.clients = clients;
        for (const listener of [...this.listeners]) {
            listener();
        }
    }
}

/**
 * The brick the `ev3-c.brick` setting names now. A name or id is passed on as written
 * (cockpit matches names without regard to case and refuses an unknown one). `active`
 * means the brick the cockpit's VS Code window shows; when no window shows one, the only
 * connected brick. Windows showing different bricks leave the choice to the setting.
 */
export function resolveBrick(client: CockpitClient, setting: string, shown: readonly string[] = []): string {
    if (setting !== 'active') {
        return client.brick(setting)?.brickId ?? setting;
    }
    // A window may still name a brick it disconnected; only a connected one counts.
    const live = shown.filter(id => client.bricks().some(b => b.brickId === id && b.connected));
    if (live.length === 1) {
        return live[0];
    }
    if (live.length > 1) {
        const names = live.map(id => client.brick(id)?.name ?? id);
        throw new CockpitLinkError(
            `EVƎ Cockpit's windows show different bricks (${names.join(', ')}); set "ev3-c.brick" to the one EVƎ C should use.`,
            'brick_ambiguous');
    }
    const online = client.bricks().filter(b => isBrickOnline([b], b.brickId));
    if (online.length === 1) {
        return online[0].brickId;
    }
    if (online.length === 0) {
        throw new CockpitLinkError('No brick is connected.', 'not_connected');
    }
    throw new CockpitLinkError(
        `Several bricks are connected (${online.map(b => b.name).join(', ')}); set "ev3-c.brick" to the one EVƎ C should use.`,
        'brick_ambiguous');
}

/** A BrickPort bound to one brick; `dispose` ends its subscriptions. */
export interface CockpitPort extends BrickPort {
    readonly brickId: string;
    dispose(): void;
}

export function createBrickPort(client: CockpitClient, brickId: string): CockpitPort {
    const brick = brickId;
    const events = new BrickEvents(client, brickId);
    return {
        brickId,
        list: async p => [...(await client.request('files/list', { brick, path: p })).entries],
        exists: async p => (await client.request('files/exists', { brick, path: p })).exists,
        readFile: p => readThroughLocalFile(client, brick, p),
        writeFile: async (p, source) => {
            await client.request('files/write', { brick, path: p, ...writeSource(source) });
        },
        removePaths: async (paths, options) => {
            const report = await client.request('files/removePaths', { brick, paths: [...paths], recursive: options?.recursive === true });
            return { leftBehind: report.leftBehind };
        },
        deployAndRun: async (files, runPath) => {
            const result = await client.request('programs/deployAndRun', { brick, files: files.map(deployFile), runPath });
            return startReport(result);
        },
        stopProgram: async () => portState(await client.request('programs/stop', { brick })),
        emergencyStop: async () => portState(await client.request('programs/emergencyStop', { brick })),
        programState: async () => portState(await client.request('programs/status', { brick })),
        onBrickEvent: listener => events.add(listener),
        dispose: () => events.dispose(),
    };
}

/**
 * Reads a brick file through a private temporary file the daemon writes. An inline read
 * is refused above 64 KiB, and a run's output may be larger (`outputCapKiB`), so every
 * read goes to a local file, whatever its size.
 */
async function readThroughLocalFile(client: CockpitClient, brick: string, brickPath: string): Promise<Uint8Array> {
    const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'ev3c-read-'));
    try {
        const local = path.join(dir, 'file');
        await client.request('files/read', { brick, path: brickPath, toLocalPath: local });
        return await fs.readFile(local);
    } finally {
        await fs.rm(dir, { recursive: true, force: true });
    }
}

function writeSource(source: FileSource): { fromLocalPath: string } | { content: string } {
    if ('localPath' in source) {
        // The daemon's working folder is not ours.
        return { fromLocalPath: path.resolve(source.localPath) };
    }
    if (source.data.length > INLINE_WRITE_LIMIT) {
        throw new CockpitLinkError(`Inline content is limited to ${INLINE_WRITE_LIMIT} bytes.`, 'invalid_request');
    }
    return { content: Buffer.from(source.data).toString('base64') };
}

function deployFile(item: DeployItem): DeployFile {
    const source = writeSource(item.source);
    return {
        remotePath: item.brickPath,
        ...('fromLocalPath' in source ? { localPath: source.fromLocalPath } : { content: source.content }),
        ...(item.skipIfSame === true ? { skipIfSameMd5: true } : {}),
    };
}

function startReport(result: DeployResult): StartReport {
    return { outcome: result.outcome, runSeq: result.runSeq, uploaded: result.uploaded, skipped: result.skipped };
}

function portState(state: ContractState): ProgramState {
    return { running: state.running, runSeq: state.runSeq };
}

/**
 * One program-state subscription shared by all listeners of a port. The library renews it
 * when the daemon is replaced, but it ends silently when the brick is signed out, so the
 * brick list is watched: when the brick is back the subscription is made again. Listeners
 * hear `reconnected` whenever the state may have changed unseen: the brick or the daemon
 * came back.
 */
class BrickEvents {
    private readonly listeners = new Set<(event: BrickEvent) => void>();
    private program: Subscription | undefined;
    private subscribing: Promise<void> | undefined;
    private watches: Subscription[] = [];
    private online = false;

    constructor(private readonly client: CockpitClient, private readonly brickId: string) {}

    async add(listener: (event: BrickEvent) => void): Promise<Subscription> {
        if (this.program === undefined) {
            // Without a live subscription the failure goes to the caller, who would
            // otherwise wait for events that never come.
            await this.subscribe();
            // Listeners that were waiting for the brick to come back hear that it has.
            this.emit({ kind: 'reconnected' });
        }
        this.watch();
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
        for (const watch of this.watches) {
            watch.dispose();
        }
        this.watches = [];
    }

    private subscribe(): Promise<void> {
        this.subscribing ??= this.client.subscribePrograms(this.brickId, state => {
            this.emit({ kind: 'program', state: portState(state) });
        }).then(subscription => {
            this.program = subscription;
            this.online = true;
        }).finally(() => {
            this.subscribing = undefined;
        });
        return this.subscribing;
    }

    private watch(): void {
        if (this.watches.length > 0) {
            return;
        }
        this.watches = [
            this.client.onBricksChanged(bricks => this.onBricks(bricks)),
            // The route came back after a loss; the subscription carried on, but a change may have been missed.
            this.client.on('bricks/reconnected', p => {
                if (p.brickId === this.brickId && this.program !== undefined) {
                    this.emit({ kind: 'reconnected' });
                }
            }),
            // A new daemon: the library renewed the subscription.
            this.client.onReconnected(() => {
                if (this.program !== undefined) {
                    this.emit({ kind: 'reconnected' });
                }
            }),
        ];
    }

    private onBricks(bricks: readonly ContractBrick[]): void {
        const online = isBrickOnline(bricks, this.brickId);
        if (!online) {
            if (this.online) {
                // Signed out, and the subscription with it.
                this.online = false;
                this.program?.dispose();
                this.program = undefined;
            }
            return;
        }
        if (this.program !== undefined || this.subscribing !== undefined || this.listeners.size === 0) {
            return;
        }
        this.subscribe().then(
            () => this.emit({ kind: 'reconnected' }),
            () => undefined, // Not followable yet; the next change of the brick list tries again.
        );
    }

    private emit(event: BrickEvent): void {
        for (const listener of [...this.listeners]) {
            try {
                listener(event);
            } catch {
                // A listener's own failure stops no other listener.
            }
        }
    }
}
