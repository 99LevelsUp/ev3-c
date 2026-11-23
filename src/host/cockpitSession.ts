import { BrickFileReader } from '../core';
import { CockpitClient, CockpitPort, createBrickPort, isBrickOnline, resolveBrick, ShownBricks } from '../cockpit/cockpitLink';

// The host's connection to EVƎ Cockpit's daemon. It connects on first use, not at
// activation, so a daemon that is not ready yet (the cockpit records how to start it when
// it starts itself) costs one failed command, not every brick command until a reload.

/** A brick's port; the port reads files itself. */
export interface BrickHandle {
    readonly port: CockpitPort;
    readonly reader: BrickFileReader;
}

/** One port per brick id for the whole connection, so its state subscription is shared. */
export class BrickPorts {
    private readonly handles = new Map<string, BrickHandle>();

    constructor(private readonly client: CockpitClient) {}

    get(brickId: string): BrickHandle {
        let handle = this.handles.get(brickId);
        if (handle === undefined) {
            const port = createBrickPort(this.client, brickId);
            handle = { port, reader: port };
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

export interface Connected {
    readonly client: CockpitClient;
    readonly ports: BrickPorts;
    /** The bricks the cockpit's windows show, which `active` follows. */
    readonly shown: ShownBricks;
}

/** The brick the setting names now on this connection. */
export function resolveSetting(connected: Connected, setting: string): string {
    return resolveBrick(connected.client, setting, connected.shown.bricks());
}

export class CockpitSession {
    private connected: Connected | undefined;
    private connecting: Promise<Connected> | undefined;
    private readonly listeners = new Set<(connected: Connected | undefined) => void>();
    private disposed = false;
    /** The brick EVƎ C holds, on the connection it holds it on. */
    private held: { readonly connected: Connected; readonly brickId: string } | undefined;

    constructor(private readonly connect: () => Promise<CockpitClient>) {}

    /** The connection, made now if there is none; a failed attempt is tried again on the next call. */
    ready(): Promise<Connected> {
        if (this.connected !== undefined) {
            return Promise.resolve(this.connected);
        }
        this.connecting ??= this.connect().then(async client => {
            const shown = await ShownBricks.follow(client);
            if (this.disposed) {
                shown.dispose();
                client.dispose();
                throw new Error('EVƎ C is shutting down.');
            }
            const connected = { client, ports: new BrickPorts(client), shown };
            this.connected = connected;
            // No daemon could be reached again: the next command connects afresh.
            client.onLost(() => {
                if (this.connected === connected) {
                    this.connected = undefined;
                    connected.ports.dispose();
                    shown.dispose();
                    client.dispose();
                    this.fire(undefined);
                }
            });
            this.fire(connected);
            return connected;
        }).finally(() => {
            this.connecting = undefined;
        });
        return this.connecting;
    }

    /** The connection if there is one, without connecting. */
    current(): Connected | undefined {
        return this.connected;
    }

    /** Hears the connection once it is made, and `undefined` once it is lost for good. */
    onChanged(listener: (connected: Connected | undefined) => void): { dispose(): void } {
        this.listeners.add(listener);
        return { dispose: () => this.listeners.delete(listener) };
    }

    /**
     * The port of the brick the setting names now; connects to the cockpit first when needed.
     * EVƎ C holds the brick itself (`bricks/connect` without a hold), so it stays connected
     * for EVƎ C when whoever connected it lets it go. The hold ends with EVƎ C's connection,
     * the library renews it on a new daemon, and a different brick releases the previous one.
     */
    async brick(setting: string): Promise<BrickHandle> {
        const connected = await this.ready();
        const { client, ports } = connected;
        const wanted = resolveSetting(connected, setting);
        if (this.held?.connected !== connected || this.held.brickId !== wanted || !isBrickOnline(client.bricks(), wanted)) {
            const brickId = (await client.request('bricks/connect', { brick: wanted })).brickId;
            const previous = this.held;
            this.held = { connected, brickId };
            if (previous?.connected === connected && previous.brickId !== brickId) {
                void client.request('bricks/disconnect', { brick: previous.brickId }).catch(() => undefined);
            }
            return ports.get(brickId);
        }
        return ports.get(wanted);
    }

    private fire(connected: Connected | undefined): void {
        for (const listener of [...this.listeners]) {
            listener(connected);
        }
    }

    dispose(): void {
        this.disposed = true;
        this.listeners.clear();
        this.connected?.ports.dispose();
        this.connected?.shown.dispose();
        this.connected?.client.dispose();
        this.connected = undefined;
    }
}
