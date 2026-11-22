import { BrickFileReader } from '../core';
import { CockpitClient, CockpitPort, createBrickPort, resolveBrick } from '../cockpit/nativeCockpit';

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
}

export class CockpitSession {
    private connected: Connected | undefined;
    private connecting: Promise<Connected> | undefined;
    private readonly listeners = new Set<(connected: Connected | undefined) => void>();
    private disposed = false;

    constructor(private readonly connect: () => Promise<CockpitClient>) {}

    /** The connection, made now if there is none; a failed attempt is tried again on the next call. */
    ready(): Promise<Connected> {
        if (this.connected !== undefined) {
            return Promise.resolve(this.connected);
        }
        this.connecting ??= this.connect().then(client => {
            if (this.disposed) {
                client.dispose();
                throw new Error('EVƎ C is shutting down.');
            }
            const connected = { client, ports: new BrickPorts(client) };
            this.connected = connected;
            // No daemon could be reached again: the next command connects afresh.
            client.onLost(() => {
                if (this.connected === connected) {
                    this.connected = undefined;
                    connected.ports.dispose();
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

    /** The port of the brick the setting names now; connects first when needed. */
    async brick(setting: string): Promise<BrickHandle> {
        const { client, ports } = await this.ready();
        return ports.get(resolveBrick(client, setting));
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
        this.connected?.client.dispose();
        this.connected = undefined;
    }
}
