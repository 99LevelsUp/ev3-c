import { Ev3cError } from '../backend/ev3c';

// What the author reads when a command fails. `ev3-c` passes on the cockpit's stable error
// `code`; each code EVƎ C can meet gets a sentence that says what to do. A daemon that could
// not start says why in `failure`. The message itself follows in the output channel.

const CONNECTION = 'The connection to the brick failed. Check the cable or the Bluetooth link in EVƎ Cockpit, then try again.';
const COCKPIT = 'EVƎ Cockpit could not serve the request. Try again; if it repeats, reload the window.';

/** By the cockpit contract's error codes. */
export const BRICK_ERROR_MESSAGES: Readonly<Record<string, string>> = {
    not_connected: 'No brick is connected. Connect one in EVƎ Cockpit, or check the "ev3-c.brick" setting.',
    brick_not_found: 'EVƎ Cockpit knows no brick by the name or id in "ev3-c.brick". Check the setting.',
    brick_ambiguous: 'Several bricks match. Set "ev3-c.brick" to the id or a unique name of the brick EVƎ C should use.',
    brick_unreachable: 'The brick did not answer. Check that it is on and connected, then try again.',
    link_lost: CONNECTION,
    brick_busy: 'Another program is writing to the brick right now. Try again when it is done.',
    program_running: 'A program is running on the brick. Stop it first, then try again.',
    program_not_found: 'The program is not on the brick. Upload the project, then run it again.',
    cancelled: 'A stop cancelled the operation, so nothing was started.',
    access_denied: 'EVƎ Cockpit\'s access mode does not allow this on the brick. EVƎ C works only inside the brick\'s projects folder.',
    needs_person: 'This needs a person\'s confirmation in EVƎ Cockpit.',
    outcome_unknown: 'EVƎ Cockpit restarted during the operation, so whether it finished is not known. Check the brick before you try again.',
    daemon_unavailable: 'EVƎ C cannot reach EVƎ Cockpit. Open VS Code with EVƎ Cockpit installed, then try again.',
    timeout: 'The brick did not answer in time. Check that it is on and connected, then try again.',
    transport_failed: CONNECTION,
    connection_failed: CONNECTION,
    channel_changed: 'The connection to the brick changed during the transfer. Try again.',
    connection_abandoned: 'The brick was disconnected while it was connecting. Connect it again in EVƎ Cockpit.',
    session_failed: CONNECTION,
    heartbeat_failed: CONNECTION,
    queue_unavailable: CONNECTION,
    protocol_failed: 'The brick sent an answer EVƎ Cockpit could not read. Try again; if it repeats, restart the brick.',
    invalid_request: COCKPIT,
    internal: COCKPIT,
    subscription_failed: COCKPIT,
};

/** Why EVƎ Cockpit's daemon did not start, by the `failure` of its start error. */
export const START_FAILURE_MESSAGES: Readonly<Record<string, string>> = {
    missing_binary: 'EVƎ Cockpit is not installed, or has not run yet. Install or open EVƎ Cockpit, then try again.',
    sha_mismatch: 'EVƎ Cockpit\'s daemon does not match its package, so it was not started. Reinstall EVƎ Cockpit.',
    not_private: 'The folder EVƎ Cockpit runs its daemon from can be reached by another account, so it was not started.',
    smart_app_control: 'Windows Smart App Control blocked EVƎ Cockpit\'s daemon. EVƎ Cockpit cannot run while it is on.',
    name_held: 'EVƎ Cockpit\'s daemon refused to start: its name is taken, or it was started elevated.',
    lock_held: 'Another EVƎ Cockpit daemon was starting or ending and none became reachable. Try again.',
    contract_mismatch: 'EVƎ C and the running EVƎ Cockpit do not speak the same contract version.',
    failed: 'EVƎ Cockpit\'s daemon did not start.',
};

/**
 * The message to show for a failed command: the sentence for the cockpit's error code when
 * there is one, otherwise the error's own message (`ev3-c` writes those for the author).
 */
export function userMessage(error: unknown): string {
    const own = error instanceof Error ? error.message : String(error);
    if (!(error instanceof Ev3cError)) {
        return own;
    }
    if (error.failure !== undefined) {
        const known = Object.hasOwn(START_FAILURE_MESSAGES, error.failure)
            ? START_FAILURE_MESSAGES[error.failure] : START_FAILURE_MESSAGES.failed;
        // The library's message says which side to update, or carries the daemon's exit and log.
        return `${known} ${own} (${error.failure})`;
    }
    const code = error.code;
    if (code === 'brick_busy') {
        // The cockpit's own message names the client that holds the brick.
        return `${own} Try again when it is done. (${code})`;
    }
    const known = code === undefined ? undefined : brickErrorMessage(code);
    return known === undefined ? own : `${known} (${code})`;
}

/** The sentence for a cockpit error code, if EVƎ C has one. */
export function brickErrorMessage(code: string): string | undefined {
    return Object.hasOwn(BRICK_ERROR_MESSAGES, code) ? BRICK_ERROR_MESSAGES[code] : undefined;
}
