import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { BuildArtifact } from '../core/build/buildProject';
import { emergencyStop, RESULT_WAIT_MS, runProject, RunOutcome, stopProgram } from '../core/brick/runControl';
import { listRunNumbers } from '../core/brick/runFolders';
import { FakeBrickPort, FakeClock } from './fakeBrickPort';

const PROJECT = '/home/root/lms2012/prjs/robot';
const RUNS = `${PROJECT}/runs`;
const REQ = `${PROJECT}/.req`;
const RBF = `${PROJECT}/robot.rbf`;

function res(run: number, launch?: string): string {
    return `run=${run}\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n${launch === undefined ? '' : `launch=${launch}\n`}`;
}

function artifacts(): BuildArtifact[] {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-run-'));
    const make = (kind: BuildArtifact['kind'], name: string, content: string): BuildArtifact => {
        const localPath = path.join(dir, name);
        fs.writeFileSync(localPath, content);
        return { kind, localPath, brickPath: `${PROJECT}/${name}` };
    };
    return [make('program', 'prog', 'program'), make('launcher', 'robot.rbf', 'launcher'), make('supervisor', 'sup', 'supervisor')];
}

function setup() {
    const clock = new FakeClock();
    const port = new FakeBrickPort(clock);
    port.putFile(`${PROJECT}/sup`, 'supervisor');
    port.putFile(`${RUNS}/next`, '2');
    port.putFile(`${RUNS}/1/res`, res(1, 'older'));
    const launches = new Map<string, Date>();
    const files = artifacts();
    const run = (): Promise<RunOutcome> => runProject({ port, reader: port, project: 'robot', artifacts: files, launches, clock });
    return { clock, port, launches, run };
}

/** What the supervisor does with .req at start: takes the launch ID and deletes the file. */
function takeLaunchId(port: FakeBrickPort): string | undefined {
    const text = port.text(REQ);
    port.files.delete(REQ);
    return text?.split('\n')[0];
}

test('a Run uploads the changed files and the launch request with the start, and a stop yields the result', async () => {
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
        setImmediate(() => port.stop());
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
    assert.equal(outcome.result.out, 'hello\n');
    assert.equal(outcome.result.err, '');
    assert.equal(outcome.result.startedAt?.getTime(), runAt);
    assert.equal(launches.get(launchId ?? ''), outcome.result.startedAt);
    assert.match(request ?? '', /^[0-9a-f]{16}\n$/, 'a fresh launch ID on one line');
    assert.deepEqual(outcome.start.uploaded, [`${PROJECT}/prog`, RBF, REQ]);
    assert.deepEqual(outcome.start.skipped, [`${PROJECT}/sup`], 'an unchanged supervisor is left');
    assert.ok(port.calls.indexOf('onBrickEvent') < port.calls.indexOf(`deployAndRun ${RBF}`), 'subscribed before the start');
    assert.ok(port.readTimes[0] >= runAt + 1000, 'the record is read on the second poll');
    assert.equal(port.listenerCount, 0, 'the state subscription ends with the wait');
});

test('a stopped state from before the start does not start the result wait', async () => {
    const { port, run } = setup();
    port.replayOnSubscribe = { kind: 'program', state: { running: false, runSeq: 0 } };
    port.onRun = () => {
        const launchId = takeLaunchId(port);
        port.putFile(`${RUNS}/2/out`, 'out');
        // Several turns later, long after a wrongly started wait would have given up.
        setImmediate(() => setImmediate(() => setImmediate(() => {
            port.putFile(`${RUNS}/2/res`, res(2, launchId));
            port.stop();
        })));
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result', `got ${outcome.kind}`);
});

test('a program that ends before the start returns is not missed', async () => {
    const { port, run } = setup();
    port.onRun = () => {
        port.putFile(`${RUNS}/2/res`, res(2, takeLaunchId(port)));
        // Heard while deployAndRun is still returning: its number is not known yet.
        port.stop();
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
});

test('a later start by someone else ends the wait for this run', async () => {
    const { port, run } = setup();
    port.onRun = () => {
        port.putFile(`${RUNS}/2/res`, res(2, takeLaunchId(port)));
        setImmediate(() => port.emit({ kind: 'program', state: { running: true, runSeq: port.runSeq + 1 } }));
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
});

test('a newer run folder from a brick-menu start, without a launch ID, is not taken as the author\'s result', async () => {
    const { port, run } = setup();
    port.onRun = () => {
        const launchId = takeLaunchId(port);
        port.putFile(`${RUNS}/2/res`, res(2, launchId));
        port.putFile(`${RUNS}/3/res`, res(3));
        setImmediate(() => port.stop());
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
    assert.ok(port.calls.includes(`readFile ${RUNS}/3/res`), 'the newer record was checked');
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
            port.stop();
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

test('a start not seen running starts the result wait at once', async () => {
    const { port, run } = setup();
    port.startOutcome = 'not_seen_running';
    port.onRun = () => port.putFile(`${RUNS}/2/res`, res(2, takeLaunchId(port)));

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
});

test('an uncertain start reads the state and is never repeated', async () => {
    const { port, run } = setup();
    port.startOutcome = 'uncertain';
    port.onRun = () => {
        port.putFile(`${RUNS}/2/res`, res(2, takeLaunchId(port)));
        port.running = false;
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.ok(port.calls.includes('programState'));
    assert.equal(port.calls.filter(call => call.startsWith('deployAndRun')).length, 1);
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
        setImmediate(() => port.stop());
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result');
    assert.equal(outcome.result.run, 2);
    assert.ok(port.calls.includes(`list ${RUNS}/2`), 'polled the run folder');
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
                port.stop();
            })));
        });
    };

    const outcome = await run();

    assert.ok(outcome.kind === 'result', `got ${outcome.kind}`);
    assert.ok(port.calls.includes('programState'));
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

test('a Run refused because a program runs uploads nothing and records no start', async () => {
    const { port, launches, run } = setup();
    port.running = true;

    await assert.rejects(run(), (error: { code?: string }) => error.code === 'program_running');

    assert.equal(port.has(REQ), false);
    assert.equal(port.has(`${PROJECT}/prog`), false);
    assert.equal(launches.size, 0);
    assert.equal(port.listenerCount, 0);
});

test('a Run a stop cancels after the upload removes the launch request', async () => {
    const { port, launches, run } = setup();
    port.startError = 'cancelled';

    await assert.rejects(run(), (error: { code?: string }) => error.code === 'cancelled');

    assert.equal(port.has(REQ), false);
    assert.equal(launches.size, 0);
});

test('a Run whose outcome is unknown keeps the launch request for the supervisor', async () => {
    const { port, run } = setup();
    port.startError = 'outcome_unknown';

    await assert.rejects(run(), (error: { code?: string }) => error.code === 'outcome_unknown');

    assert.equal(port.has(REQ), true);
});

test('Stop and Emergency Stop are forwarded unchanged', async () => {
    const { port } = setup();
    port.running = true;

    assert.deepEqual(await stopProgram(port), { running: false, runSeq: 0 });
    assert.deepEqual(await emergencyStop(port), { running: false, runSeq: 0 });
    assert.deepEqual(port.calls, ['stopProgram', 'emergencyStop']);
});
