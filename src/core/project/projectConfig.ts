import * as fs from 'node:fs/promises';
import * as path from 'node:path';
import type { RetentionRule } from '../launcher/launcherTemplate';

// The project settings a build embeds: the project name, which names the folder on the
// brick and the launcher, and the rules the supervisor applies to the run folders.
//
// ev3c.json in the project folder, every key optional:
//
//     {
//         "name": "robot",
//         "retention": { "keepRuns": 5 },     or  { "keepMiB": 2 }
//         "outputCapKiB": 64
//     }
//
// Without a name the folder name is used. Without rules the defaults hold: keep the last
// 5 runs, and cap each captured output stream (stdout, stderr) at 64 KiB. The project
// folder on the brick is a small RAM disk shared by every project, and these defaults
// keep one project's runs well under a megabyte.

export interface ProjectConfig {
    readonly name: string;
    readonly retention: RetentionRule;
    /** Size limit in bytes for each captured output stream. */
    readonly outputCapBytes: number;
}

export const PROJECT_FILE = 'ev3c.json';

export const DEFAULT_RETENTION: RetentionRule = { kind: 'runs', count: 5 };
export const DEFAULT_OUTPUT_CAP_BYTES = 64 * 1024;

// The brick refuses paths of 60 bytes or more, and the launcher's path repeats the name:
// "/home/root/lms2012/prjs/" (24) + name + "/" + name + ".rbf" stays below 60 at 15 bytes.
const NAME_PATTERN = /^[a-z0-9_-]{1,15}$/;
const NAME_RULE = 'a project name is at most 15 bytes of lowercase ASCII letters, digits, "-" or "_", and not empty';

// The supervisor takes every number as a signed 32-bit integer.
const MAX_INT32 = 0x7fffffff;
const KIB = 1024;
const MIB = 1024 * 1024;

/** Returns why `name` cannot be a project name, or undefined when it can. */
export function projectNameProblem(name: string): string | undefined {
    return NAME_PATTERN.test(name)
        ? undefined
        : `Project name ${JSON.stringify(name)} is not valid: ${NAME_RULE}.`;
}

/**
 * Reads ev3c.json from `projectDir`, fills in the defaults, and validates the result.
 * Throws with the broken rule in the message, so a build stops before compiling.
 */
export async function loadProjectConfig(projectDir: string): Promise<ProjectConfig> {
    const file = path.join(projectDir, PROJECT_FILE);
    const raw = await readJson(file);
    const where = `${PROJECT_FILE} in ${projectDir}`;

    let name: string;
    if (raw.name === undefined) {
        name = path.basename(path.resolve(projectDir));
        const problem = projectNameProblem(name);
        if (problem !== undefined) {
            throw new Error(`${problem} The name comes from the project folder; set a valid "name" in ${where}.`);
        }
    } else {
        if (typeof raw.name !== 'string') {
            throw new Error(`"name" in ${where} must be a string: ${NAME_RULE}.`);
        }
        name = raw.name;
        const problem = projectNameProblem(name);
        if (problem !== undefined) {
            throw new Error(`${problem} Change "name" in ${where}.`);
        }
    }

    const capKiB = raw.outputCapKiB === undefined
        ? DEFAULT_OUTPUT_CAP_BYTES / KIB
        : wholeNumber(raw.outputCapKiB, 'outputCapKiB', Math.floor(MAX_INT32 / KIB), where);
    const outputCapBytes = capKiB * KIB;

    const retention = readRetention(raw.retention, where);
    if (retention.kind === 'megabytes' && retention.limit * MIB < 2 * outputCapBytes) {
        throw new Error(
            `"keepMiB": ${retention.limit} in ${where} is too small: it must hold at least two capped output streams `
            + `(2 × ${capKiB} KiB), because a single run can write that much.`,
        );
    }

    return { name, retention, outputCapBytes };
}

function readRetention(value: unknown, where: string): RetentionRule {
    if (value === undefined) {
        return DEFAULT_RETENTION;
    }
    if (!isObject(value)) {
        throw new Error(`"retention" in ${where} must be an object such as { "keepRuns": 5 } or { "keepMiB": 2 }.`);
    }
    const hasRuns = value.keepRuns !== undefined;
    const hasMiB = value.keepMiB !== undefined;
    if (hasRuns === hasMiB) {
        throw new Error(`"retention" in ${where} must set either "keepRuns" or "keepMiB", not ${hasRuns ? 'both' : 'neither'}.`);
    }
    return hasRuns
        ? { kind: 'runs', count: wholeNumber(value.keepRuns, 'keepRuns', MAX_INT32, where) }
        : { kind: 'megabytes', limit: wholeNumber(value.keepMiB, 'keepMiB', Math.floor(MAX_INT32 / MIB), where) };
}

function wholeNumber(value: unknown, key: string, max: number, where: string): number {
    if (typeof value !== 'number' || !Number.isInteger(value) || value < 1 || value > max) {
        throw new Error(`"${key}" in ${where} must be a whole number from 1 to ${max}, got ${JSON.stringify(value)}.`);
    }
    return value;
}

async function readJson(file: string): Promise<Record<string, unknown>> {
    let text: string;
    try {
        text = await fs.readFile(file, 'utf8');
    } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
            return {};
        }
        throw error;
    }
    let value: unknown;
    try {
        value = JSON.parse(text);
    } catch (error) {
        throw new Error(`${file} is not valid JSON: ${(error as Error).message}`);
    }
    if (!isObject(value)) {
        throw new Error(`${file} must hold a JSON object.`);
    }
    return value;
}

function isObject(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
