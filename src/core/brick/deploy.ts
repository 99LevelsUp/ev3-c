import * as crypto from 'node:crypto';
import * as fs from 'node:fs/promises';
import { BuildArtifact } from '../build/buildProject';
import { brickErrorCode, BrickPort } from '../brickPort';

// Puts a built project onto the brick. Only files whose MD5 differs from cockpit's
// listing are uploaded, which skips the unchanged supervisor and doubles as its version
// check. Nothing is uploaded while a program runs: overwriting a running binary fails on
// the brick or leaves it truncated.

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
    if ((await port.isProgramRunning()).running) {
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
    } else {
        await port.makeDirectory(dir);
    }

    for (const artifact of artifacts) {
        const data = await fs.readFile(artifact.localPath);
        const brickMd5 = onBrick.get(nameOf(artifact.brickPath));
        if (brickMd5 !== undefined && brickMd5 === crypto.createHash('md5').update(data).digest('hex')) {
            report.unchanged.push(artifact);
            continue;
        }
        await upload(port, artifact.brickPath, data);
        report.uploaded.push(artifact);
    }
    return report;
}

async function upload(port: BrickPort, brickPath: string, data: Uint8Array): Promise<void> {
    try {
        await port.writeFile(brickPath, data);
    } catch {
        try {
            await port.writeFile(brickPath, data);
        } catch (error) {
            const code = brickErrorCode(error);
            const reason = error instanceof Error ? error.message : String(error);
            throw new BrickOperationError(`Uploading ${brickPath} failed twice (${code ?? 'no error code'}): ${reason}`, code);
        }
    }
}

function folderOf(brickPath: string): string {
    return brickPath.slice(0, brickPath.lastIndexOf('/'));
}

function nameOf(brickPath: string): string {
    return brickPath.slice(brickPath.lastIndexOf('/') + 1);
}
