//! Encoder for LEGO EV3 bytecode images (.rbf), the format the brick's firmware VM runs.
//!
//! Every number here comes from the firmware's bytecodes.h (firmware 1.09). Where that header
//! allows more than one encoding, the encoder picks the one LEGO's lmsasm assembler picks, so a
//! generated image can be compared byte for byte with an assembled one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Bytecode version written into the image header: BYTECODE_VERSION 1.09 times 100.
pub const BYTECODE_VERSION: u16 = 109;

/// Parameter encoding bits (PRIMPAR_* in bytecodes.h).
pub mod primpar {
    pub const LONG: u8 = 0x80;
    pub const VARIABLE: u8 = 0x40;
    pub const STRING_OLD: u8 = 0;
    pub const ONE_BYTE: u8 = 1;
    pub const TWO_BYTES: u8 = 2;
    pub const FOUR_BYTES: u8 = 3;
    pub const VALUE: u8 = 0x3f;
}

/// The opcodes the launcher uses.
pub mod op {
    pub const OBJECT_END: u8 = 0x0a;
    pub const ADD8: u8 = 0x10;
    pub const MOVE8_8: u8 = 0x30;
    pub const JR: u8 = 0x40;
    pub const JR_FALSE: u8 = 0x41;
    pub const JR_TRUE: u8 = 0x42;
    pub const SYSTEM: u8 = 0x60;
    pub const JR_LT8: u8 = 0x64;
    pub const JR_EQ8: u8 = 0x6c;
    pub const STRINGS: u8 = 0x7d;
    pub const UI_BUTTON: u8 = 0x83;
    pub const UI_DRAW: u8 = 0x84;
    pub const TIMER_WAIT: u8 = 0x85;
    pub const TIMER_READY: u8 = 0x86;
    pub const FILE: u8 = 0xc0;
    pub const FILENAME: u8 = 0xc6;
}

/// Sub-commands of the opcodes above that take one as their first parameter. They are
/// passed as constant parameters, hence `i32`.
pub mod sub {
    pub mod file {
        pub const OPEN_READ: i32 = 1;
        pub const OPEN_WRITE: i32 = 2;
        pub const READ_TEXT: i32 = 5;
        pub const CLOSE: i32 = 7;
        pub const REMOVE: i32 = 30;
    }
    pub mod filename {
        pub const EXIST: i32 = 16;
        pub const GET_FOLDERNAME: i32 = 23;
    }
    pub mod strings {
        pub const ADD: i32 = 2;
    }
    pub mod ui_button {
        pub const SET_BACK_BLOCK: i32 = 10;
    }
    pub mod ui_draw {
        pub const UPDATE: i32 = 0;
    }
}

/// File-name limits of the firmware, each including the terminating zero.
pub mod firmware_limits {
    /// vmPATHSIZE: folder part of a path, including its trailing '/'.
    pub const PATH_SIZE: u32 = 84;
    /// vmNAMESIZE: name part, without folder and extension.
    pub const NAME_SIZE: u32 = 32;
    /// vmEXTSIZE: extension, including its dot.
    pub const EXT_SIZE: u32 = 5;
    /// vmFILENAMESIZE: whole path.
    pub const FILENAME_SIZE: u32 = 120;
}

const IMAGE_HEADER_BYTES: u32 = 16;
const OBJECT_HEADER_BYTES: u32 = 12;

/// Why a launcher or another bytecode program cannot be encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherError(pub(crate) String);

impl fmt::Display for LauncherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LauncherError {}

/// A constant parameter in the shortest form lmsasm uses: LC0 for |v| <= 31, LC1 for
/// |v| <= 127, LC2 for |v| <= 32767, LC4 otherwise. lmsasm keeps the ranges symmetric, so -32
/// takes LC1 and -128 takes LC2 even though the VM would read the shorter forms.
pub fn encode_constant(value: i32) -> Vec<u8> {
    let magnitude = value.unsigned_abs();
    let bytes = value.to_le_bytes();
    if magnitude <= 31 {
        vec![bytes[0] & primpar::VALUE]
    } else if magnitude <= 127 {
        vec![primpar::LONG | primpar::ONE_BYTE, bytes[0]]
    } else if magnitude <= 32767 {
        vec![primpar::LONG | primpar::TWO_BYTES, bytes[0], bytes[1]]
    } else {
        let mut out = vec![primpar::LONG | primpar::FOUR_BYTES];
        out.extend_from_slice(&bytes);
        out
    }
}

/// A zero-terminated string constant. lmsasm prefixes it with PRIMPAR_LONG | PRIMPAR_STRING_OLD
/// (0x80) rather than bytecodes.h's LCS (0x84); the VM reads both the same way.
pub fn encode_string(text: &str) -> Result<Vec<u8>, LauncherError> {
    let mut out = vec![primpar::LONG | primpar::STRING_OLD];
    for ch in text.chars() {
        if ch == '\0' || !ch.is_ascii() {
            return Err(LauncherError(format!(
                "string constant {} holds a byte the VM cannot carry",
                crate::json::quote(text)
            )));
        }
        out.push(ch as u8);
    }
    out.push(0);
    Ok(out)
}

/// A local variable parameter (LV0, LV1, LV2, LV4) at a byte offset, in lmsasm's widths.
pub fn encode_local(offset: u32) -> Result<Vec<u8>, LauncherError> {
    if offset > 0x7fff_ffff {
        return Err(LauncherError(format!(
            "local offset {offset} is not a 32-bit integer"
        )));
    }
    let bytes = offset.to_le_bytes();
    Ok(if offset <= 31 {
        vec![primpar::VARIABLE | bytes[0]]
    } else if offset <= 127 {
        vec![
            primpar::LONG | primpar::VARIABLE | primpar::ONE_BYTE,
            bytes[0],
        ]
    } else if offset <= 32767 {
        vec![
            primpar::LONG | primpar::VARIABLE | primpar::TWO_BYTES,
            bytes[0],
            bytes[1],
        ]
    } else {
        let mut out = vec![primpar::LONG | primpar::VARIABLE | primpar::FOUR_BYTES];
        out.extend_from_slice(&bytes);
        out
    })
}

/// An instruction parameter: a constant, a string constant, a local variable, or a jump target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Constant(i32),
    String(String),
    Local(String),
    Label(String),
}

impl From<i32> for Operand {
    fn from(value: i32) -> Self {
        Operand::Constant(value)
    }
}

/// A string constant parameter.
pub fn string(text: &str) -> Operand {
    Operand::String(text.to_owned())
}

/// A local variable parameter.
pub fn local(name: &str) -> Operand {
    Operand::Local(name.to_owned())
}

/// A jump target parameter.
pub fn label(name: &str) -> Operand {
    Operand::Label(name.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub opcode: u8,
    pub operands: Vec<Operand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalSlot {
    pub offset: u32,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledImage {
    /// The whole .rbf file.
    pub bytes: Vec<u8>,
    /// The thread's instructions, ending with OBJECT_END.
    pub code: Vec<u8>,
    /// The instructions as emitted, before encoding.
    pub instructions: Vec<Instruction>,
    /// Each local variable's place in the thread's local memory.
    pub locals: BTreeMap<String, LocalSlot>,
    /// Each label's offset into `code`.
    pub labels: BTreeMap<String, usize>,
    pub local_bytes: u32,
}

#[derive(Debug, Clone)]
enum Item {
    Instruction(Instruction),
    Label(String),
}

/// Builds a program with one main thread (a `vmthread` in lmsasm terms): declare locals, emit
/// instructions and labels, then assemble the image.
#[derive(Debug, Clone, Default)]
pub struct ThreadBuilder {
    locals: BTreeMap<String, LocalSlot>,
    items: Vec<Item>,
    label_names: BTreeSet<String>,
    local_bytes: u32,
}

impl ThreadBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn data8(&mut self, name: &str) -> Result<(), LauncherError> {
        self.declare(name, 1, 1)
    }

    pub fn data16(&mut self, name: &str) -> Result<(), LauncherError> {
        self.declare(name, 2, 2)
    }

    pub fn data32(&mut self, name: &str) -> Result<(), LauncherError> {
        self.declare(name, 4, 4)
    }

    /// A string buffer of `size` bytes, including its terminating zero.
    pub fn data_string(&mut self, name: &str, size: u32) -> Result<(), LauncherError> {
        self.declare(name, size, 1)
    }

    pub fn emit(&mut self, opcode: u8, operands: &[Operand]) {
        self.items.push(Item::Instruction(Instruction {
            opcode,
            operands: operands.to_vec(),
        }));
    }

    pub fn label(&mut self, name: &str) -> Result<(), LauncherError> {
        if !self.label_names.insert(name.to_owned()) {
            return Err(LauncherError(format!("label {name} is defined twice")));
        }
        self.items.push(Item::Label(name.to_owned()));
        Ok(())
    }

    pub fn assemble(&self) -> Result<AssembledImage, LauncherError> {
        let mut code: Vec<u8> = Vec::new();
        let mut labels = BTreeMap::new();
        // Jump offsets are always LC2 (as in lmsasm), so their size is known before the
        // target is: emit a placeholder and patch it once every label is placed.
        let mut fixups: Vec<(usize, &str)> = Vec::new();
        let mut emitted = Vec::new();

        for item in &self.items {
            let instruction = match item {
                Item::Label(name) => {
                    labels.insert(name.clone(), code.len());
                    continue;
                }
                Item::Instruction(instruction) => instruction,
            };
            emitted.push(instruction.clone());
            code.push(instruction.opcode);
            for operand in &instruction.operands {
                match operand {
                    Operand::Constant(value) => code.extend(encode_constant(*value)),
                    Operand::String(text) => code.extend(encode_string(text)?),
                    Operand::Local(name) => {
                        let slot = self.locals.get(name).ok_or_else(|| {
                            LauncherError(format!("local {name} is not declared"))
                        })?;
                        code.extend(encode_local(slot.offset)?);
                    }
                    Operand::Label(name) => {
                        code.extend([primpar::LONG | primpar::TWO_BYTES, 0, 0]);
                        fixups.push((code.len() - 2, name));
                    }
                }
            }
        }
        code.push(op::OBJECT_END);
        emitted.push(Instruction {
            opcode: op::OBJECT_END,
            operands: Vec::new(),
        });

        for (at, name) in fixups {
            let target = *labels
                .get(name)
                .ok_or_else(|| LauncherError(format!("label {name} is not defined")))?;
            // The VM adds the offset to its instruction pointer after reading the operand.
            let offset = target as i64 - (at as i64 + 2);
            let offset = i16::try_from(offset).map_err(|_| {
                LauncherError(format!("jump to {name} is too far ({offset} bytes)"))
            })?;
            code[at..at + 2].copy_from_slice(&offset.to_le_bytes());
        }

        let code_start = IMAGE_HEADER_BYTES + OBJECT_HEADER_BYTES;
        let size = u32::try_from(code.len())
            .ok()
            .and_then(|len| len.checked_add(code_start))
            .ok_or_else(|| {
                LauncherError(format!("program of {} bytes is too large", code.len()))
            })?;
        let mut bytes = Vec::with_capacity(code.len() + code_start as usize);
        bytes.extend_from_slice(b"LEGO");
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&BYTECODE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // objects
        bytes.extend_from_slice(&0u32.to_le_bytes()); // global bytes
        // Object header of the main thread: instruction offset, owner object, trigger count,
        // local bytes.
        bytes.extend_from_slice(&code_start.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&self.local_bytes.to_le_bytes());
        bytes.extend_from_slice(&code);

        Ok(AssembledImage {
            bytes,
            code,
            instructions: emitted,
            locals: self.locals.clone(),
            labels,
            local_bytes: self.local_bytes,
        })
    }

    fn declare(&mut self, name: &str, size: u32, alignment: u32) -> Result<(), LauncherError> {
        if self.locals.contains_key(name) {
            return Err(LauncherError(format!("local {name} is declared twice")));
        }
        if size == 0 {
            return Err(LauncherError(format!("local {name} needs a positive size")));
        }
        // The VM reads DATA16 and DATA32 locals directly, and the brick's ARM9 needs them
        // aligned; lmsasm aligns them the same way.
        let offset = self.local_bytes.div_ceil(alignment) * alignment;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| LauncherError(format!("local {name} does not fit in local memory")))?;
        self.locals
            .insert(name.to_owned(), LocalSlot { offset, size });
        self.local_bytes = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected bytes are spelled out from the macros in the firmware's bytecodes.h:
    //   LC0(v) = v & 0x3F               LC1(v) = 0x81, v
    //   LC2(v) = 0x82, lo, hi           LC4(v) = 0x83, b0, b1, b2, b3
    //   LV0(i) = 0x40 | i               LV1(i) = 0xC1, i
    //   LV2(i) = 0xC2, lo, hi           LV4(i) = 0xC3, b0, b1, b2, b3
    // The width chosen for each value follows LEGO's lmsasm, so generated launchers match
    // the bytes it assembles.

    #[test]
    fn the_parameter_bits_match_bytecodes_h() {
        assert_eq!(primpar::LONG, 0x80);
        assert_eq!(primpar::VARIABLE, 0x40);
        assert_eq!(primpar::ONE_BYTE, 1);
        assert_eq!(primpar::TWO_BYTES, 2);
        assert_eq!(primpar::FOUR_BYTES, 3);
        assert_eq!(primpar::STRING_OLD, 0);
    }

    #[test]
    fn small_constants_encode_as_lc0_including_negative_ones() {
        assert_eq!(encode_constant(0), [0x00]);
        assert_eq!(encode_constant(1), [0x01]);
        assert_eq!(encode_constant(31), [0x1f]);
        assert_eq!(encode_constant(-1), [0x3f]);
        assert_eq!(encode_constant(-31), [0x21]);
    }

    #[test]
    fn one_byte_constants_encode_as_lc1() {
        assert_eq!(encode_constant(32), [0x81, 0x20]);
        assert_eq!(encode_constant(-32), [0x81, 0xe0]);
        assert_eq!(encode_constant(100), [0x81, 0x64]);
        assert_eq!(encode_constant(127), [0x81, 0x7f]);
    }

    #[test]
    fn two_byte_constants_encode_as_lc2_little_endian() {
        assert_eq!(encode_constant(128), [0x82, 0x80, 0x00]);
        assert_eq!(encode_constant(-128), [0x82, 0x80, 0xff]);
        assert_eq!(encode_constant(-129), [0x82, 0x7f, 0xff]);
        assert_eq!(encode_constant(32767), [0x82, 0xff, 0x7f]);
    }

    #[test]
    fn four_byte_constants_encode_as_lc4_little_endian() {
        assert_eq!(encode_constant(32768), [0x83, 0x00, 0x80, 0x00, 0x00]);
        assert_eq!(encode_constant(65536), [0x83, 0x00, 0x00, 0x01, 0x00]);
        assert_eq!(encode_constant(-32768), [0x83, 0x00, 0x80, 0xff, 0xff]);
        // The extremes of the 32-bit range; anything wider does not type-check as i32.
        assert_eq!(encode_constant(i32::MAX), [0x83, 0xff, 0xff, 0xff, 0x7f]);
        assert_eq!(encode_constant(i32::MIN), [0x83, 0x00, 0x00, 0x00, 0x80]);
    }

    #[test]
    fn strings_encode_as_a_long_string_parameter_with_a_zero_terminator() {
        // lmsasm writes PRIMPAR_LONG | PRIMPAR_STRING_OLD (0x80); the VM reads it exactly
        // like the newer LCS prefix (0x84).
        assert_eq!(
            encode_string("/.run"),
            Ok(vec![0x80, 0x2f, 0x2e, 0x72, 0x75, 0x6e, 0x00])
        );
        assert_eq!(encode_string(""), Ok(vec![0x80, 0x00]));
    }

    #[test]
    fn strings_that_would_not_survive_the_encoding_are_refused() {
        assert!(encode_string("a\0b").is_err());
        assert!(encode_string("Ǝ").is_err());
    }

    #[test]
    fn local_variables_encode_as_lv0_lv1_lv2_or_lv4_by_offset() {
        assert_eq!(encode_local(0), Ok(vec![0x40]));
        assert_eq!(encode_local(31), Ok(vec![0x5f]));
        assert_eq!(encode_local(32), Ok(vec![0xc1, 0x20]));
        assert_eq!(encode_local(127), Ok(vec![0xc1, 0x7f]));
        assert_eq!(encode_local(128), Ok(vec![0xc2, 0x80, 0x00]));
        assert_eq!(encode_local(32767), Ok(vec![0xc2, 0xff, 0x7f]));
        assert_eq!(encode_local(32768), Ok(vec![0xc3, 0x00, 0x80, 0x00, 0x00]));
        assert!(encode_local(0x8000_0000).is_err());
    }

    #[test]
    fn locals_are_laid_out_in_declaration_order_with_natural_alignment() {
        let mut thread = ThreadBuilder::new();
        thread.data8("A").unwrap();
        thread.data16("B").unwrap();
        thread.data8("C").unwrap();
        thread.data32("D").unwrap();
        thread.data_string("E", 3).unwrap();
        thread.data32("F").unwrap();
        let image = thread.assemble().unwrap();
        let offsets: Vec<(&str, u32)> = image
            .locals
            .iter()
            .map(|(name, slot)| (name.as_str(), slot.offset))
            .collect();
        assert_eq!(
            offsets,
            [("A", 0), ("B", 2), ("C", 4), ("D", 8), ("E", 12), ("F", 16)]
        );
        assert_eq!(image.local_bytes, 20);
    }

    #[test]
    fn a_forward_jump_resolves_to_a_positive_offset_from_the_end_of_the_jump() {
        let mut thread = ThreadBuilder::new();
        thread.data8("Flag").unwrap();
        thread.emit(op::JR_TRUE, &[local("Flag"), label("Skip")]);
        thread.emit(op::MOVE8_8, &[0.into(), local("Flag")]);
        thread.label("Skip").unwrap();
        // JR_TRUE Flag LC2(3): the MOVE8_8 0 Flag in between is 3 bytes long.
        assert_eq!(
            thread.assemble().unwrap().code,
            [
                op::JR_TRUE,
                0x40,
                0x82,
                0x03,
                0x00,
                op::MOVE8_8,
                0x00,
                0x40,
                op::OBJECT_END
            ]
        );
    }

    #[test]
    fn a_backward_jump_resolves_to_a_negative_offset_always_as_lc2() {
        let mut thread = ThreadBuilder::new();
        thread.label("Here").unwrap();
        thread.emit(op::JR, &[label("Here")]);
        // Jumping to its own start moves back over the whole 4-byte instruction.
        assert_eq!(
            thread.assemble().unwrap().code,
            [op::JR, 0x82, 0xfc, 0xff, op::OBJECT_END]
        );
    }

    #[test]
    fn undefined_duplicate_labels_and_unknown_locals_are_refused() {
        let mut missing = ThreadBuilder::new();
        missing.emit(op::JR, &[label("Nowhere")]);
        assert!(
            missing
                .assemble()
                .unwrap_err()
                .to_string()
                .contains("Nowhere")
        );

        let mut twice = ThreadBuilder::new();
        twice.label("Again").unwrap();
        assert!(
            twice
                .label("Again")
                .unwrap_err()
                .to_string()
                .contains("Again")
        );

        let mut unknown = ThreadBuilder::new();
        unknown.emit(op::MOVE8_8, &[0.into(), local("Missing")]);
        assert!(
            unknown
                .assemble()
                .unwrap_err()
                .to_string()
                .contains("Missing")
        );

        let mut declared = ThreadBuilder::new();
        declared.data8("Twice").unwrap();
        assert!(declared.data8("Twice").is_err());
        assert!(declared.data_string("Empty", 0).is_err());
    }

    #[test]
    fn the_image_has_the_lego_header_and_one_thread_header_before_the_code() {
        let mut thread = ThreadBuilder::new();
        thread.data32("X").unwrap();
        thread.emit(op::TIMER_WAIT, &[100.into(), local("X")]);
        let image = thread.assemble().unwrap();
        let bytes = &image.bytes;
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let u32_at = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        assert_eq!(&bytes[0..4], b"LEGO");
        assert_eq!(u32_at(4) as usize, bytes.len(), "image size");
        assert_eq!(u16_at(8), 109, "bytecode version 1.09");
        assert_eq!(u16_at(10), 1, "one object");
        assert_eq!(u32_at(12), 0, "no globals");
        assert_eq!(u32_at(16), 28, "instructions start after both headers");
        assert_eq!(u16_at(20), 0, "owner object");
        assert_eq!(u16_at(22), 0, "trigger count");
        assert_eq!(u32_at(24), 4, "local bytes");
        assert_eq!(&bytes[28..], image.code.as_slice());
    }
}
