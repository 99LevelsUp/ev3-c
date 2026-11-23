import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';

// What the `ev3 c` command runs must run in plain Node, so it may not import the VS Code
// API. Only the cockpit link imports the cockpit's client library, so a change of the
// library touches one file. Compiled tests live in out/__tests__, so the sources are two
// levels up.
const srcDir = path.join(__dirname, '..', '..', 'src');

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

// Everything the `ev3 c` command runs: the core, the cockpit link, what both frontends
// share, and the command itself.
for (const dir of ['core', 'cockpit', 'shared', 'cli']) {
    test(`no module under src/${dir} imports the vscode API`, () => {
        const files = sourceFiles(path.join(srcDir, dir));
        assert.ok(files.length > 0, `expected sources in src/${dir} to scan`);
        const offenders = files.filter(file => vscodeImport.test(fs.readFileSync(file, 'utf8')));
        assert.deepEqual(offenders.map(file => path.relative(srcDir, file)), []);
    });
}

test('only the cockpit link imports the cockpit\'s client library', () => {
    const libraryImport = /(?:from\s+|require\(\s*|import\(\s*)['"]@ev3-cockpit\/client/;
    const importers = sourceFiles(srcDir)
        .filter(file => !file.includes(`${path.sep}__tests__${path.sep}`))
        .filter(file => libraryImport.test(fs.readFileSync(file, 'utf8')));
    assert.deepEqual(importers.map(file => path.relative(srcDir, file).split(path.sep).join('/')), ['cockpit/cockpitLink.ts']);
});