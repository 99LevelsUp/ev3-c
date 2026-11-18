import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { checkElf, ElfCheckResult } from '../core/toolchain/elfCheck';

// Compiled tests run from out/__tests__, and tsc does not copy the binary
// fixtures, so read them from the source tree. How each fixture was built is
// recorded in fixtures/elf/README.md.
const fixtures = path.join(__dirname, '..', '..', 'src', '__tests__', 'fixtures', 'elf');

function fixture(name: string): string {
    return path.join(fixtures, name);
}

function problemsOf(result: ElfCheckResult): string[] {
    assert.equal(result.ok, false, 'expected the ELF check to reject the file');
    return result.ok ? [] : result.problems;
}

function writeTemp(name: string, bytes: Uint8Array): string {
    const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-elf-')), name);
    fs.writeFileSync(file, bytes);
    return file;
}

test('accepts a static ARMv5TE soft-float executable', async () => {
    assert.deepEqual(await checkElf(fixture('static-armv5te.elf')), { ok: true });
});

test('rejects an x86 executable as not ARM', async () => {
    const problems = problemsOf(await checkElf(fixture('x86-64.elf')));
    assert.ok(problems.some(p => /not an ARM/.test(p)), problems.join('; '));
});

test('rejects an ARMv7 executable for its architecture', async () => {
    const problems = problemsOf(await checkElf(fixture('armv7.elf')));
    assert.ok(problems.some(p => /architecture ARMv7/.test(p)), problems.join('; '));
    assert.ok(!problems.some(p => /hard-float|dynamic/.test(p)), problems.join('; '));
});

test('rejects an executable that uses VFP hardware floating point', async () => {
    const problems = problemsOf(await checkElf(fixture('hard-float.elf')));
    assert.ok(problems.some(p => /hard-float/.test(p)), problems.join('; '));
});

test('rejects a dynamically linked ARM executable', async () => {
    const problems = problemsOf(await checkElf(fixture('dynamic.elf')));
    assert.ok(problems.some(p => /dynamically linked/.test(p)), problems.join('; '));
    assert.ok(problems.some(p => /needs shared library/.test(p)), problems.join('; '));
});

test('rejects a truncated ELF file without throwing', async () => {
    const whole = fs.readFileSync(fixture('static-armv5te.elf'));
    for (const length of [0, 10, 52, 200]) {
        const problems = problemsOf(await checkElf(writeTemp(`cut-${length}.elf`, whole.subarray(0, length))));
        assert.ok(problems.some(p => /truncated|not an ELF/.test(p)), `${length}: ${problems.join('; ')}`);
    }
});

test('rejects a file that is not ELF without throwing', async () => {
    const problems = problemsOf(await checkElf(writeTemp('hello.c', Buffer.from('int main(void) { return 0; }\n'))));
    assert.ok(problems.some(p => /not an ELF/.test(p)), problems.join('; '));
});
