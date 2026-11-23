import * as fs from 'node:fs/promises';
import * as path from 'node:path';
import { BrickFileReader } from '../brickPort';
import { brickProjectDir } from './runFolders';

// Files a run hands back to its author. ev3lib appends each marked file's full brick path
// to the run folder's list, `ret`; after the run, EVƎ C downloads those files into the
// project's `returned/run-<n>/` folder. The list comes from the program, so every path
// is checked before it names a file on this computer.

/** Name of the return list inside a run folder. */
export const RETURN_LIST_FILE = 'ret';

/** Where a run's returned files land, below the project folder on this computer. */
export const RETURNED_DIR = 'returned';

/** The brick reaches the projects folder through this link as well (`prjs` lives in RAM). */
const PROJECTS_ALIAS = '/mnt/ramdisk/prjs';

/** The paths a return list names: absolute ones only, each once, in the order marked. */
export function parseReturnList(text: string): string[] {
    const seen = new Set<string>();
    for (const line of text.split('\n')) {
        const brickPath = line.trim();
        if (brickPath.startsWith('/')) {
            seen.add(path.posix.normalize(brickPath));
        }
    }
    return [...seen];
}

/** The local folder for a run's returned files. */
export function returnedDir(projectDir: string, run: number): string {
    return path.join(projectDir, RETURNED_DIR, `run-${run}`);
}

/**
 * Where a returned brick file lands below `localDir`: its path inside the brick project
 * folder when it is there, otherwise its whole brick path below `brick/`. Characters this
 * computer cannot use in a name become `_`. A path that would leave `localDir` is refused.
 */
export function localPathFor(brickPath: string, project: string, localDir: string): string {
    const normalized = path.posix.normalize(brickPath);
    if (!normalized.startsWith('/')) {
        throw new Error(`"${brickPath}" is not an absolute brick path.`);
    }
    const projectDirs = [brickProjectDir(project), `${PROJECTS_ALIAS}/${project}`];
    const inside = projectDirs.find(dir => normalized.startsWith(`${dir}/`));
    const relative = inside !== undefined ? normalized.slice(inside.length + 1) : `brick${normalized}`;
    const segments = relative.split('/').filter(segment => segment !== '' && segment !== '.');
    if (segments.length === 0 || segments.includes('..')) {
        throw new Error(`"${brickPath}" does not name a file that can be saved.`);
    }
    const local = path.join(localDir, ...segments.map(safeName));
    const back = path.relative(localDir, local);
    if (back === '' || back.startsWith('..') || path.isAbsolute(back)) {
        throw new Error(`"${brickPath}" would be saved outside ${localDir}.`);
    }
    return local;
}

export interface ReturnedFile {
    readonly brickPath: string;
    readonly localPath: string;
}

export interface ReturnedReport {
    readonly downloaded: readonly ReturnedFile[];
    readonly failed: readonly { readonly brickPath: string; readonly reason: string }[];
}

/** Downloads every listed file into `localDir`; a file that fails does not stop the rest. */
export async function downloadReturnedFiles(
    reader: BrickFileReader, project: string, brickPaths: readonly string[], localDir: string,
): Promise<ReturnedReport> {
    const downloaded: ReturnedFile[] = [];
    const failed: { brickPath: string; reason: string }[] = [];
    for (const brickPath of brickPaths) {
        try {
            const localPath = localPathFor(brickPath, project, localDir);
            await downloadFile(reader, brickPath, localPath);
            downloaded.push({ brickPath, localPath });
        } catch (error) {
            failed.push({ brickPath, reason: error instanceof Error ? error.message : String(error) });
        }
    }
    return { downloaded, failed };
}

/** Copies one brick file to a local path, creating its folders; returns its size. */
export async function downloadFile(reader: BrickFileReader, brickPath: string, localPath: string): Promise<number> {
    const data = await reader.readFile(brickPath);
    await fs.mkdir(path.dirname(localPath), { recursive: true });
    await fs.writeFile(localPath, data);
    return data.length;
}

function safeName(segment: string): string {
    // Windows refuses these and control characters in names, and drops a trailing dot or space silently.
    const kept = [...segment].map(ch => (ch.charCodeAt(0) < 0x20 || '<>:"\\|?*'.includes(ch) ? '_' : ch)).join('');
    return kept.replace(/[. ]+$/, match => '_'.repeat(match.length));
}
