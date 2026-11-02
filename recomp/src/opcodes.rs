//! Tabel opcode 6502: 151 resmi dan 80 tak resmi yang stabil.
//!
//! `OFFICIAL` dan `UNOFFICIAL` dihasilkan dari tabel referensi masswerk.at/6502
//! (bukan diketik dari ingatan) dan diuji ulang oleh nestest di Tahap 5.
//! Opcode tak resmi yang tidak stabil atau tidak diuji nestest (ANC, ALR, ARR, XAA, SHA,
//! SHX, SHY, LAS, SBX, LXA, JAM) sengaja tidak ada, jadi decode menolaknya dengan error.
//! Nama mengikuti nestest.log (mis. ISB, bukan ISC).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Implied,
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    Indirect,
    IndirectX,
    IndirectY,
    Relative,
}

impl Mode {
    /// Panjang instruksi (opcode + operand) dalam byte.
    pub fn len(self) -> u8 {
        match self {
            Mode::Implied | Mode::Accumulator => 1,
            Mode::Immediate
            | Mode::ZeroPage
            | Mode::ZeroPageX
            | Mode::ZeroPageY
            | Mode::IndirectX
            | Mode::IndirectY
            | Mode::Relative => 2,
            Mode::Absolute | Mode::AbsoluteX | Mode::AbsoluteY | Mode::Indirect => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpInfo {
    pub opcode: u8,
    pub mnemonic: &'static str,
    pub mode: Mode,
    /// Panjang instruksi dalam byte.
    pub len: u8,
    /// Siklus dasar.
    pub cycles: u8,
    /// True bila ada +1 siklus saat melewati batas halaman.
    /// Cabang (+1 diambil, +2 beda halaman) dihitung terpisah di generator.
    pub page_cross: bool,
    /// False untuk opcode tak resmi.
    pub official: bool,
}

const fn op(opcode: u8, mnemonic: &'static str, mode: Mode, len: u8, cycles: u8, page_cross: bool) -> OpInfo {
    OpInfo { opcode, mnemonic, mode, len, cycles, page_cross, official: true }
}

const fn uop(opcode: u8, mnemonic: &'static str, mode: Mode, len: u8, cycles: u8, page_cross: bool) -> OpInfo {
    OpInfo { opcode, mnemonic, mode, len, cycles, page_cross, official: false }
}

use Mode::*;

pub const OFFICIAL: [OpInfo; 151] = [
    op(0x00, "BRK", Implied, 1, 7, false),
    op(0x01, "ORA", IndirectX, 2, 6, false),
    op(0x05, "ORA", ZeroPage, 2, 3, false),
    op(0x06, "ASL", ZeroPage, 2, 5, false),
    op(0x08, "PHP", Implied, 1, 3, false),
    op(0x09, "ORA", Immediate, 2, 2, false),
    op(0x0A, "ASL", Accumulator, 1, 2, false),
    op(0x0D, "ORA", Absolute, 3, 4, false),
    op(0x0E, "ASL", Absolute, 3, 6, false),
    op(0x10, "BPL", Relative, 2, 2, false), // footnote 2
    op(0x11, "ORA", IndirectY, 2, 5, true),
    op(0x15, "ORA", ZeroPageX, 2, 4, false),
    op(0x16, "ASL", ZeroPageX, 2, 6, false),
    op(0x18, "CLC", Implied, 1, 2, false),
    op(0x19, "ORA", AbsoluteY, 3, 4, true),
    op(0x1D, "ORA", AbsoluteX, 3, 4, true),
    op(0x1E, "ASL", AbsoluteX, 3, 7, false),
    op(0x20, "JSR", Absolute, 3, 6, false),
    op(0x21, "AND", IndirectX, 2, 6, false),
    op(0x24, "BIT", ZeroPage, 2, 3, false),
    op(0x25, "AND", ZeroPage, 2, 3, false),
    op(0x26, "ROL", ZeroPage, 2, 5, false),
    op(0x28, "PLP", Implied, 1, 4, false),
    op(0x29, "AND", Immediate, 2, 2, false),
    op(0x2A, "ROL", Accumulator, 1, 2, false),
    op(0x2C, "BIT", Absolute, 3, 4, false),
    op(0x2D, "AND", Absolute, 3, 4, false),
    op(0x2E, "ROL", Absolute, 3, 6, false),
    op(0x30, "BMI", Relative, 2, 2, false), // footnote 2
    op(0x31, "AND", IndirectY, 2, 5, true),
    op(0x35, "AND", ZeroPageX, 2, 4, false),
    op(0x36, "ROL", ZeroPageX, 2, 6, false),
    op(0x38, "SEC", Implied, 1, 2, false),
    op(0x39, "AND", AbsoluteY, 3, 4, true),
    op(0x3D, "AND", AbsoluteX, 3, 4, true),
    op(0x3E, "ROL", AbsoluteX, 3, 7, false),
    op(0x40, "RTI", Implied, 1, 6, false),
    op(0x41, "EOR", IndirectX, 2, 6, false),
    op(0x45, "EOR", ZeroPage, 2, 3, false),
    op(0x46, "LSR", ZeroPage, 2, 5, false),
    op(0x48, "PHA", Implied, 1, 3, false),
    op(0x49, "EOR", Immediate, 2, 2, false),
    op(0x4A, "LSR", Accumulator, 1, 2, false),
    op(0x4C, "JMP", Absolute, 3, 3, false),
    op(0x4D, "EOR", Absolute, 3, 4, false),
    op(0x4E, "LSR", Absolute, 3, 6, false),
    op(0x50, "BVC", Relative, 2, 2, false), // footnote 2
    op(0x51, "EOR", IndirectY, 2, 5, true),
    op(0x55, "EOR", ZeroPageX, 2, 4, false),
    op(0x56, "LSR", ZeroPageX, 2, 6, false),
    op(0x58, "CLI", Implied, 1, 2, false),
    op(0x59, "EOR", AbsoluteY, 3, 4, true),
    op(0x5D, "EOR", AbsoluteX, 3, 4, true),
    op(0x5E, "LSR", AbsoluteX, 3, 7, false),
    op(0x60, "RTS", Implied, 1, 6, false),
    op(0x61, "ADC", IndirectX, 2, 6, false),
    op(0x65, "ADC", ZeroPage, 2, 3, false),
    op(0x66, "ROR", ZeroPage, 2, 5, false),
    op(0x68, "PLA", Implied, 1, 4, false),
    op(0x69, "ADC", Immediate, 2, 2, false),
    op(0x6A, "ROR", Accumulator, 1, 2, false),
    op(0x6C, "JMP", Indirect, 3, 5, false), // footnote 3
    op(0x6D, "ADC", Absolute, 3, 4, false),
    op(0x6E, "ROR", Absolute, 3, 6, false),
    op(0x70, "BVS", Relative, 2, 2, false), // footnote 2
    op(0x71, "ADC", IndirectY, 2, 5, true),
    op(0x75, "ADC", ZeroPageX, 2, 4, false),
    op(0x76, "ROR", ZeroPageX, 2, 6, false),
    op(0x78, "SEI", Implied, 1, 2, false),
    op(0x79, "ADC", AbsoluteY, 3, 4, true),
    op(0x7D, "ADC", AbsoluteX, 3, 4, true),
    op(0x7E, "ROR", AbsoluteX, 3, 7, false),
    op(0x81, "STA", IndirectX, 2, 6, false),
    op(0x84, "STY", ZeroPage, 2, 3, false),
    op(0x85, "STA", ZeroPage, 2, 3, false),
    op(0x86, "STX", ZeroPage, 2, 3, false),
    op(0x88, "DEY", Implied, 1, 2, false),
    op(0x8A, "TXA", Implied, 1, 2, false),
    op(0x8C, "STY", Absolute, 3, 4, false),
    op(0x8D, "STA", Absolute, 3, 4, false),
    op(0x8E, "STX", Absolute, 3, 4, false),
    op(0x90, "BCC", Relative, 2, 2, false), // footnote 2
    op(0x91, "STA", IndirectY, 2, 6, false),
    op(0x94, "STY", ZeroPageX, 2, 4, false),
    op(0x95, "STA", ZeroPageX, 2, 4, false),
    op(0x96, "STX", ZeroPageY, 2, 4, false),
    op(0x98, "TYA", Implied, 1, 2, false),
    op(0x99, "STA", AbsoluteY, 3, 5, false),
    op(0x9A, "TXS", Implied, 1, 2, false),
    op(0x9D, "STA", AbsoluteX, 3, 5, false),
    op(0xA0, "LDY", Immediate, 2, 2, false),
    op(0xA1, "LDA", IndirectX, 2, 6, false),
    op(0xA2, "LDX", Immediate, 2, 2, false),
    op(0xA4, "LDY", ZeroPage, 2, 3, false),
    op(0xA5, "LDA", ZeroPage, 2, 3, false),
    op(0xA6, "LDX", ZeroPage, 2, 3, false),
    op(0xA8, "TAY", Implied, 1, 2, false),
    op(0xA9, "LDA", Immediate, 2, 2, false),
    op(0xAA, "TAX", Implied, 1, 2, false),
    op(0xAC, "LDY", Absolute, 3, 4, false),
    op(0xAD, "LDA", Absolute, 3, 4, false),
    op(0xAE, "LDX", Absolute, 3, 4, false),
    op(0xB0, "BCS", Relative, 2, 2, false), // footnote 2
    op(0xB1, "LDA", IndirectY, 2, 5, true),
    op(0xB4, "LDY", ZeroPageX, 2, 4, false),
    op(0xB5, "LDA", ZeroPageX, 2, 4, false),
    op(0xB6, "LDX", ZeroPageY, 2, 4, false),
    op(0xB8, "CLV", Implied, 1, 2, false),
    op(0xB9, "LDA", AbsoluteY, 3, 4, true),
    op(0xBA, "TSX", Implied, 1, 2, false),
    op(0xBC, "LDY", AbsoluteX, 3, 4, true),
    op(0xBD, "LDA", AbsoluteX, 3, 4, true),
    op(0xBE, "LDX", AbsoluteY, 3, 4, true),
    op(0xC0, "CPY", Immediate, 2, 2, false),
    op(0xC1, "CMP", IndirectX, 2, 6, false),
    op(0xC4, "CPY", ZeroPage, 2, 3, false),
    op(0xC5, "CMP", ZeroPage, 2, 3, false),
    op(0xC6, "DEC", ZeroPage, 2, 5, false),
    op(0xC8, "INY", Implied, 1, 2, false),
    op(0xC9, "CMP", Immediate, 2, 2, false),
    op(0xCA, "DEX", Implied, 1, 2, false),
    op(0xCC, "CPY", Absolute, 3, 4, false),
    op(0xCD, "CMP", Absolute, 3, 4, false),
    op(0xCE, "DEC", Absolute, 3, 6, false),
    op(0xD0, "BNE", Relative, 2, 2, false), // footnote 2
    op(0xD1, "CMP", IndirectY, 2, 5, true),
    op(0xD5, "CMP", ZeroPageX, 2, 4, false),
    op(0xD6, "DEC", ZeroPageX, 2, 6, false),
    op(0xD8, "CLD", Implied, 1, 2, false),
    op(0xD9, "CMP", AbsoluteY, 3, 4, true),
    op(0xDD, "CMP", AbsoluteX, 3, 4, true),
    op(0xDE, "DEC", AbsoluteX, 3, 7, false),
    op(0xE0, "CPX", Immediate, 2, 2, false),
    op(0xE1, "SBC", IndirectX, 2, 6, false),
    op(0xE4, "CPX", ZeroPage, 2, 3, false),
    op(0xE5, "SBC", ZeroPage, 2, 3, false),
    op(0xE6, "INC", ZeroPage, 2, 5, false),
    op(0xE8, "INX", Implied, 1, 2, false),
    op(0xE9, "SBC", Immediate, 2, 2, false),
    op(0xEA, "NOP", Implied, 1, 2, false),
    op(0xEC, "CPX", Absolute, 3, 4, false),
    op(0xED, "SBC", Absolute, 3, 4, false),
    op(0xEE, "INC", Absolute, 3, 6, false),
    op(0xF0, "BEQ", Relative, 2, 2, false), // footnote 2
    op(0xF1, "SBC", IndirectY, 2, 5, true),
    op(0xF5, "SBC", ZeroPageX, 2, 4, false),
    op(0xF6, "INC", ZeroPageX, 2, 6, false),
    op(0xF8, "SED", Implied, 1, 2, false),
    op(0xF9, "SBC", AbsoluteY, 3, 4, true),
    op(0xFD, "SBC", AbsoluteX, 3, 4, true),
    op(0xFE, "INC", AbsoluteX, 3, 7, false),
];

pub const UNOFFICIAL: [OpInfo; 80] = [
    uop(0x03, "SLO", IndirectX, 2, 8, false),
    uop(0x04, "NOP", ZeroPage, 2, 3, false),
    uop(0x07, "SLO", ZeroPage, 2, 5, false),
    uop(0x0C, "NOP", Absolute, 3, 4, false),
    uop(0x0F, "SLO", Absolute, 3, 6, false),
    uop(0x13, "SLO", IndirectY, 2, 8, false),
    uop(0x14, "NOP", ZeroPageX, 2, 4, false),
    uop(0x17, "SLO", ZeroPageX, 2, 6, false),
    uop(0x1A, "NOP", Implied, 1, 2, false),
    uop(0x1B, "SLO", AbsoluteY, 3, 7, false),
    uop(0x1C, "NOP", AbsoluteX, 3, 4, true),
    uop(0x1F, "SLO", AbsoluteX, 3, 7, false),
    uop(0x23, "RLA", IndirectX, 2, 8, false),
    uop(0x27, "RLA", ZeroPage, 2, 5, false),
    uop(0x2F, "RLA", Absolute, 3, 6, false),
    uop(0x33, "RLA", IndirectY, 2, 8, false),
    uop(0x34, "NOP", ZeroPageX, 2, 4, false),
    uop(0x37, "RLA", ZeroPageX, 2, 6, false),
    uop(0x3A, "NOP", Implied, 1, 2, false),
    uop(0x3B, "RLA", AbsoluteY, 3, 7, false),
    uop(0x3C, "NOP", AbsoluteX, 3, 4, true),
    uop(0x3F, "RLA", AbsoluteX, 3, 7, false),
    uop(0x43, "SRE", IndirectX, 2, 8, false),
    uop(0x44, "NOP", ZeroPage, 2, 3, false),
    uop(0x47, "SRE", ZeroPage, 2, 5, false),
    uop(0x4F, "SRE", Absolute, 3, 6, false),
    uop(0x53, "SRE", IndirectY, 2, 8, false),
    uop(0x54, "NOP", ZeroPageX, 2, 4, false),
    uop(0x57, "SRE", ZeroPageX, 2, 6, false),
    uop(0x5A, "NOP", Implied, 1, 2, false),
    uop(0x5B, "SRE", AbsoluteY, 3, 7, false),
    uop(0x5C, "NOP", AbsoluteX, 3, 4, true),
    uop(0x5F, "SRE", AbsoluteX, 3, 7, false),
    uop(0x63, "RRA", IndirectX, 2, 8, false),
    uop(0x64, "NOP", ZeroPage, 2, 3, false),
    uop(0x67, "RRA", ZeroPage, 2, 5, false),
    uop(0x6F, "RRA", Absolute, 3, 6, false),
    uop(0x73, "RRA", IndirectY, 2, 8, false),
    uop(0x74, "NOP", ZeroPageX, 2, 4, false),
    uop(0x77, "RRA", ZeroPageX, 2, 6, false),
    uop(0x7A, "NOP", Implied, 1, 2, false),
    uop(0x7B, "RRA", AbsoluteY, 3, 7, false),
    uop(0x7C, "NOP", AbsoluteX, 3, 4, true),
    uop(0x7F, "RRA", AbsoluteX, 3, 7, false),
    uop(0x80, "NOP", Immediate, 2, 2, false),
    uop(0x82, "NOP", Immediate, 2, 2, false),
    uop(0x83, "SAX", IndirectX, 2, 6, false),
    uop(0x87, "SAX", ZeroPage, 2, 3, false),
    uop(0x89, "NOP", Immediate, 2, 2, false),
    uop(0x8F, "SAX", Absolute, 3, 4, false),
    uop(0x97, "SAX", ZeroPageY, 2, 4, false),
    uop(0xA3, "LAX", IndirectX, 2, 6, false),
    uop(0xA7, "LAX", ZeroPage, 2, 3, false),
    uop(0xAF, "LAX", Absolute, 3, 4, false),
    uop(0xB3, "LAX", IndirectY, 2, 5, true),
    uop(0xB7, "LAX", ZeroPageY, 2, 4, false),
    uop(0xBF, "LAX", AbsoluteY, 3, 4, true),
    uop(0xC2, "NOP", Immediate, 2, 2, false),
    uop(0xC3, "DCP", IndirectX, 2, 8, false),
    uop(0xC7, "DCP", ZeroPage, 2, 5, false),
    uop(0xCF, "DCP", Absolute, 3, 6, false),
    uop(0xD3, "DCP", IndirectY, 2, 8, false),
    uop(0xD4, "NOP", ZeroPageX, 2, 4, false),
    uop(0xD7, "DCP", ZeroPageX, 2, 6, false),
    uop(0xDA, "NOP", Implied, 1, 2, false),
    uop(0xDB, "DCP", AbsoluteY, 3, 7, false),
    uop(0xDC, "NOP", AbsoluteX, 3, 4, true),
    uop(0xDF, "DCP", AbsoluteX, 3, 7, false),
    uop(0xE2, "NOP", Immediate, 2, 2, false),
    uop(0xE3, "ISB", IndirectX, 2, 8, false),
    uop(0xE7, "ISB", ZeroPage, 2, 5, false),
    uop(0xEB, "SBC", Immediate, 2, 2, false),
    uop(0xEF, "ISB", Absolute, 3, 6, false),
    uop(0xF3, "ISB", IndirectY, 2, 8, false),
    uop(0xF4, "NOP", ZeroPageX, 2, 4, false),
    uop(0xF7, "ISB", ZeroPageX, 2, 6, false),
    uop(0xFA, "NOP", Implied, 1, 2, false),
    uop(0xFB, "ISB", AbsoluteY, 3, 7, false),
    uop(0xFC, "NOP", AbsoluteX, 3, 4, true),
    uop(0xFF, "ISB", AbsoluteX, 3, 7, false),
];

/// Tabel 256 slot: indeks = byte opcode, `None` = opcode yang tidak didukung.
pub static TABLE: [Option<OpInfo>; 256] = build_table();

const fn build_table() -> [Option<OpInfo>; 256] {
    let mut t = [None; 256];
    let mut i = 0;
    while i < OFFICIAL.len() {
        t[OFFICIAL[i].opcode as usize] = Some(OFFICIAL[i]);
        i += 1;
    }
    let mut j = 0;
    while j < UNOFFICIAL.len() {
        assert!(t[UNOFFICIAL[j].opcode as usize].is_none(), "opcode ganda");
        t[UNOFFICIAL[j].opcode as usize] = Some(UNOFFICIAL[j]);
        j += 1;
    }
    t
}

pub fn lookup(opcode: u8) -> Option<&'static OpInfo> {
    TABLE[opcode as usize].as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tepat_151_resmi_dan_80_tak_resmi_tanpa_duplikat() {
        assert_eq!(TABLE.iter().flatten().filter(|o| o.official).count(), 151);
        assert_eq!(TABLE.iter().flatten().filter(|o| !o.official).count(), 80);
        assert_eq!(TABLE.iter().flatten().count(), 231);
    }

    #[test]
    fn len_konsisten_dengan_mode() {
        for o in OFFICIAL.iter().chain(UNOFFICIAL.iter()) {
            assert_eq!(o.len, o.mode.len(), "opcode {:02X} {}", o.opcode, o.mnemonic);
        }
    }

    #[test]
    fn indeks_tabel_sama_dengan_opcode() {
        for (i, slot) in TABLE.iter().enumerate() {
            if let Some(o) = slot {
                assert_eq!(o.opcode as usize, i);
            }
        }
    }

    #[test]
    fn sampel_opcode_terkenal() {
        let chk = |b: u8, m: &str, mode: Mode, len: u8, cyc: u8| {
            let o = lookup(b).unwrap();
            assert_eq!((o.mnemonic, o.mode, o.len, o.cycles), (m, mode, len, cyc), "{b:02X}");
        };
        chk(0xA9, "LDA", Immediate, 2, 2);
        chk(0x4C, "JMP", Absolute, 3, 3);
        chk(0x6C, "JMP", Indirect, 3, 5);
        chk(0x20, "JSR", Absolute, 3, 6);
        chk(0x60, "RTS", Implied, 1, 6);
        chk(0x40, "RTI", Implied, 1, 6);
        chk(0x00, "BRK", Implied, 1, 7);
        chk(0xEA, "NOP", Implied, 1, 2);
        chk(0x0A, "ASL", Accumulator, 1, 2);
        chk(0xD0, "BNE", Relative, 2, 2);
        chk(0xB1, "LDA", IndirectY, 2, 5);
    }

    #[test]
    fn opcode_tak_stabil_tidak_didukung() {
        // JAM, ANC, ALR, ARR, XAA, SHA, SHX, SHY, LAS, SBX, LXA
        for b in [0x02u8, 0x0B, 0x4B, 0x6B, 0x8B, 0x9F, 0x9E, 0x9C, 0xBB, 0xCB, 0xAB, 0x93] {
            assert!(lookup(b).is_none(), "{b:02X}");
        }
    }

    #[test]
    fn sampel_opcode_tak_resmi() {
        let chk = |b: u8, m: &str, mode: Mode, len: u8, cyc: u8, cross: bool| {
            let o = lookup(b).unwrap();
            assert!(!o.official, "{b:02X}");
            assert_eq!((o.mnemonic, o.mode, o.len, o.cycles, o.page_cross), (m, mode, len, cyc, cross), "{b:02X}");
        };
        chk(0xA7, "LAX", ZeroPage, 2, 3, false);
        chk(0xB3, "LAX", IndirectY, 2, 5, true);
        chk(0x87, "SAX", ZeroPage, 2, 3, false);
        chk(0xC3, "DCP", IndirectX, 2, 8, false);
        chk(0xFF, "ISB", AbsoluteX, 3, 7, false);
        chk(0x1F, "SLO", AbsoluteX, 3, 7, false);
        chk(0x7B, "RRA", AbsoluteY, 3, 7, false);
        chk(0xEB, "SBC", Immediate, 2, 2, false);
        chk(0x1A, "NOP", Implied, 1, 2, false);
        chk(0x1C, "NOP", AbsoluteX, 3, 4, true);
        chk(0x04, "NOP", ZeroPage, 2, 3, false);
    }

    #[test]
    fn jumlah_per_mnemonic_masuk_akal() {
        let n = |m: &str| OFFICIAL.iter().chain(UNOFFICIAL.iter()).filter(|o| o.mnemonic == m).count();
        assert_eq!(n("LDA"), 8);
        // 8 keluarga bertabel x 7 mode, kecuali LAX (6) dan SAX (4); NOP resmi 1 + tak resmi 27
        assert_eq!(n("SLO") + n("RLA") + n("SRE") + n("RRA") + n("DCP") + n("ISB"), 42);
        assert_eq!((n("LAX"), n("SAX"), n("NOP")), (6, 4, 28));
        assert_eq!(n("STA"), 7);
        assert_eq!(n("ADC"), 8);
        assert_eq!(n("JMP"), 2);
        assert_eq!(n("LDX"), 5);
        assert_eq!(n("ASL"), 5);
        // 8 cabang, masing-masing 1 opcode mode relatif
        assert_eq!(OFFICIAL.iter().filter(|o| o.mode == Relative).count(), 8);
    }

    #[test]
    fn page_cross_hanya_untuk_baca_terindeks() {
        let o = lookup(0xBD).unwrap(); // LDA abs,X
        assert!(o.page_cross);
        let o = lookup(0x9D).unwrap(); // STA abs,X selalu 5 siklus
        assert!(!o.page_cross);
        assert_eq!(o.cycles, 5);
    }
}
