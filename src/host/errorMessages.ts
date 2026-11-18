import { brickErrorCode } from '../core';

// What the author reads when a brick operation fails. Cockpit's errors carry a stable
// `code`; each code EVƎ C can meet gets a sentence that says what to do. Cockpit's own
// message follows in the output channel.

const CONNECTION = 'The connection to the brick failed. Check the cable or the Bluetooth link in EVƎ Cockpit, then try again.';
const COCKPIT = 'EVƎ Cockpit could not serve the request. Reload the window, then try again.';

export const BRICK_ERROR_MESSAGES: Readonly<Record<string, string>> = {
    not_connected: 'No brick is connected. Connect one in EVƎ Cockpit, or check the "ev3-c.brick" setting.',
    program_running: 'A program is running on the brick. Stop it first, then try again.',
    program_not_found: 'The program is not on the brick. Upload the project, then run it again.',
    program_cancelled: 'An emergency stop cancelled the start, so the program did not run.',
    access_denied: 'EVƎ Cockpit\'s access mode does not allow this on the brick. EVƎ C works only inside the brick\'s projects folder.',
    timeout: 'The brick did not answer in time. Check that it is on and connected, then try again.',
    transport_failed: CONNECTION,
    connection_failed: CONNECTION,
    channel_changed: 'The connection to the brick changed during the transfer. Try again.',
    connection_abandoned: 'The brick was disconnected while it was connecting. Connect it again in EVƎ Cockpit.',
    session_failed: CONNECTION,
    heartbeat_failed: CONNECTION,
    queue_unavailable: CONNECTION,
    protocol_failed: 'The brick sent an answer EVƎ Cockpit could not read. Try again; if it repeats, restart the brick.',
    consumer_failed: COCKPIT,
    subscription_failed: COCKPIT,
};

/**
 * The message to show for a failed operation: the sentence for cockpit's error code when
 * there is one, otherwise the error's own message (the core's messages are written for
 * the author already).
 */
export function userMessage(error: unknown): string {
    const code = brickErrorCode(error);
    const known = code !== undefined && Object.hasOwn(BRICK_ERROR_MESSAGES, code) ? BRICK_ERROR_MESSAGES[code] : undefined;
    const own = error instanceof Error ? error.message : String(error);
    return known === undefined ? own : `${known} (${code})`;
}
