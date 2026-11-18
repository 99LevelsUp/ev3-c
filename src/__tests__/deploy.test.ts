import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { BuildArtifact } from '../core/build/buildProject';
import { BrickOperationError, deploy } from '../core/brick/deploy';
import { FakeBrickPort, FakeClock } from './fakeBrickPort';

const BRICK_DIR = '/home/root/lms2012/prjs/robot';

function artifacts(): BuildArtifact[] {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-deploy-'));
    const make = (kind: BuildArtifact['kind'], name: string, content: string): BuildArtifact => {
        const localPath = path.join(dir, name);
        fs.writeFileSync(localPath, content);
        return { kind, localPath, brickPath: `${BRICK_DIR}/${name}` };
    };
    return [
        make('program', 'prog', 'program v2'),
        make('launcher', 'robot.rbf', 'launcher v2'),
        make('supervisor', 'ev3c-sup', 'supervisor v1'),
    ];
}

function brick(): FakeBrickPort {
    const port = new FakeBrickPort(new FakeClock());
    port.mkdirs('/home/root/lms2012/prjs');
    return port;
}

function writes(port: FakeBrickPort): string[] {
    return port.calls.filter(call => call.startsWith('writeFile '));
}

test('deploy uploads only the artifacts whose MD5 differs from the brick listing', async () => {
    const port = brick();
    port.putFile(`${BRICK_DIR}/prog`, 'program v1');
    port.putFile(`${BRICK_DIR}/robot.rbf`, 'launcher v2');
    port.putFile(`${BRICK_DIR}/ev3c-sup`, 'supervisor v1');

    const report = await deploy(port, artifacts());

    assert.deepEqual(report.uploaded.map(a => a.kind), ['program']);
    assert.deepEqual(report.unchanged.map(a => a.kind), ['launcher', 'supervisor']);
    assert.deepEqual(writes(port), [`writeFile ${BRICK_DIR}/prog`]);
    assert.equal(port.text(`${BRICK_DIR}/prog`), 'program v2');
});

test('deploy uploads a file whose listing has no MD5', async () => {
    const port = brick();
    port.putFile(`${BRICK_DIR}/prog`, 'program v2');
    port.putFile(`${BRICK_DIR}/robot.rbf`, 'launcher v2');
    port.putFile(`${BRICK_DIR}/ev3c-sup`, 'supervisor v1');
    const list = port.list.bind(port);
    port.list = async (folder: string) => (await list(folder)).map(entry => ({ ...entry, md5: undefined }));

    const report = await deploy(port, artifacts());

    assert.equal(report.uploaded.length, 3);
});

test('deploy compares MD5 without regard to letter case', async () => {
    const port = brick();
    port.putFile(`${BRICK_DIR}/prog`, 'program v2');
    port.putFile(`${BRICK_DIR}/robot.rbf`, 'launcher v2');
    port.putFile(`${BRICK_DIR}/ev3c-sup`, 'supervisor v1');
    const list = port.list.bind(port);
    port.list = async (folder: string) => (await list(folder)).map(entry => ({ ...entry, md5: entry.md5?.toUpperCase() }));

    const report = await deploy(port, artifacts());

    assert.equal(report.uploaded.length, 0);
});

test('deploy creates a missing project folder and uploads everything', async () => {
    const port = brick();

    const report = await deploy(port, artifacts());

    assert.ok(port.calls.includes(`makeDirectory ${BRICK_DIR}`));
    assert.equal(report.uploaded.length, 3);
    assert.equal(port.text(`${BRICK_DIR}/ev3c-sup`), 'supervisor v1');
});

test('deploy refuses while a program runs and touches nothing on the brick', async () => {
    const port = brick();
    port.running = true;

    await assert.rejects(deploy(port, artifacts()), (error: unknown) => {
        assert.ok(error instanceof BrickOperationError);
        assert.equal(error.code, 'program_running');
        assert.match(error.message, /program is running/i);
        assert.match(error.message, /stop it/i);
        return true;
    });
    assert.deepEqual(port.calls, ['isProgramRunning']);
});

test('a failed upload is retried once', async () => {
    const port = brick();
    port.writeFailures.set(`${BRICK_DIR}/prog`, ['transport_failed']);

    const report = await deploy(port, artifacts());

    assert.equal(report.uploaded.length, 3);
    assert.equal(writes(port).filter(call => call.endsWith('/prog')).length, 2);
    assert.equal(port.text(`${BRICK_DIR}/prog`), 'program v2');
});

test('an upload that fails twice is reported with the cockpit error code', async () => {
    const port = brick();
    port.writeFailures.set(`${BRICK_DIR}/robot.rbf`, ['transport_failed', 'timeout']);

    await assert.rejects(deploy(port, artifacts()), (error: unknown) => {
        assert.ok(error instanceof BrickOperationError);
        assert.equal(error.code, 'timeout');
        assert.match(error.message, /robot\.rbf/);
        return true;
    });
    assert.equal(writes(port).filter(call => call.endsWith('/robot.rbf')).length, 2);
    assert.equal(port.has(`${BRICK_DIR}/ev3c-sup`), false, 'nothing is uploaded after the failure');
});
