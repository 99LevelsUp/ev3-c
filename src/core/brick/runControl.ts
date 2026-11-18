import * as crypto from 'node:crypto';
import { BrickEvent, brickErrorCode, BrickFileReader, BrickPort, Clock, ProgramState, systemClock } from '../brickPort';
import { brickProjectDir, listRunNumbers } from './runFolders';
import { readRunRecord, resultFromRecord, RunRecordError, RunResult } from './runResults';

// Run, Stop, and Emergency Stop. A Run is tied to its result by a launch ID, not by a
// predicted run number: a run started from the brick menu, or one still finishing,
// could take the predicted number first. EVƎ C writes the ID into `<project>/.req`
// before starting the launcher; the supervisor moves it into the run's completion record.

/** How long EVƎ C waits for the completion record after the slot stops. */
export const RESULT_WAIT_MS = 10_000;

const POLL_INTERVAL_MS = 1000;
const REQUEST_FILE = '.req';

// Cockpit refused the start, so no supervisor will take the launch ID. After any other
// failure the launcher may already run, and its supervisor may still read the request.
const NOT_STARTED_CODES = new Set(['program_running', 'program_not_found', 'access_denied', 'not_connected']);

export interface RunProjectOptions {
    port: BrickPort;
    /** Shared reader that keeps file reads at most one per second. */
    reader: BrickFileReader;
    /** Project name; the launcher is `<project>/<project>.rbf` in the brick's projects folder. */
    project: string;
    /** Launch IDs of the runs EVƎ C started, with their PC start times; this run is added. */
    launches: Map<string, Date>;
    clock?: Clock;
    /** Makes the launch ID; a fresh random one by default. */
    newLaunchId?: () => string;
}

export type RunOutcome =
    | { readonly kind: 'result'; readonly result: RunResult }
    /** The completion record did not appear in time; the run folder stays on the brick for later. */
    | { readonly kind: 'no-result'; readonly launchId: string; readonly startedAt: Date };

/**
 * Starts the project's launcher and waits until the run ends and its completion record
 * is on the brick. The wait for the record starts when cockpit reports the slot stopped,
 * when `runProgram` returns without a running slot, or when the brick reconnects with
 * no program running; it polls at most once per second for up to 10 s, and a reconnect
 * during that time starts the 10 s again.
 */
export async function runProject(options: RunProjectOptions): Promise<RunOutcome> {
    const { port, reader, project, launches } = options;
    const clock = options.clock ?? systemClock;
    const dir = brickProjectDir(project);
    const requestPath = `${dir}/${REQUEST_FILE}`;

    const runs = await listRunNumbers(port, project);
    const newestBefore = runs.length > 0 ? runs[runs.length - 1] : 0;

    const wait = new ResultWait(port, clock);
    const subscription = port.onBrickEvent(event => wait.onEvent(event));
    try {
        const launchId = (options.newLaunchId ?? newLaunchId)();
        await port.writeFile(requestPath, Buffer.from(`${launchId}\n`, 'utf8'));
        const startedAt = new Date(clock.now());
        launches.set(launchId, startedAt);
        let state: ProgramState;
        try {
            // Events before this point say nothing about this run; a program that ends
            // while runProgram is still returning sends its event after it.
            wait.arm();
            state = await port.runProgram(`${dir}/${project}.rbf`);
        } catch (error) {
            const code = brickErrorCode(error);
            if (code !== undefined && NOT_STARTED_CODES.has(code)) {
                launches.delete(launchId);
                await port.remove(requestPath).catch(() => undefined);
            }
            throw error;
        }
        if (!state.running) {
            wait.stopped();
        }
        await wait.slotStopped;

        // Runs that ended with another launch ID, or none; their records are final.
        const others = new Set<number>();
        const found = await wait.poll(async () => {
            const candidates = (await listRunNumbers(port, project))
                .filter(n => n > newestBefore && !others.has(n))
                .reverse();
            for (const candidate of candidates) {
                let record;
                try {
                    record = await readRunRecord(port, reader, project, candidate);
                } catch (error) {
                    if (!(error instanceof RunRecordError)) {
                        throw error;
                    }
                    // A record that does not parse cannot carry this run's ID.
                    others.add(candidate);
                    continue;
                }
                if (record?.launch === launchId) {
                    return { run: candidate, record };
                }
                if (record !== undefined) {
                    others.add(candidate);
                }
            }
            return undefined;
        });
        if (found === undefined) {
            return { kind: 'no-result', launchId, startedAt };
        }
        return { kind: 'result', result: await resultFromRecord(reader, project, found.run, found.record, launches) };
    } finally {
        subscription.dispose();
    }
}

/** Stops the user slot through cockpit; EVƎ C adds no stop of its own. */
export function stopProgram(port: BrickPort): Promise<ProgramState> {
    return port.stopProgram();
}

/** Cockpit's emergency stop, unchanged. */
export function emergencyStop(port: BrickPort): Promise<ProgramState> {
    return port.emergencyStop();
}

function newLaunchId(): string {
    return crypto.randomBytes(8).toString('hex');
}

/** Waits for the slot to stop, then polls until a deadline that a reconnect moves. */
class ResultWait {
    readonly slotStopped: Promise<void>;
    private resolveStopped!: () => void;
    private isArmed = false;
    private isStopped = false;
    private deadline = Number.POSITIVE_INFINITY;

    constructor(private readonly port: BrickPort, private readonly clock: Clock) {
        this.slotStopped = new Promise(resolve => { this.resolveStopped = resolve; });
    }

    /** From now on events belong to this run. */
    arm(): void {
        this.isArmed = true;
    }

    onEvent(event: BrickEvent): void {
        if (!this.isArmed) {
            return;
        }
        if (event.kind === 'program') {
            if (!event.running) {
                this.stopped();
            }
            return;
        }
        if (this.isStopped) {
            // The brick is back while polling: give the record the full wait again.
            this.deadline = this.clock.now() + RESULT_WAIT_MS;
            return;
        }
        // The "stopped" event may have been missed while disconnected; the wait starts
        // only if the program has really ended.
        void this.port.isProgramRunning().then(state => {
            if (!state.running) {
                this.stopped();
            }
        }, () => undefined);
    }

    stopped(): void {
        if (!this.isStopped) {
            this.isStopped = true;
            this.deadline = this.clock.now() + RESULT_WAIT_MS;
            this.resolveStopped();
        }
    }

    /** Calls `attempt` once per interval until it returns a value or the deadline passes; brick errors count as "not yet". */
    async poll<T>(attempt: () => Promise<T | undefined>): Promise<T | undefined> {
        for (;;) {
            try {
                const found = await attempt();
                if (found !== undefined) {
                    return found;
                }
            } catch {
                // Disconnected or a record not readable yet: try again on the next poll.
            }
            if (this.clock.now() + POLL_INTERVAL_MS > this.deadline) {
                return undefined;
            }
            await this.clock.sleep(POLL_INTERVAL_MS);
        }
    }
}
