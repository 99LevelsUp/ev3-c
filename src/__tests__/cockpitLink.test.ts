import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { BrickEvent, brickErrorCode } from '../core';
import { createBrickPort, INLINE_WRITE_LIMIT, resolveBrick, ShownBricks } from '../cockpit/cockpitLink';
import { BRICK_ID, FakeCockpitClient, FakeCockpitError, fakeBrick } from './fakeCockpit';

const P = '/home/root/lms2012/prjs/hello';
const settle = () => new Promise(resolve => setImmediate(resolve));

function setup() {
    const fake = new FakeCockpitClient();
    const port = createBrickPort(fake.asClient(), BRICK_ID);
    return { fake, port };
}

test('each port call names the brick and reads the contract\'s answer objects', async () => {
    const { fake, port } = setup();
    fake.files.set(`${P}/res`, Buffer.from('run=1\n'));

    assert.deepEqual(await port.list(P), [{ name: 'prog', kind: 'file', bytes: 3, md5: 'abc' }]);
    assert.equal(await port.exists(P), true);
    assert.equal(Buffer.from(await port.readFile(`${P}/res`)).toString(), 'run=1\n');
    await port.writeFile(`${P}/prog`, { localPath: 'build/prog' });
    await port.writeFile(`${P}/.req`, { data: Buffer.from('id\n') });
    assert.deepEqual(await port.removePaths([`${P}/runs/1`], { recursive: true }), { leftBehind: [] });
    assert.deepEqual(await port.programState(), { running: false, runSeq: 0 });

    const [, read] = fake.calls.splice(2, 1)[0];
    assert.equal(read.path, `${P}/res`);
    assert.ok(path.isAbsolute(String(read.toLocalPath)), 'read into a local file the daemon writes');
    assert.equal(fs.existsSync(String(read.toLocalPath)), false, 'the local copy is removed after the read');
    assert.deepEqual(fake.calls, [
        ['files/list', { brick: BRICK_ID, path: P }],
        ['files/exists', { brick: BRICK_ID, path: P }],
        ['files/write', { brick: BRICK_ID, path: `${P}/prog`, fromLocalPath: path.resolve('build/prog') }],
        ['files/write', { brick: BRICK_ID, path: `${P}/.req`, content: Buffer.from('id\n').toString('base64') }],
        ['files/removePaths', { brick: BRICK_ID, paths: [`${P}/runs/1`], recursive: true }],
        ['programs/status', { brick: BRICK_ID }],
    ]);
});

test('a deployment sends local files by absolute path, small ones inline, and the start\'s number comes back', async () => {
    const { fake, port } = setup();

    const report = await port.deployAndRun([
        { brickPath: `${P}/prog`, source: { localPath: 'build/prog' }, skipIfSame: true },
        { brickPath: `${P}/.req`, source: { data: Buffer.from('id\n') } },
    ], `${P}/hello.rbf`);

    assert.deepEqual(fake.calls, [['programs/deployAndRun', {
        brick: BRICK_ID,
        files: [
            { remotePath: `${P}/prog`, localPath: path.resolve('build/prog'), skipIfSameMd5: true },
            { remotePath: `${P}/.req`, content: Buffer.from('id\n').toString('base64') },
        ],
        runPath: `${P}/hello.rbf`,
    }]]);
    assert.deepEqual(report, { outcome: 'started', runSeq: 1, uploaded: [`${P}/prog`, `${P}/.req`], skipped: [] });
});

test('inline content over 4 KiB is refused before it reaches cockpit', async () => {
    const { fake, port } = setup();

    await assert.rejects(port.writeFile(`${P}/big`, { data: new Uint8Array(INLINE_WRITE_LIMIT + 1) }),
        (error: unknown) => brickErrorCode(error) === 'invalid_request');
    assert.deepEqual(fake.calls, []);
});

test('a cockpit error reaches the core unchanged, with its code', async () => {
    const { fake, port } = setup();
    for (const [method, call] of [
        ['files/write', () => port.writeFile(`${P}/prog`, { data: new Uint8Array() })],
        ['programs/deployAndRun', () => port.deployAndRun([], `${P}/hello.rbf`)],
        ['files/removePaths', () => port.removePaths([`${P}/runs/2`])],
        ['files/read', () => port.readFile(`${P}/res`)],
        ['programs/stop', () => port.stopProgram()],
    ] as const) {
        const error = new FakeCockpitError('brick_busy', 'busy', { holder: 'ev3 c run (terminal)' });
        fake.failures.set(method, error);
        await assert.rejects(call(), (thrown: unknown) => {
            assert.equal(thrown, error, `${method} must pass the cockpit error itself`);
            return true;
        });
    }
});

test('"active" is the brick the cockpit\'s window shows, even with several connected', () => {
    const fake = new FakeCockpitClient();
    fake.brickList.push(fakeBrick('bt-001653000bbb', 'BBB'));
    const client = fake.asClient();
    assert.equal(resolveBrick(client, 'active', ['bt-001653000bbb']), 'bt-001653000bbb');
    assert.equal(resolveBrick(client, 'AAA', ['bt-001653000bbb']), BRICK_ID, 'a named brick wins over the window');
    assert.throws(() => resolveBrick(client, 'active', [BRICK_ID, 'bt-001653000bbb']), (error: unknown) => {
        assert.equal(brickErrorCode(error), 'brick_ambiguous');
        assert.match(String((error as Error).message), /windows show different bricks \(AAA, BBB\)/);
        return true;
    });
});

test('the shown bricks follow daemon/status, then daemon/showsChanged, counting only cockpit windows', async () => {
    const fake = new FakeCockpitClient();
    fake.clientStatuses = [
        { name: 'EVƎ Cockpit (VS Code)', kind: 'vscode', version: '1', buildKind: 'release', shows: BRICK_ID },
        { name: 'ev3 (terminal)', kind: 'cli', version: '1', buildKind: 'release', shows: 'bt-001653000bbb' },
    ];
    const shown = await ShownBricks.follow(fake.asClient());
    assert.deepEqual(shown.bricks(), [BRICK_ID]);

    let changes = 0;
    shown.onChanged(() => changes++);
    fake.setShown('bt-001653000bbb', 'bt-001653000bbb');
    assert.deepEqual(shown.bricks(), ['bt-001653000bbb'], 'two windows on the same brick are one brick');
    fake.setShown(undefined);
    assert.deepEqual(shown.bricks(), [], 'a window that shows none');
    assert.equal(changes, 2);

    // A new daemon is asked again.
    fake.clientStatuses = [{ name: 'EVƎ Cockpit (VS Code)', kind: 'vscode', version: '1', buildKind: 'release', shows: BRICK_ID }];
    fake.daemonReplaced();
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(shown.bricks(), [BRICK_ID]);

    shown.dispose();
    fake.setShown('bt-001653000bbb');
    assert.deepEqual(shown.bricks(), [BRICK_ID], 'nothing is heard after dispose');
});

test('"active" without a window showing a brick is the only connected brick; a name or id is taken as written', () => {
    const fake = new FakeCockpitClient();
    const client = fake.asClient();
    assert.equal(resolveBrick(client, 'active'), BRICK_ID);
    assert.equal(resolveBrick(client, 'aaa'), BRICK_ID, 'a name resolves to its id');
    assert.equal(resolveBrick(client, 'bt-001653000bbb'), 'bt-001653000bbb', 'an unknown id is left to cockpit');

    fake.brickList.push(fakeBrick('bt-001653000bbb', 'BBB'));
    assert.throws(() => resolveBrick(client, 'active'), (error: unknown) => {
        assert.equal(brickErrorCode(error), 'brick_ambiguous');
        assert.match(String((error as Error).message), /AAA, BBB/);
        return true;
    });

    fake.brickList = [fakeBrick(BRICK_ID, 'AAA', false)];
    assert.throws(() => resolveBrick(client, 'active'), (error: unknown) => brickErrorCode(error) === 'not_connected');
});

test('state events reach every listener, and a brick signed in again gets a new subscription', async () => {
    const { fake, port } = setup();
    const first: BrickEvent[] = [];
    const second: BrickEvent[] = [];
    const a = await port.onBrickEvent(event => first.push(event));
    const b = await port.onBrickEvent(event => second.push(event));
    assert.equal(fake.openSubscriptions, 1, 'one cockpit subscription serves all listeners');

    fake.setRunning(true, 1);
    assert.deepEqual(first, [{ kind: 'program', state: { running: true, runSeq: 1 } }]);
    assert.deepEqual(second, first);

    // Signed out: cockpit's subscription ends without a word.
    fake.signOut();
    assert.equal(fake.openSubscriptions, 0, 'the dead subscription is let go');
    fake.signIn();
    await settle();
    assert.equal(fake.liveSubscriptions, 1, 'subscribed again once the brick is back');
    assert.deepEqual(first.slice(1), [{ kind: 'reconnected' }]);

    fake.setRunning(false, 1);
    assert.deepEqual(first.slice(2), [{ kind: 'program', state: { running: false, runSeq: 1 } }]);
    assert.deepEqual(second, first);

    a.dispose();
    b.dispose();
    assert.equal(fake.openSubscriptions, 0, 'the last listener gone ends the cockpit subscription');
    fake.signOut();
    fake.signIn();
    await settle();
    assert.equal(fake.openSubscriptions, 0);
});

test('a route that came back and a replaced daemon are told as reconnects', async () => {
    const { fake, port } = setup();
    const events: BrickEvent[] = [];
    await port.onBrickEvent(event => events.push(event));

    fake.notify('bricks/reconnected', { brickId: 'usb-other' });
    fake.notify('bricks/reconnected', { brickId: BRICK_ID });
    fake.daemonReplaced();

    assert.deepEqual(events, [{ kind: 'reconnected' }, { kind: 'reconnected' }]);
    assert.equal(fake.openSubscriptions, 1, 'the library renews the subscription itself');
});

test('a reconnect whose subscription fails is tried again on the next brick change', async () => {
    const { fake, port } = setup();
    const events: BrickEvent[] = [];
    await port.onBrickEvent(event => events.push(event));

    fake.signOut();
    fake.failures.set('programs/subscribe', new FakeCockpitError('subscription_failed'));
    fake.signIn();
    await settle();
    assert.deepEqual(events, []);
    fake.notifyBricks();
    await settle();
    assert.deepEqual(events, [{ kind: 'reconnected' }]);
    assert.equal(fake.liveSubscriptions, 1);
});

test('subscribing to a brick that cannot be followed fails with cockpit\'s error, and the next try starts afresh', async () => {
    const { fake, port } = setup();
    const error = new FakeCockpitError('not_connected');
    fake.failures.set('programs/subscribe', error);
    await assert.rejects(port.onBrickEvent(() => undefined), (thrown: unknown) => thrown === error);

    await port.onBrickEvent(() => undefined);
    assert.equal(fake.openSubscriptions, 1);
});
