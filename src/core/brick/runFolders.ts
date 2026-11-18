import { BRICK_PROJECTS_DIR } from '../build/buildProject';
import { BrickPort } from '../brickPort';

// A project's run folders on the brick: `<project>/runs/<n>/` with `out`, `err`, and the
// completion record `res`, which the supervisor renames into place as its last write.
// `runs/next` is the run counter; it is never deleted, so run numbers are never reused.

/** Name of the completion record inside a run folder. */
export const RESULT_FILE = 'res';

export function brickProjectDir(project: string): string {
    return `${BRICK_PROJECTS_DIR}/${project}`;
}

export function runsDir(project: string): string {
    return `${brickProjectDir(project)}/runs`;
}

export function runDir(project: string, run: number): string {
    return `${runsDir(project)}/${run}`;
}

/** Run numbers of the project's run folders, oldest first; none when the runs folder is missing. */
export async function listRunNumbers(port: BrickPort, project: string): Promise<number[]> {
    const dir = runsDir(project);
    if (!await port.exists(dir)) {
        return [];
    }
    return (await port.list(dir))
        .filter(entry => entry.kind === 'directory' && /^[1-9][0-9]*$/.test(entry.name))
        .map(entry => Number(entry.name))
        .sort((a, b) => a - b);
}

export interface DeleteRunsOptions {
    /** Keep this many of the newest run folders; all are deleted when absent. */
    keepLast?: number;
}

export interface DeleteRunsReport {
    /** Run folders that are gone. */
    readonly deleted: number[];
    /** Run folders left on purpose: the newest ones asked to be kept and a running run's folder. */
    readonly kept: number[];
    /** Run folders that were to be deleted but are still listed afterwards. */
    readonly stillPresent: number[];
}

/**
 * Deletes the project's run folders, all of them or all but the newest `keepLast`, one
 * folder at a time, and never the folder of the run in progress. Cockpit's recursive
 * remove deletes one folder level and reports nothing when it leaves a folder behind,
 * so the runs folder is listed again to report what is still there.
 */
export async function deleteRuns(port: BrickPort, project: string, options: DeleteRunsOptions = {}): Promise<DeleteRunsReport> {
    // Targets come from this one listing, so a run folder the supervisor creates
    // while deleting is never touched.
    const runs = await listRunNumbers(port, project);
    if (runs.length === 0) {
        return { deleted: [], kept: [], stillPresent: [] };
    }
    const keepLast = Math.max(0, options.keepLast ?? 0);
    const kept = new Set(runs.slice(runs.length - keepLast));
    const running = await runningRun(port, project, runs);
    if (running !== undefined) {
        kept.add(running);
    }

    const targets = runs.filter(run => !kept.has(run));
    for (const run of targets) {
        try {
            await port.remove(runDir(project, run), { recursive: true });
        } catch {
            // The listing below reports the folder as still present.
        }
    }

    const left = new Set(await listRunNumbers(port, project));
    return {
        deleted: targets.filter(run => !left.has(run)),
        kept: runs.filter(run => kept.has(run)),
        stillPresent: targets.filter(run => left.has(run)),
    };
}

/**
 * The run in progress: while the user slot runs, the newest run folder without a
 * completion record. A newest folder that has its record belongs to a run that has
 * ended (or the supervisor has not created the new folder yet), so nothing is protected.
 */
async function runningRun(port: BrickPort, project: string, runs: readonly number[]): Promise<number | undefined> {
    if (!(await port.isProgramRunning()).running) {
        return undefined;
    }
    const newest = runs[runs.length - 1];
    const entries = await port.list(runDir(project, newest));
    return entries.some(entry => entry.name === RESULT_FILE) ? undefined : newest;
}
