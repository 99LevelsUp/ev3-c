import * as fs from 'node:fs/promises';

// Checks that a produced executable can run on the EV3 brick: a 32-bit
// little-endian ARM ELF for an ARMv5TE-class CPU (the brick's ARM926EJ-S),
// soft-float, and statically linked. readelf is not available on Windows, and
// a toolchain that silently emitted ARMv7 or hard-float code would only fail
// on the brick, so the build reads the file itself.

export type ElfCheckResult = { ok: true } | { ok: false; problems: string[] };

const EM_ARM = 40;
const PT_DYNAMIC = 2;
const PT_INTERP = 3;
const DT_NULL = 0;
const DT_NEEDED = 1;
const DT_STRTAB = 5;
const SHT_ARM_ATTRIBUTES = 0x70000003;

// Values of Tag_CPU_arch from the ARM EABI build attributes. The brick's
// ARM926EJ-S implements ARMv5TEJ, so anything up to and including that runs.
const CPU_ARCH_NAMES = [
    'pre-ARMv4', 'ARMv4', 'ARMv4T', 'ARMv5T', 'ARMv5TE', 'ARMv5TEJ', 'ARMv6', 'ARMv6KZ', 'ARMv6T2',
    'ARMv6K', 'ARMv7', 'ARMv6-M', 'ARMv6S-M', 'ARMv7E-M', 'ARMv8-A', 'ARMv8-R', 'ARMv8-M.baseline',
    'ARMv8-M.mainline', 'ARMv8.1-A', 'ARMv8.2-A', 'ARMv8.3-A', 'ARMv8.1-M.mainline', 'ARMv9-A',
];
const MAX_CPU_ARCH = 5;

const TAG_FILE = 1;
const TAG_CPU_ARCH = 6;
const TAG_FP_ARCH = 10;
const TAG_ABI_VFP_ARGS = 28;

class Truncated extends Error {}

class Reader {
    constructor(private readonly bytes: Buffer) {}

    private need(offset: number, size: number): void {
        if (offset < 0 || offset + size > this.bytes.length) {
            throw new Truncated();
        }
    }

    u8(offset: number): number {
        this.need(offset, 1);
        return this.bytes.readUInt8(offset);
    }

    u16(offset: number): number {
        this.need(offset, 2);
        return this.bytes.readUInt16LE(offset);
    }

    u32(offset: number): number {
        this.need(offset, 4);
        return this.bytes.readUInt32LE(offset);
    }

    cString(offset: number): string {
        this.need(offset, 1);
        const end = this.bytes.indexOf(0, offset);
        if (end < 0) {
            throw new Truncated();
        }
        return this.bytes.toString('latin1', offset, end);
    }

    // ULEB128 as used by build attributes; returns the value and the next offset.
    uleb(offset: number): [number, number] {
        let value = 0;
        let shift = 0;
        for (;;) {
            const byte = this.u8(offset++);
            value += (byte & 0x7f) * 2 ** shift;
            if ((byte & 0x80) === 0) {
                return [value, offset];
            }
            shift += 7;
        }
    }
}

export async function checkElf(filePath: string): Promise<ElfCheckResult> {
    const bytes = await fs.readFile(filePath);
    try {
        const problems = inspect(new Reader(bytes), bytes);
        return problems.length === 0 ? { ok: true } : { ok: false, problems };
    } catch (error) {
        if (error instanceof Truncated) {
            return { ok: false, problems: ['file is truncated or its ELF tables point outside the file'] };
        }
        throw error;
    }
}

function inspect(r: Reader, bytes: Buffer): string[] {
    if (bytes.length < 4 || bytes.readUInt32BE(0) !== 0x7f454c46) {
        return ['not an ELF file'];
    }
    if (r.u8(4) !== 1 || r.u8(5) !== 1) {
        return ['not a 32-bit little-endian ELF file, so not an ARM executable for the brick'];
    }
    const machine = r.u16(18);
    if (machine !== EM_ARM) {
        return [`not an ARM executable (ELF machine ${machine})`];
    }

    const problems: string[] = [];
    problems.push(...checkLinking(r));
    problems.push(...checkAttributes(r));
    return problems;
}

function checkLinking(r: Reader): string[] {
    const problems: string[] = [];
    const phoff = r.u32(28);
    const phentsize = r.u16(42);
    const phnum = r.u16(44);
    let dynamic: { offset: number; size: number } | undefined;
    for (let i = 0; i < phnum; i++) {
        const ph = phoff + i * phentsize;
        const type = r.u32(ph);
        if (type === PT_INTERP) {
            const interp = r.cString(r.u32(ph + 4));
            problems.push(`dynamically linked: requests the program interpreter ${interp}`);
        } else if (type === PT_DYNAMIC) {
            dynamic = { offset: r.u32(ph + 4), size: r.u32(ph + 16) };
        }
    }
    if (dynamic !== undefined) {
        for (const name of neededLibraries(r, dynamic.offset, dynamic.size)) {
            problems.push(`dynamically linked: needs shared library ${name}`);
        }
    }
    return problems;
}

function neededLibraries(r: Reader, offset: number, size: number): string[] {
    const needed: number[] = [];
    let strtabAddress: number | undefined;
    for (let entry = offset; entry + 8 <= offset + size; entry += 8) {
        const tag = r.u32(entry);
        if (tag === DT_NULL) {
            break;
        }
        if (tag === DT_NEEDED) {
            needed.push(r.u32(entry + 4));
        } else if (tag === DT_STRTAB) {
            strtabAddress = r.u32(entry + 4);
        }
    }
    const strtab = strtabAddress === undefined ? undefined : fileOffsetOf(r, strtabAddress);
    return needed.map(nameOffset => strtab === undefined ? `#${nameOffset}` : r.cString(strtab + nameOffset));
}

// DT_STRTAB holds a virtual address; map it back to a file offset through the
// loadable segments.
function fileOffsetOf(r: Reader, address: number): number | undefined {
    const phoff = r.u32(28);
    const phentsize = r.u16(42);
    const phnum = r.u16(44);
    for (let i = 0; i < phnum; i++) {
        const ph = phoff + i * phentsize;
        const vaddr = r.u32(ph + 8);
        if (r.u32(ph) === 1 && address >= vaddr && address < vaddr + r.u32(ph + 16)) {
            return r.u32(ph + 4) + (address - vaddr);
        }
    }
    return undefined;
}

function checkAttributes(r: Reader): string[] {
    const section = findAttributesSection(r);
    if (section === undefined) {
        return ['has no .ARM.attributes section, so its CPU architecture and float ABI cannot be confirmed'];
    }
    const attributes = readFileAttributes(r, section.offset, section.size);
    const problems: string[] = [];

    const arch = attributes.get(TAG_CPU_ARCH);
    if (arch === undefined) {
        problems.push('does not record its CPU architecture (Tag_CPU_arch)');
    } else if (arch > MAX_CPU_ARCH) {
        problems.push(`built for architecture ${CPU_ARCH_NAMES[arch] ?? `#${arch}`}, but the brick's CPU is ARMv5TE`);
    }
    if ((attributes.get(TAG_ABI_VFP_ARGS) ?? 0) === 1) {
        problems.push('hard-float: passes floating-point arguments in VFP registers');
    }
    const fpArch = attributes.get(TAG_FP_ARCH) ?? 0;
    if (fpArch !== 0) {
        problems.push(`hard-float: uses VFP floating-point instructions (Tag_FP_arch ${fpArch}), but the brick has no FPU`);
    }
    return problems;
}

function findAttributesSection(r: Reader): { offset: number; size: number } | undefined {
    const shoff = r.u32(32);
    const shentsize = r.u16(46);
    const shnum = r.u16(48);
    for (let i = 0; i < shnum; i++) {
        const sh = shoff + i * shentsize;
        if (r.u32(sh + 4) === SHT_ARM_ATTRIBUTES) {
            return { offset: r.u32(sh + 16), size: r.u32(sh + 20) };
        }
    }
    return undefined;
}

// Reads the file-scope attributes of the "aeabi" vendor subsection. Layout:
// 'A', then subsections of [u32 length][vendor name\0][tag u8][u32 size][attributes...].
function readFileAttributes(r: Reader, offset: number, size: number): Map<number, number> {
    const values = new Map<number, number>();
    const end = offset + size;
    if (r.u8(offset) !== 0x41) {
        return values;
    }
    let sub = offset + 1;
    while (sub + 4 <= end) {
        const subLength = r.u32(sub);
        if (subLength < 4) {
            throw new Truncated();
        }
        const subEnd = sub + subLength;
        const vendor = r.cString(sub + 4);
        if (vendor === 'aeabi') {
            let block = sub + 4 + vendor.length + 1;
            while (block + 5 <= subEnd) {
                const blockTag = r.u8(block);
                const blockSize = r.u32(block + 1);
                if (blockSize < 5) {
                    throw new Truncated();
                }
                if (blockTag === TAG_FILE) {
                    readAttributeList(r, block + 5, block + blockSize, values);
                }
                block += blockSize;
            }
        }
        sub = subEnd;
    }
    return values;
}

function readAttributeList(r: Reader, offset: number, end: number, values: Map<number, number>): void {
    let at = offset;
    while (at < end) {
        const [tag, next] = r.uleb(at);
        at = next;
        if (tag === 4 || tag === 5 || tag === 67) {
            // CPU_raw_name, CPU_name, conformance: strings.
            at += r.cString(at).length + 1;
        } else if (tag === 32) {
            // Tag_compatibility: a number, then a string.
            at = r.uleb(at)[1];
            at += r.cString(at).length + 1;
        } else if (tag === 65) {
            // Tag_also_compatible_with: a string.
            at += r.cString(at).length + 1;
        } else if (tag > 32 && tag % 2 === 1) {
            // Unknown odd tags above 32 carry strings by convention.
            at += r.cString(at).length + 1;
        } else {
            const [value, after] = r.uleb(at);
            values.set(tag, value);
            at = after;
        }
    }
}
