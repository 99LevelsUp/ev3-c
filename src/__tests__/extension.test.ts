import { afterEach, beforeEach, test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { CLIENT_NAME } from '../backend/ev3c';
import { activate, processes } from '../extension';
import { COMMAND_IDS } from '../host/commands';
import { FakeEv3c, FakeProcess } from './fakeEv3c';

// The extension against a fake `ev3-c`: what it starts, with which environment, and what it
// shows of the events it reads.

// The vscode stub's test controls (see src/test/vscodeStub.cjs).
interface StubState {
    messages: { level: string; text: string }[];
    commands: Map<string, unknown>;
    log: { level: string; line: string }[];
    statusBarItems: { text: string; tooltip?: string; visible: boolean; command?: string }[];
    config: Map<string, unknown>;
    configListeners: ((event: { affectsConfiguration(key: string): boolean }) => void)[];
    workspaceFolders: unknown;
    isTrusted: boolean;
    quickPick: ((items: unknown[]) => unknown) | undefined;
    inputBox: string | undefined;
}
const stub = vscode as unknown as { __state: StubState; __reset(): void };

const ALL_COMMANDS = [
    'ev3-c.build', 'ev3-c.upload', 'ev3-c.run', 'ev3-c.stop', 'ev3-c.emergencyStop',
    'ev3-c.showRuns', 'ev3-c.deleteRuns', 'ev3-c.checkToolchain', 'ev3-c.downloadFile',
];
const BRICK_ID = 'usb-0016530aaaaa';
const EXE = process.platform === 'win32' ? 'ev3-c.exe' : 'ev3-c';

let fake: FakeEv3c;
let answers: Map<string, (process: FakeProcess) => void>;
let context: vscode.ExtensionContext;

/** An installed extension folder with its `ev3-c` program. */
function extensionContext(): vscode.ExtensionContext {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-host-'));
    fs.mkdirSync(path.join(dir, 'bin'));
    fs.writeFileSync(path.join(dir, 'bin', EXE), '');
    return { subscriptions: [], extensionPath: dir } as unknown as vscode.ExtensionContext;
}

function projectFolder(name = 'robot'): string {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-project-'));
    fs.writeFileSync(path.join(dir, 'ev3c.json'), JSON.stringify({ name }));
    stub.__state.workspaceFolders = [{ uri: { fsPath: dir }, name: path.basename(dir), index: 0 }];
    return dir;
}

const settle = async () => {
    for (let i = 0; i < 5; i++) {
        await new Promise(resolve => setImmediate(resolve));
    }
};
const messages = (level: string) => stub.__state.messages.filter(m => m.level === level).map(m => m.text);
const item = () => stub.__state.statusBarItems[0];

/** `watch` runs until its input ends, then is done. */
function watchUntilStopped(process: FakeProcess): void {
    process.once('input-ended', () => process.finish({ event: 'done', exitCode: 0 }));
}

beforeEach(() => {
    stub.__reset();
    stub.__state.config.set('ev3-c.brick', 'active');
    stub.__state.config.set('ev3-c.toolchain.path', '');
    fake = new FakeEv3c();
    answers = new Map([['watch', watchUntilStopped]]);
    fake.answer = process => (answers.get(process.args[0]) ?? (p => p.finish({ event: 'done', exitCode: 0 })))(process);
    processes.spawn = fake.spawn;
    context = extensionContext();
});

afterEach(() => {
    for (const subscription of context.subscriptions) {
        subscription.dispose();
    }
});

test('activation registers every command and follows the brick with ev3-c watch as EVƎ C (VS Code)', async () => {
    activate(context);
    await settle();
    assert.deepEqual([...stub.__state.commands.keys()].sort(), [...ALL_COMMANDS].sort());
    assert.deepEqual(Object.values(COMMAND_IDS).sort(), [...ALL_COMMANDS].sort());
    const [watch] = fake.of('watch');
    assert.deepEqual(watch.args, ['watch', '--events']);
    assert.equal(watch.program, path.join(context.extensionPath, 'bin', EXE));
    assert.equal(watch.env.EV3C_CLIENT_NAME, CLIENT_NAME);
    assert.equal(watch.env.EV3C_PACKAGE_ROOT, context.extensionPath);
    assert.equal(watch.env.EV3C_BRICK, 'active');
    assert.equal(watch.env.EV3C_TOOLCHAIN, undefined);

    assert.equal(item().visible, true);
    watch.send({ event: 'program', brickId: BRICK_ID, running: false, runSeq: 0 });
    await settle();
    assert.match(item().text, /idle/);
    watch.send({ event: 'program', brickId: BRICK_ID, running: true, runSeq: 1 });
    await settle();
    assert.match(item().text, /running/);
    assert.equal(item().command, COMMAND_IDS.stop);
    watch.send({ event: 'noBrick', error: { message: 'No brick is connected.', code: 'not_connected' } });
    await settle();
    assert.match(item().text, /no brick/);
    assert.match(item().tooltip ?? '', /^No brick is connected\./);

    // Another brick in the setting: the old watch ends and a new one follows that brick.
    stub.__state.config.set('ev3-c.brick', 'BBB');
    for (const listener of stub.__state.configListeners) {
        listener({ affectsConfiguration: key => key === 'ev3-c.brick' });
    }
    await settle();
    assert.equal(watch.inputEnded, true);
    assert.equal(fake.of('watch').length, 2);
    assert.equal(fake.of('watch')[1].env.EV3C_BRICK, 'BBB');
    assert.deepEqual(messages('error'), []);
});

test('a watch that cannot reach the cockpit says so in the status bar and no error is shown', async () => {
    answers.set('watch', process => process.finish({
        event: 'error', exitCode: 1, error: { message: 'No daemon was recorded.', code: 'daemon_unavailable', failure: 'missing_binary' },
    }));
    activate(context);
    await settle();
    assert.match(item().text, /not connected/);
    assert.match(item().tooltip ?? '', /No daemon was recorded/);
    assert.deepEqual(messages('error'), []);
});

test('Run shows the result, remembers its launch, and Show Runs hands the launch back', async () => {
    const dir = projectFolder();
    answers.set('run', process => {
        process.send({ event: 'building', folder: dir });
        process.send({ event: 'built', project: 'robot', artifacts: [{ brickPath: '/home/root/lms2012/prjs/robot/robot.elf' }] });
        process.send({ event: 'connected', brickId: BRICK_ID });
        process.send({
            event: 'started',
            start: { outcome: 'started', runSeq: 7, uploaded: ['a'], skipped: [] },
            launchId: 'L7',
            startedAt: '2026-11-24T08:00:00.000Z',
        });
        process.finish({
            event: 'result',
            exitCode: 0,
            project: 'robot',
            result: {
                run: 4, record: { end: 'program', exit: 0, outTruncated: false, errTruncated: false, ended: 'the program exited with code 0' },
                out: 'hi\n', err: '', returned: ['/home/root/lms2012/prjs/robot/log.csv'],
            },
            returnedFiles: { downloaded: [{ brickPath: '/home/root/lms2012/prjs/robot/log.csv', localPath: path.join(dir, 'returned', 'run-4', 'log.csv') }], failed: [] },
        });
    });
    activate(context);
    await vscode.commands.executeCommand('ev3-c.run');
    assert.deepEqual(fake.of('run')[0].args, ['run', dir, '--events']);
    assert.deepEqual(messages('error'), []);
    assert.deepEqual(messages('info'), [
        `"robot" run 4: the program exited with code 0; 1 file(s) returned to ${path.join('returned', 'run-4')}.`,
    ]);
    const lines = stub.__state.log.map(l => l.line);
    assert.ok(lines.includes('--- robot, run 4 ---'));
    assert.ok(lines.includes('  hi'));
    assert.ok(lines.some(line => line.startsWith('Returned /home/root/lms2012/prjs/robot/log.csv →')));

    answers.set('runs', process => {
        if (process.args[1] === 'show') {
            process.finish({
                event: 'result', exitCode: 0, project: 'robot',
                result: {
                    run: 4, record: { end: 'program', exit: 0, outTruncated: false, errTruncated: false, ended: 'the program exited with code 0' },
                    out: '', err: '', startedAt: '2026-11-24T08:00:00.000Z', returned: [],
                },
            });
        } else {
            process.finish({ event: 'done', exitCode: 0, project: 'robot', runs: [{ run: 4, startedAt: '2026-11-24T08:00:00.000Z' }] });
        }
    });
    stub.__state.quickPick = items => items[0];
    await vscode.commands.executeCommand('ev3-c.showRuns');
    const [list, shown] = fake.of('runs');
    assert.deepEqual(list.args, ['runs', dir, '--events']);
    assert.deepEqual(JSON.parse(list.env.EV3C_LAUNCHES ?? '{}'), { L7: Date.parse('2026-11-24T08:00:00.000Z') });
    assert.deepEqual(shown.args, ['runs', 'show', '4', dir, '--events']);
    assert.deepEqual(messages('error'), []);
});

test('a run without a recorded result warns, and a failed build shows the compiler messages and an error', async () => {
    const dir = projectFolder();
    answers.set('run', process => process.finish({
        event: 'noResult', exitCode: 1, project: 'robot',
        noResult: { launchId: 'L1', startedAt: '2026-11-24T08:00:00.000Z' },
    }));
    activate(context);
    await vscode.commands.executeCommand('ev3-c.run');
    assert.deepEqual(messages('warning'), ['"robot" ended, but no result was recorded on the brick.']);

    answers.set('build', process => {
        process.send({ event: 'building', folder: dir });
        process.send({ event: 'diagnostics', text: 'main.c:1:1: error: expected expression\n' });
        process.finish({ event: 'error', exitCode: 1, error: { message: 'The build failed: the compiler reported errors.' } });
    });
    await vscode.commands.executeCommand('ev3-c.build');
    assert.deepEqual(messages('error'), ['The build failed: the compiler reported errors.']);
    assert.ok(stub.__state.log.some(l => l.level === 'warn' && l.line === 'main.c:1:1: error: expected expression'));
});

test('a brick command that cannot reach the cockpit says how to get it, by the code ev3-c passes on', async () => {
    answers.set('stop', process => process.finish({
        event: 'error', exitCode: 1, error: { message: 'No daemon was recorded.', code: 'daemon_unavailable', failure: 'missing_binary' },
    }));
    activate(context);
    await vscode.commands.executeCommand('ev3-c.stop');
    assert.equal(messages('error').length, 1);
    assert.match(messages('error')[0], /not installed, or has not run yet/);
    assert.match(messages('error')[0], /missing_binary/);
});

test('an untrusted workspace builds nothing, and Delete Runs keeps the newest folders asked for', async () => {
    const dir = projectFolder();
    stub.__state.isTrusted = false;
    activate(context);
    await vscode.commands.executeCommand('ev3-c.build');
    assert.equal(fake.of('build').length, 0);
    assert.match(messages('error')[0], /untrusted workspace/);

    answers.set('runs', process => process.finish({ event: 'done', exitCode: 0, project: 'robot', deleted: [1, 2], kept: [3], stillPresent: [] }));
    stub.__state.quickPick = items => items[1];
    stub.__state.inputBox = '1';
    await vscode.commands.executeCommand('ev3-c.deleteRuns');
    assert.deepEqual(fake.of('runs')[0].args, ['runs', 'delete', '--keep', '1', dir, '--events']);
    assert.deepEqual(messages('info'), ['Run folders of "robot": deleted 2, kept 1.']);
});

test('the toolchain setting reaches ev3-c, and Check Toolchain says where the compiler came from', async () => {
    stub.__state.config.set('ev3-c.toolchain.path', 'C:\\zig\\zig.exe');
    answers.set('toolchain', process => process.finish({ event: 'done', exitCode: 0, path: 'C:\\zig\\zig.exe', origin: 'override' }));
    activate(context);
    await vscode.commands.executeCommand('ev3-c.checkToolchain');
    assert.equal(fake.of('toolchain')[0].env.EV3C_TOOLCHAIN, 'C:\\zig\\zig.exe');
    assert.equal(fake.of('toolchain')[0].env.EV3C_TOOLCHAIN_SETTING, 'ev3-c.toolchain.path', 'a wrong path names the setting');
    assert.deepEqual(messages('info'), ['EVƎ C\'s compiler is ready (set in "ev3-c.toolchain.path").']);
});
