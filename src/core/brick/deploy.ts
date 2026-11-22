import * as crypto from 'node:crypto';
import * as fs from 'node:fs/promises';
import { BuildArtifact } from '../build/buildProject';
import { brickErrorCode, BrickPort } from '../brickPort';

// Upload without a run: puts a built project onto the brick. Only files whose MD5 differs
// from the brick's listing are uploaded, which skips the unchanged supervisor and doubles
// as its version check. Nothing is uploaded while a program runs: overwriting a running
// binary fails on the brick or leaves it truncated. A Run uploads inside its own start
// (`runProject`), where cockpit makes the same checks under one lease.

/** A brick operation that failed; `code` is cockpit's error code when there is one. */
export class BrickOperationError extends Error {
    constructor(message: string, readonly code: string | undefined) {
        super(message);
        this.name = 'BrickOperationError';
    }
}

export interface DeployReport {
    readonly uploaded: BuildArtifact[];
    readonly unchanged: BuildArtifact[];
}

/** Uploads the artifacts of one build, which all go into the same project folder. */
export async function deploy(port: BrickPort, artifacts: readonly BuildArtifact[]): Promise<DeployReport> {
    if ((await port.programState()).running) {
        throw new BrickOperationError('A program is running on the brick. Stop it first, then upload the project again.', 'program_running');
    }
    const report: DeployReport = { uploaded: [], unchanged: [] };
    if (artifacts.length === 0) {
        return report;
    }

    const dir = folderOf(artifacts[0].brickPath);
    const onBrick = new Map<string, string | undefined>();
    if (await port.exists(dir)) {
        for (const entry of await port.list(dir)) {
            if (entry.kind === 'file') {
                onBrick.set(entry.name, entry.md5?.toLowerCase());
            }
        }
    }

    for (const artifact of artifacts) {
        const data = await fs.readFile(artifact.localPath);
        const brickMd5 = onBrick.get(nameOf(artifact.brickPath));
        if (brickMd5 !== undefined && brickMd5 === crypto.createHash('md5').update(data).digest('hex')) {
            report.unchanged.push(artifact);
            continue;
        }
        await upload(port, artifact);
        report.uploaded.push(artifact);
    }
    return report;
}

/**
 * Failures where the link dropped before or during the write. Writing the whole file
 * again is safe, and a moment later the link is often back. Anything else, above all
 * `outcome_unknown`, `brick_busy` and `cancelled`, is not retried.
 */
const RETRIED_CODES = new Set(['not_connected', 'link_lost', 'transport_failed']);
const RETRY_DELAY_MS = 250;

async function upload(port: BrickPort, artifact: BuildArtifact): Promise<void> {
    const write = () => port.writeFile(artifact.brickPath, { localPath: artifact.localPath });
    try {
        await write();
        return;
    } catch (error) {
        if (!RETRIED_CODES.has(brickErrorCode(error) ?? '')) {
            throw failed(artifact.brickPath, error, 'failed');
        }
    }
    await new Promise(resolve => setTimeout(resolve, RETRY_DELAY_MS));
    try {
        await write();
    } catch (error) {
        throw failed(artifact.brickPath, error, 'failed twice');
    }
}

function failed(brickPath: string, error: unknown, how: string): BrickOperationError {
    const code = brickErrorCode(error);
    const reason = error instanceof Error ? error.message : String(error);
    return new BrickOperationError(`Uploading ${brickPath} ${how} (${code ?? 'no error code'}): ${reason}`, code);
}

function folderOf(brickPath: string): string {
    return brickPath.slice(0, brickPath.lastIndexOf('/'));
}

function nameOf(brickPath: string): string {
    return brickPath.slice(brickPath.lastIndexOf('/') + 1);
}
