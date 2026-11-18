import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';

// The core must run in plain Node (a future CLI reuses it), so nothing under
// src/core may import the VS Code API. Compiled tests live in out/__tests__,
// so the sources are two levels up.
const coreDir = path.join(__dirname, '..', '..', 'src', 'core');

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
