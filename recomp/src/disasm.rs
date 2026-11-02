use crate::decode::{Instruction, decode};
use crate::opcodes::Mode;

/// Teks operand, mis. `#$42`, `($20),Y`, atau tujuan cabang `$C007`.
pub fn operand_text(i: &Instruction) -> String {
    let o = i.operand;
    match i.info.mode {
        Mode::Implied => String::new(),
        Mode::Accumulator => "A".to_string(),
        Mode::Immediate => format!("#${o:02X}"),
        Mode::ZeroPage => format!("${o:02X}"),
        Mode::ZeroPageX => format!("${o:02X},X"),
        Mode::ZeroPageY => format!("${o:02X},Y"),
        Mode::Absolute => format!("${o:04X}"),
        Mode::AbsoluteX => format!("${o:04X},X"),
        Mode::AbsoluteY => format!("${o:04X},Y"),
        Mode::Indirect => format!("(${o:04X})"),
        Mode::IndirectX => format!("(${o:02X},X)"),
        Mode::IndirectY => format!("(${o:02X}),Y"),
        Mode::Relative => format!("${:04X}", i.branch_target().unwrap()),
    }
}

/// Satu baris: `C000  4C F5 C5  JMP $C5F5`.
pub fn format_line(i: &Instruction, raw: &[u8]) -> String {
    let hex: Vec<String> = raw.iter().map(|b| format!("{b:02X}")).collect();
    // Opcode tak resmi diberi awalan `*`, seperti di nestest.log.
    let star = if i.info.official { "" } else { "*" };
    let text = format!("{star}{} {}", i.info.mnemonic, operand_text(i));
    format!("{:04X}  {:<8}  {}", i.addr, hex.join(" "), text.trim_end())
}

#[derive(Debug, Default)]
pub struct Listing {
    pub lines: Vec<String>,
    /// Berhenti lebih awal karena error (baris sebelumnya tetap valid).
    pub error: Option<String>,
}

/// Disassemble linear mulai `from`, paling banyak `count` instruksi.
/// `fetch` mengembalikan byte di alamat CPU tertentu, atau `None` bila tidak terpetakan.
pub fn disassemble(fetch: impl Fn(u16) -> Option<u8>, from: u16, count: usize) -> Listing {
    let mut out = Listing::default();
    let mut addr = from as u32;
    for _ in 0..count {
        if addr > 0xFFFF {
            break; // ujung ruang alamat
        }
        let mut buf = Vec::with_capacity(3);
        for k in 0..3u32 {
            match u16::try_from(addr + k).ok().and_then(&fetch) {
                Some(b) => buf.push(b),
                None => break,
            }
        }
        if buf.is_empty() {
            out.error = Some(format!("alamat ${addr:04X} tidak terpetakan ke PRG-ROM"));
            break;
        }
        match decode(&buf, addr as u16) {
            Ok(i) => {
                out.lines.push(format_line(&i, &buf[..i.len() as usize]));
                addr += i.len() as u32;
            }
            Err(e) => {
                out.error = Some(e.to_string());
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Program kecil di $C000 sebagai "memori".
    fn mem(code: &'static [u8]) -> impl Fn(u16) -> Option<u8> {
        move |a| a.checked_sub(0xC000).and_then(|o| code.get(o as usize).copied())
    }

    #[test]
    fn program_kecil_semua_mode() {
        let code: &[u8] = &[
            0x78, // SEI
            0xA2, 0xFF, // LDX #$FF
            0x9A, // TXS
            0x0A, // ASL A
            0xB5, 0x10, // LDA $10,X
            0x8D, 0x00, 0x20, // STA $2000
            0xB1, 0x20, // LDA ($20),Y
            0x6C, 0xFC, 0xFF, // JMP ($FFFC)
            0xD0, 0xF3, // BNE (mundur)
            0x20, 0x00, 0xC0, // JSR $C000
            0x60, // RTS
        ];
        let l = disassemble(mem(code), 0xC000, 11);
        assert_eq!(l.error, None);
        assert_eq!(
            l.lines,
            vec![
                "C000  78        SEI",
                "C001  A2 FF     LDX #$FF",
                "C003  9A        TXS",
                "C004  0A        ASL A",
                "C005  B5 10     LDA $10,X",
                "C007  8D 00 20  STA $2000",
                "C00A  B1 20     LDA ($20),Y",
                "C00C  6C FC FF  JMP ($FFFC)",
                "C00F  D0 F3     BNE $C004",
                "C011  20 00 C0  JSR $C000",
                "C014  60        RTS",
            ]
        );
    }

    #[test]
    fn berhenti_pada_opcode_tak_resmi_dengan_baris_sebelumnya_utuh() {
        let l = disassemble(mem(&[0xEA, 0x02, 0xEA]), 0xC000, 10);
        assert_eq!(l.lines, vec!["C000  EA        NOP"]);
        assert!(l.error.unwrap().contains("$02"));
    }

    #[test]
    fn opcode_tak_resmi_diberi_bintang() {
        let l = disassemble(mem(&[0xA7, 0x10, 0x1A, 0xEB, 0x01, 0x60]), 0xC000, 4);
        assert_eq!(
            l.lines,
            vec![
                "C000  A7 10     *LAX $10",
                "C002  1A        *NOP",
                "C003  EB 01     *SBC #$01",
                "C005  60        RTS",
            ]
        );
    }

    #[test]
    fn count_membatasi_jumlah() {
        let l = disassemble(mem(&[0xEA; 10]), 0xC000, 3);
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.error, None);
    }

    #[test]
    fn alamat_tak_terpetakan() {
        let l = disassemble(mem(&[0xEA]), 0x1000, 1);
        assert!(l.error.unwrap().contains("tidak terpetakan"));
    }

    #[test]
    fn instruksi_terpotong_di_ujung_data() {
        let l = disassemble(mem(&[0xEA, 0x4C, 0x00]), 0xC000, 5);
        assert_eq!(l.lines.len(), 1);
        assert!(l.error.unwrap().contains("terpotong"));
    }
}
