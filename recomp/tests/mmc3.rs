//! MMC3 (mapper 4) lewat hardware native lengkap: program sintetis buatan sendiri mengganti bank PRG
//! dan CHR, menjalankan kode di bank yang bisa berganti (lewat interpreter cadangan), menguji
//! mirroring, dan mengukur laju IRQ scanline.

mod common;

use common::{build_and_run, peek, Asm};
use recomp::analysis::analyze;
use recomp::config::Config;
use recomp::ines::InesHeader;

/// ROM MMC3: 8 bank PRG 8 KB (tiap byte = nomor bank, kecuali kode), 16 bank CHR 1 KB (byte = 0x40 + bank).
fn mmc3_rom() -> (InesHeader, Vec<u8>) {
    let mut d = vec![0u8; 16];
    d[0..4].copy_from_slice(b"NES\x1A");
    d[4] = 4; // 4 x 16 KB PRG
    d[5] = 2; // 2 x 8 KB CHR
    d[6] = 0x40; // mapper 4, mirroring horizontal
    let mut prg = vec![0u8; 4 * 16 * 1024];
    for b in 0..8 {
        prg[b * 8192..(b + 1) * 8192].fill(b as u8);
    }
    // Kode di bank 5 ($A000 saat R7 = 5): LDA #$99; STA $0015; RTS
    let mut banked = Asm::new(0xA100);
    banked.lda(0x99).sta(0x0015).op(&[0x60]);
    let off = 5 * 8192 + 0x100;
    prg[off..off + banked.bytes.len()].copy_from_slice(&banked.bytes);

    let mut put = |addr: u16, bytes: &[u8]| {
        let o = 7 * 8192 + (addr - 0xE000) as usize; // bank 7 = $E000-$FFFF
        prg[o..o + bytes.len()].copy_from_slice(bytes);
    };

    let mut a = Asm::new(0xE000);
    a.op(&[0x78, 0xA2, 0xFF, 0x9A]); // SEI; LDX #$FF; TXS
    // PRG mode 0: $8000 = R6 (bank 3), $A000 = R7 (bank 5), $C000 = kedua dari akhir (6)
    a.bank(6, 3).bank(7, 5);
    a.lda_abs(0x8000).sta(0x0010);
    a.lda_abs(0xA000).sta(0x0011);
    a.lda_abs(0xC000).sta(0x0012);
    // Ganti R6 ke bank 4
    a.bank(6, 4).lda_abs(0x8000).sta(0x0014);
    // Panggil kode di bank 5: tidak dianalisis, jadi dijalankan interpreter cadangan
    a.jsr(0xA100);
    // CHR: R0 = 4 -> $0000 melihat bank 4, $0400 bank 5; baca lewat $2007 (bacaan pertama = buffer basi)
    a.bank(0, 4);
    a.ppu_addr(0x0000).lda_abs(0x2007).lda_abs(0x2007).sta(0x0016);
    a.ppu_addr(0x0400).lda_abs(0x2007).lda_abs(0x2007).sta(0x0017);
    // Mirroring horizontal ($A000 = 1): $2000 = $2400
    a.lda(1).sta(0xA000);
    a.ppu_addr(0x2000).lda(0x5A).sta(0x2007);
    a.ppu_addr(0x2400).lda_abs(0x2007).lda_abs(0x2007).sta(0x0018);
    // Mirroring vertikal ($A000 = 0): $2000 = $2800, $2400 berbeda
    a.lda(0).sta(0xA000);
    a.ppu_addr(0x2000).lda(0x11).sta(0x2007);
    a.ppu_addr(0x2400).lda(0x22).sta(0x2007);
    a.ppu_addr(0x2800).lda_abs(0x2007).lda_abs(0x2007).sta(0x0019);
    // IRQ scanline: sprite di $1000 dan latar di $0000 (tepi A12 naik tiap scanline), penggambaran aktif.
    a.lda(0x08).sta(0x2000).lda(0x18).sta(0x2001);
    a.lda(10).sta(0xC000).sta(0xC001).sta(0xE001); // latch 10, reload, aktifkan IRQ
    a.op(&[0x58]); // CLI
    let spin = a.here();
    a.jmp(spin);
    put(0xE000, &a.bytes);

    // Penangan IRQ: akui, aktifkan lagi, naikkan penghitung 16-bit di $20/$21.
    put(
        0xE100,
        &[
            0x48, // PHA
            0x8D, 0x00, 0xE0, // STA $E000 (akui + nonaktifkan)
            0x8D, 0x01, 0xE0, // STA $E001 (aktifkan lagi)
            0xE6, 0x20, // INC $20
            0xD0, 0x02, // BNE +2
            0xE6, 0x21, // INC $21
            0x68, // PLA
            0x40, // RTI
        ],
    );
    put(0xE200, &[0x40]); // NMI: RTI
    put(0xFFFA, &0xE200u16.to_le_bytes());
    put(0xFFFC, &0xE000u16.to_le_bytes());
    put(0xFFFE, &0xE100u16.to_le_bytes());

    d.extend_from_slice(&prg);
    let mut chr = vec![0u8; 16 * 1024];
    for b in 0..16 {
        chr[b * 1024..(b + 1) * 1024].fill(0x40 + b as u8);
    }
    d.extend_from_slice(&chr);
    (InesHeader::parse(&d).unwrap(), d)
}

#[test]
fn mmc3_bank_chr_mirroring_dan_irq_scanline() {
    let (h, d) = mmc3_rom();
    assert_eq!(h.mapper, 4);
    let an = analyze(&h, &d, &Config::default()).unwrap();
    // Hanya bank tetap yang dianalisis: RESET, NMI, IRQ.
    assert_eq!(an.functions.keys().copied().collect::<Vec<_>>(), vec![0xE000, 0xE100, 0xE200]);

    let text = build_and_run("mmc3", &h, &d, &["10", "10", "11", "12", "14", "15", "16", "17", "18", "19", "20", "21"]);
    let get = |a: u16| peek(&text, a);
    assert_eq!(get(0x10), 3, "$8000 = R6 (bank 3)");
    assert_eq!(get(0x11), 5, "$A000 = R7 (bank 5)");
    assert_eq!(get(0x12), 6, "$C000 = bank kedua dari akhir");
    assert_eq!(get(0x14), 4, "R6 diganti ke bank 4");
    assert_eq!(get(0x15), 0x99, "kode di bank yang bisa berganti dijalankan interpreter");
    assert_eq!(get(0x16), 0x44, "CHR $0000 = bank 4");
    assert_eq!(get(0x17), 0x45, "CHR $0400 = bank 5 (R0 = 2 KB)");
    assert_eq!(get(0x18), 0x5A, "mirroring horizontal: $2000 = $2400");
    assert_eq!(get(0x19), 0x11, "mirroring vertikal: $2000 = $2800");
    // Latch 10 -> periode 11 scanline; ~241 clock per frame, 10 frame -> ~219 IRQ.
    let irqs = get(0x20) + 256 * get(0x21);
    assert!((190..=250).contains(&irqs), "jumlah IRQ dalam 10 frame: {irqs}");
}

/// ROM kecil: penangan NMI melakukan CLI lalu menunggu IRQ MMC3 masuk DI DALAM NMI itu.
/// Kalau hardware memblokir IRQ selama NMI berjalan (bukan hanya lewat flag I), NMI tidak pernah selesai.
fn nmi_irq_rom() -> (InesHeader, Vec<u8>) {
    let mut d = vec![0u8; 16];
    d[0..4].copy_from_slice(b"NES\x1A");
    d[4] = 2;
    d[5] = 1;
    d[6] = 0x40;
    let mut prg = vec![0xFFu8; 2 * 16 * 1024];
    let mut put = |addr: u16, bytes: &[u8]| {
        let o = 3 * 8192 + (addr - 0xE000) as usize; // bank terakhir dari 4 bank 8 KB
        prg[o..o + bytes.len()].copy_from_slice(bytes);
    };
    let mut a = Asm::new(0xE000);
    a.op(&[0x78, 0xA2, 0xFF, 0x9A]); // SEI; LDX #$FF; TXS
    a.lda(10).sta(0xC000).sta(0xC001).sta(0xE001); // latch 10, reload, aktifkan IRQ MMC3
    a.lda(0x88).sta(0x2000); // NMI aktif, sprite di $1000, latar di $0000
    a.lda(0x18).sta(0x2001); // penggambaran aktif
    let spin = a.here();
    a.jmp(spin); // loop utama dengan flag I menyala: IRQ hanya bisa masuk lewat CLI di NMI
    put(0xE000, &a.bytes);
    // NMI: $31 = 0; CLI; tunggu sampai penangan IRQ menaikkan $31; INC $30; RTI
    put(0xE200, &[0xA9, 0x00, 0x8D, 0x31, 0x00, 0x58, 0xAD, 0x31, 0x00, 0xF0, 0xFB, 0xE6, 0x30, 0x40]);
    // IRQ: akui, aktifkan lagi, INC $31
    put(0xE100, &[0x48, 0x8D, 0x00, 0xE0, 0x8D, 0x01, 0xE0, 0xE6, 0x31, 0x68, 0x40]);
    put(0xFFFA, &0xE200u16.to_le_bytes());
    put(0xFFFC, &0xE000u16.to_le_bytes());
    put(0xFFFE, &0xE100u16.to_le_bytes());
    d.extend_from_slice(&prg);
    d.extend_from_slice(&vec![0u8; 8 * 1024]);
    (InesHeader::parse(&d).unwrap(), d)
}

#[test]
fn irq_diterima_di_dalam_penangan_nmi_yang_melakukan_cli() {
    let (h, d) = nmi_irq_rom();
    let text = build_and_run("mmc3_nmi_irq", &h, &d, &["8", "30"]);
    // Tiap frame: NMI mulai, CLI, IRQ masuk, NMI selesai. Dalam 8 frame, NMI selesai berkali-kali.
    assert!(peek(&text, 0x30) >= 5, "NMI selesai {} kali dalam 8 frame, seharusnya >= 5", peek(&text, 0x30));
}
