import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import { BRICK_ERROR_MESSAGES, START_FAILURE_MESSAGES, userMessage } from '../host/errorMessages';
import { FakeCockpitError } from './fakeCockpit';

// Every code the native contract names that an add-on can meet, except those only the
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
    for (const code of CODES) {
        const message = userMessage(new FakeCockpitError(code));
        assert.ok(message.startsWith(BRICK_ERROR_MESSAGES[code]), `${code} → ${message}`);
    }
});

test('brick_busy names the client that holds the brick', () => {
    const message = userMessage(new FakeCockpitError('brick_busy', 'busy', { holder: 'ev3 c run (terminal)' }));
    assert.match(message, /^ev3 c run \(terminal\) is writing to the brick/);
});

test('a daemon that did not start says why, by its failure', () => {
    for (const failure of Object.keys(START_FAILURE_MESSAGES)) {
        const error = Object.assign(new FakeCockpitError('daemon_unavailable', 'detail from the library'), { failure });
        const message = userMessage(error);
        assert.ok(message.startsWith(START_FAILURE_MESSAGES[failure]), `${failure} → ${message}`);
        assert.match(message, /detail from the library/);
    }
    const unknown = Object.assign(new FakeCockpitError('daemon_unavailable', 'x'), { failure: 'something_new' });
    assert.ok(userMessage(unknown).startsWith(START_FAILURE_MESSAGES.failed));
});
