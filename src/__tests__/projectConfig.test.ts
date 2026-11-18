import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { loadProjectConfig, PROJECT_FILE } from '../core/project/projectConfig';

const NAME_RULE = /at most 15 bytes of lowercase ASCII letters, digits, "-" or "_"/;

// mkdtemp adds a mixed-case suffix, which is not a valid project name, so every
// project lives in a named folder inside the temporary one.
function projectDir(folderName: string, config?: unknown): string {
    const dir = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-project-')), folderName);
    fs.mkdirSync(dir);
    if (config !== undefined) {
        const text = typeof config === 'string' ? config : JSON.stringify(config);
        fs.writeFileSync(path.join(dir, PROJECT_FILE), text);
    }
    return dir;
}

test('a missing ev3c.json uses the folder name and the default rules', async () => {
    assert.deepEqual(await loadProjectConfig(projectDir('myrobot')), {
        name: 'myrobot',
        retention: { kind: 'runs', count: 5 },
        outputCapBytes: 64 * 1024,
    });
});

test('an ev3c.json without a name or rules uses the folder name and the default rules', async () => {
    assert.deepEqual(await loadProjectConfig(projectDir('line_follow-2', {})), {
        name: 'line_follow-2',
        retention: { kind: 'runs', count: 5 },
        outputCapBytes: 64 * 1024,
    });
});

test('ev3c.json sets the name, a run-count rule, and the output cap', async () => {
    const dir = projectDir('folder', { name: 'arm', retention: { keepRuns: 12 }, outputCapKiB: 16 });
    assert.deepEqual(await loadProjectConfig(dir), {
        name: 'arm',
        retention: { kind: 'runs', count: 12 },
        outputCapBytes: 16 * 1024,
    });
});

test('ev3c.json sets a megabyte rule', async () => {
    const dir = projectDir('folder', { name: 'arm', retention: { keepMiB: 2 } });
    assert.deepEqual(await loadProjectConfig(dir), {
        name: 'arm',
        retention: { kind: 'megabytes', limit: 2 },
        outputCapBytes: 64 * 1024,
    });
});

for (const [why, name] of [
    ['uppercase letters', 'MyRobot'],
    ['diacritics', 'robůtek'],
    ['spaces', 'my robot'],
    ['more than 15 bytes', 'a-sixteen-bytesx'],
    ['nothing', ''],
] as const) {
    test(`a name with ${why} is rejected with the rule in the message`, async () => {
        await assert.rejects(loadProjectConfig(projectDir('folder', { name })), (error: Error) => {
            assert.match(error.message, NAME_RULE);
            assert.ok(error.message.includes(JSON.stringify(name)), error.message);
            return true;
        });
    });
}

test('a 15-byte name is accepted', async () => {
    const config = await loadProjectConfig(projectDir('folder', { name: 'abcdefghij-12_4' }));
    assert.equal(config.name, 'abcdefghij-12_4');
});

test('an invalid folder name without ev3c.json is rejected and points at the name setting', async () => {
    await assert.rejects(loadProjectConfig(projectDir('My Robot')), (error: Error) => {
        assert.match(error.message, NAME_RULE);
        assert.match(error.message, /"name"/);
        assert.match(error.message, /ev3c\.json/);
        return true;
    });
});

test('a megabyte rule below two capped streams is rejected', async () => {
    const dir = projectDir('folder', { retention: { keepMiB: 1 }, outputCapKiB: 1024 });
    await assert.rejects(loadProjectConfig(dir), /at least two capped output streams/);
});

test('a megabyte rule of exactly two capped streams is accepted', async () => {
    const dir = projectDir('folder', { retention: { keepMiB: 1 }, outputCapKiB: 512 });
    const config = await loadProjectConfig(dir);
    assert.deepEqual(config.retention, { kind: 'megabytes', limit: 1 });
});

test('both retention rules at once are rejected', async () => {
    const dir = projectDir('folder', { retention: { keepRuns: 3, keepMiB: 2 } });
    await assert.rejects(loadProjectConfig(dir), /either "keepRuns" or "keepMiB"/);
});

for (const [setting, config] of [
    ['keepRuns', { retention: { keepRuns: 0 } }],
    ['keepRuns', { retention: { keepRuns: 2.5 } }],
    ['keepRuns', { retention: { keepRuns: '5' } }],
    ['keepMiB', { retention: { keepMiB: -1 } }],
    ['outputCapKiB', { outputCapKiB: 0 }],
    ['outputCapKiB', { outputCapKiB: 2 * 1024 * 1024 }],
] as const) {
    test(`${setting} ${JSON.stringify(config)} is rejected before building`, async () => {
        await assert.rejects(loadProjectConfig(projectDir('folder', config)), (error: Error) => {
            assert.ok(error.message.includes(`"${setting}"`), error.message);
            assert.match(error.message, /whole number from 1 to/);
            return true;
        });
    });
}

test('an ev3c.json that is not valid JSON is rejected with the file named', async () => {
    await assert.rejects(loadProjectConfig(projectDir('folder', '{ name: ')), /ev3c\.json/);
});
