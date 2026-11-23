import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { BuildResult, RunOutcome, RunProjectOptions, RunResult } from '../core';
import { cliCore, CliDeps, EXIT, runCli, runExitCode } from '../cli/cli';
import { findPackageRoot, toolchainCacheDir } from '../cli/main';
import { BRICK_ID, FakeCockpitClient, FakeCockpitError } from './fakeCockpit';

const settle = () => new Promise(resolve => setImmediate(resolve));

function projectFolder(name = 'hello'): string {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-cli-'));
    fs.writeFileSync(path.join(dir, 'ev3c.json'), JSON.stringify({ name }));
    return dir;
}

function built(name = 'hello'): Extract<BuildResult, { ok: true }> {
    const dir = `/home/root/lms2012/prjs/${name}`;
    return {
        ok: true,
        diagnostics: '',
        project: { name, retention: { kind: 'runs', count: 5 }, outputCapBytes: 65536 },
        artifacts: [
            { kind: 'program', localPath: '/tmp/prog', brickPath: `${dir}/prog` },
            { kind: 'launcher', localPath: `/tmp/${name}.rbf`, brickPath: `${dir}/${name}.rbf` },
            { kind: 'supervisor', localPath: '/tmp/sup', brickPath: `${dir}/sup` },
        ],
    };
}

function result(record: Partial<RunResult['record']> = {}, out = 'hello\n'): RunResult {
    return {
        run: 4,
        record: { run: 4, end: 'program', exit: 0, outTruncated: false, errTruncated: false, ...record },
        out,
        err: '',
        startedAt: new Date('2026-11-23T10:00:00Z'),
        returned: [],
    };
}

function setup(overrides: Partial<typeof cliCore> = {}) {
    const fake = new FakeCockpitClient();
    const names: string[] = [];
    const out: string[] = [];
    const err: string[] = [];
    let interrupt: (() => void) | undefined;
    const runs: RunProjectOptions[] = [];
    const core: typeof cliCore = {
        ...cliCore,
        buildProject: async () => built(),
        runProject: async (options: RunProjectOptions): Promise<RunOutcome> => {
            runs.push(options);
            return { kind: 'result', result: result(), start: { outcome: 'started', runSeq: 1, uploaded: [], skipped: [] } };
        },
        ...overrides,
    };
    const deps: CliDeps = {
        connect: async options => {
            names.push(options.name);
            return fake.asClient();
        },
        core,
        packageRoot: '/pkg',
        toolchainCacheDir: '/cache',
        version: '1.2.3',
        cwd: projectFolder(),
        env: {},
        onInterrupt: handler => {
            interrupt = handler;
            return () => { interrupt = undefined; };
        },
    };
    const cli = (...argv: string[]) => runCli(argv, { out: t => out.push(t), err: t => err.push(t) }, deps);
    return { fake, names, out, err, deps, runs, cli, interrupt: () => interrupt?.() };
}

test('--version, --help and a missing or unknown command', async () => {
    const s = setup();
    assert.equal(await s.cli('--version'), EXIT.ok);
    assert.deepEqual(s.out, ['1.2.3']);
    assert.equal(await s.cli('--help'), EXIT.ok);
    assert.match(s.out[1], /^Usage: ev3 c <command>/);
    assert.equal(await s.cli(), EXIT.usage);
    assert.equal(await s.cli('fly'), EXIT.usage);
    assert.match(s.err.at(-1) ?? '', /unknown command "fly"/);
    assert.equal(await s.cli('runs', 'delete', '--keep', 'many'), EXIT.usage);
    assert.deepEqual(s.names, [], 'no connection for any of these');
});

test('stop names the command and the terminal to the cockpit, holds the brick and stops it', async () => {
    const s = setup();
    assert.equal(await s.cli('stop'), EXIT.ok);
    assert.deepEqual(s.names, ['ev3 c stop (terminal)']);
    assert.deepEqual(s.fake.calls.filter(([m]) => m.startsWith('bricks/') || m.startsWith('programs/')), [
        ['bricks/connect', { brick: BRICK_ID }],
        ['programs/stop', { brick: BRICK_ID }],
    ]);
    assert.match(s.out[0], /Stop: the brick reports no program running/);
    assert.equal(s.fake.disposed, true, 'the connection is closed at the end');

    assert.equal(await s.cli('emergency-stop', '--json'), EXIT.ok);
    assert.deepEqual(JSON.parse(s.out[1]), { brickId: BRICK_ID, running: false, runSeq: 0 });
});

test('the brick comes from --brick, else EV3C_BRICK', async () => {
    const s = setup();
    s.fake.brickList.push({ ...s.fake.brickList[0], brickId: 'bt-001653000bbb', name: 'BBB' });
    await s.cli('stop', '--brick', 'BBB');
    (s.deps.env as Record<string, string>).EV3C_BRICK = 'BBB';
    await s.cli('emergency-stop');
    assert.deepEqual(s.fake.calls.filter(([m]) => m.startsWith('programs/')).map(([m, p]) => `${m} ${String(p.brick)}`), [
        'programs/stop bt-001653000bbb',
        'programs/emergencyStop bt-001653000bbb',
    ]);
});

test('run prints the result and exits with the program\'s own code', async () => {
    const s = setup({
        runProject: async (options: RunProjectOptions): Promise<RunOutcome> => {
            s.runs.push(options);
            return { kind: 'result', result: result({ exit: 3 }), start: { outcome: 'started', runSeq: 1, uploaded: [], skipped: [] } };
        },
    });
    assert.equal(await s.cli('run'), 3);
    assert.equal(s.runs.length, 1);
    assert.equal(s.runs[0].project, 'hello');
    assert.equal(s.runs[0].artifacts.length, 3, 'the build\'s artifacts go with the start');
    assert.match(s.out[0], /--- hello, run 4 ---/);
    assert.match(s.out[0], /exited with code 3/);
    assert.match(s.out[0], / {2}hello/);
});

test('run --json prints one object with the start and the result', async () => {
    const s = setup();
    assert.equal(await s.cli('run', '--json'), EXIT.ok);
    assert.equal(s.out.length, 1);
    const printed = JSON.parse(s.out[0]) as { project: string; start: { runSeq: number }; result: { run: number; out: string; record: { ended: string } } };
    assert.equal(printed.project, 'hello');
    assert.equal(printed.start.runSeq, 1);
    assert.equal(printed.result.out, 'hello\n');
    assert.equal(printed.result.record.ended, 'the program exited with code 0');
    assert.deepEqual(s.err, [], 'nothing but the object, not even the start line');
});

test('a run\'s exit code: the program\'s, 128+n for a signal, 130 when stopped', () => {
    const base = { run: 1, outTruncated: false, errTruncated: false };
    assert.equal(runExitCode({ ...base, end: 'program', exit: 7 }), 7);
    assert.equal(runExitCode({ ...base, end: 'program', exit: 300 }), 255);
    assert.equal(runExitCode({ ...base, end: 'program', signal: 11 }), 139);
    assert.equal(runExitCode({ ...base, end: 'slot', signal: 15 }), 130);
    assert.equal(runExitCode({ ...base, end: 'before-start' }), EXIT.failed);
});

test('Ctrl+C during a run stops the program and still prints its result', async () => {
    let stopped!: () => void;
    const stoppedNow = new Promise<void>(resolve => { stopped = resolve; });
    const s = setup({
        runProject: async (): Promise<RunOutcome> => {
            await stoppedNow;
            return { kind: 'result', result: result({ end: 'slot', exit: undefined, signal: 15 }), start: { outcome: 'started', runSeq: 1, uploaded: [], skipped: [] } };
        },
        stopProgram: async port => {
            stopped();
            return port.stopProgram();
        },
    });
    const running = s.cli('run');
    await settle();
    await settle();
    s.interrupt();
    assert.equal(await running, EXIT.stopped);
    assert.ok(s.err.some(line => /Stopping the program/.test(line)));
    assert.match(s.out[0], /stopped when its slot ended/);
});

test('Ctrl+C while building ends the command and starts nothing', async () => {
    const s = setup({ buildProject: () => new Promise<BuildResult>(() => undefined) });
    const running = s.cli('run');
    await settle();
    s.interrupt();
    assert.equal(await running, EXIT.stopped);
    assert.match(s.out.join('\n'), /nothing was started/);
    assert.deepEqual(s.fake.calls.filter(([m]) => m === 'programs/deployAndRun'), []);
});

test('a cockpit refusal is printed with its code and exits with 1; --json prints it as an object', async () => {
    const s = setup();
    s.fake.failures.set('programs/stop', new FakeCockpitError('brick_busy', 'busy', { holder: 'EVƎ C (VS Code)' }));
    assert.equal(await s.cli('stop'), EXIT.failed);
    assert.match(s.err.at(-1) ?? '', /^ev3 c: busy \[brick_busy\]$/);

    s.fake.failures.set('programs/stop', new FakeCockpitError('timeout', 'no answer'));
    assert.equal(await s.cli('stop', '--json'), EXIT.failed);
    assert.deepEqual(JSON.parse(s.out.at(-1) ?? ''), { error: { message: 'no answer', code: 'timeout' } });
});

test('a daemon that cannot start says why', async () => {
    const s = setup();
    const failing: CliDeps = {
        ...s.deps,
        connect: async () => { throw Object.assign(new FakeCockpitError('daemon_unavailable', 'EVƎ Cockpit is not installed.'), { failure: 'missing_binary' }); },
    };
    const err: string[] = [];
    assert.equal(await runCli(['stop'], { out: () => undefined, err: t => err.push(t) }, failing), EXIT.failed);
    assert.match(err[0], /not installed\. \[missing_binary\]/);
});

test('a folder without ev3c.json is refused before anything is built or connected', async () => {
    const s = setup();
    const empty = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-cli-empty-'));
    assert.equal(await s.cli('build', empty), EXIT.failed);
    assert.match(s.err[0], /is not an EVƎ C project: it has no ev3c\.json/);
    assert.deepEqual(s.names, []);
});

test('runs lists, shows and deletes through the core with the project from ev3c.json', async () => {
    const calls: string[] = [];
    const s = setup({
        listRuns: async (_port, _reader, project) => {
            calls.push(`list ${project}`);
            return [{ run: 3 }, { run: 4, record: result().record }];
        },
        readRunResult: async (_port, _reader, project, run) => {
            calls.push(`show ${project} ${run}`);
            return run === 4 ? result() : undefined;
        },
        deleteRuns: async (_port, project, options) => {
            calls.push(`delete ${project} keep ${options?.keepLast}`);
            return { deleted: [1, 2], kept: [3], stillPresent: [] };
        },
    });
    assert.equal(await s.cli('runs'), EXIT.ok);
    assert.match(s.out[0], /Run 3: no completion record/);
    assert.match(s.out[0], /Run 4: the program exited with code 0/);
    assert.equal(await s.cli('runs', 'show', '4'), EXIT.ok);
    assert.equal(await s.cli('runs', 'show', '9'), EXIT.failed);
    assert.equal(await s.cli('runs', 'delete', '--keep', '1'), EXIT.ok);
    assert.equal(await s.cli('runs', 'show'), EXIT.usage);
    assert.deepEqual(calls, ['list hello', 'show hello 4', 'show hello 9', 'delete hello keep 1']);
});

test('the compiler cache follows each OS\'s convention', () => {
    assert.equal(toolchainCacheDir({ LOCALAPPDATA: 'C:\\L' }, 'win32', 'C:\\H'), path.join('C:\\L', 'ev3-c', 'toolchain'));
    assert.equal(toolchainCacheDir({}, 'darwin', '/Users/u'), path.join('/Users/u', 'Library', 'Caches', 'ev3-c', 'toolchain'));
    assert.equal(toolchainCacheDir({ XDG_CACHE_HOME: '/x' }, 'linux', '/home/u'), path.join('/x', 'ev3-c', 'toolchain'));
    assert.equal(toolchainCacheDir({}, 'linux', '/home/u'), path.join('/home/u', '.cache', 'ev3-c', 'toolchain'));
});

test('the install folder is found above the program, where ev3lib\'s headers are', () => {
    const root = findPackageRoot(__dirname);
    assert.ok(fs.existsSync(path.join(root, 'ev3lib', 'include', 'ev3.h')));
});

test('run downloads the files the run returned into returned/run-<n> and reports them', async () => {
    const downloads: { project: string; paths: readonly string[]; localDir: string }[] = [];
    const s = setup({
        runProject: async (): Promise<RunOutcome> => ({
            kind: 'result',
            result: { ...result(), returned: ['/home/root/lms2012/prjs/hello/log.csv'] },
            start: { outcome: 'started', runSeq: 1, uploaded: [], skipped: [] },
        }),
        downloadReturnedFiles: async (_reader, project, paths, localDir) => {
            downloads.push({ project, paths, localDir });
            return { downloaded: paths.map(brickPath => ({ brickPath, localPath: path.join(localDir, 'log.csv') })), failed: [] };
        },
    });
    assert.equal(await s.cli('run', '--json'), EXIT.ok);
    assert.equal(downloads.length, 1);
    assert.equal(downloads[0].localDir, path.join(s.deps.cwd, 'returned', 'run-4'));
    const printed = JSON.parse(s.out[0]) as { result: { returned: string[] }; returnedFiles: { downloaded: { localPath: string }[] } };
    assert.deepEqual(printed.result.returned, ['/home/root/lms2012/prjs/hello/log.csv']);
    assert.equal(printed.returnedFiles.downloaded[0].localPath, path.join(s.deps.cwd, 'returned', 'run-4', 'log.csv'));

    assert.equal(await s.cli('run'), EXIT.ok);
    assert.match(s.out[1], /Returned files:\n {2}\/home\/root\/lms2012\/prjs\/hello\/log\.csv/);
    assert.match(s.out[1], /Returned \/home\/root\/lms2012\/prjs\/hello\/log\.csv → /);
});

test('a run that returned nothing downloads nothing', async () => {
    let called = false;
    const s = setup({
        downloadReturnedFiles: async () => {
            called = true;
            return { downloaded: [], failed: [] };
        },
    });
    assert.equal(await s.cli('run'), EXIT.ok);
    assert.equal(called, false);
});

test('get downloads one brick file, by default under its own name here', async () => {
    const s = setup();
    s.fake.files.set('/home/root/lms2012/prjs/hello/log.csv', Buffer.from('1,2\n'));
    assert.equal(await s.cli('get', '/home/root/lms2012/prjs/hello/log.csv'), EXIT.ok);
    assert.equal(fs.readFileSync(path.join(s.deps.cwd, 'log.csv'), 'utf8'), '1,2\n');
    assert.match(s.out[0], /Downloaded \/home\/root\/lms2012\/prjs\/hello\/log\.csv \(4 bytes\)/);

    assert.equal(await s.cli('get', '/home/root/lms2012/prjs/hello/log.csv', 'copy.csv', '--json'), EXIT.ok);
    assert.equal(fs.readFileSync(path.join(s.deps.cwd, 'copy.csv'), 'utf8'), '1,2\n');
    assert.equal((JSON.parse(s.out[1]) as { bytes: number }).bytes, 4);

    assert.equal(await s.cli('get', 'log.csv'), EXIT.usage, 'a brick path is absolute');
    assert.equal(await s.cli('get'), EXIT.usage);
});
