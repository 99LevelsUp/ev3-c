import * as crypto from 'node:crypto';
import { execFile } from 'node:child_process';
import { createReadStream } from 'node:fs';
import * as fs from 'node:fs/promises';
import * as path from 'node:path';
import { promisify } from 'node:util';
import bundledManifest from './manifest.json';

// Obtains the pinned cross-compiler. The compiler is not bundled with the
// extension: it is downloaded on first use from the URL in manifest.json,
// verified against the pinned SHA-256 before it is extracted, and kept in a
// cache folder the host supplies. A host setting can point at a compiler
// instead, which skips the download.

export interface HostToolchain {
    /** Archive file name, appended to the manifest's base URL. */
    file: string;
    sha256: string;
    /** Path of the compiler executable inside the extracted archive. */
    entry: string;
}

export interface ToolchainManifest {
    compiler: string;
    version: string;
    baseUrl: string;
    hosts: Record<string, HostToolchain>;
    targetFlags: string[];
}

export const manifest: ToolchainManifest = bundledManifest;

/** Fetches `url` into the file `destination`. */
export type Downloader = (url: string, destination: string) => Promise<void>;

export interface CompilerOverride {
    path: string;
    /** Name of the host setting the path came from, quoted in errors. */
    setting: string;
}

export interface EnsureCompilerOptions {
    cacheDir: string;
    override?: CompilerOverride;
    download?: Downloader;
    manifest?: ToolchainManifest;
    /** Manifest host key such as `win32-x64`; defaults to the running host. */
    host?: string;
}

export interface CompilerInfo {
    path: string;
    /** The pinned version; unknown for a compiler set by the host. */
    version?: string;
    origin: 'override' | 'cache' | 'download';
}

const execFileAsync = promisify(execFile);

export async function ensureCompiler(options: EnsureCompilerOptions): Promise<CompilerInfo> {
    const pinned = options.manifest ?? manifest;
    if (options.override !== undefined) {
        const { path: overridePath, setting } = options.override;
        if (!await exists(overridePath)) {
            throw new Error(`The compiler set in "${setting}" does not exist: ${overridePath}`);
        }
        return { path: overridePath, origin: 'override' };
    }

    const host = options.host ?? `${process.platform}-${process.arch}`;
    const entry = pinned.hosts[host];
    if (entry === undefined) {
        throw new Error(`No ${pinned.compiler} ${pinned.version} build is pinned for this host (${host}).`);
    }

    const installDir = path.join(options.cacheDir, `${pinned.compiler}-${pinned.version}-${host}`);
    const compilerPath = path.join(installDir, entry.entry);
    if (await exists(compilerPath)) {
        return { path: compilerPath, version: pinned.version, origin: 'cache' };
    }

    await fs.mkdir(options.cacheDir, { recursive: true });
    const archive = path.join(options.cacheDir, entry.file);
    let origin: CompilerInfo['origin'] = 'cache';
    if (!await exists(archive) || await sha256Of(archive) !== entry.sha256) {
        origin = 'download';
        await (options.download ?? fetchToFile)(pinned.baseUrl + entry.file, archive);
        const actual = await sha256Of(archive);
        if (actual !== entry.sha256) {
            await fs.rm(archive, { force: true });
            throw new Error(
                `Downloaded ${entry.file} does not match the pinned checksum and was deleted. `
                + `Expected SHA-256 ${entry.sha256}, got ${actual}.`,
            );
        }
    }

    // Extract next to the final folder and rename it into place, so an
    // interrupted extraction is never mistaken for a usable compiler.
    const partial = `${installDir}.partial`;
    await fs.rm(partial, { recursive: true, force: true });
    await fs.mkdir(partial, { recursive: true });
    await execFileAsync(systemTar(), ['-xf', archive, '-C', partial]);
    await fs.rm(installDir, { recursive: true, force: true });
    await fs.rename(partial, installDir);
    if (!await exists(compilerPath)) {
        throw new Error(`${entry.file} does not contain ${entry.entry}.`);
    }
    return { path: compilerPath, version: pinned.version, origin };
}

// Windows ships bsdtar in System32, which extracts both .zip and .tar.xz. Name
// it explicitly: a GNU tar earlier on PATH (Git for Windows) cannot read .zip.
function systemTar(): string {
    return process.platform === 'win32'
        ? path.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe')
        : 'tar';
}

async function fetchToFile(url: string, destination: string): Promise<void> {
    const response = await fetch(url);
    if (!response.ok) {
        throw new Error(`Downloading ${url} failed: HTTP ${response.status}.`);
    }
    await fs.writeFile(destination, new Uint8Array(await response.arrayBuffer()));
}

async function sha256Of(file: string): Promise<string> {
    const hash = crypto.createHash('sha256');
    for await (const chunk of createReadStream(file)) {
        hash.update(chunk as Buffer);
    }
    return hash.digest('hex');
}

async function exists(file: string): Promise<boolean> {
    try {
        await fs.access(file);
        return true;
    } catch {
        return false;
    }
}
