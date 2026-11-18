import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import {
    encodeConstant,
    encodeLocal,
    encodeString,
    label,
    local,
    op,
    PRIMPAR,
    ThreadBuilder,
} from '../core/launcher/bytecode';

// Expected bytes are spelled out from the macros in the firmware's bytecodes.h:
//   LC0(v) = v & 0x3F               LC1(v) = 0x81, v
//   LC2(v) = 0x82, lo, hi           LC4(v) = 0x83, b0, b1, b2, b3
//   LV0(i) = 0x40 | i               LV1(i) = 0xC1, i
//   LV2(i) = 0xC2, lo, hi           LV4(i) = 0xC3, b0, b1, b2, b3
// The width chosen for each value follows LEGO's lmsasm, so generated launchers match
// the bytes it assembles.

test('the parameter bits match bytecodes.h', () => {
    assert.equal(PRIMPAR.LONG, 0x80);
    assert.equal(PRIMPAR.VARIABLE, 0x40);
    assert.equal(PRIMPAR.ONE_BYTE, 1);
    assert.equal(PRIMPAR.TWO_BYTES, 2);
    assert.equal(PRIMPAR.FOUR_BYTES, 3);
    assert.equal(PRIMPAR.STRING_OLD, 0);
});

test('small constants encode as LC0, including negative ones', () => {
    assert.deepEqual(encodeConstant(0), [0x00]);
    assert.deepEqual(encodeConstant(1), [0x01]);
    assert.deepEqual(encodeConstant(31), [0x1f]);
    assert.deepEqual(encodeConstant(-1), [0x3f]);
    assert.deepEqual(encodeConstant(-31), [0x21]);
});

test('one-byte constants encode as LC1', () => {
    assert.deepEqual(encodeConstant(32), [0x81, 0x20]);
    assert.deepEqual(encodeConstant(-32), [0x81, 0xe0]);
    assert.deepEqual(encodeConstant(100), [0x81, 0x64]);
    assert.deepEqual(encodeConstant(127), [0x81, 0x7f]);
});

test('two-byte constants encode as LC2, little-endian', () => {
    assert.deepEqual(encodeConstant(128), [0x82, 0x80, 0x00]);
    assert.deepEqual(encodeConstant(-128), [0x82, 0x80, 0xff]);
    assert.deepEqual(encodeConstant(-129), [0x82, 0x7f, 0xff]);
    assert.deepEqual(encodeConstant(32767), [0x82, 0xff, 0x7f]);
});

test('four-byte constants encode as LC4, little-endian', () => {
    assert.deepEqual(encodeConstant(32768), [0x83, 0x00, 0x80, 0x00, 0x00]);
    assert.deepEqual(encodeConstant(65536), [0x83, 0x00, 0x00, 0x01, 0x00]);
    assert.deepEqual(encodeConstant(-32768), [0x83, 0x00, 0x80, 0xff, 0xff]);
});

test('constants outside the 32-bit range or not integers are refused', () => {
    assert.throws(() => encodeConstant(0x80000000));
    assert.throws(() => encodeConstant(1.5));
});

test('strings encode as a long string parameter with a zero terminator', () => {
    // lmsasm writes PRIMPAR_LONG | PRIMPAR_STRING_OLD (0x80); the VM reads it exactly
    // like the newer LCS prefix (0x84).
    assert.deepEqual(encodeString('/.run'), [0x80, 0x2f, 0x2e, 0x72, 0x75, 0x6e, 0x00]);
    assert.deepEqual(encodeString(''), [0x80, 0x00]);
});

test('strings that would not survive the encoding are refused', () => {
    assert.throws(() => encodeString('a\0b'));
    assert.throws(() => encodeString('Ǝ'));
});

test('local variables encode as LV0, LV1, LV2, or LV4 by offset', () => {
    assert.deepEqual(encodeLocal(0), [0x40]);
    assert.deepEqual(encodeLocal(31), [0x5f]);
    assert.deepEqual(encodeLocal(32), [0xc1, 0x20]);
    assert.deepEqual(encodeLocal(127), [0xc1, 0x7f]);
    assert.deepEqual(encodeLocal(128), [0xc2, 0x80, 0x00]);
    assert.deepEqual(encodeLocal(32767), [0xc2, 0xff, 0x7f]);
    assert.deepEqual(encodeLocal(32768), [0xc3, 0x00, 0x80, 0x00, 0x00]);
});

test('locals are laid out in declaration order with natural alignment', () => {
    const thread = new ThreadBuilder();
    thread.data8('A');
    thread.data16('B');
    thread.data8('C');
    thread.data32('D');
    thread.dataString('E', 3);
    thread.data32('F');
    const image = thread.assemble();
    assert.deepEqual(
        Object.fromEntries([...image.locals].map(([name, slot]) => [name, slot.offset])),
        { A: 0, B: 2, C: 4, D: 8, E: 12, F: 16 },
    );
    assert.equal(image.localBytes, 20);
});

test('a forward jump resolves to a positive offset from the end of the jump', () => {
    const thread = new ThreadBuilder();
    thread.data8('Flag');
    thread.emit(op.JR_TRUE, local('Flag'), label('Skip'));
    thread.emit(op.MOVE8_8, 0, local('Flag'));
    thread.label('Skip');
    const code = thread.assemble().code;
    // JR_TRUE Flag LC2(3): the MOVE8_8 0 Flag in between is 3 bytes long.
    assert.deepEqual(code, [op.JR_TRUE, 0x40, 0x82, 0x03, 0x00, op.MOVE8_8, 0x00, 0x40, op.OBJECT_END]);
});

test('a backward jump resolves to a negative offset, always as LC2', () => {
    const thread = new ThreadBuilder();
    thread.label('Here');
    thread.emit(op.JR, label('Here'));
    // Jumping to its own start moves back over the whole 4-byte instruction.
    assert.deepEqual(thread.assemble().code, [op.JR, 0x82, 0xfc, 0xff, op.OBJECT_END]);
});

test('undefined, duplicate labels and unknown locals are refused', () => {
    const missing = new ThreadBuilder();
    missing.emit(op.JR, label('Nowhere'));
    assert.throws(() => missing.assemble(), /Nowhere/);

    const twice = new ThreadBuilder();
    twice.label('Again');
    assert.throws(() => twice.label('Again'), /Again/);

    const unknown = new ThreadBuilder();
    unknown.emit(op.MOVE8_8, 0, local('Missing'));
    assert.throws(() => unknown.assemble(), /Missing/);
});

test('the image has the LEGO header and one thread header before the code', () => {
    const thread = new ThreadBuilder();
    thread.data32('X');
    thread.emit(op.TIMER_WAIT, 100, local('X'));
    const image = thread.assemble();
    const bytes = image.bytes;
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    assert.equal(String.fromCharCode(...bytes.subarray(0, 4)), 'LEGO');
    assert.equal(view.getUint32(4, true), bytes.length, 'image size');
    assert.equal(view.getUint16(8, true), 109, 'bytecode version 1.09');
    assert.equal(view.getUint16(10, true), 1, 'one object');
    assert.equal(view.getUint32(12, true), 0, 'no globals');
    assert.equal(view.getUint32(16, true), 28, 'instructions start after both headers');
    assert.equal(view.getUint16(20, true), 0, 'owner object');
    assert.equal(view.getUint16(22, true), 0, 'trigger count');
    assert.equal(view.getUint32(24, true), 4, 'local bytes');
    assert.deepEqual([...bytes.subarray(28)], image.code);
});
