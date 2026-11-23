import { afterEach, beforeEach, test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { activate, CLIENT_NAME, cockpitConnector } from '../extension';
import { BuildProjectOptions, BuildResult } from '../core';
import { connectCockpit } from '../cockpit/cockpitLink';
import { COMMAND_IDS, defaultCore, registerProjectCommands } from '../host/commands';
import { BRICK_ID, FakeCockpitClient, FakeCockpitError } from './fakeCockpit';

// The vscode stub's test controls (see src/test/vscodeStub.cjs).
interface StubState {
    messages: { level: string; text: string }[];
    commands: Map<string, unknown>;
    output: string[];
    log: { level: string; line: string }[];
    statusBarItems: { text: string; visible: boolean; command?: string }[];
    config: Map<string, unknown>;
    workspaceFolders: unknown;
    isTrusted: boolean;
    extensions: Map<string, unknown>;
}
const stub = vscode as unknown as { __state: StubState; __reset(): void };

const ALL_COMMANDS = [
    'ev3-c.build', 'ev3-c.upload', 'ev3-c.run', 'ev3-c.stop', 'ev3-c.emergencyStop',
    'ev3-c.showRuns', 'ev3-c.deleteRuns', 'ev3-c.checkToolchain',
];

function context(): vscode.ExtensionContext {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-host-'));
    return {
        subscriptions: [],
        extensionPath: dir,
        globalStorageUri: { fsPath: path.join(dir, 'storage') },
        extension: { packageJSON: { version: '1.2.3' } },
    } as unknown as vscode.ExtensionContext;
}

/** Cockpit's daemon answers through a fake client; the connections made are recorded. */
function installCockpit(): { fake: FakeCockpitClient; connects: { name: string; version: string }[] } {
    const fake = new FakeCockpitClient();
    const connects: { name: string; version: string }[] = [];
    cockpitConnector.connect = async options => {
        connects.push({ name: options.name, version: options.version });
        return fake.asClient();
    };
    return { fake, connects };
}

/** No daemon can be started: the cockpit is not installed. */
function noCockpit(): { attempts: number } {
    const counter = { attempts: 0 };
    cockpitConnector.connect = async () => {
        counter.attempts++;
        throw Object.assign(new FakeCockpitError('daemon_unavailable', 'No daemon was recorded.'), { failure: 'missing_binary' });
    };
    return counter;
}

function projectFolder(config: object | undefined): string {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-project-'));
    if (config !== undefined) {
        fs.writeFileSync(path.join(dir, 'ev3c.json'), JSON.stringify(config));
    }
    fs.writeFileSync(path.join(dir, 'main.c'), 'int main(void) { return 0; }\n');
    stub.__state.workspaceFolders = [{ uri: { fsPath: dir }, name: path.basename(dir), index: 0 }];
    return dir;
}

const settle = () => new Promise(resolve => setImmediate(resolve));
const errors = () => stub.__state.messages.filter(m => m.level === 'error');
const brickCalls = (fake: FakeCockpitClient) => fake.calls.filter(([method]) => !['programs/subscribe', 'programs/status', 'bricks/connect', 'daemon/status', 'log/notice'].includes(method));

beforeEach(() => {
    stub.__reset();
    stub.__state.config.set('ev3-c.brick', 'active');
    stub.__state.config.set('ev3-c.toolchain.path', '');
});

afterEach(() => {
    cockpitConnector.connect = connectCockpit;
});

test('without cockpit, activation registers every command, logs a warning and shows no error', async () => {
    const counter = noCockpit();
    activate(context());
    await settle();
    assert.deepEqual(errors(), []);
    assert.deepEqual([...stub.__state.commands.keys()].sort(), [...ALL_COMMANDS].sort());
    assert.ok(stub.__state.log.some(l => l.level === 'warn' && /could not connect to EVƎ Cockpit/.test(l.line)));
    assert.match(stub.__state.statusBarItems[0].text, /not connected/);

    // A brick command tries again and says why it cannot reach the brick.
    await vscode.commands.executeCommand('ev3-c.stop');
    assert.equal(counter.attempts, 2);
    assert.equal(errors().length, 1);
    assert.match(errors()[0].text, /not installed, or has not run yet/);
    assert.match(errors()[0].text, /missing_binary/);
});

test('with cockpit, EVƎ C connects as "EVƎ C (VS Code)" with its version and the status bar follows the slot', async () => {
    const { fake, connects } = installCockpit();
    activate(context());
    await settle();
    assert.deepEqual(errors(), []);
    assert.deepEqual(connects, [{ name: CLIENT_NAME, version: '1.2.3' }]);
    assert.equal(typeof fake.questionHandler, 'function', 'questions for a person are answered');
    assert.deepEqual([...stub.__state.commands.keys()].sort(), [...ALL_COMMANDS].sort());
    assert.deepEqual(Object.values(COMMAND_IDS).sort(), [...ALL_COMMANDS].sort());

    const item = stub.__state.statusBarItems[0];
    assert.equal(item.visible, true);
    assert.match(item.text, /idle/i);
    fake.setRunning(true, 1);
    assert.match(item.text, /running/i);

    // Signed out, the subscription ends; once the brick is back the bar asks again.
    fake.signOut();
    assert.match(item.text, /no brick/i);
    fake.running = false;
    fake.signIn();
    await settle();
    await settle();
    assert.match(item.text, /idle/i);
    fake.setRunning(true, 2);
    assert.match(item.text, /running/i, 'the status follows the new subscription');
});

test('a daemon lost for good is connected afresh by the next brick command', async () => {
    const first = installCockpit();
    activate(context());
    await settle();
    const second = new FakeCockpitClient();
    cockpitConnector.connect = async () => second.asClient();

    first.fake.lose(new Error('gone'));
    assert.equal(first.fake.disposed, true);
    assert.match(stub.__state.statusBarItems[0].text, /not connected/);

    await vscode.commands.executeCommand('ev3-c.stop');
    assert.deepEqual(brickCalls(second), [['programs/stop', { brick: BRICK_ID }]]);
});

test('package.json contributes every command under the "EV3 C" category, and both settings', () => {
    const manifest = JSON.parse(fs.readFileSync(path.join(__dirname, '..', '..', 'package.json'), 'utf8')) as {
        contributes: { commands: { command: string; category: string }[]; configuration: { properties: Record<string, { default: unknown }> } };
    };
    const contributed = manifest.contributes.commands;
    assert.deepEqual(contributed.map(c => c.command).sort(), [...ALL_COMMANDS].sort());
    assert.ok(contributed.every(c => c.category === 'EV3 C'));
    const properties = manifest.contributes.configuration.properties;
    assert.equal(properties['ev3-c.brick'].default, 'active');
    assert.equal(properties['ev3-c.toolchain.path'].default, '');
});

test('Build in an untrusted workspace is refused with a message and starts no compiler', async () => {
    projectFolder({ name: 'hello' });
    stub.__state.isTrusted = false;
    const builds: BuildProjectOptions[] = [];
    const output = vscode.window.createOutputChannel('EVƎ C', { log: true });
    registerProjectCommands({
        output,
        packageRoot: os.tmpdir(),
        toolchainCacheDir: os.tmpdir(),
        core: {
            ...defaultCore,
            buildProject: async (options: BuildProjectOptions): Promise<BuildResult> => {
                builds.push(options);
                return { ok: false, diagnostics: '' };
            },
        },
    });
    await vscode.commands.executeCommand('ev3-c.build');
    assert.deepEqual(builds, []);
    assert.equal(stub.__state.messages.length, 1);
    assert.match(stub.__state.messages[0].text, /trust/i);
    // The failure lands in the log at error level, with the message shown, so it lines up with EVƎ Cockpit's log.
    assert.equal(stub.__state.log.length, 2);
    assert.equal(stub.__state.log[0].level, 'error');
    assert.match(stub.__state.log[0].line, /ev3-c\.build failed/);
    assert.equal(stub.__state.log[1].level, 'error');
    assert.match(stub.__state.log[1].line, /^Shown: .*trust/i);
});

test('Upload and Run in an untrusted workspace are refused and send cockpit nothing', async () => {
    const { fake } = installCockpit();
    activate(context());
    await settle();
    projectFolder({ name: 'hello' });
    stub.__state.isTrusted = false;
    for (const id of ['ev3-c.upload', 'ev3-c.run']) {
        fake.calls.length = 0;
        stub.__state.messages.length = 0;
        await vscode.commands.executeCommand(id);
        assert.deepEqual(brickCalls(fake), [], id);
        assert.equal(stub.__state.messages.length, 1, id);
        assert.match(stub.__state.messages[0].text, /trust/i, id);
    }
});

test('Run with no project folder open shows a message and sends cockpit nothing', async () => {
    const { fake } = installCockpit();
    activate(context());
    await settle();
    fake.calls.length = 0;
    await vscode.commands.executeCommand('ev3-c.run');
    assert.deepEqual(brickCalls(fake), []);
    assert.equal(errors().length, 1);
    assert.match(errors()[0].text, /ev3c\.json/);
});

test('Run with an invalid project name shows the core\'s validation message and sends cockpit nothing', async () => {
    const { fake } = installCockpit();
    activate(context());
    await settle();
    projectFolder({ name: 'Hello World' });
    fake.calls.length = 0;
    await vscode.commands.executeCommand('ev3-c.run');
    assert.deepEqual(brickCalls(fake), []);
    assert.equal(errors().length, 1);
    assert.match(errors()[0].text, /Project name "Hello World" is not valid/);
});

test('Stop and Emergency Stop reach the brick the setting names', async () => {
    const { fake } = installCockpit();
    stub.__state.config.set('ev3-c.brick', 'AAA');
    activate(context());
    await settle();
    fake.calls.length = 0;
    await vscode.commands.executeCommand('ev3-c.stop');
    await vscode.commands.executeCommand('ev3-c.emergencyStop');
    assert.deepEqual(brickCalls(fake), [
        ['programs/stop', { brick: BRICK_ID }],
        ['programs/emergencyStop', { brick: BRICK_ID }],
    ]);
    assert.deepEqual(fake.calls.filter(([method]) => method === 'bricks/connect'), [['bricks/connect', { brick: BRICK_ID }]],
        'EVƎ C holds the brick itself, once while it stays connected');
});

test('"active" follows the brick the cockpit\'s window shows, and the status bar with it', async () => {
    const { fake } = installCockpit();
    fake.brickList.push({ ...fake.brickList[0], brickId: 'bt-001653000bbb', name: 'BBB' });
    activate(context());
    await settle();
    const item = stub.__state.statusBarItems[0];
    assert.match(item.text, /no brick/i, 'two bricks connected and no window shows one');

    fake.setShown('bt-001653000bbb');
    await settle();
    assert.match(item.text, /idle/i);
    await vscode.commands.executeCommand('ev3-c.stop');
    assert.deepEqual(brickCalls(fake), [['programs/stop', { brick: 'bt-001653000bbb' }]]);
    assert.deepEqual(errors(), []);
});

test('another brick in the setting is connected and the previous hold released', async () => {
    const { fake } = installCockpit();
    fake.brickList.push({ ...fake.brickList[0], brickId: 'bt-001653000bbb', name: 'BBB', connected: false });
    stub.__state.config.set('ev3-c.brick', 'AAA');
    activate(context());
    await settle();
    await vscode.commands.executeCommand('ev3-c.stop');
    stub.__state.config.set('ev3-c.brick', 'BBB');
    await vscode.commands.executeCommand('ev3-c.stop');
    await settle();
    assert.deepEqual(fake.calls.filter(([method]) => method.startsWith('bricks/')), [
        ['bricks/connect', { brick: BRICK_ID }],
        ['bricks/connect', { brick: 'bt-001653000bbb' }],
        ['bricks/disconnect', { brick: BRICK_ID }],
    ]);
    assert.deepEqual(errors(), []);
});

test('a brick another client is writing to is named in the error', async () => {
    const { fake } = installCockpit();
    activate(context());
    await settle();
    fake.failures.set('programs/stop', new FakeCockpitError('brick_busy', 'busy', { holder: 'ev3 c run (terminal)' }));
    await vscode.commands.executeCommand('ev3-c.stop');
    assert.equal(errors().length, 1);
    assert.match(errors()[0].text, /ev3 c run \(terminal\) is writing to the brick/);
});

test('a notification EVƎ C shows is logged as "Shown" and sent to the daemon\'s log', async () => {
    const { fake } = installCockpit();
    activate(context());
    await settle();
    fake.calls.length = 0;
    await vscode.commands.executeCommand('ev3-c.stop');
    fake.running = true;
    fake.failures.set('programs/emergencyStop', new FakeCockpitError('timeout'));
    await vscode.commands.executeCommand('ev3-c.emergencyStop');
    await settle();
    const notices = fake.calls.filter(([method]) => method === 'log/notice').map(([, params]) => params);
    assert.equal(notices.length, 1);
    assert.equal(notices[0].level, 'error');
    assert.match(String(notices[0].text), /did not answer in time/);
    assert.ok(stub.__state.log.some(l => l.level === 'error' && /^Shown: The brick did not answer in time/.test(l.line)));
});
