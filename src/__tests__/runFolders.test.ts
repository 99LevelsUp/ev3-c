import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import { deleteRuns, listRunNumbers } from '../core/brick/runFolders';
import { FakeBrickPort, FakeClock } from './fakeBrickPort';

const RUNS = '/home/root/lms2012/prjs/robot/runs';

function brickWithRuns(complete: number[], incomplete: number[] = []): FakeBrickPort {
    const port = new FakeBrickPort(new FakeClock());
    port.putFile(`${RUNS}/next`, String(Math.max(0, ...complete, ...incomplete) + 1));
    for (const run of complete) {
        port.putFile(`${RUNS}/${run}/out`, 'out');
        port.putFile(`${RUNS}/${run}/res`, `run=${run}\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n`);
    }
    for (const run of incomplete) {
        port.putFile(`${RUNS}/${run}/out`, 'out');
    }
    return port;
}

function removals(port: FakeBrickPort): string[] {
    return port.calls.filter(call => call.startsWith('remove'));
}

test('run folders are listed by number in numeric order, other entries ignored', async () => {
    const port = brickWithRuns([10, 2, 9]);
    port.mkdirs(`${RUNS}/tmp`);
    port.mkdirs(`${RUNS}/3a`);
    port.putFile(`${RUNS}/next.tmp`, '11');

    assert.deepEqual(await listRunNumbers(port, 'robot'), [2, 9, 10]);
});

test('a project without a runs folder has no runs', async () => {
    const port = new FakeBrickPort(new FakeClock());
    port.mkdirs('/home/root/lms2012/prjs/robot');

    assert.deepEqual(await listRunNumbers(port, 'robot'), []);
});

test('"delete all but the last 2" removes the older folders in one call and reports one still present', async () => {
    const port = brickWithRuns([1, 2, 3, 4, 5]);
    // A folder with a subfolder survives the fake's recursive removal, which deletes one level only.
    port.mkdirs(`${RUNS}/2/data`);

    const report = await deleteRuns(port, 'robot', { keepLast: 2 });

    assert.deepEqual(removals(port), [`removePaths -r ${RUNS}/1 ${RUNS}/2 ${RUNS}/3`]);
    assert.equal(port.calls.at(-1), `list ${RUNS}`, 'the runs folder is listed again after deleting');
    assert.deepEqual(report, { deleted: [1, 3], kept: [4, 5], stillPresent: [2] });
    assert.equal(port.text(`${RUNS}/next`), '6', 'the run counter is never touched');
});

test('"delete all" during a run keeps the running program\'s folder', async () => {
    const port = brickWithRuns([1, 2], [3]);
    port.running = true;

    const report = await deleteRuns(port, 'robot');

    assert.deepEqual(report, { deleted: [1, 2], kept: [3], stillPresent: [] });
    assert.ok(port.has(`${RUNS}/3/out`));
});

test('"delete all" with no program running also removes a folder left without a result', async () => {
    const port = brickWithRuns([1, 2], [3]);

    const report = await deleteRuns(port, 'robot');

    assert.deepEqual(report, { deleted: [1, 2, 3], kept: [], stillPresent: [] });
});

test('while the slot runs but the newest run is complete, no folder is protected', async () => {
    const port = brickWithRuns([1, 2]);
    port.running = true;

    const report = await deleteRuns(port, 'robot');

    assert.deepEqual(report, { deleted: [1, 2], kept: [], stillPresent: [] });
});

test('a folder that appears during deletion is not deleted', async () => {
    const port = brickWithRuns([1, 2]);
    port.onRemove = () => port.putFile(`${RUNS}/3/out`, 'new run');

    const report = await deleteRuns(port, 'robot');

    assert.deepEqual(report.deleted, [1, 2]);
    assert.ok(port.has(`${RUNS}/3`));
    assert.ok(!removals(port).some(call => call.endsWith('/3')));
});

test('deleting from a project without runs does nothing', async () => {
    const port = new FakeBrickPort(new FakeClock());
    port.mkdirs('/home/root/lms2012/prjs/robot');

    assert.deepEqual(await deleteRuns(port, 'robot'), { deleted: [], kept: [], stillPresent: [] });
    assert.deepEqual(removals(port), []);
});

test('keeping more runs than there are keeps them all', async () => {
    const port = brickWithRuns([1, 2, 3]);

    const report = await deleteRuns(port, 'robot', { keepLast: 5 });

    assert.deepEqual(report, { deleted: [], kept: [1, 2, 3], stillPresent: [] });
    assert.deepEqual(removals(port), []);
});
