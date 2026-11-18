import { RunRecord, RunResult, RunSummary } from '../core';

// Turns run results into the lines the "EVƎ C" output channel shows.

/** How a run ended, in one phrase. */
export function howItEnded(record: RunRecord): string {
    switch (record.end) {
    case 'program':
        if (record.signal !== undefined) {
            return `the program was ended by signal ${record.signal}`;
        }
        return record.exit !== undefined ? `the program exited with code ${record.exit}` : 'the program ended';
    case 'slot': {
        const how = record.signal !== undefined ? ` (signal ${record.signal})`
            : record.exit !== undefined ? ` (exit code ${record.exit})` : '';
        return `the program was stopped when its slot ended${how}`;
    }
    case 'before-start':
        return 'the slot ended before the program started';
    }
}

/** When EVƎ C started the run, or that it was started on the brick. */
export function startedText(startedAt: Date | undefined): string {
    return startedAt === undefined
        ? 'not started by EVƎ C in this session (no PC start time)'
        : `${startedAt.toLocaleString()} (PC time)`;
}

export function formatRunResult(project: string, result: RunResult): string[] {
    const { record } = result;
    return [
        `--- ${project}, run ${result.run} ---`,
        `Started: ${startedText(result.startedAt)}`,
        `Ended: ${howItEnded(record)}`,
        ...stream('Output (stdout)', result.out, record.outTruncated),
        ...stream('Errors (stderr)', result.err, record.errTruncated),
    ];
}

export function formatNoResult(project: string, launchId: string, startedAt: Date): string[] {
    return [
        `--- ${project}, run started ${startedText(startedAt)} ---`,
        `No result was recorded on the brick (launch ${launchId}). "EV3 C: Show Runs" lists the run if its record appears later.`,
    ];
}

/** One line of the run list: the run number, how it ended, and when EVƎ C started it. */
export function summaryLabel(summary: RunSummary): { label: string; description: string } {
    return {
        label: `Run ${summary.run}`,
        description: summary.record === undefined
            ? 'no completion record (running, or interrupted)'
            : `${howItEnded(summary.record)}; started ${startedText(summary.startedAt)}`,
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
