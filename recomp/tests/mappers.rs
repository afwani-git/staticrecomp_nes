//! Mapper 1 (MMC1), 2 (UxROM), dan 3 (CNROM) lewat hardware native lengkap, dengan program sintetis
//! buatan sendiri.
//!  - MMC1: tidak ada wilayah PRG yang pasti tetap, jadi seluruh kode dijalankan interpreter.
//!  - UxROM: bank terakhir tetap di-recompile; bank yang berganti diinterpretasi.
//!  - CNROM: PRG tetap (seperti NROM), hanya CHR yang berganti.

mod common;

use common::{build_and_run, peek, Asm};
use recomp::analysis::analyze;
use recomp::config::Config;
use recomp::ines::InesHeader;

/// Header iNES + isi PRG dan CHR yang diberikan.
fn ines(mapper: u8, prg: &[u8], chr: &[u8]) -> (InesHeader, Vec<u8>) {
    let mut d = vec![0u8; 16];
    d[0..4].copy_from_slice(b"NES\x1A");
    d[4] = (prg.len() / 16384) as u8;
    d[5] = (chr.len() / 8192) as u8;
    d[6] = (mapper & 0x0F) << 4;
    d[7] = mapper & 0xF0;
    d.extend_from_slice(prg);
    d.extend_from_slice(chr);
    (InesHeader::parse(&d).unwrap(), d)
}

/// PRG 4 bank 16 KB, tiap bank berisi 0xA0 + nomor banknya.
fn marked_prg() -> Vec<u8> {
    let mut prg = vec![0u8; 4 * 16384];
    for b in 0..4 {
        prg[b * 16384..(b + 1) * 16384].fill(0xA0 + b as u8);
    }
    prg
}

fn put(prg: &mut [u8], bank: usize, offset: usize, bytes: &[u8]) {
    let o = bank * 16384 + offset;
    prg[o..o + bytes.len()].copy_from_slice(bytes);
}

/// Vektor di akhir bank terakhir: NMI dan IRQ ke `handler`, RESET ke `reset`.
fn vectors(prg: &mut [u8], bank: usize, base: u16, reset: u16, handler: u16) {
    put(prg, bank, 0x3FFA, &handler.to_le_bytes());
    put(prg, bank, 0x3FFC, &reset.to_le_bytes());
    put(prg, bank, 0x3FFE, &handler.to_le_bytes());
    let _ = base;
}

/// Satu tulisan MMC1 lengkap: lima bit, LSB dulu.
fn mmc1(a: &mut Asm, addr: u16, value: u8) {
    for i in 0..5 {
        a.lda((value >> i) & 1).sta(addr);
    }
}

#[test]
fn mmc1_bank_prg_chr_ram_dan_mirroring_seluruhnya_lewat_interpreter() {
    let mut prg = marked_prg();
    let mut a = Asm::new(0xC000);
    a.op(&[0x78, 0xA2, 0xFF, 0x9A]); // SEI; LDX #$FF; TXS
    a.lda(0x80).sta(0x8000); // reset register geser
    mmc1(&mut a, 0x8000, 0x0E); // kontrol: vertikal, mode PRG 3 ($C000 tetap), CHR 8 KB
    mmc1(&mut a, 0xE000, 1); // bank PRG 1 di $8000
    a.lda_abs(0x8000).sta(0x0010);
    a.jsr(0x8100); // kode di bank 1
    mmc1(&mut a, 0xE000, 2);
    a.lda_abs(0x8000).sta(0x0012);
    // CHR-RAM
    a.ppu_addr(0x0000).lda(0x5A).sta(0x2007);
    a.ppu_addr(0x0000).lda_abs(0x2007).lda_abs(0x2007).sta(0x0014);
    // Mirroring vertikal: $2000 = $2800
    a.ppu_addr(0x2000).lda(0x11).sta(0x2007);
    a.ppu_addr(0x2800).lda_abs(0x2007).lda_abs(0x2007).sta(0x0015);
    // Mirroring horizontal (kontrol 0x0F): $2000 = $2400
    mmc1(&mut a, 0x8000, 0x0F);
    a.ppu_addr(0x2000).lda(0x22).sta(0x2007);
    a.ppu_addr(0x2400).lda_abs(0x2007).lda_abs(0x2007).sta(0x0016);
    let spin = a.here();
    a.jmp(spin);
    put(&mut prg, 3, 0, &a.bytes);
    put(&mut prg, 3, 0x100, &[0x40]); // RTI
    let mut banked = Asm::new(0x8100);
    banked.lda(0x77).sta(0x0011).op(&[0x60]);
    put(&mut prg, 1, 0x100, &banked.bytes);
    vectors(&mut prg, 3, 0xC000, 0xC000, 0xC100);

    let (h, d) = ines(1, &prg, &[]);
    // Tidak ada wilayah tetap: tidak ada fungsi yang di-recompile.
    assert!(analyze(&h, &d, &Config::default()).unwrap().functions.is_empty());
    let text = build_and_run("map_mmc1", &h, &d, &["3", "10", "11", "12", "14", "15", "16"]);
    assert_eq!(peek(&text, 0x10), 0xA1, "bank 1 di $8000");
    assert_eq!(peek(&text, 0x11), 0x77, "kode di bank yang berganti dijalankan");
    assert_eq!(peek(&text, 0x12), 0xA2, "bank 2 di $8000");
    assert_eq!(peek(&text, 0x14), 0x5A, "CHR-RAM");
    assert_eq!(peek(&text, 0x15), 0x11, "mirroring vertikal");
    assert_eq!(peek(&text, 0x16), 0x22, "mirroring horizontal");
}

#[test]
fn uxrom_bank_tetap_direcompile_dan_bank_berganti_diinterpretasi() {
    let mut prg = marked_prg();
    let mut a = Asm::new(0xC000);
    a.op(&[0x78, 0xA2, 0xFF, 0x9A]);
    a.lda(2).sta(0x8000); // bank 2 di $8000
    a.lda_abs(0x8000).sta(0x0010);
    a.jsr(0x8100); // kode di bank 2
    a.lda(1).sta(0x8000);
    a.lda_abs(0x8000).sta(0x0012);
    a.lda_abs(0xC000).sta(0x0013); // bank terakhir tetap: byte pertamanya adalah SEI ($78)
    let spin = a.here();
    a.jmp(spin);
    put(&mut prg, 3, 0, &a.bytes);
    put(&mut prg, 3, 0x100, &[0x40]);
    let mut banked = Asm::new(0x8100);
    banked.lda(0x66).sta(0x0011).op(&[0x60]);
    put(&mut prg, 2, 0x100, &banked.bytes);
    vectors(&mut prg, 3, 0xC000, 0xC000, 0xC100);

    let (h, d) = ines(2, &prg, &[]);
    let an = analyze(&h, &d, &Config::default()).unwrap();
    assert!(an.functions.contains_key(&0xC000), "RESET di bank tetap di-recompile");
    let text = build_and_run("map_uxrom", &h, &d, &["3", "10", "11", "12", "13"]);
    assert_eq!(peek(&text, 0x10), 0xA2, "bank 2 di $8000");
    assert_eq!(peek(&text, 0x11), 0x66, "kode di bank berganti dijalankan interpreter");
    assert_eq!(peek(&text, 0x12), 0xA1, "bank 1 di $8000");
    assert_eq!(peek(&text, 0x13), 0x78, "$C000 tetap bank terakhir");
}

#[test]
fn cnrom_bank_chr_berganti_prg_tetap() {
    // PRG 32 KB (seperti NROM-256), CHR 4 bank 8 KB, tiap bank berisi 0xC0 + nomor banknya.
    let mut prg = vec![0xEAu8; 2 * 16384];
    let mut a = Asm::new(0x8000);
    a.op(&[0x78, 0xA2, 0xFF, 0x9A]);
    a.lda(2).sta(0x8000); // CHR bank 2
    a.ppu_addr(0x0000).lda_abs(0x2007).lda_abs(0x2007).sta(0x0010);
    a.lda(1).sta(0x8000); // CHR bank 1
    a.ppu_addr(0x0000).lda_abs(0x2007).lda_abs(0x2007).sta(0x0011);
    a.lda_abs(0x8001).sta(0x0012); // PRG tidak berubah: byte kedua program ($A2 = LDX)
    let spin = a.here();
    a.jmp(spin);
    prg[..a.bytes.len()].copy_from_slice(&a.bytes);
    prg[0x100] = 0x40; // RTI
    prg[0x7FFA..0x7FFC].copy_from_slice(&0x8100u16.to_le_bytes());
    prg[0x7FFC..0x7FFE].copy_from_slice(&0x8000u16.to_le_bytes());
    prg[0x7FFE..0x8000].copy_from_slice(&0x8100u16.to_le_bytes());
    let mut chr = vec![0u8; 4 * 8192];
    for b in 0..4 {
        chr[b * 8192..(b + 1) * 8192].fill(0xC0 + b as u8);
    }

    let (h, d) = ines(3, &prg, &chr);
    let an = analyze(&h, &d, &Config::default()).unwrap();
    assert!(an.functions.contains_key(&0x8000), "CNROM: PRG tetap, semuanya di-recompile");
    let text = build_and_run("map_cnrom", &h, &d, &["3", "10", "11", "12"]);
    assert_eq!(peek(&text, 0x10), 0xC2, "CHR bank 2");
    assert_eq!(peek(&text, 0x11), 0xC1, "CHR bank 1");
    assert_eq!(peek(&text, 0x12), 0xA2, "PRG tetap");
}
