import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as crypto from 'node:crypto';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { execFileSync } from 'node:child_process';
import { ensureCompiler, ToolchainManifest, Downloader } from '../core/toolchain/toolchainManager';

const HOST = 'test-host';
const ARCHIVE = 'fake-zig.tar.xz';
const ENTRY = 'fake-zig/zig';

// Same tar the manager uses: on Windows the bsdtar in System32, which reads
// both .zip and .tar.xz; elsewhere the tar on PATH.
function systemTar(): string {
    return process.platform === 'win32'
        ? path.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe')
        : 'tar';
}

function tempDir(): string {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-toolchain-'));
}

// A local stand-in for the Zig archive: one folder holding a fake compiler.
function makeFakeArchive(): { file: string; sha256: string } {
    const work = tempDir();
    fs.mkdirSync(path.join(work, 'fake-zig'));
    fs.writeFileSync(path.join(work, 'fake-zig', 'zig'), 'fake compiler\n');
    const file = path.join(work, ARCHIVE);
    execFileSync(systemTar(), ['-cJf', file, '-C', work, 'fake-zig']);
    const sha256 = crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
    return { file, sha256 };
}

function manifestFor(sha256: string): ToolchainManifest {
    return {
        compiler: 'zig',
        version: '9.9.9',
        baseUrl: 'https://example.invalid/zig/',
        hosts: { [HOST]: { file: ARCHIVE, sha256, entry: ENTRY } },
        targetFlags: [],
    };
}

function recordingDownloader(source?: string): { download: Downloader; urls: string[] } {
    const urls: string[] = [];
    const download: Downloader = (url, destination) => {
        urls.push(url);
        if (source === undefined) {
            return Promise.reject(new Error('network must not be used'));
        }
        fs.copyFileSync(source, destination);
        return Promise.resolve();
    };
    return { download, urls };
}

test('a cached archive with a matching hash is reused without a download', async () => {
    const archive = makeFakeArchive();
    const cacheDir = tempDir();
    fs.copyFileSync(archive.file, path.join(cacheDir, ARCHIVE));
    const { download, urls } = recordingDownloader();

    const compiler = await ensureCompiler({ cacheDir, download, manifest: manifestFor(archive.sha256), host: HOST });

    assert.deepEqual(urls, []);
    assert.equal(compiler.origin, 'cache');
    assert.equal(fs.readFileSync(compiler.path, 'utf8'), 'fake compiler\n');
});

test('a missing archive is downloaded from the manifest URL, extracted, and then reused', async () => {
    const archive = makeFakeArchive();
    const cacheDir = tempDir();
    const { download, urls } = recordingDownloader(archive.file);
    const options = { cacheDir, download, manifest: manifestFor(archive.sha256), host: HOST };

    const first = await ensureCompiler(options);
    const second = await ensureCompiler(options);

    assert.deepEqual(urls, ['https://example.invalid/zig/fake-zig.tar.xz']);
    assert.equal(first.origin, 'download');
    assert.equal(second.origin, 'cache');
    assert.equal(second.path, first.path);
    assert.equal(fs.readFileSync(first.path, 'utf8'), 'fake compiler\n');
});

test('a cached archive with a wrong hash is replaced by a fresh download', async () => {
    const archive = makeFakeArchive();
    const cacheDir = tempDir();
    fs.writeFileSync(path.join(cacheDir, ARCHIVE), 'partial download');
    const { download, urls } = recordingDownloader(archive.file);

    const compiler = await ensureCompiler({ cacheDir, download, manifest: manifestFor(archive.sha256), host: HOST });

    assert.equal(urls.length, 1);
    assert.equal(compiler.origin, 'download');
    assert.equal(fs.readFileSync(compiler.path, 'utf8'), 'fake compiler\n');
});

test('a downloaded archive with a wrong hash is deleted and the error names both hashes', async () => {
    const archive = makeFakeArchive();
    const cacheDir = tempDir();
    const expected = '0'.repeat(64);
    const { download } = recordingDownloader(archive.file);

    await assert.rejects(
        ensureCompiler({ cacheDir, download, manifest: manifestFor(expected), host: HOST }),
        (error: Error) => {
            assert.match(error.message, new RegExp(expected));
            assert.match(error.message, new RegExp(archive.sha256));
            return true;
        },
    );
    assert.equal(fs.existsSync(path.join(cacheDir, ARCHIVE)), false);
});

test('an override path that exists is used without a download', async () => {
    const cacheDir = tempDir();
    const compilerPath = path.join(tempDir(), 'zig');
    fs.writeFileSync(compilerPath, '');
    const { download, urls } = recordingDownloader();

    const compiler = await ensureCompiler({
        cacheDir,
        download,
        override: { path: compilerPath, setting: 'ev3c.compilerPath' },
        manifest: manifestFor('0'.repeat(64)),
        host: HOST,
    });

    assert.deepEqual(urls, []);
    assert.equal(compiler.origin, 'override');
    assert.equal(compiler.path, compilerPath);
});

test('an override path that does not exist fails with a message naming the setting', async () => {
    const missing = path.join(tempDir(), 'no-such-zig');
    const { download } = recordingDownloader();

    await assert.rejects(
        ensureCompiler({
            cacheDir: tempDir(),
            download,
            override: { path: missing, setting: 'ev3c.compilerPath' },
            manifest: manifestFor('0'.repeat(64)),
            host: HOST,
        }),
        (error: Error) => {
            assert.match(error.message, /ev3c\.compilerPath/);
            assert.ok(error.message.includes(missing));
            return true;
        },
    );
});
