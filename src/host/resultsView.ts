// Turns the run results `ev3-c` reports into lines for the "EVƎ C" output channel. The shapes
// are those of `ev3 c … --json` (docs/cli.md). Nothing here imports VS Code.

/** A run's completion record; `ended` says in words how it ended. */
export interface RunRecordJson {
    readonly end: string;
    readonly exit?: number;
    readonly signal?: number;
    readonly outTruncated: boolean;
    readonly errTruncated: boolean;
    readonly launch?: string;
    readonly ended: string;
}

export interface RunResultJson {
    readonly run: number;
    readonly record: RunRecordJson;
    /** Absent when it could not be read from the brick. */
    readonly out?: string;
    readonly err?: string;
    /** When this computer started the run, if EVƎ C started it in this session. */
    readonly startedAt?: string;
    /** The brick paths of the files the run returned. */
    readonly returned: readonly string[];
}

export interface ReturnedFilesJson {
    readonly downloaded: readonly { brickPath: string; localPath: string }[];
    readonly failed: readonly { brickPath: string; reason: string }[];
}

export interface RunEntryJson {
    readonly run: number;
    readonly record?: RunRecordJson;
    readonly startedAt?: string;
}

/** When EVƎ C started the run, or that it was started elsewhere. */
export function startedText(startedAt: string | undefined): string {
    const date = startedAt === undefined ? undefined : new Date(startedAt);
    return date === undefined || Number.isNaN(date.getTime())
        ? 'not started by EVƎ C in this session (no PC start time)'
        : `${date.toLocaleString()} (PC time)`;
}

export function formatRunResult(project: string, result: RunResultJson): string[] {
    const { record } = result;
    return [
        `--- ${project}, run ${result.run} ---`,
        `Started: ${startedText(result.startedAt)}`,
        `Ended: ${record.ended}`,
        ...stream('Output (stdout)', result.out, record.outTruncated),
        ...stream('Errors (stderr)', result.err, record.errTruncated),
        ...(result.returned.length === 0 ? [] : ['Returned files:', ...result.returned.map(brickPath => `  ${brickPath}`)]),
    ];
}

/** What became of a run's returned files on this computer. */
export function formatReturned(report: ReturnedFilesJson): string[] {
    return [
        ...report.downloaded.map(file => `Returned ${file.brickPath} → ${file.localPath}`),
        ...report.failed.map(file => `Could not return ${file.brickPath}: ${file.reason}`),
    ];
}

export function formatNoResult(project: string, launchId: string, startedAt: string): string[] {
    return [
        `--- ${project}, run started ${startedText(startedAt)} ---`,
        `No result was recorded on the brick (launch ${launchId}). "EV3 C: Show Runs" lists the run if its record appears later.`,
    ];
}

/** One line of the run list: the run number, how it ended, and when EVƎ C started it. */
export function summaryLabel(entry: RunEntryJson): { label: string; description: string } {
    return {
        label: `Run ${entry.run}`,
        description: entry.record === undefined
            ? 'no completion record (running, or interrupted)'
            : `${entry.record.ended}; started ${startedText(entry.startedAt)}`,
    };
}

function stream(title: string, text: string | undefined, truncated: boolean): string[] {
    const note = truncated ? ', truncated at the size limit' : '';
    if (text === undefined) {
        return [`${title}${note}: could not be read from the brick`];
    }
    if (text === '') {
        return [`${title}${note}: (empty)`];
    }
    return [`${title}${note}:`, ...text.replace(/\n$/, '').split('\n').map(line => `  ${line}`)];
}
