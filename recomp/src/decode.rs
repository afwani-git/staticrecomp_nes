use std::fmt;

use crate::opcodes::{self, Mode, OpInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Byte opcode tidak ada di tabel (151 resmi + 80 tak resmi yang stabil).
    Unofficial { addr: u16, opcode: u8 },
    /// Data habis sebelum seluruh operand terbaca.
    Truncated { addr: u16, need: u8, have: u8 },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Unofficial { addr, opcode } => write!(
                f,
                "opcode ${opcode:02X} di ${addr:04X} tidak didukung (hanya 151 opcode resmi dan 80 tak resmi yang stabil)"
            ),
            DecodeError::Truncated { addr, need, have } => write!(
                f,
                "instruksi di ${addr:04X} terpotong: butuh {need} byte, tersisa {have}"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub addr: u16,
    pub info: &'static OpInfo,
    /// Operand mentah: 0 (tanpa operand), 1 byte, atau 2 byte little-endian.
    pub operand: u16,
}

impl Instruction {
    pub fn len(&self) -> u16 {
        self.info.len as u16
    }

    /// Alamat instruksi berikutnya (dengan wrap 16-bit).
    pub fn next_addr(&self) -> u16 {
        self.addr.wrapping_add(self.len())
    }

    /// Alamat tujuan untuk cabang (relatif terhadap instruksi berikutnya).
    pub fn branch_target(&self) -> Option<u16> {
        if self.info.mode != Mode::Relative {
            return None;
        }
        let offset = self.operand as u8 as i8;
        Some(self.next_addr().wrapping_add(offset as u16))
    }
}

/// Decode satu instruksi dari awal `bytes`, yang berada di alamat CPU `addr`.
pub fn decode(bytes: &[u8], addr: u16) -> Result<Instruction, DecodeError> {
    let Some(&opcode) = bytes.first() else {
        return Err(DecodeError::Truncated { addr, need: 1, have: 0 });
    };
    let info = opcodes::lookup(opcode).ok_or(DecodeError::Unofficial { addr, opcode })?;
    if bytes.len() < info.len as usize {
        return Err(DecodeError::Truncated {
            addr,
            need: info.len,
            have: bytes.len() as u8,
        });
    }
    let operand = match info.len {
        1 => 0,
        2 => bytes[1] as u16,
        _ => u16::from_le_bytes([bytes[1], bytes[2]]),
    };
    Ok(Instruction { addr, info, operand })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(bytes: &[u8], addr: u16) -> Instruction {
        decode(bytes, addr).unwrap()
    }

    #[test]
    fn satu_tes_per_mode() {
        let cases: &[(&[u8], Mode, u16)] = &[
            (&[0xEA], Mode::Implied, 0),
            (&[0x0A], Mode::Accumulator, 0),
            (&[0xA9, 0x42], Mode::Immediate, 0x42),
            (&[0xA5, 0x10], Mode::ZeroPage, 0x10),
            (&[0xB5, 0x10], Mode::ZeroPageX, 0x10),
            (&[0xB6, 0x10], Mode::ZeroPageY, 0x10),
            (&[0xAD, 0x34, 0x12], Mode::Absolute, 0x1234),
            (&[0xBD, 0x34, 0x12], Mode::AbsoluteX, 0x1234),
            (&[0xB9, 0x34, 0x12], Mode::AbsoluteY, 0x1234),
            (&[0x6C, 0x34, 0x12], Mode::Indirect, 0x1234),
            (&[0xA1, 0x20], Mode::IndirectX, 0x20),
            (&[0xB1, 0x20], Mode::IndirectY, 0x20),
            (&[0xD0, 0x05], Mode::Relative, 5),
        ];
        for (bytes, mode, operand) in cases {
            let i = d(bytes, 0x8000);
            assert_eq!((i.info.mode, i.operand), (*mode, *operand), "{bytes:02X?}");
            assert_eq!(i.len() as usize, bytes.len());
        }
    }

    #[test]
    fn operand_16_bit_little_endian() {
        assert_eq!(d(&[0x4C, 0xF5, 0xC5], 0xC000).operand, 0xC5F5);
    }

    #[test]
    fn cabang_maju_dan_mundur() {
        // BNE +5 di $C000: berikutnya $C002, tujuan $C007
        assert_eq!(d(&[0xD0, 0x05], 0xC000).branch_target(), Some(0xC007));
        // BNE -2 ($FE) = loop ke dirinya sendiri
        assert_eq!(d(&[0xD0, 0xFE], 0xC000).branch_target(), Some(0xC000));
        // BNE -128
        assert_eq!(d(&[0xD0, 0x80], 0xC100).branch_target(), Some(0xC082));
        // bukan cabang
        assert_eq!(d(&[0xEA], 0xC000).branch_target(), None);
    }

    #[test]
    fn opcode_tak_resmi_error_jelas() {
        let e = decode(&[0x02], 0xC123).unwrap_err();
        assert_eq!(e, DecodeError::Unofficial { addr: 0xC123, opcode: 0x02 });
        let msg = e.to_string();
        assert!(msg.contains("$02") && msg.contains("$C123"), "{msg}");
    }

    #[test]
    fn data_terpotong() {
        assert_eq!(
            decode(&[0x4C, 0x00], 0xFFFD).unwrap_err(),
            DecodeError::Truncated { addr: 0xFFFD, need: 3, have: 2 }
        );
        assert!(decode(&[], 0x8000).is_err());
    }
}
