import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import { Ev3cError } from '../backend/ev3c';
import { BRICK_ERROR_MESSAGES, brickErrorMessage, START_FAILURE_MESSAGES, userMessage } from '../host/errorMessages';

// Every code the cockpit's contract names that an add-on can meet, except those only the
// cockpit's own clients see (pairing, Bluetooth lists, device permissions).
const CODES = [
    'not_connected', 'brick_not_found', 'brick_ambiguous', 'brick_unreachable', 'link_lost', 'brick_busy',
    'program_running', 'program_not_found', 'cancelled', 'access_denied', 'needs_person', 'outcome_unknown',
    'daemon_unavailable', 'timeout', 'transport_failed', 'connection_failed', 'channel_changed', 'connection_abandoned',
    'session_failed', 'heartbeat_failed', 'queue_unavailable', 'protocol_failed', 'invalid_request', 'internal',
    'subscription_failed',
];

test('each cockpit error code EVƎ C meets maps to its own user message', () => {
    assert.deepEqual(Object.keys(BRICK_ERROR_MESSAGES).sort(), [...CODES].sort());
    assert.equal(userMessage(new Error('something odd')), 'something odd');
    assert.equal(userMessage(new Ev3cError('The build failed: the compiler reported errors.')),
        'The build failed: the compiler reported errors.');
    for (const code of CODES.filter(code => code !== 'brick_busy')) {
        const message = userMessage(new Ev3cError('from the cockpit', code));
        assert.equal(message, `${brickErrorMessage(code)} (${code})`);
    }
    assert.equal(brickErrorMessage('something_new'), undefined);
    assert.equal(userMessage(new Ev3cError('own words', 'something_new')), 'own words');
});

test('brick_busy keeps the cockpit\'s message, which names the client that holds the brick', () => {
    const message = userMessage(new Ev3cError('ev3 c run (terminal) is writing to the brick.', 'brick_busy'));
    assert.equal(message, 'ev3 c run (terminal) is writing to the brick. Try again when it is done. (brick_busy)');
});

test('a daemon that did not start says why, by its failure', () => {
    for (const failure of Object.keys(START_FAILURE_MESSAGES)) {
        const message = userMessage(new Ev3cError('detail from the library', 'daemon_unavailable', failure));
        assert.ok(message.startsWith(START_FAILURE_MESSAGES[failure]), `${failure} → ${message}`);
        assert.match(message, /detail from the library/);
    }
    const unknown = new Ev3cError('x', 'daemon_unavailable', 'something_new');
    assert.ok(userMessage(unknown).startsWith(START_FAILURE_MESSAGES.failed));
});
