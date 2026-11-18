'use strict';
// Builds ev3lib and the supervisor for this host with Zig and runs their C
// tests. The tests fork and exec processes and map device files, so they need
// Linux or macOS, and the supervisor tests also read /proc, so they need
// Linux. Elsewhere this script says what it skips (CI runs everything on
// Linux).
const path = require('path');
const { spawnSync } = require('child_process');
const { root, loadCore, ensureZig, sourcesIn, zig, buildEv3lib } = require('./ev3lib-build.cjs');

async function main() {
    if (process.platform === 'win32') {
        console.log('Native tests need POSIX (fork, exec, mmap) and run on Linux or macOS; skipped on Windows.');
        return 0;
    }
    const compiler = await ensureZig(loadCore());
    const buildDir = 'build/native';
    const archive = `${buildDir}/libev3.a`;
    buildEv3lib(compiler, ['-g'], `${buildDir}/obj`, archive);

    const tests = sourcesIn('ev3lib/tests').filter(file => path.basename(file).startsWith('test_'));
    if (tests.length === 0) {
        console.error('run-native-tests: no test_*.c files in ev3lib/tests');
        return 1;
    }
    const runs = [];
    for (const test of tests) {
        const executable = `${buildDir}/${path.basename(test, '.c')}`;
        // Linked through the archive, as programs on the brick are, so a test
        // also proves that ev3lib's start-up and exit code gets linked in.
        zig(compiler, ['cc', '-g', '-Wall', '-Wextra', '-Werror', '-Iev3lib/include', test, archive, '-o', executable]);
        runs.push({ test, executable, args: [] });
    }

    // The supervisor: test_runfolder is linked with the run-folder code, and
    // test_supervisor drives a host build of the whole supervisor as a separate
    // process. Zig's debug builds trap undefined behaviour, which would turn
    // the test's deliberate null dereference into SIGILL instead of SIGSEGV.
    if (process.platform === 'linux') {
        const supervisorFlags = ['cc', '-g', '-fno-sanitize=undefined', '-Wall', '-Wextra', '-Werror', '-Isupervisor/src'];
        const hostSupervisor = `${buildDir}/supervisor`;
        zig(compiler, [...supervisorFlags, ...sourcesIn('supervisor/src'), '-o', hostSupervisor]);
        zig(compiler, [...supervisorFlags, 'supervisor/tests/test_runfolder.c', 'supervisor/src/runfolder.c', '-o', `${buildDir}/test_runfolder`]);
        zig(compiler, [...supervisorFlags, 'supervisor/tests/test_supervisor.c', '-o', `${buildDir}/test_supervisor`]);
        runs.push({ test: 'supervisor/tests/test_runfolder.c', executable: `${buildDir}/test_runfolder`, args: [] });
        runs.push({ test: 'supervisor/tests/test_supervisor.c', executable: `${buildDir}/test_supervisor`, args: [hostSupervisor] });
    } else {
        console.log('Supervisor tests read /proc and run on Linux only; skipped.');
    }

    const failed = [];
    for (const { test, executable, args } of runs) {
        console.log(`running ${executable}`);
        const result = spawnSync(path.join(root, executable), args, { cwd: root, stdio: 'inherit' });
        if (result.error || result.status !== 0) {
            failed.push(`${test} (${result.error?.message ?? `exit code ${result.status}, signal ${result.signal}`})`);
        }
    }
    if (failed.length > 0) {
        console.error(`native tests failed:\n  ${failed.join('\n  ')}`);
        return 1;
    }
    console.log(`native tests passed: ${runs.length} file(s)`);
    return 0;
}

main().then(code => process.exit(code), error => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
});
