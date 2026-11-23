import * as fs from 'node:fs';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import { CliDeps, CliIo, runCli } from '../cli/cli';

// `ev3 c mcp`: EVƎ C's project operations as MCP tools for an agent, over stdio.
//
// - Standard output carries MCP messages only, one JSON-RPC message per line; anything else
//   goes to standard error. The server ends when its client closes standard input.
// - The tools are the `ev3 c` commands that work on a project: build, upload, run, and the
//   project's runs. Bricks, files, stopping and telemetry are EVƎ Cockpit's own MCP tools
//   (`ev3 mcp`); nothing here repeats them.
// - Each tool runs the command with --json and answers with its object. Nothing reaches the
//   cockpit's daemon until a tool needs a brick, and then as kind `mcp`, named after the
//   agent, which can never confirm anything on a person's behalf.
// - A project folder must lie inside the roots the MCP client declares, or inside the working
//   folder when it declares none: the tools build there and write returned files below it.
// - Cancelling a run stops the program, as Ctrl+C does in a terminal.

/** The protocol versions this server speaks; the first is the newest. */
const PROTOCOL_VERSIONS = ['2025-11-25', '2025-06-18', '2025-03-26', '2024-11-05'];

/** How this server introduces itself to the daemon when the client gave no name. */
export const MCP_CLIENT_NAME = 'agent';

interface ToolSpec {
    readonly name: string;
    readonly description: string;
    readonly inputSchema: {
        readonly type: 'object';
        readonly properties: Readonly<Record<string, { readonly type: 'string' | 'integer'; readonly description: string; readonly minimum?: number }>>;
        readonly required?: readonly string[];
        readonly additionalProperties: false;
    };
    /** The `ev3 c` arguments for a call; the project folder is resolved and checked already. */
    readonly argv: (args: Readonly<Record<string, unknown>>, project: string) => string[];
}

const PROJECT = { type: 'string', description: 'The project folder on this computer: the folder that holds ev3c.json.' } as const;
const BRICK = { type: 'string', description: 'The brick, by name or id. Default: the brick EVƎ Cockpit\'s VS Code window shows, else the only connected brick.' } as const;

export const TOOLS: readonly ToolSpec[] = [
    {
        name: 'build_project',
        description: 'Build an EVƎ C project (C for the LEGO EV3 with its original firmware) into the program, its launcher and the supervisor. Returns the artifacts and any compiler warnings; a failed build returns the compiler errors.',
        inputSchema: { type: 'object', properties: { project: PROJECT }, required: ['project'], additionalProperties: false },
        argv: (_args, project) => ['build', project],
    },
    {
        name: 'upload_project',
        description: 'Build an EVƎ C project and upload what changed to the brick, without running it. Refused while a program runs on the brick.',
        inputSchema: { type: 'object', properties: { project: PROJECT, brick: BRICK }, required: ['project'], additionalProperties: false },
        argv: (args, project) => ['upload', project, ...brickArg(args)],
    },
    {
        name: 'run_project',
        description: 'Build an EVƎ C project, upload it and run it on the brick, then wait for the run to end and return its result: how it ended (exit code or signal), its stdout and stderr, and the files it returned, which are downloaded into the project\'s returned/run-<n> folder. Cancelling the call stops the program. To stop a program you did not start here, use EVƎ Cockpit\'s stop_program.',
        inputSchema: { type: 'object', properties: { project: PROJECT, brick: BRICK }, required: ['project'], additionalProperties: false },
        argv: (args, project) => ['run', project, ...brickArg(args)],
    },
    {
        name: 'list_project_runs',
        description: 'List an EVƎ C project\'s runs kept on the brick, oldest first, with how each ended; a run without a record is still running or was interrupted.',
        inputSchema: { type: 'object', properties: { project: PROJECT, brick: BRICK }, required: ['project'], additionalProperties: false },
        argv: (args, project) => ['runs', project, ...brickArg(args)],
    },
    {
        name: 'show_project_run',
        description: 'Return one run of an EVƎ C project: how it ended, its output, and the files it returned, downloaded into the project\'s returned/run-<n> folder.',
        inputSchema: {
            type: 'object',
            properties: { project: PROJECT, run: { type: 'integer', minimum: 1, description: 'The run number, as list_project_runs gives it.' }, brick: BRICK },
            required: ['project', 'run'],
            additionalProperties: false,
        },
        argv: (args, project) => ['runs', 'show', String(args.run), project, ...brickArg(args)],
    },
    {
        name: 'delete_project_runs',
        description: 'Delete an EVƎ C project\'s run folders on the brick, all of them or all but the newest `keep`. The folder of a run in progress is never deleted.',
        inputSchema: {
            type: 'object',
            properties: { project: PROJECT, keep: { type: 'integer', minimum: 0, description: 'How many of the newest runs to keep; 0 or none deletes all.' }, brick: BRICK },
            required: ['project'],
            additionalProperties: false,
        },
        argv: (args, project) => ['runs', 'delete', project, ...(args.keep !== undefined ? ['--keep', String(args.keep)] : []), ...brickArg(args)],
    },
];

function brickArg(args: Readonly<Record<string, unknown>>): string[] {
    return typeof args.brick === 'string' && args.brick.trim() !== '' ? ['--brick', args.brick.trim()] : [];
}

type JsonRpcId = string | number;
interface Message {
    readonly jsonrpc?: string;
    readonly id?: JsonRpcId | null;
    readonly method?: string;
    readonly params?: Record<string, unknown>;
    readonly result?: unknown;
    readonly error?: unknown;
}

export interface McpStreams {
    /** Sends one JSON-RPC message. */
    send(message: object): void;
    /** Writes to standard error. */
    log(text: string): void;
}

export class McpServer {
    private clientName = MCP_CLIENT_NAME;
    private rootsCapable = false;
    private nextAsk = 0;
    private readonly asked = new Map<string, (message: Message) => void>();
    /** Ctrl+C handlers of running tool calls, by request id. */
    private readonly cancels = new Map<string, () => void>();

    constructor(private readonly deps: CliDeps, private readonly streams: McpStreams) {}

    /** Handles one line of input. */
    async receive(line: string): Promise<void> {
        let message: Message;
        try {
            message = JSON.parse(line) as Message;
        } catch {
            this.streams.send({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'Parse error' } });
            return;
        }
        if (message.method === undefined) {
            // An answer to something this server asked the client.
            const key = String(message.id);
            this.asked.get(key)?.(message);
            this.asked.delete(key);
            return;
        }
        if (message.id === undefined || message.id === null) {
            if (message.method === 'notifications/cancelled') {
                this.cancels.get(String(message.params?.requestId))?.();
            }
            return;
        }
        const id = message.id;
        try {
            const result = await this.handle(message.method, message.params ?? {}, id);
            if (result !== undefined) {
                this.streams.send({ jsonrpc: '2.0', id, result });
            }
        } catch (error) {
            this.streams.send({ jsonrpc: '2.0', id, error: { code: -32603, message: error instanceof Error ? error.message : String(error) } });
        }
    }

    private async handle(method: string, params: Record<string, unknown>, id: JsonRpcId): Promise<unknown> {
        switch (method) {
        case 'initialize': return this.initialize(params);
        case 'ping': return {};
        case 'tools/list': return { tools: TOOLS.map(({ name, description, inputSchema }) => ({ name, description, inputSchema })) };
        case 'tools/call': return this.call(params, id);
        default:
            this.streams.send({ jsonrpc: '2.0', id, error: { code: -32601, message: `Method not found: ${method}` } });
            return undefined;
        }
    }

    private initialize(params: Record<string, unknown>): object {
        const info = params.clientInfo as { name?: unknown } | undefined;
        if (typeof info?.name === 'string' && info.name.trim() !== '') {
            this.clientName = info.name.trim().slice(0, 150);
        }
        this.rootsCapable = typeof params.capabilities === 'object' && params.capabilities !== null && 'roots' in params.capabilities;
        const asked = typeof params.protocolVersion === 'string' ? params.protocolVersion : '';
        return {
            protocolVersion: PROTOCOL_VERSIONS.includes(asked) ? asked : PROTOCOL_VERSIONS[0],
            capabilities: { tools: {} },
            serverInfo: { name: 'ev3-c', version: this.deps.version },
        };
    }

    private async call(params: Record<string, unknown>, id: JsonRpcId): Promise<object> {
        const tool = TOOLS.find(t => t.name === params.name);
        if (tool === undefined) {
            return toolError(`Unknown tool ${String(params.name)}.`);
        }
        const args = (typeof params.arguments === 'object' && params.arguments !== null ? params.arguments : {}) as Record<string, unknown>;
        const problem = validate(tool, args);
        if (problem !== undefined) {
            return toolError(`Invalid arguments for ${tool.name}: ${problem}`);
        }
        let project: string;
        try {
            project = await this.confine(String(args.project));
        } catch (error) {
            return toolError(error instanceof Error ? error.message : String(error));
        }

        const out: string[] = [];
        const err: string[] = [];
        const io: CliIo = { out: text => out.push(text), err: text => err.push(text) };
        const key = String(id);
        const deps: CliDeps = {
            ...this.deps,
            connect: options => this.deps.connect({ ...options, name: `${this.clientName} (EVƎ C MCP)`, kind: 'mcp' }),
            onInterrupt: handler => {
                this.cancels.set(key, handler);
                return () => { this.cancels.delete(key); };
            },
        };
        const code = await runCli([...tool.argv(args, project), '--json'], io, deps);
        const printed = out.join('\n');
        let value: Record<string, unknown> | undefined;
        try {
            value = JSON.parse(printed) as Record<string, unknown>;
        } catch {
            value = undefined;
        }
        if (value === undefined) {
            // A usage problem or a failure before any object: what the command said.
            return toolError([printed, ...err].filter(text => text !== '').join('\n') || `ev3 c exited with ${code}.`);
        }
        const notes = err.filter(text => text.trim() !== '');
        const result = notes.length > 0 ? { ...value, messages: notes } : value;
        return {
            content: [{ type: 'text', text: JSON.stringify(result, null, 2) }],
            structuredContent: result,
            isError: 'error' in value,
        };
    }

    /** The project folder, resolved and checked to lie inside the client's roots. */
    private async confine(given: string): Promise<string> {
        const roots = await this.roots();
        const resolved = path.resolve(this.deps.cwd, given);
        const real = fs.existsSync(resolved) ? fs.realpathSync(resolved) : resolved;
        const inside = roots.some(root => {
            const relative = path.relative(root, real);
            return relative === '' || (!relative.startsWith('..') && !path.isAbsolute(relative));
        });
        if (!inside) {
            throw new Error(`${given} lies outside the folders this agent may work in (${roots.join(', ')}).`);
        }
        return real;
    }

    /** The client's roots as local folders, or the working folder when it declares none. */
    private async roots(): Promise<string[]> {
        const fallback = [fs.realpathSync(this.deps.cwd)];
        if (!this.rootsCapable) {
            return fallback;
        }
        const answer = await this.ask('roots/list', {});
        const roots = (answer?.result as { roots?: { uri?: unknown }[] } | undefined)?.roots;
        if (!Array.isArray(roots)) {
            return fallback;
        }
        const folders = roots.flatMap(root => {
            if (typeof root.uri !== 'string' || !root.uri.startsWith('file:')) {
                return [];
            }
            const folder = fileURLToPath(root.uri);
            return [fs.existsSync(folder) ? fs.realpathSync(folder) : folder];
        });
        return folders.length > 0 ? folders : fallback;
    }

    /** Sends a request to the client and waits for its answer, at most 30 s. */
    private ask(method: string, params: object): Promise<Message | undefined> {
        const id = `ev3-c-${++this.nextAsk}`;
        return new Promise(resolve => {
            const timer = setTimeout(() => {
                this.asked.delete(id);
                resolve(undefined);
            }, 30_000);
            timer.unref?.();
            this.asked.set(id, message => {
                clearTimeout(timer);
                resolve(message);
            });
            this.streams.send({ jsonrpc: '2.0', id, method, params });
        });
    }
}

function validate(tool: ToolSpec, args: Readonly<Record<string, unknown>>): string | undefined {
    const { properties, required = [] } = tool.inputSchema;
    for (const name of required) {
        if (args[name] === undefined) {
            return `"${name}" is required`;
        }
    }
    for (const [name, value] of Object.entries(args)) {
        const spec = properties[name];
        if (spec === undefined) {
            return `unknown argument "${name}"`;
        }
        if (spec.type === 'string' && typeof value !== 'string') {
            return `"${name}" must be a string`;
        }
        if (spec.type === 'integer' && (typeof value !== 'number' || !Number.isInteger(value) || value < (spec.minimum ?? Number.MIN_SAFE_INTEGER))) {
            return `"${name}" must be a whole number${spec.minimum !== undefined ? ` from ${spec.minimum}` : ''}`;
        }
    }
    return undefined;
}

function toolError(text: string): object {
    return { content: [{ type: 'text', text }], isError: true };
}

/** Serves MCP on this process's standard input and output until the client closes input. */
export function serveStdio(deps: CliDeps): Promise<number> {
    const server = new McpServer(deps, {
        send: message => { process.stdout.write(`${JSON.stringify(message)}\n`); },
        log: text => { process.stderr.write(`${text}\n`); },
    });
    return new Promise(resolve => {
        let buffer = '';
        process.stdin.setEncoding('utf8');
        process.stdin.on('data', (chunk: string) => {
            buffer += chunk;
            for (let newline = buffer.indexOf('\n'); newline >= 0; newline = buffer.indexOf('\n')) {
                const line = buffer.slice(0, newline).trim();
                buffer = buffer.slice(newline + 1);
                if (line !== '') {
                    void server.receive(line);
                }
            }
        });
        process.stdin.on('end', () => resolve(0));
    });
}
