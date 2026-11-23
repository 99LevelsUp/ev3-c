import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { downloadReturnedFiles, localPathFor, parseReturnList, readRunResult, returnedDir } from '../core';
import { FakeBrickPort, FakeClock } from './fakeBrickPort';

const PROJECT = '/home/root/lms2012/prjs/robot';
const LOCAL = path.join(os.tmpdir(), 'ev3c-returned-fixed');

test('a return list keeps absolute paths once each, in the order marked', () => {
    assert.deepEqual(parseReturnList('/a/b.txt\nrelative.txt\n\n/a/./b.txt\n/c/d\r\n/a/x/../e\n'), ['/a/b.txt', '/c/d', '/a/e']);
});

test('a file in the project lands at its path in the project; any other below brick/', () => {
    assert.equal(localPathFor(`${PROJECT}/data/log.csv`, 'robot', LOCAL), path.join(LOCAL, 'data', 'log.csv'));
    assert.equal(localPathFor('/mnt/ramdisk/prjs/robot/log.csv', 'robot', LOCAL), path.join(LOCAL, 'log.csv'), 'prjs reached through RAM');
    assert.equal(localPathFor('/media/card/out.txt', 'robot', LOCAL), path.join(LOCAL, 'brick', 'media', 'card', 'out.txt'));
    assert.equal(localPathFor('/home/root/lms2012/prjs/robotics/x', 'robot', LOCAL),
        path.join(LOCAL, 'brick', 'home', 'root', 'lms2012', 'prjs', 'robotics', 'x'), 'a sibling project is not this project');
});

test('a returned path never leaves the local folder and never names it', () => {
    for (const bad of ['relative.txt', `${PROJECT}/`]) {
        assert.throws(() => localPathFor(bad, 'robot', LOCAL), bad);
    }
    // Normalized first, so climbing out of the project ends up below brick/, still inside.
    assert.equal(localPathFor(`${PROJECT}/../../../../../../etc/passwd`, 'robot', LOCAL), path.join(LOCAL, 'brick', 'etc', 'passwd'));
});

test('names this computer cannot hold are made safe', () => {
    assert.equal(localPathFor(`${PROJECT}/a:b*c?.txt`, 'robot', LOCAL), path.join(LOCAL, 'a_b_c_.txt'));
    assert.equal(localPathFor(`${PROJECT}/end. `, 'robot', LOCAL), path.join(LOCAL, 'end__'));
});

test('a run\'s result lists its returned files, and they download into returned/run-<n>', async () => {
    const port = new FakeBrickPort(new FakeClock());
    port.putFile(`${PROJECT}/runs/3/res`, 'run=3\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n');
    port.putFile(`${PROJECT}/runs/3/out`, '');
    port.putFile(`${PROJECT}/runs/3/ret`, `${PROJECT}/log.csv\n/media/card/missing.txt\n`);
    port.putFile(`${PROJECT}/log.csv`, '1,2\n');

    const result = await readRunResult(port, port, 'robot', 3, new Map());
    assert.deepEqual(result?.returned, [`${PROJECT}/log.csv`, '/media/card/missing.txt']);

    const projectDir = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-returned-'));
    const local = returnedDir(projectDir, 3);
    const report = await downloadReturnedFiles(port, 'robot', result?.returned ?? [], local);
    assert.deepEqual(report.downloaded, [{ brickPath: `${PROJECT}/log.csv`, localPath: path.join(projectDir, 'returned', 'run-3', 'log.csv') }]);
    assert.equal(fs.readFileSync(report.downloaded[0].localPath, 'utf8'), '1,2\n');
    assert.equal(report.failed.length, 1, 'a missing file is reported and does not stop the rest');
    assert.equal(report.failed[0].brickPath, '/media/card/missing.txt');
});

test('a run without a return list returned nothing', async () => {
    const port = new FakeBrickPort(new FakeClock());
    port.putFile(`${PROJECT}/runs/1/res`, 'run=1\nend=program\nexit=0\nout_truncated=0\nerr_truncated=0\n');
    assert.deepEqual((await readRunResult(port, port, 'robot', 1, new Map()))?.returned, []);
});
