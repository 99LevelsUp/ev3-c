import { BrickFileReader, BrickPort } from '../brickPort';
import { parseReturnList, RETURN_LIST_FILE } from './returnedFiles';
import { listRunNumbers, RESULT_FILE, runDir } from './runFolders';

// How a run ended, read from its run folder. The completion record `res` is `key=value`
// text, one pair per line:
//   run=<n>                          always
//   end=program|slot|before-start    always
//   exit=<code>                      when the program exited
//   signal=<number>                  when a signal ended the program
//   out_truncated=0|1, err_truncated=0|1
//   launch=<id>                      only when EVƎ C started the run

export type RunEnd = 'program' | 'slot' | 'before-start';

export interface RunRecord {
    readonly run: number;
    /** The program ended on its own, the slot ended and stopped it, or the slot ended before it started. */
    readonly end: RunEnd;
    readonly exit?: number;
    readonly signal?: number;
    readonly outTruncated: boolean;
    readonly errTruncated: boolean;
    /** The launch ID EVƎ C wrote before starting the run; absent for a run started on the brick. */
    readonly launch?: string;
}

export interface RunResult {
    readonly run: number;
    readonly record: RunRecord;
    /** Captured stdout; absent when it could not be read from the brick. */
    readonly out?: string;
    /** Captured stderr; absent when it could not be read from the brick. */
    readonly err?: string;
    /** When EVƎ C started the run, by the PC's clock; absent for a run EVƎ C did not start. */
    readonly startedAt?: Date;
    /** Brick paths of the files the run marked for return, in the order marked; empty when none. */
    readonly returned: readonly string[];
}

export interface RunSummary {
    readonly run: number;
    /** Absent while the run has no completion record (in progress, or interrupted). */
    readonly record?: RunRecord;
    readonly startedAt?: Date;
}

/** A completion record without a valid run number or end. */
export class RunRecordError extends Error {
    constructor(message: string) {
        super(message);
        this.name = 'RunRecordError';
    }
}

const OUT_FILE = 'out';
const ERR_FILE = 'err';
const ENDS: readonly string[] = ['program', 'slot', 'before-start'];

export function parseRunRecord(text: string): RunRecord {
    const values = new Map<string, string>();
    for (const line of text.split('\n')) {
        const eq = line.indexOf('=');
        if (eq > 0) {
            values.set(line.slice(0, eq), line.slice(eq + 1).replace(/\r$/, ''));
        }
    }
    const run = integer(values.get('run'));
    const end = values.get('end');
    if (run === undefined || end === undefined || !ENDS.includes(end)) {
        throw new RunRecordError(`The run's completion record is not valid: ${JSON.stringify(text)}`);
    }
    const record: { -readonly [K in keyof RunRecord]: RunRecord[K] } = {
        run,
        end: end as RunEnd,
        outTruncated: values.get('out_truncated') === '1',
        errTruncated: values.get('err_truncated') === '1',
    };
    const exit = integer(values.get('exit'));
    const signal = integer(values.get('signal'));
    const launch = values.get('launch');
    if (exit !== undefined) {
        record.exit = exit;
    }
    if (signal !== undefined) {
        record.signal = signal;
    }
    if (launch !== undefined && launch !== '') {
        record.launch = launch;
    }
    return record;
}

/**
 * The result of one run, or undefined while it has no completion record. `launches`
 * maps the launch IDs of runs EVƎ C started to their PC start times.
 */
export async function readRunResult(
    port: BrickPort, reader: BrickFileReader, project: string, run: number, launches: ReadonlyMap<string, Date>,
): Promise<RunResult | undefined> {
    const dir = runDir(project, run);
    const names = new Set((await port.list(dir)).map(entry => entry.name));
    if (!names.has(RESULT_FILE)) {
        return undefined;
    }
    const record = parseRunRecord(text(await reader.readFile(`${dir}/${RESULT_FILE}`)));
    return resultFromRecord(reader, project, run, record, launches, names);
}

/**
 * Completes a run's result from its already-read record by reading both outputs and the
 * return list. A file that is missing (per `present`, when given) or cannot be read is
 * left out; a missing return list means the run marked no file.
 */
export async function resultFromRecord(
    reader: BrickFileReader,
    project: string,
    run: number,
    record: RunRecord,
    launches: ReadonlyMap<string, Date>,
    present?: ReadonlySet<string>,
): Promise<RunResult> {
    const dir = runDir(project, run);
    const readOutput = async (name: string): Promise<string | undefined> => {
        if (present !== undefined && !present.has(name)) {
            return undefined;
        }
        try {
            return text(await reader.readFile(`${dir}/${name}`));
        } catch {
            return undefined;
        }
    };
    const out = await readOutput(OUT_FILE);
    const err = await readOutput(ERR_FILE);
    const list = await readOutput(RETURN_LIST_FILE);
    return { run, record, out, err, startedAt: startTime(record, launches), returned: list === undefined ? [] : parseReturnList(list) };
}

/**
 * Every run folder of the project, oldest first, with its completion record and, for
 * runs EVƎ C started, the PC start time. Reads one record per second.
 */
export async function listRuns(
    port: BrickPort, reader: BrickFileReader, project: string, launches: ReadonlyMap<string, Date>,
): Promise<RunSummary[]> {
    const summaries: RunSummary[] = [];
    for (const run of await listRunNumbers(port, project)) {
        const record = await readRunRecord(port, reader, project, run);
        summaries.push(record === undefined ? { run } : { run, record, startedAt: startTime(record, launches) });
    }
    return summaries;
}

/** The completion record of a run, or undefined while it has none. */
export async function readRunRecord(port: BrickPort, reader: BrickFileReader, project: string, run: number): Promise<RunRecord | undefined> {
    const dir = runDir(project, run);
    if (!(await port.list(dir)).some(entry => entry.name === RESULT_FILE)) {
        return undefined;
    }
    return parseRunRecord(text(await reader.readFile(`${dir}/${RESULT_FILE}`)));
}

function startTime(record: RunRecord, launches: ReadonlyMap<string, Date>): Date | undefined {
    return record.launch === undefined ? undefined : launches.get(record.launch);
}

function integer(value: string | undefined): number | undefined {
    return value !== undefined && /^-?[0-9]+$/.test(value) ? Number(value) : undefined;
}

function text(data: Uint8Array): string {
    return Buffer.from(data).toString('utf8');
}
