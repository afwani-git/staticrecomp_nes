use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

use anyhow::{Result, bail};

use crate::config::{Config, JumpConfig};
use crate::decode::{Instruction, decode};
use crate::ines::InesHeader;
use crate::opcodes::Mode;

const VECTORS: [(&str, u16); 3] = [("NMI", 0xFFFA), ("RESET", 0xFFFC), ("IRQ", 0xFFFE)];
/// Berapa instruksi ke belakang dicari `PHA` kedua untuk pola push+RTS.
const PUSH_RTS_LOOKBACK: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynKind {
    /// `JMP (indirect)`.
    IndirectJmp,
    /// `RTS` yang dipakai sebagai lompatan (alamat di-push dulu).
    PushRts,
    /// `RTI` yang dipakai sebagai lompatan (alamat dan flag di-push dulu).
    /// Hanya dikenali lewat TOML.
    PushRti,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicJump {
    pub at: u16,
    pub kind: DynKind,
    /// Terdeteksi oleh heuristik push+RTS.
    pub heuristic: bool,
    /// Ditandai atau diisi lewat TOML.
    pub configured: bool,
    pub targets: Vec<u16>,
    /// Bila ada, target adalah subrutin yang kembali ke alamat ini (lihat `returns_to` di TOML).
    pub returns_to: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub entry: u16,
    /// Asal entri: "RESET", "NMI", "IRQ", "JSR", atau "config".
    pub roles: Vec<String>,
    /// Alamat semua instruksi yang terjangkau dari entri tanpa melewati JSR.
    pub body: BTreeSet<u16>,
}

impl Function {
    pub fn name(&self) -> String {
        format!("func_{:04X}", self.entry)
    }
}

/// Rentang alamat CPU (inklusif) yang tidak dipakai instruksi mana pun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: u16,
    pub end: u16,
}

impl Range {
    pub fn len(&self) -> usize {
        (self.end - self.start) as usize + 1
    }
}

#[derive(Debug, Default)]
pub struct Analysis {
    pub functions: BTreeMap<u16, Function>,
    /// Semua instruksi yang terjangkau.
    pub instructions: BTreeMap<u16, Instruction>,
    /// Lompatan dinamis per (entri fungsi, alamat instruksi): alamat yang sama
    /// bisa dinamis di satu fungsi dan biasa di fungsi lain.
    pub dynamic: BTreeMap<(u16, u16), DynamicJump>,
    /// Instruksi yang dimiliki lebih dari satu fungsi: alamat -> entri fungsi.
    pub shared: BTreeMap<u16, Vec<u16>>,
    pub unreachable: Vec<Range>,
    pub warnings: Vec<String>,
}

impl Analysis {
    fn warn(&mut self, msg: String) {
        if !self.warnings.contains(&msg) {
            self.warnings.push(msg);
        }
    }
}

struct Source<'a> {
    header: &'a InesHeader,
    data: &'a [u8],
    /// Byte kode RAM yang dideklarasikan lewat TOML.
    ram: BTreeMap<u16, u8>,
}

impl Source<'_> {
    fn byte(&self, addr: u16) -> Option<u8> {
        self.ram.get(&addr).copied().or_else(|| self.header.read_prg(self.data, addr))
    }

    fn mapped(&self, addr: u16) -> bool {
        self.byte(addr).is_some()
    }

    /// Sampai 3 byte mulai dari `addr`, berhenti di ujung PRG-ROM atau kode RAM.
    fn bytes(&self, addr: u16) -> Vec<u8> {
        (0..3u32)
            .map_while(|k| u16::try_from(addr as u32 + k).ok())
            .map_while(|a| self.byte(a))
            .collect()
    }
}

pub fn analyze(header: &InesHeader, data: &[u8], config: &Config) -> Result<Analysis> {
    if !header.mapper_supported() {
        bail!("{}", crate::ines::unsupported_mapper_message(header.mapper));
    }
    if header.prg_banks == 0 {
        bail!("ROM tidak punya PRG-ROM");
    }
    let mut ram = BTreeMap::new();
    for r in &config.ram_code {
        if r.addr as usize + r.bytes.len() > 0x8000 {
            bail!("ram_code di ${:04X} ({} byte) keluar dari RAM ($0000-$7FFF)", r.addr, r.bytes.len());
        }
        for (i, &b) in r.bytes.iter().enumerate() {
            ram.insert(r.addr + i as u16, b);
        }
    }
    let src = Source { header, data, ram };
    let mut an = Analysis::default();

    let mut cfg_jumps: BTreeMap<u16, Vec<&JumpConfig>> = BTreeMap::new();
    for j in &config.jump {
        cfg_jumps.entry(j.at).or_default().push(j);
    }

    // Titik awal: vektor dan entri dari TOML.
    let mut roles: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    let mut pending: Vec<u16> = Vec::new();
    let has_fixed = header.analysis_region().1 > 0;
    if !has_fixed {
        an.warn(format!(
            "mapper {} tidak punya bank PRG yang pasti tetap: seluruh kode dijalankan interpreter",
            header.mapper
        ));
    }
    for (name, vec_addr) in VECTORS.iter().filter(|_| has_fixed).copied() {
        match (header.read_prg(data, vec_addr), header.read_prg(data, vec_addr + 1)) {
            (Some(lo), Some(hi)) => {
                let entry = u16::from_le_bytes([lo, hi]);
                if src.mapped(entry) {
                    add_role(&mut roles, entry, name);
                    pending.push(entry);
                } else {
                    an.warn(format!("vektor {name} menuju ${entry:04X} di luar PRG-ROM"));
                }
            }
            _ => an.warn(format!("vektor {name} tidak terbaca")),
        }
    }
    for e in &config.entry {
        if src.mapped(e.addr) {
            add_role(&mut roles, e.addr, "config");
            pending.push(e.addr);
        } else {
            an.warn(format!("entri TOML ${:04X} di luar PRG-ROM", e.addr));
        }
    }

    // Telusuri fungsi satu per satu; JSR menambah fungsi baru.
    let mut done: BTreeSet<u16> = BTreeSet::new();
    let mut pos = 0;
    while pos < pending.len() {
        let entry = pending[pos];
        pos += 1;
        if !done.insert(entry) {
            continue;
        }
        let (body, calls) = explore(entry, &src, &cfg_jumps, &mut an);
        an.functions.insert(
            entry,
            Function { entry, roles: roles.get(&entry).cloned().unwrap_or_default(), body },
        );
        for (caller, target) in calls {
            if !src.mapped(target) {
                an.warn(format!("JSR di ${caller:04X} menuju ${target:04X} di luar PRG-ROM"));
                continue;
            }
            add_role(&mut roles, target, "JSR");
            pending.push(target);
        }
    }
    // Peran yang baru muncul setelah fungsi dibuat (mis. entri juga jadi target JSR).
    for (entry, f) in an.functions.iter_mut() {
        f.roles = roles.get(entry).cloned().unwrap_or_default();
    }

    // Validasi isi TOML terhadap hasil analisis.
    for (&at, _) in &cfg_jumps {
        match an.instructions.get(&at) {
            None => an.warn(format!("TOML: ${at:04X} tidak pernah terjangkau")),
            Some(_) if !an.dynamic.keys().any(|&(_, a)| a == at) => an.warn(format!(
                "TOML: ${at:04X} bukan JMP (indirect), RTI, atau RTS dinamis (RTS dinamis bila pola push+RTS terdeteksi atau `function` diisi)"
            )),
            Some(_) => {}
        }
    }

    // Instruksi yang dimiliki lebih dari satu fungsi.
    let mut owners: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    for f in an.functions.values() {
        for &a in &f.body {
            owners.entry(a).or_default().push(f.entry);
        }
    }
    an.shared = owners.into_iter().filter(|(_, v)| v.len() > 1).collect();

    an.unreachable = unreachable_ranges(header, &an.instructions);
    Ok(an)
}

fn add_role(roles: &mut BTreeMap<u16, Vec<String>>, entry: u16, role: &str) {
    let r = roles.entry(entry).or_default();
    if !r.iter().any(|x| x == role) {
        r.push(role.to_string());
    }
}

/// Telusuri satu fungsi dari `entry`. Mengembalikan isi fungsi dan daftar
/// (alamat JSR, tujuan) yang ditemukan.
fn explore(
    entry: u16,
    src: &Source,
    cfg_jumps: &BTreeMap<u16, Vec<&JumpConfig>>,
    an: &mut Analysis,
) -> (BTreeSet<u16>, Vec<(u16, u16)>) {
    let mut body = BTreeSet::new();
    let mut calls = Vec::new();
    // Instruksi sebelumnya di jalur kedatangan pertama; dipakai heuristik push+RTS.
    let mut prev: HashMap<u16, u16> = HashMap::new();
    let mut stack: Vec<(u16, Option<u16>)> = vec![(entry, None)];

    while let Some((addr, from)) = stack.pop() {
        if body.contains(&addr) {
            continue;
        }
        let bytes = src.bytes(addr);
        if bytes.is_empty() {
            let f = from.unwrap_or(entry);
            an.warn(format!("alur dari ${f:04X} keluar dari PRG-ROM di ${addr:04X}"));
            continue;
        }
        let ins = match decode(&bytes, addr) {
            Ok(i) => i,
            Err(e) => {
                an.warn(e.to_string());
                continue;
            }
        };
        if let Some(f) = from {
            prev.insert(addr, f);
        }
        body.insert(addr);
        an.instructions.insert(addr, ins);

        let next = ins.next_addr();
        match (ins.info.mnemonic, ins.info.mode) {
            ("JMP", Mode::Absolute) => stack.push((ins.operand, Some(addr))),
            ("JMP", _) => {
                mark_dynamic(an, cfg_jumps, entry, addr, DynKind::IndirectJmp, false, &mut stack, &mut calls);
            }
            ("JSR", _) => {
                calls.push((addr, ins.operand));
                stack.push((next, Some(addr)));
            }
            ("RTS", _) => {
                let heuristic = looks_like_push_rts(&prev, &an.instructions, addr);
                mark_dynamic(an, cfg_jumps, entry, addr, DynKind::PushRts, heuristic, &mut stack, &mut calls);
            }
            ("RTI", _) => {
                mark_dynamic(an, cfg_jumps, entry, addr, DynKind::PushRti, false, &mut stack, &mut calls);
            }
            ("BRK", _) => an.warn(format!("BRK di ${addr:04X}: jalur dihentikan")),
            (_, Mode::Relative) => {
                stack.push((ins.branch_target().unwrap(), Some(addr)));
                stack.push((next, Some(addr)));
            }
            _ => stack.push((next, Some(addr))),
        }
    }
    (body, calls)
}

/// Catat lompatan dinamis (bila memang dinamis di fungsi `entry`) dan telusuri
/// target dari TOML.
fn mark_dynamic(
    an: &mut Analysis,
    cfg_jumps: &BTreeMap<u16, Vec<&JumpConfig>>,
    entry: u16,
    at: u16,
    kind: DynKind,
    heuristic: bool,
    stack: &mut Vec<(u16, Option<u16>)>,
    calls: &mut Vec<(u16, u16)>,
) {
    let mut targets: Vec<u16> = Vec::new();
    let mut configured = false;
    let mut forced = false;
    let mut returns_to = None;
    for j in cfg_jumps.get(&at).into_iter().flatten() {
        if j.function.is_none_or(|f| f == entry) {
            configured = true;
            forced |= j.function == Some(entry);
            targets.extend(&j.targets);
            returns_to = returns_to.or(j.returns_to);
        }
    }
    if kind == DynKind::PushRts && !heuristic && !forced {
        return; // RTS biasa di fungsi ini
    }
    if kind == DynKind::PushRti && !configured {
        return; // RTI biasa (akhir handler interrupt)
    }
    let d = an.dynamic.entry((entry, at)).or_insert_with(|| DynamicJump {
        at,
        kind,
        heuristic: false,
        configured,
        targets: targets.clone(),
        returns_to,
    });
    d.heuristic |= heuristic;
    match returns_to {
        // Target = subrutin sendiri; eksekusi lanjut di alamat kembali.
        Some(ret) => {
            for t in targets {
                calls.push((at, t));
            }
            stack.push((ret, Some(at)));
        }
        None => {
            for t in targets {
                stack.push((t, Some(at)));
            }
        }
    }
}

/// Pola `... PHA ... PHA RTS`: `RTS` didahului `PHA`, dan ada `PHA` lain dalam
/// beberapa instruksi sebelumnya tanpa pop di antaranya.
fn looks_like_push_rts(
    prev: &HashMap<u16, u16>,
    instrs: &BTreeMap<u16, Instruction>,
    rts: u16,
) -> bool {
    let mnemonic = |a: u16| instrs.get(&a).map(|i| i.info.mnemonic);
    let Some(&last) = prev.get(&rts) else { return false };
    if mnemonic(last) != Some("PHA") {
        return false;
    }
    let mut cur = last;
    for _ in 0..PUSH_RTS_LOOKBACK {
        let Some(&q) = prev.get(&cur) else { return false };
        match mnemonic(q) {
            Some("PHA") => return true,
            Some("PLA" | "PLP" | "JSR" | "RTS" | "RTI") | None => return false,
            _ => cur = q,
        }
    }
    false
}

fn unreachable_ranges(header: &InesHeader, instrs: &BTreeMap<u16, Instruction>) -> Vec<Range> {
    // Hanya wilayah yang bisa dianalisis statis (seluruh PRG untuk NROM, bank tetap untuk MMC3).
    let (region_start, region_len) = header.analysis_region();
    let mut used = vec![false; region_len];
    for i in instrs.values() {
        for k in 0..i.len() {
            if let Some(ix) = header.prg_index(i.addr.wrapping_add(k)) {
                if ix >= region_start && ix - region_start < region_len {
                    used[ix - region_start] = true;
                }
            }
        }
    }
    // Enam byte terakhir adalah vektor, bukan "tak terjangkau".
    let limit = region_len.saturating_sub(6);
    let base = header.prg_base() as usize;
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for ix in 0..=limit {
        let free = ix < limit && !used[ix];
        match (free, start) {
            (true, None) => start = Some(ix),
            (false, Some(s)) => {
                out.push(Range { start: (base + s) as u16, end: (base + ix - 1) as u16 });
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Laporan teks untuk CLI.
pub fn render(an: &Analysis, header: &InesHeader) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "Fungsi             : {}", an.functions.len());
    let _ = writeln!(s, "Instruksi terjangkau: {}", an.instructions.len());

    let _ = writeln!(s, "\nFungsi:");
    for f in an.functions.values() {
        let _ = writeln!(
            s,
            "  ${:04X}  {:<10} {:>5} instruksi  [{}]",
            f.entry,
            f.name(),
            f.body.len(),
            f.roles.join(", ")
        );
    }

    let _ = writeln!(s, "\nLompatan dinamis: {}", an.dynamic.len());
    for (&(func, _), d) in &an.dynamic {
        let what = match (d.kind, d.heuristic, d.configured) {
            (DynKind::IndirectJmp, _, _) => "JMP (indirect)",
            (DynKind::PushRts, true, _) => "RTS (kemungkinan push+RTS)",
            (DynKind::PushRts, false, _) => "RTS (ditandai di TOML)",
            (DynKind::PushRti, _, _) => "RTI (ditandai di TOML)",
        };
        let t = if d.targets.is_empty() {
            "PERLU DIISI di TOML".to_string()
        } else {
            d.targets.iter().map(|t| format!("${t:04X}")).collect::<Vec<_>>().join(", ")
        };
        let _ = writeln!(s, "  ${:04X}  (di func_{func:04X})  {what}  -> {t}", d.at);
    }

    if !an.shared.is_empty() {
        let _ = writeln!(s, "\nInstruksi dimiliki >1 fungsi (akan diduplikasi): {}", an.shared.len());
        for (a, owners) in an.shared.iter().take(10) {
            let o: Vec<String> = owners.iter().map(|e| format!("${e:04X}")).collect();
            let _ = writeln!(s, "  ${a:04X}  di fungsi {}", o.join(", "));
        }
        if an.shared.len() > 10 {
            let _ = writeln!(s, "  ... dan {} lainnya", an.shared.len() - 10);
        }
    }

    let _ = writeln!(s, "\nPeringatan: {}", an.warnings.len());
    for w in &an.warnings {
        let _ = writeln!(s, "  {w}");
    }

    let total: usize = an.unreachable.iter().map(Range::len).sum();
    let prg = header.analysis_region().1;
    if prg == 0 {
        let _ = writeln!(s, "\nROM tak terjangkau: tidak berlaku (tidak ada bank PRG tetap yang dianalisis)");
    } else {
        let _ = writeln!(
            s,
            "\nROM tak terjangkau: {total} dari {prg} byte wilayah yang dianalisis ({:.1}%), tidak termasuk 6 byte vektor",
            total as f64 * 100.0 / prg as f64
        );
    }
    for r in &an.unreachable {
        let _ = writeln!(s, "  ${:04X}-${:04X}  ({} byte)", r.start, r.end, r.len());
    }
    let _ = writeln!(
        s,
        "\nCatatan: kode yang dijalankan dari RAM atau menimpa dirinya sendiri tidak terdeteksi."
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EntryConfig, JumpConfig, RamCodeConfig};

    const RTI_AT: u16 = 0xFF00;

    /// ROM NROM-128 palsu. Sisa PRG diisi $FF (opcode tak resmi),
    /// ada `RTI` di $FF00 sebagai handler NMI/IRQ bawaan.
    fn make_vec(code: &[(u16, &[u8])], reset: u16, nmi: u16, irq: u16) -> (InesHeader, Vec<u8>) {
        let mut d = vec![0xFFu8; 16 + 0x4000 + 0x2000];
        d[0..4].copy_from_slice(b"NES\x1A");
        d[4] = 1;
        d[5] = 1;
        d[6] = 0;
        d[7] = 0;
        d[8..16].fill(0);
        let put = |d: &mut Vec<u8>, addr: u16, bytes: &[u8]| {
            let o = 16 + (addr as usize - 0xC000);
            d[o..o + bytes.len()].copy_from_slice(bytes);
        };
        put(&mut d, RTI_AT, &[0x40]);
        for (a, b) in code {
            put(&mut d, *a, b);
        }
        put(&mut d, 0xFFFA, &nmi.to_le_bytes());
        put(&mut d, 0xFFFC, &reset.to_le_bytes());
        put(&mut d, 0xFFFE, &irq.to_le_bytes());
        (InesHeader::parse(&d).unwrap(), d)
    }

    fn make(code: &[(u16, &[u8])]) -> (InesHeader, Vec<u8>) {
        make_vec(code, 0xC000, RTI_AT, RTI_AT)
    }

    fn run(code: &[(u16, &[u8])], cfg: &Config) -> Analysis {
        let (h, d) = make(code);
        analyze(&h, &d, cfg).unwrap()
    }

    fn body(an: &Analysis, entry: u16) -> Vec<u16> {
        an.functions[&entry].body.iter().copied().collect()
    }

    fn cfg_jump(at: u16, targets: &[u16]) -> Config {
        Config { jump: vec![JumpConfig { at, targets: targets.to_vec(), function: None, returns_to: None }], ..Config::default() }
    }

    #[test]
    fn program_lurus_dari_reset() {
        let an = run(&[(0xC000, &[0xA9, 0x01, 0x60])], &Config::default());
        assert_eq!(body(&an, 0xC000), vec![0xC000, 0xC002]);
        // NMI dan IRQ menunjuk ke entri yang sama -> satu fungsi dengan dua peran
        assert_eq!(an.functions.len(), 2);
        assert_eq!(an.functions[&RTI_AT].roles, vec!["NMI", "IRQ"]);
        assert_eq!(an.functions[&0xC000].roles, vec!["RESET"]);
        assert!(an.warnings.is_empty(), "{:?}", an.warnings);
        assert!(an.dynamic.is_empty());
    }

    #[test]
    fn cabang_menelusuri_dua_arah() {
        // BEQ +3: jatuh ke $C002 (RTS) atau lompat ke $C005 (RTS)
        let an = run(&[(0xC000, &[0xF0, 0x03, 0x60]), (0xC005, &[0x60])], &Config::default());
        assert_eq!(body(&an, 0xC000), vec![0xC000, 0xC002, 0xC005]);
    }

    #[test]
    fn jsr_membuat_fungsi_baru() {
        let an = run(
            &[(0xC000, &[0x20, 0x10, 0xC0, 0x60]), (0xC010, &[0xA9, 0x00, 0x60])],
            &Config::default(),
        );
        assert_eq!(body(&an, 0xC000), vec![0xC000, 0xC003]);
        assert_eq!(body(&an, 0xC010), vec![0xC010, 0xC012]);
        assert_eq!(an.functions[&0xC010].roles, vec!["JSR"]);
    }

    #[test]
    fn jmp_abs_tetap_dalam_fungsi() {
        let an = run(&[(0xC000, &[0x4C, 0x10, 0xC0]), (0xC010, &[0x60])], &Config::default());
        assert_eq!(body(&an, 0xC000), vec![0xC000, 0xC010]);
        assert!(!an.functions.contains_key(&0xC010));
    }

    #[test]
    fn jmp_indirect_ditandai_dinamis() {
        let an = run(&[(0xC000, &[0x6C, 0x00, 0x02])], &Config::default());
        let d = &an.dynamic[&(0xC000, 0xC000)];
        assert_eq!(d.kind, DynKind::IndirectJmp);
        assert!(d.targets.is_empty());
        assert!(render(&an, &make(&[]).0).contains("PERLU DIISI"));
    }

    #[test]
    fn target_toml_ikut_ditelusuri() {
        let an = run(
            &[(0xC000, &[0x6C, 0x00, 0x02]), (0xC020, &[0x60]), (0xC030, &[0xEA, 0x60])],
            &cfg_jump(0xC000, &[0xC020, 0xC030]),
        );
        assert_eq!(an.dynamic[&(0xC000, 0xC000)].targets, vec![0xC020, 0xC030]);
        assert_eq!(body(&an, 0xC000), vec![0xC000, 0xC020, 0xC030, 0xC031]);
        assert!(an.warnings.is_empty(), "{:?}", an.warnings);
    }

    #[test]
    fn pola_push_rts_ditandai() {
        // LDA #$C0; PHA; LDA #$0F; PHA; RTS
        let an = run(
            &[(0xC000, &[0xA9, 0xC0, 0x48, 0xA9, 0x0F, 0x48, 0x60])],
            &Config::default(),
        );
        let d = &an.dynamic[&(0xC000, 0xC006)];
        assert_eq!(d.kind, DynKind::PushRts);
        assert!(d.heuristic && !d.configured);
    }

    #[test]
    fn rts_biasa_tidak_ditandai() {
        // PHA lalu PLA lalu RTS; satu PHA lalu RTS; fungsi kosong
        for code in [&[0xA9, 0x01, 0x48, 0x68, 0x60][..], &[0x48, 0x60], &[0x60]] {
            let an = run(&[(0xC000, code)], &Config::default());
            assert!(an.dynamic.is_empty(), "{code:02X?}");
        }
    }

    #[test]
    fn push_rts_dengan_target_toml() {
        let code: &[(u16, &[u8])] = &[
            (0xC000, &[0xA9, 0xC0, 0x48, 0xA9, 0x0F, 0x48, 0x60]),
            (0xC020, &[0x60]),
        ];
        let an = run(code, &cfg_jump(0xC006, &[0xC020]));
        assert_eq!(body(&an, 0xC000).last(), Some(&0xC020));
        assert!(an.dynamic[&(0xC000, 0xC006)].configured && an.dynamic[&(0xC000, 0xC006)].heuristic);
    }

    #[test]
    fn rts_biasa_dipaksa_dinamis_dengan_function() {
        let mut cfg = cfg_jump(0xC002, &[0xC050]);
        cfg.jump[0].function = Some(0xC000);
        let an = run(&[(0xC000, &[0xA9, 0x01, 0x60]), (0xC050, &[0x60])], &cfg);
        let d = &an.dynamic[&(0xC000, 0xC002)];
        assert!(d.configured && !d.heuristic);
        assert!(body(&an, 0xC000).contains(&0xC050));
        // tanpa `function`, config saja tidak membuat RTS biasa jadi dinamis
        let an = run(
            &[(0xC000, &[0xA9, 0x01, 0x60]), (0xC050, &[0x60])],
            &cfg_jump(0xC002, &[0xC050]),
        );
        assert!(an.dynamic.is_empty());
        assert!(an.warnings.iter().any(|w| w.contains("bukan JMP")), "{:?}", an.warnings);
    }

    #[test]
    fn jmp_indirect_dengan_returns_to_memanggil_target_sebagai_fungsi() {
        let mut cfg = cfg_jump(0xC000, &[0xC020]);
        cfg.jump[0].returns_to = Some(0xC003);
        let an = run(
            &[(0xC000, &[0x6C, 0x00, 0x00]), (0xC003, &[0x60]), (0xC020, &[0xEA, 0x60])],
            &cfg,
        );
        // target jadi fungsi sendiri, bukan bagian dari fungsi pemanggil
        assert_eq!(body(&an, 0xC020), vec![0xC020, 0xC021]);
        assert_eq!(an.functions[&0xC020].roles, vec!["JSR"]);
        // alamat kembali tetap bagian dari fungsi pemanggil
        assert_eq!(body(&an, 0xC000), vec![0xC000, 0xC003]);
        assert_eq!(an.dynamic[&(0xC000, 0xC000)].returns_to, Some(0xC003));
    }

    #[test]
    fn rti_dinamis_hanya_lewat_toml() {
        let code: &[(u16, &[u8])] = &[(0xC000, &[0x40]), (0xC010, &[0x60])];
        let an = run(code, &Config::default());
        assert!(an.dynamic.is_empty());
        let an = run(code, &cfg_jump(0xC000, &[0xC010]));
        let d = &an.dynamic[&(0xC000, 0xC000)];
        assert_eq!(d.kind, DynKind::PushRti);
        assert!(body(&an, 0xC000).contains(&0xC010));
    }

    #[test]
    fn rts_yang_sama_dinamis_di_satu_fungsi_dan_biasa_di_fungsi_lain() {
        // RESET: push alamat lalu jatuh ke RTS di $C006.
        // Fungsi lain ($C010) memanggil $C006 lewat JSR, jadi di sana RTS itu biasa.
        let cfg = Config {
            jump: vec![JumpConfig { at: 0xC006, targets: vec![0xC020], function: None, returns_to: None }],
            entry: vec![EntryConfig { addr: 0xC010 }],
            ..Config::default()
        };
        let an = run(
            &[
                (0xC000, &[0xA9, 0xC0, 0x48, 0xA9, 0x1F, 0x48, 0x60]),
                (0xC010, &[0x20, 0x06, 0xC0, 0x60]),
                (0xC020, &[0x60]),
            ],
            &cfg,
        );
        assert!(an.dynamic.contains_key(&(0xC000, 0xC006)));
        assert!(!an.dynamic.contains_key(&(0xC006, 0xC006)), "{:?}", an.dynamic.keys());
        assert!(!an.dynamic.contains_key(&(0xC010, 0xC006)));
        assert!(body(&an, 0xC000).contains(&0xC020));
        assert!(!body(&an, 0xC006).contains(&0xC020));
    }

    #[test]
    fn kode_ram_dari_toml_ikut_dianalisis() {
        let cfg = Config {
            jump: vec![JumpConfig { at: 0xC000, targets: vec![0x0300], function: None, returns_to: None }],
            ram_code: vec![RamCodeConfig { addr: 0x0300, bytes: vec![0xA9, 0xAA, 0x60] }],
            ..Config::default()
        };
        let an = run(&[(0xC000, &[0x6C, 0xFF, 0x02])], &cfg);
        assert_eq!(body(&an, 0xC000), vec![0x0300, 0x0302, 0xC000]);
        assert!(an.warnings.is_empty(), "{:?}", an.warnings);
        // RAM tidak dihitung sebagai ROM tak terjangkau
        assert_eq!(an.unreachable[0].start, 0xC003);
        // tanpa ram_code, target RAM dilaporkan di luar PRG-ROM
        let an = run(&[(0xC000, &[0x6C, 0xFF, 0x02])], &cfg_jump(0xC000, &[0x0300]));
        assert!(an.warnings.iter().any(|w| w.contains("$0300")), "{:?}", an.warnings);
    }

    #[test]
    fn ram_code_di_luar_ram_ditolak() {
        let (h, d) = make(&[]);
        let cfg = Config {
            ram_code: vec![RamCodeConfig { addr: 0x7FFF, bytes: vec![0xEA, 0xEA] }],
            ..Config::default()
        };
        assert!(analyze(&h, &d, &cfg).is_err());
    }

    #[test]
    fn toml_ke_instruksi_salah_jadi_peringatan() {
        let an = run(&[(0xC000, &[0xEA, 0x60])], &cfg_jump(0xC000, &[0xC010]));
        assert!(an.warnings.iter().any(|w| w.contains("bukan JMP")), "{:?}", an.warnings);
        let an = run(&[(0xC000, &[0x60])], &cfg_jump(0xC999, &[0xC010]));
        assert!(an.warnings.iter().any(|w| w.contains("tidak pernah terjangkau")));
    }

    #[test]
    fn entri_tambahan_dari_toml() {
        let cfg = Config { jump: vec![], entry: vec![EntryConfig { addr: 0xC800 }], ..Config::default() };
        let an = run(&[(0xC000, &[0x60]), (0xC800, &[0xEA, 0x60])], &cfg);
        assert_eq!(body(&an, 0xC800), vec![0xC800, 0xC801]);
        assert_eq!(an.functions[&0xC800].roles, vec!["config"]);
    }

    #[test]
    fn rentang_tak_terjangkau() {
        let an = run(&[(0xC000, &[0xA9, 0x01, 0x60])], &Config::default());
        // kode di C000-C002 dan RTI di FF00; vektor FFFA-FFFF dikecualikan
        assert_eq!(
            an.unreachable,
            vec![
                Range { start: 0xC003, end: 0xFEFF },
                Range { start: 0xFF01, end: 0xFFF9 },
            ]
        );
    }

    #[test]
    fn opcode_tak_resmi_jadi_peringatan() {
        let an = run(&[(0xC000, &[0xEA, 0x02])], &Config::default());
        assert_eq!(body(&an, 0xC000), vec![0xC000]);
        assert!(an.warnings.iter().any(|w| w.contains("$02")), "{:?}", an.warnings);
    }

    #[test]
    fn jsr_dan_jmp_keluar_rom_jadi_peringatan() {
        let an = run(
            &[(0xC000, &[0x20, 0x00, 0x02, 0x4C, 0x00, 0x03])],
            &Config::default(),
        );
        // setelah JSR analisis lanjut ke JMP; keduanya dilaporkan
        assert!(body(&an, 0xC000).contains(&0xC003));
        assert!(an.warnings.iter().any(|w| w.contains("$0200")), "{:?}", an.warnings);
        assert!(an.warnings.iter().any(|w| w.contains("$0300")), "{:?}", an.warnings);
        assert!(!an.functions.contains_key(&0x0200));
    }

    #[test]
    fn brk_menghentikan_jalur_dengan_peringatan() {
        let an = run(&[(0xC000, &[0x00, 0xEA])], &Config::default());
        assert_eq!(body(&an, 0xC000), vec![0xC000]);
        assert!(an.warnings.iter().any(|w| w.contains("BRK")));
    }

    #[test]
    fn ekor_bersama_terdeteksi() {
        let an = run(
            &[
                (0xC000, &[0x20, 0x10, 0xC0, 0x20, 0x20, 0xC0, 0x60]),
                (0xC010, &[0x4C, 0x30, 0xC0]),
                (0xC020, &[0x4C, 0x30, 0xC0]),
                (0xC030, &[0x60]),
            ],
            &Config::default(),
        );
        assert_eq!(an.shared.get(&0xC030), Some(&vec![0xC010, 0xC020]));
        assert_eq!(an.shared.len(), 1);
    }

    #[test]
    fn vektor_ke_luar_rom_jadi_peringatan() {
        let (h, d) = make_vec(&[(0xC000, &[0x60])], 0xC000, 0x0200, RTI_AT);
        let an = analyze(&h, &d, &Config::default()).unwrap();
        assert!(an.warnings.iter().any(|w| w.contains("NMI")), "{:?}", an.warnings);
        assert!(an.functions.contains_key(&0xC000));
    }

    #[test]
    fn mapper_tak_didukung_ditolak() {
        let (_, mut d) = make(&[]);
        d[6] = 0x50; // mapper 5 (MMC5)
        let h = InesHeader::parse(&d).unwrap();
        assert!(analyze(&h, &d, &Config::default()).is_err());
    }

    #[test]
    fn mapper_tanpa_bank_tetap_diinterpretasi_seluruhnya() {
        let (_, mut d) = make(&[(0xC000, &[0x60])]);
        d[6] = 0x70; // mapper 7 (AxROM): seluruh 32 KB berganti
        let h = InesHeader::parse(&d).unwrap();
        let an = analyze(&h, &d, &Config::default()).unwrap();
        assert!(an.functions.is_empty());
        assert!(an.warnings.iter().any(|w| w.contains("seluruh kode dijalankan interpreter")), "{:?}", an.warnings);
        // vektor tidak terbaca: tidak ada peringatan palsu tentang vektor
        assert!(!an.warnings.iter().any(|w| w.contains("vektor")), "{:?}", an.warnings);
        assert!(render(&an, &h).contains("tidak berlaku"));
    }

    #[test]
    fn render_memuat_bagian_utama() {
        let (h, d) = make(&[(0xC000, &[0x60])]);
        let an = analyze(&h, &d, &Config::default()).unwrap();
        let text = render(&an, &h);
        for needle in ["RESET", "func_C000", "Lompatan dinamis: 0", "ROM tak terjangkau"] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
    }
}
