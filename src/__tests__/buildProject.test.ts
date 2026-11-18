import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { buildProject, BuildProjectOptions } from '../core/build/buildProject';
import type { CompileRequest, CompileResult } from '../core/toolchain/compiler';
import { buildLauncher } from '../core/launcher/launcherTemplate';

// Compiled tests run from out/__tests__; the binary fixtures stay in the source tree.
const elfFixtures = path.join(__dirname, '..', '..', 'src', '__tests__', 'fixtures', 'elf');

interface Setup {
    options: BuildProjectOptions;
    projectDir: string;
    runtimeDir: string;
    requests: CompileRequest[];
}

// A package root with a fake runtime, a project folder, and a compile step that
// writes a fixture ELF instead of running Zig.
function setup(config: unknown, elfFixture = 'static-armv5te.elf', fail?: string): Setup {
    const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-build-'));
    const packageRoot = path.join(temp, 'package');
    const runtimeDir = path.join(packageRoot, 'out', 'runtime');
    fs.mkdirSync(runtimeDir, { recursive: true });
    fs.writeFileSync(path.join(runtimeDir, 'libev3.a'), 'fake archive');
    fs.writeFileSync(path.join(runtimeDir, 'supervisor'), 'fake supervisor');
    fs.mkdirSync(path.join(packageRoot, 'ev3lib', 'include'), { recursive: true });

    const projectDir = path.join(temp, 'project');
    fs.mkdirSync(path.join(projectDir, 'notes'), { recursive: true });
    fs.writeFileSync(path.join(projectDir, 'main.c'), 'int main(void) { return 0; }\n');
    fs.writeFileSync(path.join(projectDir, 'drive.c'), 'void drive(void) {}\n');
    fs.writeFileSync(path.join(projectDir, 'readme.txt'), 'not a source\n');
    fs.writeFileSync(path.join(projectDir, 'notes', 'old.c'), 'not compiled\n');
    fs.writeFileSync(path.join(projectDir, 'ev3c.json'), JSON.stringify(config));

    const compilerPath = path.join(temp, 'zig');
    fs.writeFileSync(compilerPath, '');

    const requests: CompileRequest[] = [];
    const compile = (request: CompileRequest): Promise<CompileResult> => {
        requests.push(request);
        const elfPath = path.resolve(request.cwd ?? '.', request.output);
        if (fail !== undefined) {
            return Promise.resolve({ ok: false, elfPath, exitCode: 1, diagnostics: fail });
        }
        fs.copyFileSync(path.join(elfFixtures, elfFixture), elfPath);
        return Promise.resolve({ ok: true, elfPath, exitCode: 0, diagnostics: '' });
    };

    return {
        options: {
            projectDir,
            packageRoot,
            toolchain: { cacheDir: path.join(temp, 'cache'), override: { path: compilerPath, setting: 'test.compiler' } },
            compile,
        },
        projectDir,
        runtimeDir,
        requests,
    };
}

test('a successful build returns program, launcher, and supervisor with brick paths under 60 bytes', async () => {
    const name = 'abcdefghij-12_4';
    const { options, projectDir, runtimeDir } = setup({ name, retention: { keepMiB: 3 }, outputCapKiB: 32 });

    const result = await buildProject(options);

    assert.equal(result.ok, true);
    assert.ok(result.ok);
    const folder = `/home/root/lms2012/prjs/${name}`;
    assert.deepEqual(result.artifacts, [
        { kind: 'program', localPath: path.join(projectDir, 'build', 'prog'), brickPath: `${folder}/prog` },
        { kind: 'launcher', localPath: path.join(projectDir, 'build', `${name}.rbf`), brickPath: `${folder}/${name}.rbf` },
        { kind: 'supervisor', localPath: path.join(runtimeDir, 'supervisor'), brickPath: `${folder}/sup` },
    ]);
    for (const artifact of result.artifacts) {
        assert.ok(Buffer.byteLength(artifact.brickPath) < 60, artifact.brickPath);
        assert.ok(fs.existsSync(artifact.localPath), artifact.localPath);
    }
    assert.deepEqual(
        fs.readFileSync(result.artifacts[1].localPath),
        Buffer.from(buildLauncher({ retention: { kind: 'megabytes', limit: 3 }, outputCapBytes: 32 * 1024 })),
    );
    assert.deepEqual(fs.readFileSync(result.artifacts[0].localPath), fs.readFileSync(path.join(elfFixtures, 'static-armv5te.elf')));
});

test('the compiler gets the project sources, the ev3lib headers, the forced start-up, and libev3.a', async () => {
    const { options, projectDir, runtimeDir, requests } = setup({ name: 'robot' });

    await buildProject(options);

    assert.equal(requests.length, 1);
    const [request] = requests;
    assert.equal(request.compilerPath, options.toolchain.override?.path);
    assert.equal(request.cwd, projectDir);
    assert.deepEqual(request.sources, ['drive.c', 'main.c']);
    assert.deepEqual(request.includeDirs, [path.join(options.packageRoot, 'ev3lib', 'include')]);
    assert.deepEqual(request.libraries, ['-Wl,-u,ev3_core_init', path.join(runtimeDir, 'libev3.a')]);
    assert.equal(path.resolve(projectDir, request.output), path.join(projectDir, 'build', 'prog'));
});

test('a runtime folder given by the caller replaces the one in the package', async () => {
    const { options, runtimeDir, requests } = setup({ name: 'robot' });
    const elsewhere = path.join(path.dirname(options.packageRoot), 'runtime-copy');
    fs.cpSync(runtimeDir, elsewhere, { recursive: true });

    const result = await buildProject({ ...options, runtimeDir: elsewhere });

    assert.ok(result.ok);
    assert.equal(result.artifacts[2].localPath, path.join(elsewhere, 'supervisor'));
    assert.deepEqual(requests[0].libraries.slice(-1), [path.join(elsewhere, 'libev3.a')]);
});

test('a compile error returns the compiler diagnostics and leaves no artifacts', async () => {
    const good = setup({ name: 'robot' });
    assert.ok((await buildProject(good.options)).ok);
    const diagnostics = "main.c:1:18: error: use of undeclared identifier 'x'\n";
    const failing = { ...good.options, compile: setup({ name: 'robot' }, undefined, diagnostics).options.compile };

    const result = await buildProject(failing);

    assert.deepEqual(result, { ok: false, diagnostics });
    const buildDir = path.join(good.projectDir, 'build');
    assert.deepEqual(fs.existsSync(buildDir) ? fs.readdirSync(buildDir) : [], []);
});

test('an invalid project name stops the build before compiling', async () => {
    const { options, requests } = setup({ name: 'My Robot' });
    await assert.rejects(buildProject(options), /at most 15 bytes/);
    assert.equal(requests.length, 0);
});

test('a missing supervisor in the runtime folder stops the build with a clear message', async () => {
    const { options, runtimeDir, requests } = setup({ name: 'robot' });
    fs.rmSync(path.join(runtimeDir, 'supervisor'));
    await assert.rejects(buildProject(options), (error: Error) => {
        assert.match(error.message, /supervisor/);
        assert.ok(error.message.includes(runtimeDir), error.message);
        return true;
    });
    assert.equal(requests.length, 0);
});

test('a program that fails the ELF check is refused and no launcher is written', async () => {
    const { options, projectDir } = setup({ name: 'robot' }, 'armv7.elf');
    await assert.rejects(buildProject(options), /ARMv7/);
    assert.equal(fs.existsSync(path.join(projectDir, 'build', 'robot.rbf')), false);
});

test('a project without C sources is refused', async () => {
    const { options, projectDir, requests } = setup({ name: 'robot' });
    fs.rmSync(path.join(projectDir, 'main.c'));
    fs.rmSync(path.join(projectDir, 'drive.c'));
    await assert.rejects(buildProject(options), /no \.c files/);
    assert.equal(requests.length, 0);
});
