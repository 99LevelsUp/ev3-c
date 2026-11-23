import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { pathToFileURL } from 'node:url';
import { BuildResult, RunOutcome } from '../core';
import { cliCore, CliDeps } from '../cli/cli';
import { ConnectOptions } from '../cockpit/cockpitLink';
import { McpServer, TOOLS } from '../mcp/server';
import { FakeCockpitClient, FakeCockpitError } from './fakeCockpit';

const settle = () => new Promise(resolve => setImmediate(resolve));

/** EVƎ Cockpit's own MCP tools (`ev3 mcp`); EVƎ C's may not repeat any of them. */
const COCKPIT_TOOLS = [
    'list_bricks', 'connect_brick', 'disconnect_brick', 'brick_info', 'brick_storage', 'list_files', 'read_file', 'write_file',
    'remove_file', 'remove_paths', 'make_directory', 'move_file', 'deploy_and_run', 'run_program', 'stop_program', 'emergency_stop',
    'program_status', 'list_telemetry_rows', 'read_telemetry', 'clear_bluetooth_list', 'get_settings', 'set_setting',
];

function project(root: string, name = 'hello'): string {
    const dir = path.join(root, name);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'ev3c.json'), JSON.stringify({ name }));
    return dir;
}

function built(): Extract<BuildResult, { ok: true }> {
    return {
        ok: true, diagnostics: '',
        project: { name: 'hello', retention: { kind: 'runs', count: 5 }, outputCapBytes: 65536 },
        artifacts: [{ kind: 'program', localPath: '/tmp/prog', brickPath: '/home/root/lms2012/prjs/hello/prog' }],
    };
}

function setup(overrides: Partial<typeof cliCore> = {}) {
    const cwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-mcp-')));
    const fake = new FakeCockpitClient();
    const connects: ConnectOptions[] = [];
    const sent: Record<string, unknown>[] = [];
    const deps: CliDeps = {
        connect: async options => {
            connects.push(options);
            return fake.asClient();
        },
        core: {
            ...cliCore,
            buildProject: async () => built(),
            runProject: async (): Promise<RunOutcome> => ({
                kind: 'result',
                result: { run: 2, record: { run: 2, end: 'program', exit: 0, outTruncated: false, errTruncated: false }, out: 'hi\n', err: '', returned: [] },
                start: { outcome: 'started', runSeq: 1, uploaded: [], skipped: [] },
            }),
            ...overrides,
        },
        packageRoot: '/pkg',
        toolchainCacheDir: '/cache',
        version: '1.2.3',
        cwd,
        env: {},
        onInterrupt: () => () => undefined,
    };
    const server = new McpServer(deps, { send: message => sent.push(message as Record<string, unknown>), log: () => undefined });
    let next = 0;
    const request = async (method: string, params: object = {}): Promise<Record<string, unknown>> => {
        const id = ++next;
        await server.receive(JSON.stringify({ jsonrpc: '2.0', id, method, params }));
        const answer = sent.find(m => m.id === id);
        assert.ok(answer, `no answer to ${method}`);
        return answer;
    };
    const call = async (name: string, args: object) => (await request('tools/call', { name, arguments: args })).result as {
        content: { text: string }[]; structuredContent?: Record<string, unknown>; isError: boolean;
    };
    return { cwd, fake, connects, sent, server, request, call };
}

test('initialize answers with a version it speaks and names the server', async () => {
    const s = setup();
    const known = (await s.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'Claude Code' } })).result as Record<string, unknown>;
    assert.equal(known.protocolVersion, '2025-06-18');
    assert.deepEqual(known.serverInfo, { name: 'ev3-c', version: '1.2.3' });
    const unknown = (await s.request('initialize', { protocolVersion: '1999-01-01' })).result as Record<string, unknown>;
    assert.equal(unknown.protocolVersion, '2025-11-25');
    assert.deepEqual((await s.request('ping')).result, {});
    assert.equal(((await s.request('nothing/here')).error as { code: number }).code, -32601);
});

test('the tools work on projects and repeat none of EVƎ Cockpit\'s', async () => {
    const s = setup();
    const tools = ((await s.request('tools/list')).result as { tools: { name: string; inputSchema: { required: string[] } }[] }).tools;
    assert.deepEqual(tools.map(t => t.name), TOOLS.map(t => t.name));
    for (const tool of tools) {
        assert.ok(!COCKPIT_TOOLS.includes(tool.name), `${tool.name} is a cockpit tool`);
        assert.ok(tool.inputSchema.required.includes('project'), `${tool.name} works on a project`);
    }
});

test('run_project runs the project and answers with the run\'s object, as the agent, kind mcp', async () => {
    const s = setup();
    await s.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'Claude Code' } });
    const dir = project(s.cwd);
    const result = await s.call('run_project', { project: 'hello' });
    assert.equal(result.isError, false);
    const value = result.structuredContent as { project: string; result: { out: string; record: { exit: number } } };
    assert.equal(value.project, 'hello');
    assert.equal(value.result.out, 'hi\n');
    assert.equal(value.result.record.exit, 0);
    assert.deepEqual(JSON.parse(result.content[0].text), value);
    assert.deepEqual(s.connects.map(c => [c.name, c.kind]), [['Claude Code (EVƎ C MCP)', 'mcp']]);
    assert.ok(fs.existsSync(dir));
});

test('a project outside the working folder is refused when the client declares no roots', async () => {
    const s = setup();
    await s.request('initialize', { capabilities: {} });
    const elsewhere = project(fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-mcp-out-')));
    const result = await s.call('build_project', { project: elsewhere });
    assert.equal(result.isError, true);
    assert.match(result.content[0].text, /outside the folders this agent may work in/);
    assert.deepEqual(s.connects, []);
});

test('with roots, a project inside one of them is allowed', async () => {
    const s = setup();
    await s.request('initialize', { capabilities: { roots: {} } });
    const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-mcp-root-')));
    const dir = project(root, 'robot');
    const calling = s.server.receive(JSON.stringify({ jsonrpc: '2.0', id: 'c1', method: 'tools/call', params: { name: 'build_project', arguments: { project: dir } } }));
    await settle();
    const ask = s.sent.find(m => m.method === 'roots/list');
    assert.ok(ask, 'the server asks for the roots');
    await s.server.receive(JSON.stringify({ jsonrpc: '2.0', id: ask.id, result: { roots: [{ uri: pathToFileURL(root).href }] } }));
    await calling;
    const result = (s.sent.find(m => m.id === 'c1')?.result) as { isError: boolean };
    assert.equal(result.isError, false);
});

test('bad arguments and a cockpit refusal come back as tool errors, not protocol errors', async () => {
    const s = setup();
    project(s.cwd);
    assert.match((await s.call('show_project_run', { project: 'hello' })).content[0].text, /"run" is required/);
    assert.match((await s.call('show_project_run', { project: 'hello', run: 0 })).content[0].text, /"run" must be a whole number from 1/);
    assert.match((await s.call('build_project', { project: 'hello', extra: 1 })).content[0].text, /unknown argument "extra"/);
    assert.equal((await s.call('no_such_tool', {})).isError, true);

    s.fake.failures.set('bricks/connect', new FakeCockpitError('brick_busy', 'EVƎ C (VS Code) is writing to the brick.'));
    const busy = await s.call('list_project_runs', { project: 'hello' });
    assert.equal(busy.isError, true);
    assert.deepEqual(busy.structuredContent, { error: { message: 'EVƎ C (VS Code) is writing to the brick.', code: 'brick_busy' } });
});

test('cancelling a run stops the program, as Ctrl+C does', async () => {
    let release!: () => void;
    const stopped = new Promise<void>(resolve => { release = resolve; });
    let stops = 0;
    const s = setup({
        runProject: async (): Promise<RunOutcome> => {
            await stopped;
            return {
                kind: 'result',
                result: { run: 2, record: { run: 2, end: 'slot', signal: 15, outTruncated: false, errTruncated: false }, out: '', err: '', returned: [] },
                start: { outcome: 'started', runSeq: 1, uploaded: [], skipped: [] },
            };
        },
        stopProgram: async port => {
            stops++;
            release();
            return port.stopProgram();
        },
    });
    project(s.cwd);
    const calling = s.server.receive(JSON.stringify({ jsonrpc: '2.0', id: 'r1', method: 'tools/call', params: { name: 'run_project', arguments: { project: 'hello' } } }));
    for (let i = 0; i < 5; i++) {
        await settle();
    }
    await s.server.receive(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/cancelled', params: { requestId: 'r1' } }));
    await calling;
    assert.equal(stops, 1);
    const result = s.sent.find(m => m.id === 'r1')?.result as { structuredContent: { result: { record: { end: string } } } };
    assert.equal(result.structuredContent.result.record.end, 'slot');
});

test('a line that is not JSON gets a parse error', async () => {
    const s = setup();
    await s.server.receive('{not json');
    assert.equal((s.sent[0].error as { code: number }).code, -32700);
});
