//! Checks that a produced executable can run on the EV3 brick: a 32-bit little-endian ARM
//! ELF for an ARMv5TE-class CPU (the brick's ARM926EJ-S), soft-float, and statically linked.
//! readelf is not available on Windows, and a toolchain that silently emitted ARMv7 or
//! hard-float code would only fail on the brick, so the build reads the file itself.

use std::collections::HashMap;
use std::io;
use std::path::Path;

const EM_ARM: u16 = 40;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const DT_NULL: u32 = 0;
const DT_NEEDED: u32 = 1;
const DT_STRTAB: u32 = 5;
const SHT_ARM_ATTRIBUTES: u32 = 0x7000_0003;

// Values of Tag_CPU_arch from the ARM EABI build attributes. The brick's ARM926EJ-S
// implements ARMv5TEJ, so anything up to and including that runs.
const CPU_ARCH_NAMES: [&str; 23] = [
    "pre-ARMv4",
    "ARMv4",
    "ARMv4T",
    "ARMv5T",
    "ARMv5TE",
    "ARMv5TEJ",
    "ARMv6",
    "ARMv6KZ",
    "ARMv6T2",
    "ARMv6K",
    "ARMv7",
    "ARMv6-M",
    "ARMv6S-M",
    "ARMv7E-M",
    "ARMv8-A",
    "ARMv8-R",
    "ARMv8-M.baseline",
    "ARMv8-M.mainline",
    "ARMv8.1-A",
    "ARMv8.2-A",
    "ARMv8.3-A",
    "ARMv8.1-M.mainline",
    "ARMv9-A",
];
const MAX_CPU_ARCH: u64 = 5;

const TAG_FILE: u8 = 1;
const TAG_CPU_ARCH: u64 = 6;
const TAG_FP_ARCH: u64 = 10;
const TAG_ABI_VFP_ARGS: u64 = 28;

const TRUNCATED: &str = "file is truncated or its ELF tables point outside the file";

/// A read outside the file: the file is cut short or a table offset is wrong.
struct Truncated;

type Read<T> = Result<T, Truncated>;

/// Little-endian reads at file offsets. Offsets are u64, so sums of 32-bit fields cannot
/// overflow, and every read outside the file is `Truncated`.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn slice(&self, offset: u64, size: u64) -> Read<&[u8]> {
        let end = offset.checked_add(size).ok_or(Truncated)?;
        if end > self.bytes.len() as u64 {
            return Err(Truncated);
        }
        Ok(&self.bytes[offset as usize..end as usize])
    }

    fn u8(&self, offset: u64) -> Read<u8> {
        Ok(self.slice(offset, 1)?[0])
    }

    fn u16(&self, offset: u64) -> Read<u16> {
        let b = self.slice(offset, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&self, offset: u64) -> Read<u32> {
        let b = self.slice(offset, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// The zero-terminated bytes at `offset`, without the terminator.
    fn c_string(&self, offset: u64) -> Read<&[u8]> {
        let rest = self
            .slice(offset, 1)
            .map(|_| &self.bytes[offset as usize..])?;
        let end = rest.iter().position(|&b| b == 0).ok_or(Truncated)?;
        Ok(&rest[..end])
    }

    /// Offset just past the zero-terminated string at `offset`.
    fn skip_c_string(&self, offset: u64) -> Read<u64> {
        Ok(offset + self.c_string(offset)?.len() as u64 + 1)
    }

    /// ULEB128 as used by build attributes; returns the value and the next offset. A value
    /// too wide for 64 bits saturates.
    fn uleb(&self, mut offset: u64) -> Read<(u64, u64)> {
        let mut value: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8(offset)?;
            offset += 1;
            let part = u64::from(byte & 0x7f);
            if part != 0 {
                value = match part.checked_shl(shift) {
                    Some(shifted) if shifted >> shift == part => value.saturating_add(shifted),
                    _ => u64::MAX,
                };
            }
            if byte & 0x80 == 0 {
                return Ok((value, offset));
            }
            shift = shift.saturating_add(7);
        }
    }
}

/// Reads `file_path` and returns why the brick cannot run it; empty when it can. `Err` is
/// only for a file that cannot be read.
pub fn check_elf(file_path: &Path) -> io::Result<Vec<String>> {
    Ok(elf_problems(&std::fs::read(file_path)?))
}

/// Why the brick cannot run the executable in `bytes`; empty when it can.
pub fn elf_problems(bytes: &[u8]) -> Vec<String> {
    inspect(&Reader { bytes }).unwrap_or_else(|Truncated| vec![TRUNCATED.to_owned()])
}

fn inspect(r: &Reader) -> Read<Vec<String>> {
    if !r.bytes.starts_with(b"\x7fELF") {
        return Ok(vec!["not an ELF file".to_owned()]);
    }
    if r.u8(4)? != 1 || r.u8(5)? != 1 {
        return Ok(vec![
            "not a 32-bit little-endian ELF file, so not an ARM executable for the brick"
                .to_owned(),
        ]);
    }
    let machine = r.u16(18)?;
    if machine != EM_ARM {
        return Ok(vec![format!(
            "not an ARM executable (ELF machine {machine})"
        )]);
    }

    let mut problems = check_linking(r)?;
    problems.extend(check_attributes(r)?);
    Ok(problems)
}

/// Offsets of the program headers.
fn program_headers(r: &Reader) -> Read<impl Iterator<Item = u64>> {
    let phoff = u64::from(r.u32(28)?);
    let phentsize = u64::from(r.u16(42)?);
    let phnum = u64::from(r.u16(44)?);
    Ok((0..phnum).map(move |i| phoff + i * phentsize))
}

fn check_linking(r: &Reader) -> Read<Vec<String>> {
    let mut problems = Vec::new();
    let mut dynamic = None;
    for ph in program_headers(r)? {
        let kind = r.u32(ph)?;
        if kind == PT_INTERP {
            let interp = latin1(r.c_string(u64::from(r.u32(ph + 4)?))?);
            problems.push(format!(
                "dynamically linked: requests the program interpreter {interp}"
            ));
        } else if kind == PT_DYNAMIC {
            dynamic = Some((u64::from(r.u32(ph + 4)?), u64::from(r.u32(ph + 16)?)));
        }
    }
    if let Some((offset, size)) = dynamic {
        for name in needed_libraries(r, offset, size)? {
            problems.push(format!("dynamically linked: needs shared library {name}"));
        }
    }
    Ok(problems)
}

fn needed_libraries(r: &Reader, offset: u64, size: u64) -> Read<Vec<String>> {
    let mut needed = Vec::new();
    let mut strtab_address = None;
    let mut entry = offset;
    while entry + 8 <= offset + size {
        let tag = r.u32(entry)?;
        if tag == DT_NULL {
            break;
        }
        if tag == DT_NEEDED {
            needed.push(u64::from(r.u32(entry + 4)?));
        } else if tag == DT_STRTAB {
            strtab_address = Some(u64::from(r.u32(entry + 4)?));
        }
        entry += 8;
    }
    let strtab = match strtab_address {
        Some(address) => file_offset_of(r, address)?,
        None => None,
    };
    needed
        .into_iter()
        .map(|name_offset| match strtab {
            Some(strtab) => Ok(latin1(r.c_string(strtab + name_offset)?)),
            None => Ok(format!("#{name_offset}")),
        })
        .collect()
}

// DT_STRTAB holds a virtual address; map it back to a file offset through the loadable
// segments.
fn file_offset_of(r: &Reader, address: u64) -> Read<Option<u64>> {
    for ph in program_headers(r)? {
        let vaddr = u64::from(r.u32(ph + 8)?);
        if r.u32(ph)? == PT_LOAD && address >= vaddr && address < vaddr + u64::from(r.u32(ph + 16)?)
        {
            return Ok(Some(u64::from(r.u32(ph + 4)?) + (address - vaddr)));
        }
    }
    Ok(None)
}

fn check_attributes(r: &Reader) -> Read<Vec<String>> {
    let Some((offset, size)) = find_attributes_section(r)? else {
        return Ok(vec![
            "has no .ARM.attributes section, so its CPU architecture and float ABI cannot be confirmed"
                .to_owned(),
        ]);
    };
    let attributes = read_file_attributes(r, offset, size)?;
    let mut problems = Vec::new();

    match attributes.get(&TAG_CPU_ARCH) {
        None => problems.push("does not record its CPU architecture (Tag_CPU_arch)".to_owned()),
        Some(&arch) if arch > MAX_CPU_ARCH => {
            let name = usize::try_from(arch)
                .ok()
                .and_then(|index| CPU_ARCH_NAMES.get(index))
                .map_or_else(|| format!("#{arch}"), |name| (*name).to_owned());
            problems.push(format!(
                "built for architecture {name}, but the brick's CPU is ARMv5TE"
            ));
        }
        Some(_) => {}
    }
    if attributes.get(&TAG_ABI_VFP_ARGS).copied().unwrap_or(0) == 1 {
        problems.push("hard-float: passes floating-point arguments in VFP registers".to_owned());
    }
    let fp_arch = attributes.get(&TAG_FP_ARCH).copied().unwrap_or(0);
    if fp_arch != 0 {
        problems.push(format!(
            "hard-float: uses VFP floating-point instructions (Tag_FP_arch {fp_arch}), but the brick has no FPU"
        ));
    }
    Ok(problems)
}

fn find_attributes_section(r: &Reader) -> Read<Option<(u64, u64)>> {
    let shoff = u64::from(r.u32(32)?);
    let shentsize = u64::from(r.u16(46)?);
    let shnum = u64::from(r.u16(48)?);
    for i in 0..shnum {
        let sh = shoff + i * shentsize;
        if r.u32(sh + 4)? == SHT_ARM_ATTRIBUTES {
            return Ok(Some((
                u64::from(r.u32(sh + 16)?),
                u64::from(r.u32(sh + 20)?),
            )));
        }
    }
    Ok(None)
}

// Reads the file-scope attributes of the "aeabi" vendor subsection. Layout:
// 'A', then subsections of [u32 length][vendor name\0][tag u8][u32 size][attributes...].
fn read_file_attributes(r: &Reader, offset: u64, size: u64) -> Read<HashMap<u64, u64>> {
    let mut values = HashMap::new();
    let end = offset + size;
    if r.u8(offset)? != b'A' {
        return Ok(values);
    }
    let mut sub = offset + 1;
    while sub + 4 <= end {
        let sub_length = u64::from(r.u32(sub)?);
        if sub_length < 4 {
            return Err(Truncated);
        }
        let sub_end = sub + sub_length;
        let vendor = r.c_string(sub + 4)?;
        if vendor == b"aeabi" {
            let mut block = sub + 4 + vendor.len() as u64 + 1;
            while block + 5 <= sub_end {
                let block_tag = r.u8(block)?;
                let block_size = u64::from(r.u32(block + 1)?);
                if block_size < 5 {
                    return Err(Truncated);
                }
                if block_tag == TAG_FILE {
                    read_attribute_list(r, block + 5, block + block_size, &mut values)?;
                }
                block += block_size;
            }
        }
        sub = sub_end;
    }
    Ok(values)
}

fn read_attribute_list(
    r: &Reader,
    offset: u64,
    end: u64,
    values: &mut HashMap<u64, u64>,
) -> Read<()> {
    let mut at = offset;
    while at < end {
        let (tag, next) = r.uleb(at)?;
        at = next;
        match tag {
            // CPU_raw_name, CPU_name, conformance: strings.
            4 | 5 | 67 => at = r.skip_c_string(at)?,
            // Tag_compatibility: a number, then a string.
            32 => at = r.skip_c_string(r.uleb(at)?.1)?,
            // Tag_also_compatible_with: a string.
            65 => at = r.skip_c_string(at)?,
            // Unknown odd tags above 32 carry strings by convention.
            _ if tag > 32 && tag % 2 == 1 => at = r.skip_c_string(at)?,
            _ => {
                let (value, after) = r.uleb(at)?;
                values.insert(tag, value);
                at = after;
            }
        }
    }
    Ok(())
}

/// Bytes as Latin-1 text, one character per byte.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::testutil::repo_path;

    // How each fixture was built is recorded in fixtures/elf/README.md.
    fn fixture(name: &str) -> PathBuf {
        repo_path("src/__tests__/fixtures/elf").join(name)
    }

    fn problems_of(name: &str) -> Vec<String> {
        let problems = check_elf(&fixture(name)).unwrap();
        assert!(
            !problems.is_empty(),
            "expected the ELF check to reject {name}"
        );
        problems
    }

    fn any(problems: &[String], needle: &str) -> bool {
        problems.iter().any(|p| p.contains(needle))
    }

    #[test]
    fn accepts_a_static_armv5te_soft_float_executable() {
        assert_eq!(
            check_elf(&fixture("static-armv5te.elf")).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn rejects_an_x86_executable_as_not_arm() {
        let problems = problems_of("x86-64.elf");
        assert!(any(&problems, "not an ARM"), "{problems:?}");
    }

    #[test]
    fn rejects_an_armv7_executable_for_its_architecture() {
        let problems = problems_of("armv7.elf");
        assert!(any(&problems, "architecture ARMv7"), "{problems:?}");
        assert!(
            !any(&problems, "hard-float") && !any(&problems, "dynamic"),
            "{problems:?}"
        );
    }

    #[test]
    fn rejects_an_executable_that_uses_vfp_hardware_floating_point() {
        let problems = problems_of("hard-float.elf");
        assert!(any(&problems, "hard-float"), "{problems:?}");
    }

    #[test]
    fn rejects_a_dynamically_linked_arm_executable() {
        let problems = problems_of("dynamic.elf");
        assert!(any(&problems, "dynamically linked"), "{problems:?}");
        assert!(any(&problems, "needs shared library"), "{problems:?}");
    }

    #[test]
    fn rejects_a_truncated_elf_file_without_panicking() {
        let whole = std::fs::read(fixture("static-armv5te.elf")).unwrap();
        for length in [0, 10, 52, 200] {
            let problems = elf_problems(&whole[..length]);
            assert!(
                any(&problems, "truncated") || any(&problems, "not an ELF"),
                "{length}: {problems:?}"
            );
        }
        // Every prefix is either accepted or refused, never a panic.
        for length in 0..whole.len() {
            elf_problems(&whole[..length]);
        }
    }

    #[test]
    fn rejects_a_file_that_is_not_elf_without_panicking() {
        let problems = elf_problems(b"int main(void) { return 0; }\n");
        assert!(any(&problems, "not an ELF"), "{problems:?}");
    }

    #[test]
    fn a_long_uleb_saturates_instead_of_overflowing() {
        let mut bytes = vec![0xff; 20];
        bytes.push(0x01);
        let r = Reader { bytes: &bytes };
        assert!(matches!(r.uleb(0), Ok((u64::MAX, 21))));
    }
}
