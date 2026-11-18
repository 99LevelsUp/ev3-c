// Encoder for LEGO EV3 bytecode images (.rbf), the format the brick's firmware VM runs.
//
// Every number here comes from the firmware's bytecodes.h (firmware 1.09). Where that header
// allows more than one encoding, the encoder picks the one LEGO's lmsasm assembler picks, so a
// generated image can be compared byte for byte with an assembled one.

/** Bytecode version written into the image header: BYTECODE_VERSION 1.09 times 100. */
export const BYTECODE_VERSION = 109;

/** Parameter encoding bits (PRIMPAR_* in bytecodes.h). */
export const PRIMPAR = {
    LONG: 0x80,
    VARIABLE: 0x40,
    STRING_OLD: 0,
    ONE_BYTE: 1,
    TWO_BYTES: 2,
    FOUR_BYTES: 3,
    VALUE: 0x3f,
} as const;

/** The opcodes the launcher uses. */
export const op = {
    OBJECT_END: 0x0a,
    ADD8: 0x10,
    MOVE8_8: 0x30,
    JR: 0x40,
    JR_FALSE: 0x41,
    JR_TRUE: 0x42,
    SYSTEM: 0x60,
    JR_LT8: 0x64,
    JR_EQ8: 0x6c,
    STRINGS: 0x7d,
    UI_BUTTON: 0x83,
    TIMER_WAIT: 0x85,
    TIMER_READY: 0x86,
    FILE: 0xc0,
    FILENAME: 0xc6,
} as const;

/** Sub-commands of the opcodes above that take one as their first parameter. */
export const sub = {
    FILE: { OPEN_READ: 1, OPEN_WRITE: 2, READ_TEXT: 5, CLOSE: 7, REMOVE: 30 },
    FILENAME: { EXIST: 16, GET_FOLDERNAME: 23 },
    STRINGS: { ADD: 2 },
    UI_BUTTON: { SET_BACK_BLOCK: 10 },
} as const;

/** File-name limits of the firmware, each including the terminating zero. */
export const FIRMWARE_LIMITS = {
    /** vmPATHSIZE: folder part of a path, including its trailing '/'. */
    PATH_SIZE: 84,
    /** vmNAMESIZE: name part, without folder and extension. */
    NAME_SIZE: 32,
    /** vmEXTSIZE: extension, including its dot. */
    EXT_SIZE: 5,
    /** vmFILENAMESIZE: whole path. */
    FILENAME_SIZE: 120,
} as const;

const IMAGE_HEADER_BYTES = 16;
const OBJECT_HEADER_BYTES = 12;

function littleEndian(value: number, bytes: number): number[] {
    const out: number[] = [];
    for (let i = 0; i < bytes; i++) {
        out.push((value >>> (8 * i)) & 0xff);
    }
    return out;
}

function checkInt32(value: number, what: string): void {
    if (!Number.isInteger(value) || value < -0x80000000 || value > 0x7fffffff) {
        throw new RangeError(`${what} ${value} is not a 32-bit integer`);
    }
}

/**
 * A constant parameter in the shortest form lmsasm uses: LC0 for |v| <= 31, LC1 for
 * |v| <= 127, LC2 for |v| <= 32767, LC4 otherwise. lmsasm keeps the ranges symmetric, so -32
 * takes LC1 and -128 takes LC2 even though the VM would read the shorter forms.
 */
export function encodeConstant(value: number): number[] {
    checkInt32(value, 'constant');
    const magnitude = Math.abs(value);
    if (magnitude <= 31) {
        return [value & PRIMPAR.VALUE];
    }
    if (magnitude <= 127) {
        return [PRIMPAR.LONG | PRIMPAR.ONE_BYTE, value & 0xff];
    }
    if (magnitude <= 32767) {
        return [PRIMPAR.LONG | PRIMPAR.TWO_BYTES, ...littleEndian(value, 2)];
    }
    return [PRIMPAR.LONG | PRIMPAR.FOUR_BYTES, ...littleEndian(value, 4)];
}

/**
 * A zero-terminated string constant. lmsasm prefixes it with PRIMPAR_LONG | PRIMPAR_STRING_OLD
 * (0x80) rather than bytecodes.h's LCS (0x84); the VM reads both the same way.
 */
export function encodeString(text: string): number[] {
    const out: number[] = [PRIMPAR.LONG | PRIMPAR.STRING_OLD];
    for (const ch of text) {
        const code = ch.charCodeAt(0);
        if (code === 0 || code > 0x7f) {
            throw new RangeError(`string constant ${JSON.stringify(text)} holds a byte the VM cannot carry`);
        }
        out.push(code);
    }
    out.push(0);
    return out;
}

/** A local variable parameter (LV0, LV1, LV2, LV4) at a byte offset, in lmsasm's widths. */
export function encodeLocal(offset: number): number[] {
    checkInt32(offset, 'local offset');
    if (offset < 0) {
        throw new RangeError(`local offset ${offset} is negative`);
    }
    if (offset <= 31) {
        return [PRIMPAR.VARIABLE | offset];
    }
    if (offset <= 127) {
        return [PRIMPAR.LONG | PRIMPAR.VARIABLE | PRIMPAR.ONE_BYTE, offset];
    }
    if (offset <= 32767) {
        return [PRIMPAR.LONG | PRIMPAR.VARIABLE | PRIMPAR.TWO_BYTES, ...littleEndian(offset, 2)];
    }
    return [PRIMPAR.LONG | PRIMPAR.VARIABLE | PRIMPAR.FOUR_BYTES, ...littleEndian(offset, 4)];
}

/**
 * An instruction parameter: a number is a constant, the others name a string constant, a local
 * variable, or a jump target.
 */
export type Operand =
    | number
    | { readonly kind: 'string'; readonly text: string }
    | { readonly kind: 'local'; readonly name: string }
    | { readonly kind: 'label'; readonly name: string };

export function str(text: string): Operand {
    return { kind: 'string', text };
}

export function local(name: string): Operand {
    return { kind: 'local', name };
}

export function label(name: string): Operand {
    return { kind: 'label', name };
}

export interface Instruction {
    readonly opcode: number;
    readonly operands: readonly Operand[];
}

export interface LocalSlot {
    readonly offset: number;
    readonly size: number;
}

export interface AssembledImage {
    /** The whole .rbf file. */
    readonly bytes: Uint8Array;
    /** The thread's instructions, ending with OBJECT_END. */
    readonly code: number[];
    /** The instructions as emitted, before encoding. */
    readonly instructions: readonly Instruction[];
    /** Each local variable's place in the thread's local memory. */
    readonly locals: ReadonlyMap<string, LocalSlot>;
    /** Each label's offset into `code`. */
    readonly labels: ReadonlyMap<string, number>;
    readonly localBytes: number;
}

/**
 * Builds a program with one main thread (a `vmthread` in lmsasm terms): declare locals, emit
 * instructions and labels, then assemble the image.
 */
export class ThreadBuilder {
    private readonly locals = new Map<string, LocalSlot>();
    private readonly instructions: (Instruction | { readonly label: string })[] = [];
    private readonly labelNames = new Set<string>();
    private localBytes = 0;

    data8(name: string): void {
        this.declare(name, 1, 1);
    }

    data16(name: string): void {
        this.declare(name, 2, 2);
    }

    data32(name: string): void {
        this.declare(name, 4, 4);
    }

    /** A string buffer of `size` bytes, including its terminating zero. */
    dataString(name: string, size: number): void {
        this.declare(name, size, 1);
    }

    emit(opcode: number, ...operands: Operand[]): void {
        this.instructions.push({ opcode, operands });
    }

    label(name: string): void {
        if (this.labelNames.has(name)) {
            throw new Error(`label ${name} is defined twice`);
        }
        this.labelNames.add(name);
        this.instructions.push({ label: name });
    }

    assemble(): AssembledImage {
        const code: number[] = [];
        const labels = new Map<string, number>();
        // Jump offsets are always LC2 (as in lmsasm), so their size is known before the
        // target is: emit a placeholder and patch it once every label is placed.
        const fixups: { at: number; name: string }[] = [];
        const emitted: Instruction[] = [];

        for (const item of this.instructions) {
            if ('label' in item) {
                labels.set(item.label, code.length);
                continue;
            }
            emitted.push(item);
            code.push(item.opcode);
            for (const operand of item.operands) {
                if (typeof operand === 'number') {
                    code.push(...encodeConstant(operand));
                } else if (operand.kind === 'string') {
                    code.push(...encodeString(operand.text));
                } else if (operand.kind === 'local') {
                    const slot = this.locals.get(operand.name);
                    if (!slot) {
                        throw new Error(`local ${operand.name} is not declared`);
                    }
                    code.push(...encodeLocal(slot.offset));
                } else {
                    code.push(PRIMPAR.LONG | PRIMPAR.TWO_BYTES, 0, 0);
                    fixups.push({ at: code.length - 2, name: operand.name });
                }
            }
        }
        code.push(op.OBJECT_END);
        emitted.push({ opcode: op.OBJECT_END, operands: [] });

        for (const { at, name } of fixups) {
            const target = labels.get(name);
            if (target === undefined) {
                throw new Error(`label ${name} is not defined`);
            }
            // The VM adds the offset to its instruction pointer after reading the operand.
            const offset = target - (at + 2);
            if (offset < -32768 || offset > 32767) {
                throw new RangeError(`jump to ${name} is too far (${offset} bytes)`);
            }
            code[at] = offset & 0xff;
            code[at + 1] = (offset >> 8) & 0xff;
        }

        const codeStart = IMAGE_HEADER_BYTES + OBJECT_HEADER_BYTES;
        const size = codeStart + code.length;
        const header = [
            ...'LEGO'.split('').map(c => c.charCodeAt(0)),
            ...littleEndian(size, 4),
            ...littleEndian(BYTECODE_VERSION, 2),
            ...littleEndian(1, 2), // objects
            ...littleEndian(0, 4), // global bytes
            // Object header of the main thread: instruction offset, owner object, trigger
            // count, local bytes.
            ...littleEndian(codeStart, 4),
            ...littleEndian(0, 2),
            ...littleEndian(0, 2),
            ...littleEndian(this.localBytes, 4),
        ];

        return {
            bytes: Uint8Array.from([...header, ...code]),
            code,
            instructions: emitted,
            locals: new Map(this.locals),
            labels,
            localBytes: this.localBytes,
        };
    }

    private declare(name: string, size: number, alignment: number): void {
        if (this.locals.has(name)) {
            throw new Error(`local ${name} is declared twice`);
        }
        if (!Number.isInteger(size) || size <= 0) {
            throw new RangeError(`local ${name} needs a positive size`);
        }
        // The VM reads DATA16 and DATA32 locals directly, and the brick's ARM9 needs them
        // aligned; lmsasm aligns them the same way.
        const offset = Math.ceil(this.localBytes / alignment) * alignment;
        this.locals.set(name, { offset, size });
        this.localBytes = offset + size;
    }
}
