import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';

// The core and the cockpit link must run in plain Node (the `ev3 c` command reuses them),
// so nothing under src/core or src/cockpit may import the VS Code API. Only the cockpit
// link imports the cockpit's client library, so the renames at the cockpit's switch touch
// one file. Compiled tests live in out/__tests__, so the sources are two levels up.
const srcDir = path.join(__dirname, '..', '..', 'src');
const coreDir = path.join(srcDir, 'core');
const cockpitDir = path.join(srcDir, 'cockpit');

function sourceFiles(dir: string): string[] {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) {
            return sourceFiles(full);
        }
        return entry.name.endsWith('.ts') ? [full] : [];
    });
}

const vscodeImport = /(?:from\s+|require\(\s*|import\(\s*)['"]vscode['"]/;

test('no core module imports the vscode API', () => {
    const files = sourceFiles(coreDir);
    assert.ok(files.length > 0, 'expected core sources to scan');
    const offenders = files.filter(file => vscodeImport.test(fs.readFileSync(file, 'utf8')));
    assert.deepEqual(offenders.map(file => path.relative(coreDir, file)), []);
});

test('no cockpit link module imports the vscode API', () => {
    const offenders = sourceFiles(cockpitDir).filter(file => vscodeImport.test(fs.readFileSync(file, 'utf8')));
    assert.deepEqual(offenders.map(file => path.relative(cockpitDir, file)), []);
});

test('only the cockpit link imports the cockpit\'s client library', () => {
    const libraryImport = /(?:from\s+|require\(\s*|import\(\s*)['"]@ev3-cockpit\/client/;
    const importers = sourceFiles(srcDir)
        .filter(file => !file.includes(`${path.sep}__tests__${path.sep}`))
        .filter(file => libraryImport.test(fs.readFileSync(file, 'utf8')));
    assert.deepEqual(importers.map(file => path.relative(srcDir, file).split(path.sep).join('/')), ['cockpit/nativeCockpit.ts']);
});