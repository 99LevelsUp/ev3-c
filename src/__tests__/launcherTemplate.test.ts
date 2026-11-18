import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { AssembledImage, encodeConstant, encodeLocal, Instruction, op, Operand, sub } from '../core/launcher/bytecode';
import { assembleLauncher, buildLauncher, LauncherOptions, SUPERVISOR_FILE, supervisorArguments } from '../core/launcher/launcherTemplate';

// Compiled tests run from out/__tests__, and tsc does not copy fixtures, so read them from
// the source tree. launcher.rbf.hex is assembled from launcher.lms by
// scripts/regen-launcher-golden.cjs with LEGO's lmsasm.
const fixtures = path.join(__dirname, '..', '..', 'src', '__tests__', 'fixtures', 'launcher');

const goldenOptions: LauncherOptions = { retention: { kind: 'runs', count: 5 }, outputCapBytes: 65536 };

// Limits from the firmware's bytecodes.h; the checks below mirror cMemoryCheckFilename in
// c_memory.c, which splits a path at its last '/' and its name at the first '.'.
const vmPATHSIZE = 84;
const vmNAMESIZE = 32;
const vmEXTSIZE = 5;
const vmFILENAMESIZE = 120;
const filenameCharset = /^[A-Za-z0-9 _.\-/~]*$/;

// Cockpit refuses brick paths of 60 bytes or more.
const cockpitPathLimit = 60;
const prjs = '/home/root/lms2012/prjs';
const longestName = 'abcdefghijklmno';

function firmwareFilenameProblems(file: string): string[] {
    const slash = file.lastIndexOf('/');
    const dir = file.slice(0, slash + 1);
    const rest = file.slice(slash + 1);
    const dot = rest.indexOf('.');
    const name = dot < 0 ? rest : rest.slice(0, dot);
    const ext = dot < 0 ? '' : rest.slice(dot);
    const problems: string[] = [];
    if (file.length >= vmFILENAMESIZE) { problems.push(`path ${file.length} >= ${vmFILENAMESIZE}`); }
    if (dir.length >= vmPATHSIZE) { problems.push(`folder ${dir.length} >= ${vmPATHSIZE}`); }
    if (name.length >= vmNAMESIZE) { problems.push(`name ${name.length} >= ${vmNAMESIZE}`); }
    if (ext.length >= vmEXTSIZE) { problems.push(`extension ${ext.length} >= ${vmEXTSIZE}`); }
    if (!filenameCharset.test(file)) { problems.push('character outside the firmware file-name set'); }
    return problems;
}

function goldenBytes(): number[] {
    const hex = fs.readFileSync(path.join(fixtures, 'launcher.rbf.hex'), 'utf8').trim().split(/\s+/);
    return hex.map(h => parseInt(h, 16));
}

function isLocal(operand: Operand | undefined, name: string): boolean {
    return typeof operand === 'object' && operand.kind === 'local' && operand.name === name;
}

function isString(operand: Operand | undefined): operand is { kind: 'string'; text: string } {
    return typeof operand === 'object' && operand.kind === 'string';
}

function localName(operand: Operand | undefined): string {
    assert.ok(typeof operand === 'object' && operand.kind === 'local', `expected a local, got ${JSON.stringify(operand)}`);
    return operand.name;
}

interface Evaluation {
    /** Final string value of each string local. */
    values: Map<string, string>;
    /** Every path handed to FILE or FILENAME, in program order. */
    paths: string[];
    /** The shell command handed to SYSTEM. */
    commands: string[];
    /** Destinations whose STRINGS ADD result would overflow the local. */
    overflows: string[];
}

/**
 * Follows the launcher's string building for a given folder, as the VM would. The PID read
 * out of .run is taken to be the longest PID the brick's kernel can hand out (5 digits).
 */
function evaluateStrings(image: AssembledImage, folder: string, pid = '32768'): Evaluation {
    const values = new Map<string, string>();
    const paths: string[] = [];
    const commands: string[] = [];
    const overflows: string[] = [];
    const text = (operand: Operand | undefined): string => {
        if (isString(operand)) {
            return operand.text;
        }
        const name = localName(operand);
        return values.get(name) ?? '';
    };
    const store = (name: string, value: string): void => {
        const size = image.locals.get(name)?.size ?? 0;
        if (value.length + 1 > size) {
            overflows.push(`${name}: ${value.length + 1} > ${size}`);
        }
        values.set(name, value);
    };
    for (const { opcode, operands } of image.instructions) {
        const [cmd] = operands;
        if (opcode === op.FILENAME && cmd === sub.FILENAME.GET_FOLDERNAME) {
            const length = operands[1] as number;
            store(localName(operands[2]), folder.slice(0, length - 1));
        } else if (opcode === op.FILENAME && cmd === sub.FILENAME.EXIST) {
            paths.push(text(operands[1]));
        } else if (opcode === op.STRINGS && cmd === sub.STRINGS.ADD) {
            store(localName(operands[3]), text(operands[1]) + text(operands[2]));
        } else if (opcode === op.FILE && (cmd === sub.FILE.REMOVE || cmd === sub.FILE.OPEN_WRITE || cmd === sub.FILE.OPEN_READ)) {
            paths.push(text(operands[1]));
        } else if (opcode === op.FILE && cmd === sub.FILE.READ_TEXT) {
            const length = operands[3] as number;
            store(localName(operands[4]), pid.slice(0, length));
        } else if (opcode === op.SYSTEM) {
            commands.push(text(operands[0]));
        }
    }
    return { values, paths, commands, overflows };
}

function indexOfSequence(haystack: number[], needle: number[], from = 0): number {
    for (let i = from; i + needle.length <= haystack.length; i++) {
        if (needle.every((b, j) => haystack[i + j] === b)) {
            return i;
        }
    }
    return -1;
}

test('the launcher matches the bytes lmsasm assembles from launcher.lms', () => {
    assert.deepEqual([...buildLauncher(goldenOptions)], goldenBytes());
});

test('the image size field equals the file length and the code starts after the headers', () => {
    const bytes = buildLauncher(goldenOptions);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    assert.equal(view.getUint32(4, true), bytes.length);
    assert.equal(view.getUint16(10, true), 1, 'one thread');
    // 16-byte image header plus one 12-byte object header.
    assert.equal(view.getUint32(16, true), 28);
    assert.equal(bytes[28], op.FILENAME, 'first instruction builds the folder name');
    assert.equal(bytes[bytes.length - 1], op.OBJECT_END);
});

test('the supervisor command carries the project folder, the retention rule, and the output cap in that order', () => {
    const folder = `${prjs}/demo`;
    const runs = evaluateStrings(assembleLauncher(goldenOptions), folder);
    assert.deepEqual(runs.commands, [`${folder}/sup ${folder} keep=5 cap=65536 &`]);

    const megabytes = evaluateStrings(
        assembleLauncher({ retention: { kind: 'megabytes', limit: 2 }, outputCapBytes: 4096 }),
        folder,
    );
    assert.deepEqual(megabytes.commands, [`${folder}/sup ${folder} keepmb=2 cap=4096 &`]);
});

test('supervisor arguments refuse values the supervisor cannot read as a positive 32-bit number', () => {
    assert.equal(supervisorArguments(goldenOptions), 'keep=5 cap=65536');
    for (const bad of [0, -1, 1.5, Number.NaN, 0x80000000]) {
        assert.throws(() => supervisorArguments({ retention: { kind: 'runs', count: bad }, outputCapBytes: 65536 }));
        assert.throws(() => supervisorArguments({ retention: { kind: 'megabytes', limit: bad }, outputCapBytes: 65536 }));
        assert.throws(() => supervisorArguments({ retention: { kind: 'runs', count: 5 }, outputCapBytes: bad }));
    }
});

test('a 15-byte project name gives a launcher path under the cockpit limit', () => {
    assert.equal(Buffer.byteLength(longestName), 15);
    const launcherPath = `${prjs}/${longestName}/${longestName}.rbf`;
    assert.ok(launcherPath.length < cockpitPathLimit, `${launcherPath.length} bytes`);
    assert.deepEqual(firmwareFilenameProblems(launcherPath), []);
});

test('with a 15-byte project name every path and the command fit their buffers and the firmware rules', () => {
    // The VM reports the folder in the form the program was started with: absolute from
    // cockpit, relative to the VM's working directory from the brick menu.
    const widest = { retention: { kind: 'megabytes', limit: 0x7fffffff }, outputCapBytes: 0x7fffffff } as const;
    for (const folder of [`${prjs}/${longestName}`, `../prjs/${longestName}`]) {
        for (const options of [goldenOptions, widest]) {
            const result = evaluateStrings(assembleLauncher(options), folder);
            assert.equal(result.values.get('Folder'), folder, 'the folder name is not truncated');
            assert.deepEqual(result.overflows, [], folder);
            assert.equal(result.paths.length > 0, true);
            for (const file of result.paths) {
                assert.deepEqual(firmwareFilenameProblems(file), [], file);
            }
            assert.equal(result.commands.length, 1);
            assert.deepEqual(firmwareFilenameProblems(`${folder}/${SUPERVISOR_FILE}`), []);
        }
    }
    // The supervisor is uploaded through cockpit next to the launcher.
    const supervisorPath = `${prjs}/${longestName}/${SUPERVISOR_FILE}`;
    assert.ok(supervisorPath.length < cockpitPathLimit, `${supervisorPath.length} bytes`);
});

test('no buffer overflows even for the longest folder the firmware can run a program from', () => {
    // A program's folder plus its trailing '/' is shorter than vmPATHSIZE.
    const folder = '/' + 'x'.repeat(vmPATHSIZE - 3);
    const result = evaluateStrings(
        assembleLauncher({ retention: { kind: 'megabytes', limit: 0x7fffffff }, outputCapBytes: 0x7fffffff }),
        folder,
    );
    assert.equal(result.values.get('Folder'), folder);
    assert.deepEqual(result.overflows, []);
});

test('every marker path is built from the folder name, never a bare dot-name', () => {
    const image = assembleLauncher(goldenOptions);
    const markers: string[] = [];
    for (const { opcode, operands } of image.instructions) {
        if (opcode === op.FILE || opcode === op.FILENAME) {
            assert.ok(!operands.some(isString), `file instruction with a literal path: ${JSON.stringify(operands)}`);
        }
        operands.filter(isString).filter(s => s.text.includes('/.') || s.text.startsWith('.')).forEach(s => {
            assert.equal(opcode, op.STRINGS);
            assert.equal(operands[0], sub.STRINGS.ADD);
            assert.ok(isLocal(operands[1], 'Folder'), `${s.text} is appended to the folder`);
            assert.equal(operands[2], s);
            markers.push(s.text);
        });
    }
    assert.deepEqual(markers.sort(), ['/.bak', '/.run', '/.tok']);

    const folder = `${prjs}/demo`;
    const { paths } = evaluateStrings(image, folder);
    for (const marker of ['.run', '.bak', '.tok']) {
        assert.ok(paths.includes(`${folder}/${marker}`), `${marker} is used as ${folder}/${marker}`);
    }
});

test('the slot token is created before the supervisor starts and never closed', () => {
    const { instructions } = assembleLauncher(goldenOptions);
    const openToken = instructions.findIndex(i => i.opcode === op.FILE && i.operands[0] === sub.FILE.OPEN_WRITE);
    const start = instructions.findIndex(i => i.opcode === op.SYSTEM);
    assert.ok(openToken >= 0 && openToken < start, 'token opened before the supervisor starts');
    assert.ok(isLocal(instructions[openToken].operands[1], 'TokPath'));
    const closes = instructions.filter(i => i.opcode === op.FILE && i.operands[0] === sub.FILE.CLOSE);
    assert.equal(closes.length, 1, 'only the single PID read is closed');
    assert.ok(isLocal(closes[0].operands[1], 'hRun'));
    assert.equal(instructions.filter(i => i.opcode === op.SYSTEM).length, 1, 'the supervisor starts once');
});

test('the launcher waits at most 6 s for the run marker, in 100 ms ticks', () => {
    const { instructions } = assembleLauncher(goldenOptions);
    const ticks = instructions.filter(i => i.opcode === op.TIMER_WAIT);
    assert.ok(ticks.length > 0 && ticks.every(i => i.operands[0] === 100));
    const limit = instructions.find(i => i.opcode === op.JR_LT8);
    assert.ok(limit && isLocal(limit.operands[0], 'Count'));
    assert.equal((limit.operands[1] as number) * 100, 6000);
});

test('the poll loop ends when the run marker or /proc/<pid> of the PID read from it is gone', () => {
    const image = assembleLauncher(goldenOptions);
    const { instructions, locals, labels, code } = image;
    const slot = (name: string): number => {
        const found = locals.get(name);
        assert.ok(found, name);
        return found.offset;
    };

    // The PID comes out of .run, and /proc/<pid> is built from it.
    const values = evaluateStrings(image, `${prjs}/demo`, '1234').values;
    assert.equal(values.get('Proc'), '/proc/1234');
    const read = instructions.find(i => i.opcode === op.FILE && i.operands[0] === sub.FILE.OPEN_READ) as Instruction;
    assert.ok(isLocal(read.operands[1], 'RunPath') && isLocal(read.operands[2], 'hRun'));

    // In the bytes, both checks after Poll jump to Done when the file is missing.
    const done = labels.get('Done');
    const poll = labels.get('Poll');
    assert.ok(done !== undefined && poll !== undefined);
    for (const target of ['RunPath', 'Proc']) {
        const check = [
            op.FILENAME, ...encodeConstant(sub.FILENAME.EXIST), ...encodeLocal(slot(target)), ...encodeLocal(slot('Flag')),
            op.JR_FALSE, ...encodeLocal(slot('Flag')), 0x82,
        ];
        const at = indexOfSequence(code, check, poll);
        assert.ok(at >= 0, `${target} check in the poll loop`);
        const offsetAt = at + check.length;
        const offset = (code[offsetAt] | (code[offsetAt + 1] << 8)) << 16 >> 16;
        assert.equal(offsetAt + 2 + offset, done, `${target} missing jumps to the end`);
    }
    assert.equal(code[done], op.OBJECT_END);
});
