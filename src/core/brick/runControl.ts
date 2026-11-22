import * as crypto from 'node:crypto';
import * as path from 'node:path';
import { BuildArtifact } from '../build/buildProject';
import { BrickEvent, brickErrorCode, BrickFileReader, BrickPort, Clock, DeployItem, ProgramState, StartReport, systemClock } from '../brickPort';
import { brickProjectDir, listRunNumbers } from './runFolders';
import { readRunRecord, resultFromRecord, RunRecordError, RunResult } from './runResults';

// Run, Stop, and Emergency Stop. A Run is tied to its result by a launch ID, not by a
// predicted run number: a run started from the brick menu, or one still finishing,
// could take the predicted number first. EVƎ C puts the ID into `<project>/.req` in the
// same cockpit operation that uploads the project and starts the launcher, so no other
// client can start something in between; the supervisor moves the ID into the run's
// completion record.

/** How long EVƎ C waits for the completion record after the slot stops. */
export const RESULT_WAIT_MS = 10_000;

const POLL_INTERVAL_MS = 1000;
const REQUEST_FILE = '.req';

// Cockpit refused or cancelled the start, so no supervisor will take the launch ID. After
// any other failure, `outcome_unknown` above all, the launcher may already run, and its
// supervisor may still read the request.
const NOT_STARTED_CODES = new Set([
    'program_running', 'program_not_found', 'access_denied', 'not_connected', 'brick_busy', 'brick_not_found',
    'brick_ambiguous', 'cancelled', 'invalid_request',
]);

export interface RunProjectOptions {
    port: BrickPort;
    /** Shared reader that keeps file reads at most one per second. */
    reader: BrickFileReader;
    /** Project name; the launcher is `<project>/<project>.rbf` in the brick's projects folder. */
    project: string;
    /** The build's artifacts, uploaded as part of the start when they differ from the brick's. */
    artifacts: readonly BuildArtifact[];
    /** Launch IDs of the runs EVƎ C started, with their PC start times; this run is added. */
    launches: Map<string, Date>;
    clock?: Clock;
    /** Makes the launch ID; a fresh random one by default. */
    newLaunchId?: () => string;
}

export type RunOutcome =
    | { readonly kind: 'result'; readonly result: RunResult; readonly start: StartReport }
    /** The completion record did not appear in time; the run folder stays on the brick for later. */
    | { readonly kind: 'no-result'; readonly launchId: string; readonly startedAt: Date; readonly start: StartReport };

/**
 * Uploads the project, starts its launcher, and waits until the run ends and its
 * completion record is on the brick. The run has ended when cockpit reports the slot
 * stopped under this start's number or a later one, or when the start was not seen
 * running. The wait for the record then polls at most once per second for up to 10 s,
 * and a reconnect during that time starts the 10 s again.
 */
export async function runProject(options: RunProjectOptions): Promise<RunOutcome> {
    const { port, reader, project, launches } = options;
    const clock = options.clock ?? systemClock;
    const dir = brickProjectDir(project);
    const requestPath = `${dir}/${REQUEST_FILE}`;

    const runs = await listRunNumbers(port, project);
    const newestBefore = runs.length > 0 ? runs[runs.length - 1] : 0;

    // Subscribed before the start, so the end of a short run is never missed.
    const wait = new ResultWait(port, clock);
    const subscription = await port.onBrickEvent(event => wait.onEvent(event));
    try {
        const launchId = (options.newLaunchId ?? newLaunchId)();
        const startedAt = new Date(clock.now());
        launches.set(launchId, startedAt);
        const files: DeployItem[] = [
            ...options.artifacts.map(artifact => ({
                brickPath: artifact.brickPath,
                source: { localPath: path.resolve(artifact.localPath) },
                skipIfSame: true,
            })),
            { brickPath: requestPath, source: { data: Buffer.from(`${launchId}\n`, 'utf8') } },
        ];
        let start: StartReport;
        try {
            start = await port.deployAndRun(files, `${dir}/${project}.rbf`);
        } catch (error) {
            const code = brickErrorCode(error);
            if (code !== undefined && NOT_STARTED_CODES.has(code)) {
                launches.delete(launchId);
                await port.removePaths([requestPath]).catch(() => undefined);
            }
            throw error;
        }
        wait.started(start);
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
            return { kind: 'no-result', launchId, startedAt, start };
        }
        return { kind: 'result', result: await resultFromRecord(reader, project, found.run, found.record, launches), start };
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

/**
 * Waits for the slot to stop, then polls until a deadline that a reconnect moves. States
 * heard before the start's number is known are kept, so a run that ends while the start
 * is still returning is not missed.
 */
class ResultWait {
    readonly slotStopped: Promise<void>;
    private resolveStopped!: () => void;
    private runSeq: number | undefined;
    /** The highest start number heard together with a stopped slot or a later start. */
    private endedSeq = 0;
    private isStopped = false;
    private deadline = Number.POSITIVE_INFINITY;

    constructor(private readonly port: BrickPort, private readonly clock: Clock) {
        this.slotStopped = new Promise(resolve => { this.resolveStopped = resolve; });
    }

    /** The start returned: from now on states are measured against its number. */
    started(start: StartReport): void {
        this.runSeq = start.runSeq;
        if (start.outcome === 'not_seen_running') {
            this.stopped();
        } else if (start.outcome === 'uncertain') {
            // The reply to the start was lost; the brick's state says whether anything runs.
            this.check();
        } else {
            this.judge();
        }
    }

    onEvent(event: BrickEvent): void {
        if (event.kind === 'program') {
            this.note(event.state);
            return;
        }
        if (this.isStopped) {
            // The brick is back while polling: give the record the full wait again.
            this.deadline = this.clock.now() + RESULT_WAIT_MS;
            return;
        }
        // The end may have been missed while disconnected. A daemon that replaced the one
        // that started this run counts starts from 0 again, so after a reconnect a stopped
        // slot ends the run whatever its number.
        this.check(true);
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

    /** Asks the brick's state once the start's number is known; `anyStop` takes any stopped slot as the end. */
    private check(anyStop = false): void {
        if (this.runSeq === undefined) {
            return;
        }
        void this.port.programState().then(state => {
            if (anyStop && !state.running) {
                this.stopped();
            } else {
                this.note(state);
            }
        }, () => undefined);
    }

    private note(state: ProgramState): void {
        // A stopped slot ends the start it is told with; any later start ends this one too.
        const ended = state.running ? state.runSeq - 1 : state.runSeq;
        this.endedSeq = Math.max(this.endedSeq, ended);
        this.judge();
    }

    private judge(): void {
        if (this.runSeq !== undefined && this.endedSeq >= this.runSeq) {
            this.stopped();
        }
    }
}
