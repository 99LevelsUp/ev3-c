import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import { BrickEvent, brickErrorCode } from '../core';
import { connectCockpit, createCockpitPort, resolveBrickId } from '../host/cockpitAdapter';
import { BRICK_ERROR_MESSAGES, userMessage } from '../host/errorMessages';
import { brick, BRICK_ID, FakeCockpitError, FakeConsumer } from './fakeCockpit';

const P = '/home/root/lms2012/prjs/hello';

test('each brick port call becomes the matching cockpit call on the bound brick', async () => {
    const consumer = new FakeConsumer();
    const port = createCockpitPort(consumer, BRICK_ID);
    const data = new Uint8Array([7]);

    assert.deepEqual(await port.list(P), [{ name: 'prog', kind: 'file', bytes: 10, md5: 'abc' }]);
    assert.equal(await port.exists(P), true);
    await port.makeDirectory(P);
    assert.deepEqual(await port.readFile(`${P}/res`), new Uint8Array([1, 2, 3]));
    await port.writeFile(`${P}/.req`, data);
    await port.remove(`${P}/runs/1`, { recursive: true });
    await port.remove(`${P}/.req`);
    assert.deepEqual(await port.runProgram(`${P}/hello.rbf`), { running: true });
    assert.deepEqual(await port.stopProgram(), { running: false });
    assert.deepEqual(await port.emergencyStop(), { running: false });
    assert.deepEqual(await port.isProgramRunning(), { running: false });

    assert.deepEqual(consumer.calls, [
        ['list', BRICK_ID, P],
        ['exists', BRICK_ID, P],
        ['makeDirectory', BRICK_ID, P],
        ['readFile', BRICK_ID, `${P}/res`],
        ['writeFile', BRICK_ID, `${P}/.req`, data],
        ['remove', BRICK_ID, `${P}/runs/1`, { recursive: true }],
        ['remove', BRICK_ID, `${P}/.req`, undefined],
        ['runProgram', BRICK_ID, `${P}/hello.rbf`],
        ['stopProgram', BRICK_ID],
        ['emergencyStop', BRICK_ID],
        ['isProgramRunning', BRICK_ID],
    ]);
});

test('a cockpit error reaches the core unchanged, with its code', async () => {
    const consumer = new FakeConsumer();
    const port = createCockpitPort(consumer, BRICK_ID);
    for (const [method, call] of [
        ['writeFile', () => port.writeFile(`${P}/prog`, new Uint8Array())],
        ['runProgram', () => port.runProgram(`${P}/hello.rbf`)],
        ['remove', () => port.remove(`${P}/runs/2`, { recursive: true })],
        ['readFile', () => port.readFile(`${P}/res`)],
    ] as const) {
        const error = new FakeCockpitError('access_denied');
        consumer.failures.set(method, error);
        await assert.rejects(call(), (thrown: unknown) => {
            assert.equal(thrown, error, `${method} must pass the cockpit error itself`);
            assert.equal(brickErrorCode(thrown), 'access_denied');
            return true;
        });
    }
});

test('the brick setting resolves to the active brick or names a brick by id', () => {
    const consumer = new FakeConsumer();
    assert.equal(resolveBrickId(consumer, 'active'), BRICK_ID);
    assert.equal(resolveBrickId(consumer, 'bt-001653aabbcc'), 'bt-001653aabbcc');

    consumer.bricks = [brick(BRICK_ID, { signedIn: false, active: false })];
    assert.throws(() => resolveBrickId(consumer, 'active'), (error: unknown) => brickErrorCode(error) === 'not_connected');
});

test('state events reach every listener, and a reconnect re-creates the cockpit subscription', () => {
    const consumer = new FakeConsumer();
    const port = createCockpitPort(consumer, BRICK_ID);
    const first: BrickEvent[] = [];
    const second: BrickEvent[] = [];
    const a = port.onBrickEvent(event => first.push(event));
    const b = port.onBrickEvent(event => second.push(event));
    assert.equal(consumer.programSubscriptions, 1, 'one cockpit subscription serves all listeners');

    consumer.setRunning(true);
    assert.deepEqual(first, [{ kind: 'program', running: true }]);
    assert.deepEqual(second, [{ kind: 'program', running: true }]);

    // The session ends: cockpit drops the subscription without a word.
    consumer.disconnect();
    assert.equal(consumer.programSubscriptions, 0);
    consumer.reconnect();
    assert.equal(consumer.programSubscriptions, 1, 'subscribed again after the reconnect');
    assert.deepEqual(first.slice(1), [{ kind: 'reconnected' }]);

    // Changes after the reconnect arrive through the new subscription.
    consumer.setRunning(false);
    assert.deepEqual(first.slice(2), [{ kind: 'program', running: false }]);
    assert.deepEqual(second, first);

    a.dispose();
    b.dispose();
    assert.equal(consumer.programSubscriptions, 0, 'the last listener gone ends the cockpit subscription');
    consumer.reconnect();
    assert.equal(consumer.programSubscriptions, 0);
});

test('a lost link counts as a disconnect even while the brick stays signed in', () => {
    const consumer = new FakeConsumer();
    const port = createCockpitPort(consumer, BRICK_ID);
    const events: BrickEvent[] = [];
    port.onBrickEvent(event => events.push(event));

    consumer.disconnect(BRICK_ID, true);
    assert.equal(consumer.programSubscriptions, 0);
    consumer.reconnect();
    assert.equal(consumer.programSubscriptions, 1);
    assert.deepEqual(events, [{ kind: 'reconnected' }]);
});

test('a reconnect whose subscription fails is retried on the next brick change', () => {
    const consumer = new FakeConsumer();
    const port = createCockpitPort(consumer, BRICK_ID);
    const events: BrickEvent[] = [];
    port.onBrickEvent(event => events.push(event));

    consumer.disconnect();
    consumer.failures.set('onProgramStateChanged', new FakeCockpitError('subscription_failed'));
    consumer.reconnect();
    assert.deepEqual(events, []);
    consumer.notifyBricks();
    assert.deepEqual(events, [{ kind: 'reconnected' }]);
    assert.equal(consumer.programSubscriptions, 1);
});

test('a new listener while the shared subscription is lost subscribes again or gets cockpit\'s error', () => {
    const consumer = new FakeConsumer();
    const port = createCockpitPort(consumer, BRICK_ID);
    const events: BrickEvent[] = [];
    port.onBrickEvent(event => events.push(event));

    consumer.disconnect();
    consumer.failures.set('onProgramStateChanged', new FakeCockpitError('subscription_failed'));
    consumer.reconnect();
    assert.equal(consumer.programSubscriptions, 0);

    // A Run joining now must not wait on a subscription that does not exist.
    const error = new FakeCockpitError('not_connected');
    consumer.failures.set('onProgramStateChanged', error);
    assert.throws(() => port.onBrickEvent(() => undefined), (thrown: unknown) => thrown === error);

    const joined: BrickEvent[] = [];
    port.onBrickEvent(event => joined.push(event));
    assert.equal(consumer.programSubscriptions, 1);
    assert.deepEqual(events, [{ kind: 'reconnected' }], 'the waiting listener hears the brick is back');
    consumer.setRunning(true);
    assert.deepEqual(joined, [{ kind: 'program', running: true }]);
});

test('subscribing to a brick that cannot be followed fails with cockpit\'s error', () => {
    const consumer = new FakeConsumer();
    const port = createCockpitPort(consumer, BRICK_ID);
    const error = new FakeCockpitError('not_connected');
    consumer.failures.set('onProgramStateChanged', error);
    assert.throws(() => port.onBrickEvent(() => undefined), (thrown: unknown) => thrown === error);
    // Nothing is left behind: the next subscription starts afresh.
    port.onBrickEvent(() => undefined);
    assert.equal(consumer.programSubscriptions, 1);
});

test('activation finds cockpit, checks API version 1, and registers the consumer "ev3-c"', async () => {
    const consumer = new FakeConsumer();
    const names: string[] = [];
    const api = { apiVersion: 1, registerConsumer: (name: string) => { names.push(name); return consumer; } };
    const connected = await connectCockpit(id => (id === '99LevelsUp.ev3-cockpit' ? { activate: async () => api } : undefined));
    assert.equal(connected.ok, true);
    assert.deepEqual(names, ['ev3-c']);

    const missing = await connectCockpit(() => undefined);
    assert.equal(missing.ok, false);
    assert.match(missing.ok ? '' : missing.problem, /EVƎ Cockpit/);

    const newer = await connectCockpit(() => ({ activate: async () => ({ apiVersion: 2, registerConsumer: () => consumer }) }));
    assert.equal(newer.ok, false);
    assert.match(newer.ok ? '' : newer.problem, /version 2/);

    const broken = await connectCockpit(() => ({ activate: async () => { throw new Error('boom'); } }));
    assert.equal(broken.ok, false);
    assert.match(broken.ok ? '' : broken.problem, /boom/);
});

test('each cockpit error code EVƎ C meets maps to its own user message', () => {
    const codes = [
        'not_connected', 'program_running', 'program_not_found', 'program_cancelled', 'access_denied',
        'timeout', 'transport_failed', 'connection_failed', 'channel_changed', 'connection_abandoned',
        'session_failed', 'heartbeat_failed', 'queue_unavailable', 'protocol_failed',
        'consumer_failed', 'subscription_failed',
    ];
    assert.deepEqual(Object.keys(BRICK_ERROR_MESSAGES).sort(), [...codes].sort());
    const fallback = userMessage(new Error('something odd'));
    assert.equal(fallback, 'something odd');
    for (const code of codes) {
        const message = userMessage(new FakeCockpitError(code));
        assert.ok(message.startsWith(BRICK_ERROR_MESSAGES[code]), `${code} → ${message}`);
        assert.ok(message.length > 20, `${code} has a real message`);
    }
});
