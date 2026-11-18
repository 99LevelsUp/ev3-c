import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import { PacedReader } from '../core/brickPort';
import { emergencyStop, RESULT_WAIT_MS, runProject, RunOutcome, stopProgram } from '../core/brick/runControl';
import { listRunNumbers } from '../core/brick/runFolders';
import { FakeBrickPort, FakeClock } from './fakeBrickPort';

const PROJECT = '/home/root/lms2012/prjs/robot';
const RUNS = `${PROJECT}/runs`;
const REQ = `${PROJECT}/.req`;
const STOPPED = { kind: 'program', running: false } as const;

function res(run: number, launch?: string): string {
    return `run=${run}\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n${launch === undefined ? '' : `launch=${launch}\n`}`;
}

function setup() {
    const clock = new FakeClock();
    const port = new FakeBrickPort(clock);
    port.putFile(`${PROJECT}/robot.rbf`, 'launcher');
    port.putFile(`${RUNS}/next`, '2');
    port.putFile(`${RUNS}/1/res`, res(1, 'older'));
    const launches = new Map<string, Date>();
    const run = (): Promise<RunOutcome> => runProject({ port, reader: new PacedReader(port, clock), project: 'robot', launches, clock });
    return { clock, port, launches, run };
}

/** What the supervisor does with .req at start: takes the launch ID and deletes the file. */
function takeLaunchId(port: FakeBrickPort): string | undefined {
    const text = port.text(REQ);
    port.files.delete(REQ);
    return text?.split('\n')[0];
}

function assertPacedReads(port: FakeBrickPort): void {
    for (let i = 1; i < port.readTimes.length; i++) {
        assert.ok(port.readTimes[i] - port.readTimes[i - 1] >= 1000, `reads at ${port.readTimes.join(', ')}`);
    }
}

test('after Run, a "stopped" event with the record on the second poll yields the result with the PC start time', async () => {
    const { clock, port, launches, run } = setup();
    let runAt = 0;
    let request: string | undefined;
    let launchId: string | undefined;
    port.onRun = () => {
        runAt = clock.now();
        request = port.text(REQ);
        launchId = takeLaunchId(port);
        port.putFile(`${RUNS}/2/out`, 'hello\n');
        port.putFile(`${RUNS}/2/err`, '');
        clock.schedule(runAt + 500, () => port.putFile(`${RUNS}/2/res`, res(2, launchId)));
        setImmediate(() => port.emit(STOPPED));
    };

    const outcome = await run();

    assert.equal(outcome.kind, 'result');
    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
    assert.equal(outcome.result.out, 'hello\n');
    assert.equal(outcome.result.err, '');
    assert.equal(outcome.result.startedAt?.getTime(), runAt);
    assert.equal(launches.get(launchId ?? ''), outcome.result.startedAt);
    assert.match(request ?? '', /^[0-9a-f]{16}\n$/, 'a fresh launch ID on one line');
    assert.ok(port.calls.indexOf(`writeFile ${REQ}`) < port.calls.indexOf(`runProgram ${PROJECT}/robot.rbf`), '.req is written before the run');
    assert.ok(port.readTimes[0] >= runAt + 1000, 'the record is read on the second poll');
    assertPacedReads(port);
    assert.equal(port.listenerCount, 0, 'the state subscription ends with the wait');
});

test('a "stopped" event that arrives before the run starts does not start the result wait', async () => {
    const { port, run } = setup();
    port.replayOnSubscribe = STOPPED;
    port.onRun = () => {
        const launchId = takeLaunchId(port);
        port.putFile(`${RUNS}/2/out`, 'out');
        // Several turns later, long after a wrongly started wait would have given up.
        setImmediate(() => setImmediate(() => setImmediate(() => {
            port.putFile(`${RUNS}/2/res`, res(2, launchId));
            port.emit(STOPPED);
        })));
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result', `got ${outcome.kind}`);
});

test('a newer run folder from a brick-menu start, without a launch ID, is not taken as the author\'s result', async () => {
    const { port, run } = setup();
    port.onRun = () => {
        const launchId = takeLaunchId(port);
        port.putFile(`${RUNS}/2/res`, res(2, launchId));
        port.putFile(`${RUNS}/3/res`, res(3));
        setImmediate(() => port.emit(STOPPED));
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
    assert.ok(port.calls.includes(`readFile ${RUNS}/3/res`), 'the newer record was checked');
    assertPacedReads(port);
});

test('a record that never appears within 10 s yields "no result recorded" and the run stays listed', async () => {
    const { clock, port, launches, run } = setup();
    let stoppedAt = 0;
    let launchId: string | undefined;
    port.onRun = () => {
        launchId = takeLaunchId(port);
        port.putFile(`${RUNS}/2/out`, 'partial');
        setImmediate(() => {
            stoppedAt = clock.now();
            port.emit(STOPPED);
        });
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'no-result');
    assert.equal(outcome.launchId, launchId);
    assert.equal(launches.get(outcome.launchId), outcome.startedAt);
    const waited = clock.now() - stoppedAt;
    assert.ok(waited >= RESULT_WAIT_MS && waited <= RESULT_WAIT_MS + 1000, `waited ${waited} ms`);
    assert.deepEqual(await listRunNumbers(port, 'robot'), [1, 2]);
});

test('runProgram reporting not running still starts the result wait', async () => {
    const { port, run } = setup();
    port.runProgramResult = { running: false };
    port.onRun = () => port.putFile(`${RUNS}/2/res`, res(2, takeLaunchId(port)));

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
});

test('a reconnect during the wait resumes polling instead of failing', async () => {
    const { clock, port, run } = setup();
    let reconnectAt = 0;
    port.onRun = () => {
        const at = clock.now();
        reconnectAt = at + 9_000;
        const launchId = takeLaunchId(port);
        port.putFile(`${RUNS}/2/out`, 'out');
        clock.schedule(at + 500, () => { port.disconnected = true; });
        clock.schedule(reconnectAt, () => {
            port.disconnected = false;
            port.emit({ kind: 'reconnected' });
        });
        // After the first 10 s: only the wait restarted by the reconnect finds the record.
        clock.schedule(at + 12_000, () => port.putFile(`${RUNS}/2/res`, res(2, launchId)));
        setImmediate(() => port.emit(STOPPED));
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
    assert.ok(port.calls.includes('list /home/root/lms2012/prjs/robot/runs/2'), 'polled the run folder');
    assert.ok(port.readTimes[0] >= reconnectAt, 'the record is read after the reconnect');
});

test('a reconnect while the program still runs keeps waiting for it to stop', async () => {
    const { port, run } = setup();
    port.onRun = () => {
        const launchId = takeLaunchId(port);
        port.putFile(`${RUNS}/2/out`, 'out');
        setImmediate(() => {
            port.emit({ kind: 'reconnected' });
            // Several turns later, long after a wrongly started wait would have given up.
            setImmediate(() => setImmediate(() => setImmediate(() => {
                port.putFile(`${RUNS}/2/res`, res(2, launchId));
                port.running = false;
                port.emit(STOPPED);
            })));
        });
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result', `got ${outcome.kind}`);
    assert.ok(port.calls.includes('isProgramRunning'));
});

test('a reconnect after the program ended while disconnected starts the result wait', async () => {
    const { port, run } = setup();
    port.onRun = () => {
        port.putFile(`${RUNS}/2/res`, res(2, takeLaunchId(port)));
        setImmediate(() => {
            port.running = false;
            port.emit({ kind: 'reconnected' });
        });
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
});

test('a Run cockpit refuses removes the launch request and records no start', async () => {
    const { port, launches, run } = setup();
    port.runProgramError = 'program_running';

    await assert.rejects(run(), (error: { code?: string }) => error.code === 'program_running');

    assert.equal(port.has(REQ), false);
    assert.equal(launches.size, 0);
    assert.equal(port.listenerCount, 0);
});

test('a Run whose outcome is unknown keeps the launch request for the supervisor', async () => {
    const { port, run } = setup();
    port.runProgramError = 'timeout';

    await assert.rejects(run(), (error: { code?: string }) => error.code === 'timeout');

    assert.equal(port.has(REQ), true);
});

test('Stop and Emergency Stop are forwarded unchanged', async () => {
    const { port } = setup();
    port.running = true;

    assert.deepEqual(await stopProgram(port), { running: false });
    assert.deepEqual(await emergencyStop(port), { running: false });
    assert.deepEqual(port.calls, ['stopProgram', 'emergencyStop']);
});
