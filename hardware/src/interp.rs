//! Interpreter cadangan 6502: menjalankan kode langsung dari memori ketika hasil recompile tidak
//! punya kode untuknya (lompatan dinamis yang tidak terdaftar, JSR ke alamat yang tidak
//! dianalisis, kode RAM yang berubah). Memakai `read8`/`write8` yang sama, jadi perilaku, siklus,
//! dan hook poll/trace-nya identik dengan kode hasil recompile.
//!
//! Tabel opcode (mnemonic, mode, panjang, siklus, tambahan lintas halaman) dihasilkan recomp dari
//! tabel yang sama dengan generator kode, lalu diberikan lewat `Game::ops`.

use crate::cpu::{Bus, Cpu};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mn {
    None,
    ADC, AND, ASL, BCC, BCS, BEQ, BIT, BMI, BNE, BPL, BRK, BVC, BVS, CLC, CLD, CLI, CLV, CMP, CPX, CPY,
    DCP, DEC, DEX, DEY, EOR, INC, INX, INY, ISB, JMP, JSR, LAX, LDA, LDX, LDY, LSR, NOP, ORA, PHA, PHP,
    PLA, PLP, RLA, ROL, ROR, RRA, RTI, RTS, SAX, SBC, SEC, SED, SEI, SLO, SRE, STA, STX, STY, TAX, TAY,
    TSX, TXA, TXS, TYA,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Am {
    Imp, Acc, Imm, Zp, Zpx, Zpy, Abs, Abx, Aby, Ind, Inx, Iny, Rel,
}

#[derive(Clone, Copy, Debug)]
pub struct Op {
    pub mn: Mn,
    pub mode: Am,
    pub len: u8,
    pub cycles: u8,
    /// Menambah satu siklus bila indeks melewati batas halaman.
    pub cross: bool,
}

/// Nilai bawaan untuk opcode yang tidak dikenal.
pub const NO_OP: Op = Op { mn: Mn::None, mode: Am::Imp, len: 1, cycles: 0, cross: false };

const MAX_FRAMES: usize = 64;

/// Alamat kembali milik interpreter: dari JSR yang dijalankannya sendiri. Bila `t` ada di
/// dalamnya, buang bingkai sampai itu dan kembalikan true.
fn is_ours(frames: &[u16; MAX_FRAMES], n: &mut usize, t: u16) -> bool {
    for i in (1..=*n).rev() {
        if frames[i - 1] == t {
            *n = i - 1;
            return true;
        }
    }
    false
}

/// Jalankan mulai `pc` sampai ada RTS/RTI yang menuju alamat yang bukan milik interpreter
/// sendiri; alamat itu dikembalikan (seperti sebuah fungsi hasil recompile yang selesai).
/// Program utama yang menjalankan titik masuk paling atas sebaiknya melanjutkan dengan
/// `interpret` pada alamat yang dikembalikan, karena CPU sungguhan akan lanjut di sana.
pub fn interpret<B: Bus>(s: &mut Cpu<B>, mut pc: u16) -> u16 {
    let mut frames = [0u16; MAX_FRAMES];
    let mut nframes = 0usize;
    let mut first = true;

    loop {
        // Awal fungsi hasil recompile: serahkan ke kode native (kecuali instruksi pertama,
        // supaya fungsi yang melempar ke interpreter tidak berputar tanpa akhir).
        if !first {
            if let Some(r) = (s.game.call_entry)(s, pc) {
                if is_ours(&frames, &mut nframes, r) {
                    pc = r;
                    continue;
                }
                return r;
            }
        }
        first = false;

        s.poll();
        let opcode = s.read8(pc);
        let op = s.game.ops[opcode as usize];
        let (mn, mode, len) = (op.mn, op.mode, op.len);
        let next = pc.wrapping_add(u16::from(len));
        let mut npc = next;
        let mut operand: u16 = 0;
        let mut ea: u16 = 0;

        if mn == Mn::None {
            s.fault(pc);
        }
        if len >= 2 {
            operand = u16::from(s.read8(pc.wrapping_add(1)));
        }
        if len == 3 {
            operand |= u16::from(s.read8(pc.wrapping_add(2))) << 8;
        }

        s.trace(pc);
        s.cycles += u64::from(op.cycles);

        match mode {
            Am::Zp | Am::Abs => ea = operand,
            Am::Zpx => ea = u16::from((operand as u8).wrapping_add(s.x)),
            Am::Zpy => ea = u16::from((operand as u8).wrapping_add(s.y)),
            Am::Abx => {
                ea = operand.wrapping_add(u16::from(s.x));
                if op.cross && (operand & 0xFF) + u16::from(s.x) > 0xFF {
                    s.cycles += 1;
                }
            }
            Am::Aby => {
                ea = operand.wrapping_add(u16::from(s.y));
                if op.cross && (operand & 0xFF) + u16::from(s.y) > 0xFF {
                    s.cycles += 1;
                }
            }
            Am::Inx => ea = s.rd16zp((operand as u8).wrapping_add(s.x)),
            Am::Iny => {
                let b = s.rd16zp(operand as u8);
                ea = b.wrapping_add(u16::from(s.y));
                if op.cross && (b & 0xFF) + u16::from(s.y) > 0xFF {
                    s.cycles += 1;
                }
            }
            _ => {}
        }

        // Nilai operand memori: konstanta untuk mode immediate, selain itu dibaca di `ea`.
        macro_rules! mv {
            () => {
                if mode == Am::Imm { operand as u8 } else { s.read8(ea) }
            };
        }
        macro_rules! branch {
            ($cond:expr) => {
                if $cond {
                    let target = next.wrapping_add(operand as u8 as i8 as u16);
                    s.cycles += 1 + u64::from((next ^ target) & 0xFF00 != 0);
                    npc = target;
                }
            };
        }

        match mn {
            Mn::LDA => { s.a = mv!(); s.set_nz(s.a); }
            Mn::LDX => { s.x = mv!(); s.set_nz(s.x); }
            Mn::LDY => { s.y = mv!(); s.set_nz(s.y); }
            Mn::STA => s.write8(ea, s.a),
            Mn::STX => s.write8(ea, s.x),
            Mn::STY => s.write8(ea, s.y),
            Mn::TAX => { s.x = s.a; s.set_nz(s.x); }
            Mn::TAY => { s.y = s.a; s.set_nz(s.y); }
            Mn::TXA => { s.a = s.x; s.set_nz(s.a); }
            Mn::TYA => { s.a = s.y; s.set_nz(s.a); }
            Mn::TSX => { s.x = s.sp; s.set_nz(s.x); }
            Mn::TXS => s.sp = s.x,
            Mn::PHA => s.push8(s.a),
            Mn::PLA => { s.a = s.pull8(); s.set_nz(s.a); }
            Mn::PHP => { let p = s.get_p(true); s.push8(p); }
            Mn::PLP => { let p = s.pull8(); s.set_p(p); s.i_changed(1); }
            Mn::AND => { s.a &= mv!(); s.set_nz(s.a); }
            Mn::ORA => { s.a |= mv!(); s.set_nz(s.a); }
            Mn::EOR => { s.a ^= mv!(); s.set_nz(s.a); }
            Mn::ADC => { let m = mv!(); s.adc(m); }
            Mn::SBC => { let m = mv!(); s.sbc(m); }
            Mn::CMP => { let m = mv!(); s.cmp(s.a, m); }
            Mn::CPX => { let m = mv!(); s.cmp(s.x, m); }
            Mn::CPY => { let m = mv!(); s.cmp(s.y, m); }
            Mn::BIT => { let m = mv!(); s.bit(m); }
            Mn::INC => { let v = s.read8(ea).wrapping_add(1); s.write8(ea, v); s.set_nz(v); }
            Mn::DEC => { let v = s.read8(ea).wrapping_sub(1); s.write8(ea, v); s.set_nz(v); }
            Mn::INX => { s.x = s.x.wrapping_add(1); s.set_nz(s.x); }
            Mn::INY => { s.y = s.y.wrapping_add(1); s.set_nz(s.y); }
            Mn::DEX => { s.x = s.x.wrapping_sub(1); s.set_nz(s.x); }
            Mn::DEY => { s.y = s.y.wrapping_sub(1); s.set_nz(s.y); }
            Mn::ASL => {
                if mode == Am::Acc { s.a = s.asl(s.a); } else { let v = s.read8(ea); let r = s.asl(v); s.write8(ea, r); }
            }
            Mn::LSR => {
                if mode == Am::Acc { s.a = s.lsr(s.a); } else { let v = s.read8(ea); let r = s.lsr(v); s.write8(ea, r); }
            }
            Mn::ROL => {
                if mode == Am::Acc { s.a = s.rol(s.a); } else { let v = s.read8(ea); let r = s.rol(v); s.write8(ea, r); }
            }
            Mn::ROR => {
                if mode == Am::Acc { s.a = s.ror(s.a); } else { let v = s.read8(ea); let r = s.ror(v); s.write8(ea, r); }
            }
            Mn::CLC => s.c = 0,
            Mn::SEC => s.c = 1,
            Mn::CLI => { s.i = 0; s.i_changed(1); }
            Mn::SEI => s.i = 1,
            Mn::CLV => s.v = 0,
            Mn::CLD => s.d = 0,
            Mn::SED => s.d = 1,
            Mn::BCC => branch!(s.c == 0),
            Mn::BCS => branch!(s.c != 0),
            Mn::BEQ => branch!(s.z != 0),
            Mn::BNE => branch!(s.z == 0),
            Mn::BMI => branch!(s.n != 0),
            Mn::BPL => branch!(s.n == 0),
            Mn::BVC => branch!(s.v == 0),
            Mn::BVS => branch!(s.v != 0),
            Mn::JMP => {
                if mode == Am::Abs {
                    npc = operand;
                } else {
                    // bug 6502: byte tinggi pointer tidak melewati batas halaman
                    let lo = s.read8(operand);
                    let hi = s.read8((operand & 0xFF00) | (operand.wrapping_add(1) & 0x00FF));
                    npc = u16::from(lo) | (u16::from(hi) << 8);
                }
            }
            Mn::JSR => {
                let ret = pc.wrapping_add(2);
                s.push8((ret >> 8) as u8);
                s.push8(ret as u8);
                if let Some(r) = (s.game.call_entry)(s, operand) {
                    // Fungsi native kembali ke alamat tujuan RTS-nya.
                    if r == next {
                        npc = next;
                    } else if is_ours(&frames, &mut nframes, r) {
                        npc = r;
                    } else {
                        return r;
                    }
                } else {
                    if nframes >= MAX_FRAMES {
                        s.fault(pc);
                    }
                    frames[nframes] = next;
                    nframes += 1;
                    npc = operand;
                }
            }
            Mn::RTS => {
                let lo = s.pull8();
                let hi = s.pull8();
                let t = ((u16::from(hi) << 8) | u16::from(lo)).wrapping_add(1);
                if !is_ours(&frames, &mut nframes, t) {
                    return t;
                }
                npc = t;
            }
            Mn::RTI => {
                let p = s.pull8();
                s.set_p(p);
                s.i_changed(0);
                let lo = s.pull8();
                let hi = s.pull8();
                let t = (u16::from(hi) << 8) | u16::from(lo);
                if !is_ours(&frames, &mut nframes, t) {
                    return t;
                }
                npc = t;
            }
            // Opcode tak resmi: kombinasi dua instruksi resmi.
            Mn::NOP => {
                if mode != Am::Imp && mode != Am::Imm {
                    let _ = s.read8(ea);
                }
            }
            Mn::LAX => { s.a = mv!(); s.x = s.a; s.set_nz(s.a); }
            Mn::SAX => s.write8(ea, s.a & s.x),
            Mn::DCP => { let v = s.read8(ea).wrapping_sub(1); s.write8(ea, v); s.cmp(s.a, v); }
            Mn::ISB => { let v = s.read8(ea).wrapping_add(1); s.write8(ea, v); s.sbc(v); }
            Mn::SLO => { let r = s.read8(ea); let v = s.asl(r); s.write8(ea, v); s.a |= v; s.set_nz(s.a); }
            Mn::RLA => { let r = s.read8(ea); let v = s.rol(r); s.write8(ea, v); s.a &= v; s.set_nz(s.a); }
            Mn::SRE => { let r = s.read8(ea); let v = s.lsr(r); s.write8(ea, v); s.a ^= v; s.set_nz(s.a); }
            Mn::RRA => { let r = s.read8(ea); let v = s.ror(r); s.write8(ea, v); s.adc(v); }
            // BRK dan lainnya: tidak didukung
            Mn::BRK | Mn::None => s.fault(pc),
        }
        pc = npc;
    }
}
