import { beforeEach, test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { activate } from '../extension';
import { BuildProjectOptions, BuildResult } from '../core';
import { COMMAND_IDS, defaultCore, registerProjectCommands } from '../host/commands';
import { FakeConsumer } from './fakeCockpit';

// The vscode stub's test controls (see src/test/vscodeStub.cjs).
interface StubState {
    messages: { level: string; text: string }[];
    commands: Map<string, unknown>;
    output: string[];
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
const BRICK_FREE_COMMANDS = ['ev3-c.build', 'ev3-c.checkToolchain'];

function context(): vscode.ExtensionContext {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-host-'));
    return {
        subscriptions: [],
        extensionPath: dir,
        globalStorageUri: { fsPath: path.join(dir, 'storage') },
    } as unknown as vscode.ExtensionContext;
}

function installCockpit(apiVersion: number): FakeConsumer {
    const consumer = new FakeConsumer();
    stub.__state.extensions.set('99LevelsUp.ev3-cockpit', {
        activate: async () => ({ apiVersion, registerConsumer: () => consumer }),
    });
    return consumer;
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

beforeEach(() => {
    stub.__reset();
    stub.__state.config.set('ev3-c.brick', 'active');
    stub.__state.config.set('ev3-c.toolchain.path', '');
});

test('without cockpit, activation shows one clear error and registers no brick commands', async () => {
    await activate(context());
    assert.equal(errors().length, 1);
    assert.match(errors()[0].text, /EVƎ Cockpit/);
    assert.deepEqual([...stub.__state.commands.keys()].sort(), BRICK_FREE_COMMANDS);
});

test('with a cockpit API other than version 1, activation shows one clear error and registers no brick commands', async () => {
    const consumer = installCockpit(2);
    await activate(context());
    assert.equal(errors().length, 1);
    assert.match(errors()[0].text, /version 2/);
    assert.deepEqual([...stub.__state.commands.keys()].sort(), BRICK_FREE_COMMANDS);
    assert.deepEqual(consumer.calls, []);
});

test('with cockpit API version 1, every command is registered and the status bar shows the slot state', async () => {
    const consumer = installCockpit(1);
    await activate(context());
    await settle();
    assert.deepEqual(errors(), []);
    assert.deepEqual([...stub.__state.commands.keys()].sort(), [...ALL_COMMANDS].sort());
    assert.deepEqual(Object.values(COMMAND_IDS).sort(), [...ALL_COMMANDS].sort());

    const item = stub.__state.statusBarItems[0];
    assert.equal(item.visible, true);
    assert.match(item.text, /idle/i);
    consumer.setRunning(true);
    assert.match(item.text, /running/i);

    // A lost connection ends cockpit's subscription; after the reconnect the bar asks again.
    consumer.disconnect();
    consumer.running = false;
    consumer.reconnect();
    await settle();
    assert.match(item.text, /idle/i);
    consumer.setRunning(true);
    assert.match(item.text, /running/i, 'the status follows the new subscription');
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
    const output = vscode.window.createOutputChannel('EVƎ C');
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
});

test('Upload and Run in an untrusted workspace are refused and call no cockpit method', async () => {
    const consumer = installCockpit(1);
    await activate(context());
    await settle();
    projectFolder({ name: 'hello' });
    stub.__state.isTrusted = false;
    for (const id of ['ev3-c.upload', 'ev3-c.run']) {
        consumer.calls.length = 0;
        stub.__state.messages.length = 0;
        await vscode.commands.executeCommand(id);
        assert.deepEqual(consumer.calls, [], id);
        assert.equal(stub.__state.messages.length, 1, id);
        assert.match(stub.__state.messages[0].text, /trust/i, id);
    }
});

test('Run with no project folder open shows a message and calls no cockpit method', async () => {
    const consumer = installCockpit(1);
    await activate(context());
    await settle();
    consumer.calls.length = 0;
    await vscode.commands.executeCommand('ev3-c.run');
    assert.deepEqual(consumer.calls, []);
    assert.equal(errors().length, 1);
    assert.match(errors()[0].text, /ev3c\.json/);
});

test('Run with an invalid project name shows the core\'s validation message and calls no cockpit method', async () => {
    const consumer = installCockpit(1);
    await activate(context());
    await settle();
    projectFolder({ name: 'Hello World' });
    consumer.calls.length = 0;
    await vscode.commands.executeCommand('ev3-c.run');
    assert.deepEqual(consumer.calls, []);
    assert.equal(errors().length, 1);
    assert.match(errors()[0].text, /Project name "Hello World" is not valid/);
});

test('Stop and Emergency Stop call cockpit\'s stopProgram and emergencyStop on the configured brick', async () => {
    const consumer = installCockpit(1);
    stub.__state.config.set('ev3-c.brick', 'usb-0016533f1234');
    await activate(context());
    await settle();
    consumer.calls.length = 0;
    await vscode.commands.executeCommand('ev3-c.stop');
    await vscode.commands.executeCommand('ev3-c.emergencyStop');
    assert.deepEqual(consumer.calls.filter(c => c[0] !== 'listBricks'), [
        ['stopProgram', 'usb-0016533f1234'],
        ['emergencyStop', 'usb-0016533f1234'],
    ]);
});
