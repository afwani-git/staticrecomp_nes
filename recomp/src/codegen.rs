//! Generator kode Rust dari hasil analisis.
//!
//! Keluarannya tiga berkas: `game.rs` (item Rust: fungsi hasil recompile, tabel opcode, ROM),
//! `prg.bin`, dan `chr.bin`. `game.rs` dimaksudkan untuk di-`include!` di dalam sebuah modul
//! pada crate yang bergantung pada `hardware`; ia memakai `include_bytes!` relatif terhadap
//! lokasinya, jadi ketiga berkas harus berdampingan.
//!
//! Fungsi hasil recompile berbentuk `fn func_XXXX<B: Bus>(s: &mut Cpu<B>) -> u16`. Karena Rust
//! tidak punya `goto`, alur di dalam fungsi berupa `loop { match pc { ... } }`: tiap target
//! lompatan adalah satu lengan `match`, dan lompatan berarti mengisi `pc` lalu `continue`.

use std::collections::BTreeSet;
use std::fmt::Write;

use crate::analysis::{Analysis, Function};
use crate::decode::Instruction;
use crate::disasm::operand_text;
use crate::ines::{InesHeader, Mirroring};
use crate::opcodes::{self, Mode};

pub struct GeneratedFile {
    pub name: &'static str,
    pub content: Vec<u8>,
}

impl GeneratedFile {
    /// Isi sebagai teks (untuk berkas Rust).
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.content).expect("berkas ini teks")
    }
}

pub fn generate(an: &Analysis, header: &InesHeader, data: &[u8]) -> Vec<GeneratedFile> {
    let prg_start = header.prg_offset();
    let prg = &data[prg_start..prg_start + header.prg_len()];
    let chr_start = prg_start + header.prg_len();
    let chr = &data[chr_start..chr_start + header.chr_len()];
    vec![
        GeneratedFile { name: "game.rs", content: game_rs(an, header).into_bytes() },
        GeneratedFile { name: "prg.bin", content: prg.to_vec() },
        GeneratedFile { name: "chr.bin", content: chr.to_vec() },
    ]
}

fn game_rs(an: &Analysis, header: &InesHeader) -> String {
    let mirroring = match header.mirroring {
        Mirroring::Horizontal => 0,
        Mirroring::Vertical => 1,
        Mirroring::FourScreen => 2,
    };
    let mut s = String::new();
    s.push_str("// Dihasilkan oleh recomp. Jangan diedit.\n");
    s.push_str("use hardware::cpu::{Bus, Cpu, Game};\n");
    s.push_str("use hardware::interp::{Am, Mn, Op, NO_OP};\n");
    s.push_str("use hardware::nes::Rom;\n\n");
    s.push_str("/// Mirroring: 0 horizontal, 1 vertikal, 2 four-screen. `chr` kosong = cartridge memakai CHR-RAM.\n");
    let _ = writeln!(
        s,
        "pub static ROM: Rom = Rom {{ prg: include_bytes!(\"prg.bin\"), chr: include_bytes!(\"chr.bin\"), mirroring: {mirroring}, mapper: {} }};\n",
        header.mapper
    );
    s.push_str(&ops_table());
    s.push('\n');
    s.push_str(&call_entry(an));
    for f in an.functions.values() {
        s.push('\n');
        s.push_str(&gen_function(f, an));
    }
    s
}

/// Nama Rust untuk `Mode` (varian `Am` di hardware::interp).
fn am(mode: Mode) -> &'static str {
    match mode {
        Mode::Implied => "Imp",
        Mode::Accumulator => "Acc",
        Mode::Immediate => "Imm",
        Mode::ZeroPage => "Zp",
        Mode::ZeroPageX => "Zpx",
        Mode::ZeroPageY => "Zpy",
        Mode::Absolute => "Abs",
        Mode::AbsoluteX => "Abx",
        Mode::AbsoluteY => "Aby",
        Mode::Indirect => "Ind",
        Mode::IndirectX => "Inx",
        Mode::IndirectY => "Iny",
        Mode::Relative => "Rel",
    }
}

/// Tabel opcode untuk interpreter cadangan, dari tabel yang sama dengan generator ini.
fn ops_table() -> String {
    let mut s = String::from("pub static OPS: [Op; 256] = [\n");
    for b in 0..=255u8 {
        match opcodes::lookup(b) {
            Some(o) => {
                let _ = writeln!(
                    s,
                    "    Op {{ mn: Mn::{}, mode: Am::{}, len: {}, cycles: {}, cross: {} }}, // {:02X}",
                    o.mnemonic,
                    am(o.mode),
                    o.len,
                    o.cycles,
                    o.page_cross,
                    b
                );
            }
            None => {
                let _ = writeln!(s, "    NO_OP, // {b:02X}");
            }
        }
    }
    s.push_str("];\n");
    s
}

/// Titik masuk: vektor RESET/NMI/IRQ dibaca dari memori oleh runtime, jadi bekerja untuk bank mana pun.
fn call_entry(an: &Analysis) -> String {
    let mut s = String::new();
    s.push_str("/// Bila `addr` adalah awal fungsi hasil recompile, jalankan dan kembalikan alamat tujuan RTS-nya.\n");
    s.push_str("pub fn call_entry<B: Bus>(s: &mut Cpu<B>, addr: u16) -> Option<u16> {\n    match addr {\n");
    for f in an.functions.values() {
        let _ = writeln!(s, "        0x{:04X} => Some({}(s)),", f.entry, f.name());
    }
    s.push_str("        _ => None,\n    }\n}\n\n");
    s.push_str("/// Tabel yang dibutuhkan runtime untuk menjalankan game ini.\n");
    s.push_str("pub fn game<B: Bus>() -> Game<B> {\n    Game { call_entry: call_entry::<B>, ops: &OPS }\n}\n");
    s
}

struct FnCx<'a> {
    an: &'a Analysis,
    entry: u16,
    body: &'a BTreeSet<u16>,
    labels: BTreeSet<u16>,
}

impl FnCx<'_> {
    /// Lompat ke alamat di dalam fungsi, atau serahkan ke interpreter bila alamat itu tak dianalisis.
    fn goto(&mut self, target: u16) -> String {
        if self.body.contains(&target) {
            self.labels.insert(target);
            format!("pc = 0x{target:04X}; continue;")
        } else {
            format!("return s.interpret(0x{target:04X});")
        }
    }
}

/// Instruksi di RAM bisa berubah saat jalan: periksa isinya, bila beda serahkan ke interpreter.
fn ram_check(ins: &Instruction) -> String {
    if ins.addr >= 0x8000 {
        return String::new();
    }
    let mut bytes = vec![ins.info.opcode];
    if ins.info.len >= 2 {
        bytes.push(ins.operand as u8);
    }
    if ins.info.len == 3 {
        bytes.push((ins.operand >> 8) as u8);
    }
    let conds: Vec<String> = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| format!("s.read8(0x{:04X}) != 0x{b:02X}", ins.addr.wrapping_add(i as u16)))
        .collect();
    format!("if {} {{ return s.interpret(0x{:04X}); }} ", conds.join(" || "), ins.addr)
}

fn falls_through(ins: &Instruction) -> bool {
    !matches!(ins.info.mnemonic, "JMP" | "RTS" | "RTI" | "BRK")
}

fn gen_function(f: &Function, an: &Analysis) -> String {
    let mut cx = FnCx { an, entry: f.entry, body: &f.body, labels: BTreeSet::new() };
    let addrs: Vec<u16> = f.body.iter().copied().collect();
    let name = f.name();

    // Fungsi tanpa badan, atau entri di luar badan: semuanya diserahkan ke interpreter.
    if addrs.is_empty() || !f.body.contains(&f.entry) {
        return format!("pub fn {name}<B: Bus>(s: &mut Cpu<B>) -> u16 {{\n    return s.interpret(0x{:04X});\n}}\n", f.entry);
    }
    cx.labels.insert(f.entry);

    // Satu entri per instruksi, ditambah lompatan penutup (goto/return) untuk instruksi yang jatuh ke
    // alamat yang bukan tetangganya. `fall` = alamat instruksi berikutnya bila ia tetangga langsung.
    struct Line {
        addr: u16,
        text: String,
        fall: Option<u16>,
    }
    let mut lines: Vec<Line> = Vec::new();
    for (i, &addr) in addrs.iter().enumerate() {
        let ins = an.instructions[&addr];
        let star = if ins.info.official { "" } else { "*" };
        let text = format!("{star}{} {}", ins.info.mnemonic, operand_text(&ins));
        let stmts = gen_stmts(&ins, &mut cx);
        let mut line = format!(
            "/* {:04X}  {} */ s.poll(); {}s.trace(0x{addr:04X}); s.cycles += {};{}{}",
            addr,
            text.trim_end(),
            ram_check(&ins),
            ins.info.cycles,
            if stmts.is_empty() { "" } else { " " },
            stmts
        );
        let mut fall = None;
        if falls_through(&ins) {
            let next = ins.next_addr();
            if addrs.get(i + 1) == Some(&next) {
                fall = Some(next);
            } else {
                let g = cx.goto(next);
                let _ = write!(line, " {g}");
            }
        }
        lines.push(Line { addr, text: line, fall });
    }

    // Lengan baru dimulai di tiap target lompatan, di instruksi pertama, dan setelah celah alamat.
    let mut s = format!("pub fn {name}<B: Bus>(s: &mut Cpu<B>) -> u16 {{\n");
    let _ = writeln!(s, "    let mut pc: u16 = 0x{:04X};", f.entry);
    s.push_str("    loop {\n        match pc {\n");
    let mut prev_fall: Option<u16> = None;
    for (i, l) in lines.iter().enumerate() {
        let contiguous = prev_fall == Some(l.addr);
        let starts_arm = i == 0 || cx.labels.contains(&l.addr) || !contiguous;
        if starts_arm {
            if i > 0 {
                if contiguous {
                    let _ = writeln!(s, "                pc = 0x{:04X};", l.addr);
                }
                s.push_str("            }\n");
            }
            let _ = writeln!(s, "            0x{:04X} => {{", l.addr);
        }
        let _ = writeln!(s, "                {}", l.text);
        prev_fall = l.fall;
    }
    s.push_str("            }\n            _ => unreachable!(\"pc di luar fungsi\"),\n        }\n    }\n}\n");
    s
}

fn gen_stmts(ins: &Instruction, cx: &mut FnCx) -> String {
    let info = ins.info;
    let addr = ins.addr;
    match (info.mnemonic, info.mode) {
        ("JMP", Mode::Absolute) => cx.goto(ins.operand),
        ("JMP", _) => {
            let p = ins.operand;
            let head = format!(
                "{{ let p: u16 = 0x{p:04X}; let lo = s.read8(p); let hi = s.read8((p & 0xFF00) | (p.wrapping_add(1) & 0x00FF)); let t = u16::from(lo) | (u16::from(hi) << 8); "
            );
            let fallback = format!("return s.interpret_from(0x{addr:04X}, t);");
            format!("{head}{} }}", dispatch(ins, cx, &fallback))
        }
        ("JSR", _) => {
            let ret = addr.wrapping_add(2);
            let call = if cx.an.functions.contains_key(&ins.operand) {
                format!(
                    "{{ let r = func_{:04X}(s); if r != 0x{:04X} {{ return r; }} }}",
                    ins.operand,
                    ins.next_addr()
                )
            } else {
                format!(
                    "{{ let r = s.interpret(0x{:04X}); if r != 0x{:04X} {{ return r; }} }}",
                    ins.operand,
                    ins.next_addr()
                )
            };
            format!("s.push8(0x{:02X}); s.push8(0x{:02X}); {call}", ret >> 8, ret & 0xFF)
        }
        ("RTS", _) => {
            if cx.an.dynamic.contains_key(&(cx.entry, addr)) {
                format!(
                    "{{ let lo = s.pull8(); let hi = s.pull8(); let t = ((u16::from(hi) << 8) | u16::from(lo)).wrapping_add(1); {} }}",
                    dispatch(ins, cx, "return t;")
                )
            } else {
                "{ let lo = s.pull8(); let hi = s.pull8(); return ((u16::from(hi) << 8) | u16::from(lo)).wrapping_add(1); }"
                    .to_string()
            }
        }
        ("RTI", _) => {
            if cx.an.dynamic.contains_key(&(cx.entry, addr)) {
                format!(
                    "{{ let p = s.pull8(); s.set_p(p); s.i_changed(0); let lo = s.pull8(); let hi = s.pull8(); let t = (u16::from(hi) << 8) | u16::from(lo); {} }}",
                    dispatch(ins, cx, "return t;")
                )
            } else {
                "{ let p = s.pull8(); s.set_p(p); s.i_changed(0); let lo = s.pull8(); let hi = s.pull8(); return (u16::from(hi) << 8) | u16::from(lo); }"
                    .to_string()
            }
        }
        ("BRK", _) => format!("s.fault(0x{addr:04X});"),
        (_, Mode::Relative) => {
            let target = ins.branch_target().unwrap();
            let next = ins.next_addr();
            let cross = (next & 0xFF00 != target & 0xFF00) as u8;
            let cond = match info.mnemonic {
                "BCC" => "s.c == 0",
                "BCS" => "s.c != 0",
                "BEQ" => "s.z != 0",
                "BNE" => "s.z == 0",
                "BMI" => "s.n != 0",
                "BPL" => "s.n == 0",
                "BVC" => "s.v == 0",
                "BVS" => "s.v != 0",
                m => unreachable!("mnemonic cabang tak dikenal: {m}"),
            };
            let g = cx.goto(target);
            format!("if {cond} {{ s.cycles += {}; {g} }}", 1 + cross)
        }
        (m, Mode::Implied | Mode::Accumulator) => implied(m, info.mode).to_string(),
        _ => gen_memory_op(ins),
    }
}

/// `match t { ... }` untuk lompatan dinamis. Alamat yang tidak terdaftar menjalankan `fallback`.
fn dispatch(ins: &Instruction, cx: &mut FnCx, fallback: &str) -> String {
    let targets: BTreeSet<u16> = cx
        .an
        .dynamic
        .get(&(cx.entry, ins.addr))
        .map(|d| d.targets.iter().copied().collect())
        .unwrap_or_default();
    let returns_to = cx.an.dynamic.get(&(cx.entry, ins.addr)).and_then(|d| d.returns_to);
    let mut s = String::from("match t { ");
    for t in targets {
        match returns_to {
            // Target adalah subrutin yang kembali ke `ret`: panggil, lalu lanjut di sana.
            Some(ret) if cx.an.functions.contains_key(&t) => {
                let g = cx.goto(ret);
                let _ = write!(
                    s,
                    "0x{t:04X} => {{ let r = func_{t:04X}(s); if r != 0x{ret:04X} {{ return r; }} {g} }} "
                );
            }
            Some(ret) => {
                let g = cx.goto(ret);
                let _ = write!(
                    s,
                    "0x{t:04X} => {{ let r = s.interpret(0x{t:04X}); if r != 0x{ret:04X} {{ return r; }} {g} }} "
                );
            }
            None => {
                let g = cx.goto(t);
                let _ = write!(s, "0x{t:04X} => {{ {g} }} ");
            }
        }
    }
    let _ = write!(s, "_ => {{ {fallback} }} }}");
    s
}

fn implied(mnemonic: &str, mode: Mode) -> &'static str {
    match (mnemonic, mode) {
        ("NOP", _) => "",
        ("TAX", _) => "s.x = s.a; s.set_nz(s.x);",
        ("TAY", _) => "s.y = s.a; s.set_nz(s.y);",
        ("TXA", _) => "s.a = s.x; s.set_nz(s.a);",
        ("TYA", _) => "s.a = s.y; s.set_nz(s.a);",
        ("TSX", _) => "s.x = s.sp; s.set_nz(s.x);",
        ("TXS", _) => "s.sp = s.x;",
        ("PHA", _) => "s.push8(s.a);",
        ("PLA", _) => "s.a = s.pull8(); s.set_nz(s.a);",
        ("PHP", _) => "{ let p = s.get_p(true); s.push8(p); }",
        ("PLP", _) => "{ let p = s.pull8(); s.set_p(p); } s.i_changed(1);",
        ("INX", _) => "s.x = s.x.wrapping_add(1); s.set_nz(s.x);",
        ("INY", _) => "s.y = s.y.wrapping_add(1); s.set_nz(s.y);",
        ("DEX", _) => "s.x = s.x.wrapping_sub(1); s.set_nz(s.x);",
        ("DEY", _) => "s.y = s.y.wrapping_sub(1); s.set_nz(s.y);",
        ("CLC", _) => "s.c = 0;",
        ("SEC", _) => "s.c = 1;",
        ("CLI", _) => "s.i = 0; s.i_changed(1);",
        ("SEI", _) => "s.i = 1;",
        ("CLV", _) => "s.v = 0;",
        ("CLD", _) => "s.d = 0;",
        ("SED", _) => "s.d = 1;",
        ("ASL", Mode::Accumulator) => "s.a = s.asl(s.a);",
        ("LSR", Mode::Accumulator) => "s.a = s.lsr(s.a);",
        ("ROL", Mode::Accumulator) => "s.a = s.rol(s.a);",
        ("ROR", Mode::Accumulator) => "s.a = s.ror(s.a);",
        (m, _) => unreachable!("mnemonic implied tak dikenal: {m}"),
    }
}

fn gen_memory_op(ins: &Instruction) -> String {
    let info = ins.info;
    let o = ins.operand;
    let lo = o & 0xFF;
    let mut setup = String::new();
    // Ungkapan nilai operand; untuk mode memori, `ea` didefinisikan di `setup`.
    let m = match info.mode {
        Mode::Immediate => format!("0x{lo:02X}u8"),
        mode => {
            let ea = match mode {
                Mode::ZeroPage => format!("0x{lo:02X}"),
                Mode::ZeroPageX => format!("u16::from(0x{lo:02X}u8.wrapping_add(s.x))"),
                Mode::ZeroPageY => format!("u16::from(0x{lo:02X}u8.wrapping_add(s.y))"),
                Mode::Absolute => format!("0x{o:04X}"),
                Mode::AbsoluteX | Mode::AbsoluteY => {
                    let r = if mode == Mode::AbsoluteX { 'x' } else { 'y' };
                    if info.page_cross {
                        setup.push_str(&format!("s.cycles += u64::from(0x{lo:02X}u16 + u16::from(s.{r}) > 0xFF); "));
                    }
                    format!("0x{o:04X}u16.wrapping_add(u16::from(s.{r}))")
                }
                Mode::IndirectX => format!("s.rd16zp(0x{lo:02X}u8.wrapping_add(s.x))"),
                Mode::IndirectY => {
                    setup.push_str(&format!("let b: u16 = s.rd16zp(0x{lo:02X}); "));
                    if info.page_cross {
                        setup.push_str("s.cycles += u64::from((b & 0xFF) + u16::from(s.y) > 0xFF); ");
                    }
                    "b.wrapping_add(u16::from(s.y))".to_string()
                }
                _ => unreachable!("mode tak terduga untuk {}", info.mnemonic),
            };
            setup.push_str(&format!("let ea: u16 = {ea}; "));
            "s.read8(ea)".to_string()
        }
    };
    let body = match info.mnemonic {
        "LDA" => format!("s.a = {m}; s.set_nz(s.a);"),
        "LDX" => format!("s.x = {m}; s.set_nz(s.x);"),
        "LDY" => format!("s.y = {m}; s.set_nz(s.y);"),
        "STA" => "s.write8(ea, s.a);".to_string(),
        "STX" => "s.write8(ea, s.x);".to_string(),
        "STY" => "s.write8(ea, s.y);".to_string(),
        "AND" => format!("let t = {m}; s.a &= t; s.set_nz(s.a);"),
        "ORA" => format!("let t = {m}; s.a |= t; s.set_nz(s.a);"),
        "EOR" => format!("let t = {m}; s.a ^= t; s.set_nz(s.a);"),
        "ADC" => format!("let t = {m}; s.adc(t);"),
        "SBC" => format!("let t = {m}; s.sbc(t);"),
        "CMP" => format!("let t = {m}; s.cmp(s.a, t);"),
        "CPX" => format!("let t = {m}; s.cmp(s.x, t);"),
        "CPY" => format!("let t = {m}; s.cmp(s.y, t);"),
        "BIT" => format!("let t = {m}; s.bit(t);"),
        "INC" => "let v = s.read8(ea).wrapping_add(1); s.write8(ea, v); s.set_nz(v);".to_string(),
        "DEC" => "let v = s.read8(ea).wrapping_sub(1); s.write8(ea, v); s.set_nz(v);".to_string(),
        // Opcode tak resmi: kombinasi dua instruksi resmi.
        "NOP" if info.mode == Mode::Immediate => String::new(),
        "NOP" => "let _ = s.read8(ea);".to_string(),
        "LAX" => format!("let t = {m}; s.a = t; s.x = t; s.set_nz(t);"),
        "SAX" => "s.write8(ea, s.a & s.x);".to_string(),
        "DCP" => "let v = s.read8(ea).wrapping_sub(1); s.write8(ea, v); s.cmp(s.a, v);".to_string(),
        "ISB" => "let v = s.read8(ea).wrapping_add(1); s.write8(ea, v); s.sbc(v);".to_string(),
        "SLO" => "let r = s.read8(ea); let v = s.asl(r); s.write8(ea, v); s.a |= v; s.set_nz(s.a);".to_string(),
        "RLA" => "let r = s.read8(ea); let v = s.rol(r); s.write8(ea, v); s.a &= v; s.set_nz(s.a);".to_string(),
        "SRE" => "let r = s.read8(ea); let v = s.lsr(r); s.write8(ea, v); s.a ^= v; s.set_nz(s.a);".to_string(),
        "RRA" => "let r = s.read8(ea); let v = s.ror(r); s.write8(ea, v); s.adc(v);".to_string(),
        "ASL" => "let r = s.read8(ea); let v = s.asl(r); s.write8(ea, v);".to_string(),
        "LSR" => "let r = s.read8(ea); let v = s.lsr(r); s.write8(ea, v);".to_string(),
        "ROL" => "let r = s.read8(ea); let v = s.rol(r); s.write8(ea, v);".to_string(),
        "ROR" => "let r = s.read8(ea); let v = s.ror(r); s.write8(ea, v);".to_string(),
        other => unreachable!("mnemonic tak dikenal: {other}"),
    };
    if setup.is_empty() && body.is_empty() {
        return String::new(); // NOP immediate
    }
    format!("{{ {setup}{body} }}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze;
    use crate::config::{Config, JumpConfig};

    /// ROM NROM-128 palsu dengan `code` di alamat masing-masing dan RESET di $C000.
    fn make(code: &[(u16, &[u8])]) -> (InesHeader, Vec<u8>) {
        let mut d = vec![0xFFu8; 16 + 0x4000 + 0x2000];
        d[0..4].copy_from_slice(b"NES\x1A");
        d[4] = 1;
        d[5] = 1;
        d[6..16].fill(0);
        let put = |d: &mut Vec<u8>, addr: u16, bytes: &[u8]| {
            let o = 16 + (addr as usize - 0xC000);
            d[o..o + bytes.len()].copy_from_slice(bytes);
        };
        for (a, b) in code {
            put(&mut d, *a, b);
        }
        put(&mut d, 0xFFFA, &0xC000u16.to_le_bytes());
        put(&mut d, 0xFFFC, &0xC000u16.to_le_bytes());
        put(&mut d, 0xFFFE, &0xC000u16.to_le_bytes());
        (InesHeader::parse(&d).unwrap(), d)
    }

    fn game_for(code: &[(u16, &[u8])], cfg: &Config) -> String {
        let (h, d) = make(code);
        let an = analyze(&h, &d, cfg).unwrap();
        game_rs(&an, &h)
    }

    fn line_for<'a>(c: &'a str, needle: &str) -> &'a str {
        // Hanya baris instruksi (bukan tabel opcode, yang juga menyebut nama mnemonic).
        c.lines().find(|l| l.contains("s.poll();") && l.contains(needle)).unwrap_or_else(|| panic!("tidak ada '{needle}' di:\n{c}"))
    }

    #[test]
    fn instruksi_dasar() {
        let c = game_for(&[(0xC000, &[0xA9, 0x42, 0x69, 0x01, 0x85, 0x10, 0xE8, 0x60])], &Config::default());
        assert!(line_for(&c, "LDA #$42").contains("s.cycles += 2; { s.a = 0x42u8; s.set_nz(s.a); }"));
        assert!(line_for(&c, "ADC #$01").contains("let t = 0x01u8; s.adc(t);"));
        assert!(line_for(&c, "STA $10").contains("s.cycles += 3; { let ea: u16 = 0x10; s.write8(ea, s.a); }"));
        assert!(line_for(&c, "INX").contains("s.x = s.x.wrapping_add(1); s.set_nz(s.x);"));
        assert!(line_for(&c, "RTS").contains("return ((u16::from(hi) << 8) | u16::from(lo)).wrapping_add(1);"));
        assert!(line_for(&c, "LDA #$42").contains("s.poll(); s.trace(0xC000);"));
    }

    #[test]
    fn mode_pengalamatan_dan_lintas_halaman() {
        let c = game_for(
            &[(
                0xC000,
                &[
                    0xB5, 0xF8, // LDA $F8,X
                    0xBD, 0xF5, 0xC0, // LDA $C0F5,X (lintas halaman)
                    0x9D, 0xF5, 0xC0, // STA $C0F5,X (tanpa +1)
                    0xB1, 0x40, // LDA ($40),Y
                    0xA1, 0x20, // LDA ($20,X)
                    0x60,
                ],
            )],
            &Config::default(),
        );
        assert!(line_for(&c, "LDA $F8,X").contains("0xF8u8.wrapping_add(s.x)"));
        assert!(line_for(&c, "LDA $C0F5,X").contains("s.cycles += u64::from(0xF5u16 + u16::from(s.x) > 0xFF);"));
        assert!(!line_for(&c, "STA $C0F5,X").contains("> 0xFF"));
        assert!(line_for(&c, "LDA ($40),Y").contains("(b & 0xFF) + u16::from(s.y) > 0xFF"));
        assert!(line_for(&c, "LDA ($20,X)").contains("s.rd16zp(0x20u8.wrapping_add(s.x))"));
    }

    #[test]
    fn cabang_menghitung_siklus_dan_lengan() {
        // BNE mundur ke dirinya sendiri di halaman yang sama; BEQ lintas halaman
        let c = game_for(
            &[
                (0xC000, &[0xD0, 0xFE, 0xF0, 0x7E, 0x60]), // BNE $C000 ; BEQ -> C082
                (0xC082, &[0x60]),
            ],
            &Config::default(),
        );
        assert!(c.contains("0xC000 => {"));
        assert!(line_for(&c, "BNE $C000").contains("if s.z == 0 { s.cycles += 1; pc = 0xC000; continue; }"));
        assert!(line_for(&c, "BEQ $C082").contains("pc = 0xC082; continue;"));
        // tidak ada lengan untuk instruksi yang bukan target lompatan
        assert!(!c.contains("0xC002 => {"));
    }

    #[test]
    fn beda_halaman_menambah_dua() {
        // BEQ di $C0F0: berikutnya $C0F2, target $C100 (halaman lain)
        let c = game_for(
            &[(0xC000, &[0x4C, 0xF0, 0xC0]), (0xC0F0, &[0xF0, 0x0E, 0x60]), (0xC100, &[0x60])],
            &Config::default(),
        );
        assert!(line_for(&c, "BEQ $C100").contains("s.cycles += 2;"));
    }

    #[test]
    fn jsr_push_dan_panggil() {
        let c = game_for(&[(0xC000, &[0x20, 0x10, 0xC0, 0x60]), (0xC010, &[0x60])], &Config::default());
        // alamat kembali yang di-push = alamat byte terakhir JSR = $C002
        assert!(line_for(&c, "JSR $C010").contains(
            "s.push8(0xC0); s.push8(0x02); { let r = func_C010(s); if r != 0xC003 { return r; } }"
        ));
        assert!(c.contains("pub fn func_C010<B: Bus>(s: &mut Cpu<B>) -> u16"));
    }

    #[test]
    fn jmp_indirect_dengan_match_dan_bug_halaman() {
        let cfg = Config {
            jump: vec![JumpConfig { at: 0xC000, targets: vec![0xC010, 0xC020], function: None, returns_to: None }],
            ..Config::default()
        };
        let c = game_for(&[(0xC000, &[0x6C, 0x20, 0x00]), (0xC010, &[0x60]), (0xC020, &[0x60])], &cfg);
        let l = line_for(&c, "JMP ($0020)");
        assert!(l.contains("(p & 0xFF00) | (p.wrapping_add(1) & 0x00FF)"));
        assert!(l.contains("0xC010 => { pc = 0xC010; continue; }"));
        assert!(l.contains("0xC020 => { pc = 0xC020; continue; }"));
        assert!(l.contains("_ => { return s.interpret_from(0xC000, t); }"));
    }

    #[test]
    fn rts_dinamis_membaca_stack() {
        let cfg = Config {
            jump: vec![JumpConfig { at: 0xC006, targets: vec![0xC010], function: None, returns_to: None }],
            ..Config::default()
        };
        let c = game_for(&[(0xC000, &[0xA9, 0xC0, 0x48, 0xA9, 0x0F, 0x48, 0x60]), (0xC010, &[0x60])], &cfg);
        let l = line_for(&c, "RTS");
        assert!(l.contains("s.pull8()") && l.contains("0xC010 =>"), "{l}");
        // alamat yang tidak ada di TOML diperlakukan sebagai RTS biasa
        assert!(l.contains("_ => { return t; }"), "{l}");
    }

    #[test]
    fn instruksi_ram_memeriksa_isinya_saat_jalan() {
        use crate::config::RamCodeConfig;
        let cfg = Config {
            jump: vec![JumpConfig { at: 0xC000, targets: vec![0x0300], function: None, returns_to: None }],
            ram_code: vec![RamCodeConfig { addr: 0x0300, bytes: vec![0xA9, 0xAA, 0x60] }],
            ..Config::default()
        };
        let c = game_for(&[(0xC000, &[0x6C, 0xFF, 0x02])], &cfg);
        let l = line_for(&c, "LDA #$AA");
        assert!(
            l.contains("if s.read8(0x0300) != 0xA9 || s.read8(0x0301) != 0xAA { return s.interpret(0x0300); }"),
            "{l}"
        );
        // instruksi di ROM tidak diperiksa
        assert!(!line_for(&c, "JMP ($02FF)").contains("s.fault(0xC000)"));
    }

    #[test]
    fn jmp_indirect_returns_to_menghasilkan_panggilan() {
        let cfg = Config {
            jump: vec![JumpConfig { at: 0xC000, targets: vec![0xC020], function: None, returns_to: Some(0xC003) }],
            ..Config::default()
        };
        let c = game_for(&[(0xC000, &[0x6C, 0x00, 0x00]), (0xC003, &[0x60]), (0xC020, &[0x60])], &cfg);
        let l = line_for(&c, "JMP ($0000)");
        assert!(
            l.contains("0xC020 => { let r = func_C020(s); if r != 0xC003 { return r; } pc = 0xC003; continue; }"),
            "{l}"
        );
        assert!(c.contains("pub fn func_C020<B: Bus>(s: &mut Cpu<B>) -> u16"));
    }

    #[test]
    fn rti_dinamis_membaca_flag_lalu_alamat() {
        let cfg = Config {
            jump: vec![JumpConfig { at: 0xC000, targets: vec![0xC010], function: None, returns_to: None }],
            ..Config::default()
        };
        let c = game_for(&[(0xC000, &[0x40]), (0xC010, &[0x60])], &cfg);
        let l = line_for(&c, "RTI");
        assert!(l.contains("s.set_p(p);") && l.contains("(u16::from(hi) << 8) | u16::from(lo);"), "{l}");
        assert!(l.contains("0xC010 => { pc = 0xC010; continue; }"), "{l}");
        // tanpa TOML, RTI tetap kembali dari fungsi
        let c = game_for(&[(0xC000, &[0x40])], &Config::default());
        assert!(line_for(&c, "RTI").contains("return (u16::from(hi) << 8) | u16::from(lo);"));
    }

    #[test]
    fn opcode_tak_resmi_dibuat_dari_instruksi_resmi() {
        let c = game_for(
            &[(
                0xC000,
                &[
                    0xA7, 0x10, // LAX $10
                    0x87, 0x11, // SAX $11
                    0xC7, 0x10, // DCP $10
                    0xE7, 0x10, // ISB $10
                    0x07, 0x10, // SLO $10
                    0x27, 0x10, // RLA $10
                    0x47, 0x10, // SRE $10
                    0x67, 0x10, // RRA $10
                    0x1A, // NOP
                    0x80, 0x01, // NOP #$01
                    0x04, 0x10, // NOP $10
                    0x1C, 0xF5, 0xC0, // NOP $C0F5,X
                    0xEB, 0x01, // SBC #$01
                    0x60,
                ],
            )],
            &Config::default(),
        );
        assert!(line_for(&c, "*LAX $10").contains("let t = s.read8(ea); s.a = t; s.x = t; s.set_nz(t);"));
        assert!(line_for(&c, "*SAX $11").contains("s.write8(ea, s.a & s.x);"));
        assert!(line_for(&c, "*DCP $10").contains("s.cmp(s.a, v);"));
        assert!(line_for(&c, "*ISB $10").contains("s.sbc(v);"));
        assert!(line_for(&c, "*SLO $10").contains("let v = s.asl(r); s.write8(ea, v); s.a |= v;"));
        assert!(line_for(&c, "*RLA $10").contains("let v = s.rol(r); s.write8(ea, v); s.a &= v;"));
        assert!(line_for(&c, "*SRE $10").contains("let v = s.lsr(r); s.write8(ea, v); s.a ^= v;"));
        assert!(line_for(&c, "*RRA $10").contains("let v = s.ror(r); s.write8(ea, v); s.adc(v);"));
        assert!(line_for(&c, "*NOP #$01").trim_end().ends_with("s.cycles += 2;"));
        assert!(line_for(&c, "*NOP $10").contains("let _ = s.read8(ea);"));
        // NOP abs,X membaca dan menambah siklus lintas halaman
        let l = line_for(&c, "*NOP $C0F5,X");
        assert!(l.contains("s.cycles += u64::from(0xF5u16 + u16::from(s.x) > 0xFF);") && l.contains("let _ = s.read8(ea);"), "{l}");
        assert!(line_for(&c, "*SBC #$01").contains("let t = 0x01u8; s.sbc(t);"));
    }

    #[test]
    fn brk_dan_jalur_rusak_jadi_fault() {
        let c = game_for(&[(0xC000, &[0xEA, 0x02])], &Config::default());
        // opcode tak didukung di $C001: jatuh dari NOP menuju alamat yang tidak dianalisis,
        // jadi interpreter yang melanjutkan (dan baru berhenti di opcode itu)
        assert!(c.contains("return s.interpret(0xC001);"), "{c}");
        let c = game_for(&[(0xC000, &[0x00])], &Config::default());
        assert!(line_for(&c, "BRK").contains("s.fault(0xC000);"));
    }

    #[test]
    fn entri_bukan_alamat_terendah_dimulai_dari_entri() {
        // RESET di $C010 melompat mundur ke $C000
        let (h, mut d) = make(&[(0xC000, &[0xEA, 0x60]), (0xC010, &[0x4C, 0x00, 0xC0])]);
        let o = 16 + 0x3FFC;
        d[o..o + 2].copy_from_slice(&0xC010u16.to_le_bytes());
        let an = analyze(&h, &d, &Config::default()).unwrap();
        let c = game_rs(&an, &h);
        let body: Vec<&str> = c.lines().skip_while(|l| !l.contains("pub fn func_C010")).collect();
        assert_eq!(body[1], "    let mut pc: u16 = 0xC010;", "{c}");
        // $C000 punya lengan sendiri karena menjadi target JMP
        assert!(body.iter().any(|l| l.trim() == "0xC000 => {"), "{c}");
    }

    #[test]
    fn titik_masuk_rom_dan_berkas() {
        let (h, d) = make(&[(0xC000, &[0x40])]);
        let an = analyze(&h, &d, &Config::default()).unwrap();
        let files = generate(&an, &h, &d);
        let get = |n: &str| files.iter().find(|f| f.name == n).unwrap();
        let c = get("game.rs").text();
        assert!(c.contains("pub fn call_entry<B: Bus>"));
        assert!(c.contains("0xC000 => Some(func_C000(s)),"));
        assert!(c.contains("pub fn game<B: Bus>() -> Game<B>"));
        assert!(c.contains("mirroring: 0, mapper: 0 }"), "{c}");
        assert_eq!(files.len(), 3);
        let prg = &get("prg.bin").content;
        assert_eq!(prg.len(), 16384);
        assert_eq!(&prg[..2], &[0x40, 0xFF]);
        assert_eq!(get("chr.bin").content.len(), 8192);
    }

    #[test]
    fn tabel_opcode_interpreter_lengkap() {
        let t = ops_table();
        // baris tabel: LDA #imm = $A9 (mnemonic LDA, mode immediate, panjang 2, 2 siklus)
        let row = |b: u8| t.lines().find(|l| l.trim_end().ends_with(&format!("// {b:02X}"))).unwrap().to_string();
        assert!(row(0xA9).contains("Op { mn: Mn::LDA, mode: Am::Imm, len: 2, cycles: 2, cross: false }"));
        assert!(row(0xBD).contains("cross: true")); // LDA abs,X
        assert!(row(0x9D).contains("cross: false")); // STA abs,X
        assert!(row(0x02).contains("NO_OP")); // JAM tidak didukung
        assert_eq!(t.lines().filter(|l| l.contains("// ")).count(), 256);
    }
}
