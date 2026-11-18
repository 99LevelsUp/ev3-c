import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import { PacedReader } from '../core/brickPort';
import { listRuns, parseRunRecord, readRunResult } from '../core/brick/runResults';
import { FakeBrickPort, FakeClock } from './fakeBrickPort';

const RUNS = '/home/root/lms2012/prjs/robot/runs';

function setup(): { port: FakeBrickPort; reader: PacedReader; clock: FakeClock } {
    const clock = new FakeClock();
    const port = new FakeBrickPort(clock);
    port.putFile(`${RUNS}/next`, '5');
    return { port, reader: new PacedReader(port, clock), clock };
}

test('a completion record with an exit code and a launch ID is parsed', () => {
    assert.deepEqual(parseRunRecord('run=7\nend=program\nexit=3\nout_truncated=0\nerr_truncated=1\nlaunch=abc-123\n'), {
        run: 7,
        end: 'program',
        exit: 3,
        outTruncated: false,
        errTruncated: true,
        launch: 'abc-123',
    });
});

test('a completion record of a run killed by a signal has no exit code and no launch ID', () => {
    assert.deepEqual(parseRunRecord('run=2\nend=slot\nsignal=9\nout_truncated=0\nerr_truncated=0\n'), {
        run: 2,
        end: 'slot',
        signal: 9,
        outTruncated: false,
        errTruncated: false,
    });
});

test('unknown keys in a completion record are ignored', () => {
    const record = parseRunRecord('run=3\nend=before-start\nret=later\nout_truncated=0\nerr_truncated=0\n');
    assert.equal(record.end, 'before-start');
    assert.equal(record.run, 3);
});

for (const text of ['end=program\nout_truncated=0\nerr_truncated=0\n', 'run=1\nout_truncated=0\n', 'run=x\nend=program\n', 'run=1\nend=crashed\n']) {
    test(`a completion record without a valid run number or end is rejected: ${JSON.stringify(text)}`, () => {
        assert.throws(() => parseRunRecord(text), /completion record/);
    });
}

test('a run result joins the record, both outputs, and the PC start time', async () => {
    const { port, reader } = setup();
    port.putFile(`${RUNS}/4/out`, 'hello\n');
    port.putFile(`${RUNS}/4/err`, 'oops\n');
    port.putFile(`${RUNS}/4/res`, 'run=4\nend=program\nsignal=11\nout_truncated=0\nerr_truncated=0\nlaunch=L4\n');
    const startedAt = new Date(2026, 10, 18, 10, 0, 0);

    const result = await readRunResult(port, reader, 'robot', 4, new Map([['L4', startedAt]]));

    assert.ok(result);
    assert.equal(result.run, 4);
    assert.equal(result.record.signal, 11);
    assert.equal(result.out, 'hello\n');
    assert.equal(result.err, 'oops\n');
    assert.equal(result.startedAt, startedAt);
});

test('a run without a completion record has no result yet', async () => {
    const { port, reader } = setup();
    port.putFile(`${RUNS}/4/out`, 'partial');
    port.putFile(`${RUNS}/4/res.tmp`, 'run=4\nend=program\n');

    assert.equal(await readRunResult(port, reader, 'robot', 4, new Map()), undefined);
    assert.deepEqual(port.readTimes, [], 'nothing is read before the record exists');
});

test('an output stream that is not on the brick is left out instead of shown as empty', async () => {
    const { port, reader } = setup();
    port.putFile(`${RUNS}/4/res`, 'run=4\nend=before-start\nout_truncated=0\nerr_truncated=0\n');

    const result = await readRunResult(port, reader, 'robot', 4, new Map());

    assert.ok(result);
    assert.equal(result.out, undefined);
    assert.equal(result.err, undefined);
    assert.equal(result.startedAt, undefined);
});

test('listing shows a run started on the brick without a PC time next to runs EVƎ C started', async () => {
    const { port, reader } = setup();
    const first = new Date(2026, 10, 18, 9, 0, 0);
    const third = new Date(2026, 10, 18, 9, 5, 0);
    port.putFile(`${RUNS}/1/res`, 'run=1\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\nlaunch=A\n');
    port.putFile(`${RUNS}/2/res`, 'run=2\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n');
    port.putFile(`${RUNS}/3/res`, 'run=3\nend=slot\nsignal=15\nout_truncated=0\nerr_truncated=0\nlaunch=B\n');
    port.putFile(`${RUNS}/4/out`, 'still running');

    const runs = await listRuns(port, reader, 'robot', new Map([['A', first], ['B', third]]));

    assert.deepEqual(runs.map(run => [run.run, run.startedAt, run.record?.end]), [
        [1, first, 'program'],
        [2, undefined, 'program'],
        [3, third, 'slot'],
        [4, undefined, undefined],
    ]);
});

test('listing reads the completion records at most once per second', async () => {
    const { port, reader } = setup();
    for (const run of [1, 2, 3]) {
        port.putFile(`${RUNS}/${run}/res`, `run=${run}\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n`);
    }

    await listRuns(port, reader, 'robot', new Map());

    assert.equal(port.readTimes.length, 3);
    for (let i = 1; i < port.readTimes.length; i++) {
        assert.ok(port.readTimes[i] - port.readTimes[i - 1] >= 1000, `reads ${port.readTimes.join(', ')}`);
    }
});
