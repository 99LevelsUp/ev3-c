import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as path from 'node:path';
import { Ev3c, Ev3cError, Ev3cEvent, findProgram, LineReader } from '../backend/ev3c';
import { FakeEv3c } from './fakeEv3c';

function ev3c(fake: FakeEv3c, env: Record<string, string> = {}): Ev3c {
    return new Ev3c(() => 'ev3-c', () => env, fake.spawn);
}

test('a command runs as ev3-c <args> --events with the environment and tells its events in order', async () => {
    const fake = new FakeEv3c();
    fake.answer = process => {
        process.send({ event: 'building', folder: 'C:\\robot' });
        process.send({ event: 'connected', brickId: 'usb-0016530aaaaa' });
        process.finish({ event: 'done', exitCode: 0, project: 'robot' });
    };
    const heard: Ev3cEvent[] = [];
    const done = await ev3c(fake, { EV3C_BRICK: 'active' }).run(['upload', 'C:\\robot'], { onEvent: event => heard.push(event) });
    assert.deepEqual(fake.started[0].args, ['upload', 'C:\\robot', '--events']);
    assert.equal(fake.started[0].env.EV3C_BRICK, 'active');
    assert.deepEqual(heard.map(event => event.event), ['building', 'connected']);
    assert.equal(done.project, 'robot');
});

test('an error event fails with the cockpit\'s code and why the daemon did not start', async () => {
    const fake = new FakeEv3c();
    fake.answer = process => process.finish({
        event: 'error', exitCode: 1, error: { message: 'No daemon.', code: 'daemon_unavailable', failure: 'missing_binary' },
    });
    const error = await ev3c(fake).run(['stop']).then(() => undefined, (e: unknown) => e);
    assert.ok(error instanceof Ev3cError);
    assert.equal(error.message, 'No daemon.');
    assert.equal(error.code, 'daemon_unavailable');
    assert.equal(error.failure, 'missing_binary');
});

test('a run that exits with its program\'s code still resolves with its result', async () => {
    const fake = new FakeEv3c();
    fake.answer = process => process.finish({ event: 'result', exitCode: 3, result: { run: 1 } });
    const end = await ev3c(fake).run(['run', 'x']);
    assert.equal(end.exitCode, 3);
});

test('a program that ends without a terminal event fails with what it wrote to standard error', async () => {
    const fake = new FakeEv3c();
    fake.answer = process => process.exit(101, 'thread main panicked');
    await assert.rejects(ev3c(fake).run(['build', 'x']), /ended without a result \(exit code 101\): thread main panicked/);
});

test('aborting a command sends one interrupt line; stopping a followed command ends its input', async () => {
    const fake = new FakeEv3c();
    fake.answer = () => undefined;
    const controller = new AbortController();
    const running = ev3c(fake).run(['run', 'x'], { signal: controller.signal });
    await new Promise(resolve => setImmediate(resolve));
    controller.abort();
    const process = fake.started[0];
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(process.input, 'interrupt\n');
    assert.equal(process.inputEnded, false, 'the command goes on to tell its result');
    process.finish({ event: 'result', exitCode: 130 });
    assert.equal((await running).exitCode, 130);

    fake.answer = watch => watch.once('input-ended', () => watch.finish({ event: 'done', exitCode: 0 }));
    const followed = ev3c(fake).follow(['watch'], () => undefined);
    await new Promise(resolve => setImmediate(resolve));
    followed.stop();
    assert.equal((await followed.ended).event, 'done');
});

test('lines are split on line ends, also with a character split across chunks', () => {
    const lines: string[] = [];
    const reader = new LineReader(line => lines.push(line));
    const bytes = Buffer.from('{"text":"EVƎ"}\r\n\n{"a":1}', 'utf8');
    const split = bytes.indexOf(0x86); // inside the two bytes of Ǝ
    reader.push(bytes.subarray(0, split));
    reader.push(bytes.subarray(split));
    reader.end();
    assert.deepEqual(lines, ['{"text":"EVƎ"}', '{"a":1}']);
});

test('the program is the packaged one, else the one Cargo built last', () => {
    const exe = process.platform === 'win32' ? 'ev3-c.exe' : 'ev3-c';
    const root = path.join('x', 'ext');
    const packaged = path.join(root, 'bin', exe);
    const release = path.join(root, 'backend', 'target', 'release', exe);
    const debug = path.join(root, 'backend', 'target', 'debug', exe);
    const written = (times: Record<string, number>) => (file: string) => times[file];
    assert.equal(findProgram(root, written({ [packaged]: 1, [release]: 2, [debug]: 3 })), packaged);
    assert.equal(findProgram(root, written({ [release]: 2, [debug]: 3 })), debug, 'a newer debug build wins over an old release');
    assert.equal(findProgram(root, written({ [release]: 3, [debug]: 2 })), release);
    assert.equal(findProgram(root, written({ [debug]: 1 })), debug);
    assert.throws(() => findProgram(root, written({})), /is missing/);
});
