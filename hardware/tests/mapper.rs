//! Tes unit mapper. PRG/CHR sintetis: tiap bank 8 KB PRG / 1 KB CHR berisi nomor banknya sendiri di
//! setiap byte, jadi bank yang terlihat bisa dibaca langsung.

use hardware::mapper::Mapper;

/// 256 KB PRG: 32 bank 8 KB.
fn prg() -> &'static [u8] {
    let v: Vec<u8> = (0..32u8).flat_map(|b| std::iter::repeat_n(b, 8192)).collect();
    Box::leak(v.into_boxed_slice())
}

/// 128 KB CHR: 128 bank 1 KB.
fn chr() -> &'static [u8] {
    let v: Vec<u8> = (0..128u8).flat_map(|b| std::iter::repeat_n(b, 1024)).collect();
    Box::leak(v.into_boxed_slice())
}

fn mk(id: u8, prg: &'static [u8], chr: &'static [u8], mirroring: u8) -> Mapper {
    Mapper::new(id, prg, chr, mirroring).unwrap_or_else(|| panic!("mapper {id} seharusnya didukung"))
}

fn mmc3(mirroring: u8) -> Mapper {
    mk(4, prg(), chr(), mirroring)
}

/// Tulis register bank MMC3: pilih `reg` (dengan bit mode) lalu isi `value`.
fn bank(m: &mut Mapper, reg: u8, mode_bits: u8, value: u8) {
    m.cpu_write(0x8000, reg | mode_bits);
    m.cpu_write(0x8001, value);
}

/// Satu clock IRQ: A12 rendah cukup lama, lalu naik.
fn scanline(m: &mut Mapper, now: &mut u64) {
    *now += 20;
    m.a12(false, *now);
    *now += 20;
    m.a12(true, *now);
}

/// Tulis 5 bit (LSB dulu) ke register geser MMC1.
fn mmc1(m: &mut Mapper, addr: u16, value: u8) {
    for i in 0..5 {
        m.cpu_write(addr, (value >> i) & 1);
    }
}

#[test]
fn mapper_tak_didukung_ditolak() {
    assert!(Mapper::new(5, prg(), chr(), 0).is_none(), "MMC5 ditolak");
}

#[test]
fn nrom_16kb_tampil_dua_kali_dan_32kb_langsung() {
    let mut p16 = vec![0u8; 16384];
    let mut p32 = vec![0u8; 32768];
    p16[0] = 0xA1;
    p16[16383] = 0xA2;
    p32[0] = 0xB1;
    p32[16384] = 0xB2;
    p32[32767] = 0xB3;
    let (p16, p32): (&'static [u8], &'static [u8]) = (Box::leak(p16.into()), Box::leak(p32.into()));
    let c = &chr()[..8192];

    let m = mk(0, p16, c, 1);
    assert!(m.cpu_read(0x8000) == 0xA1 && m.cpu_read(0xC000) == 0xA1, "NROM-128 mirror");
    assert!(m.cpu_read(0xBFFF) == 0xA2 && m.cpu_read(0xFFFF) == 0xA2, "NROM-128 ujung");
    let mut m = mk(0, p32, c, 1);
    assert!(m.cpu_read(0x8000) == 0xB1 && m.cpu_read(0xC000) == 0xB2 && m.cpu_read(0xFFFF) == 0xB3, "NROM-256 langsung");
    m.cpu_write(0x8000, 0x55); // ROM: tulis diabaikan
    assert_eq!(m.cpu_read(0x8000), 0xB1, "NROM tulis diabaikan");
}

#[test]
fn mmc3_prg_mode_0_dan_1() {
    // Mode 0: $8000=R6, $A000=R7, $C000=kedua dari akhir, $E000=terakhir.
    let mut m = mmc3(1);
    bank(&mut m, 6, 0, 3);
    bank(&mut m, 7, 0, 5);
    assert!(m.cpu_read(0x8000) == 3 && m.cpu_read(0x9FFF) == 3, "PRG mode 0: $8000 = R6");
    assert!(m.cpu_read(0xA000) == 5 && m.cpu_read(0xBFFF) == 5, "PRG mode 0: $A000 = R7");
    assert!(m.cpu_read(0xC000) == 30 && m.cpu_read(0xDFFF) == 30, "PRG mode 0: $C000 = kedua dari akhir");
    assert!(m.cpu_read(0xE000) == 31 && m.cpu_read(0xFFFF) == 31, "$E000 selalu bank terakhir");

    // Mode 1 (bit 6 di $8000): $8000 dan $C000 bertukar.
    m.cpu_write(0x8000, 0x46); // pilih R6, mode 1
    assert_eq!(m.cpu_read(0x8000), 30, "PRG mode 1: $8000 = kedua dari akhir");
    assert_eq!(m.cpu_read(0xC000), 3, "PRG mode 1: $C000 = R6");
    assert!(m.cpu_read(0xA000) == 5 && m.cpu_read(0xE000) == 31, "mode 1: $A000 dan $E000 tetap");
}

#[test]
fn mmc3_dua_bit_atas_diabaikan_dan_register_berselang() {
    let mut m = mmc3(1);
    m.cpu_write(0x9FFE, 6); // genap = pilih bank
    m.cpu_write(0x9FFF, 0xC3); // ganjil = data; bank 0xC3 & 0x3F = 3
    assert_eq!(m.cpu_read(0x8000), 3, "R6 diabaikan dua bit atas, register di alamat berselang");
    assert!(m.prg_bank_8k(0x8000) == 3 && m.prg_bank_8k(0xE000) == 31, "pengamatan bank PRG");
}

#[test]
fn mmc3_chr_tanpa_dan_dengan_inversi() {
    let mut m = mmc3(1);
    bank(&mut m, 0, 0, 5); // ganjil -> 4,5
    bank(&mut m, 1, 0, 10);
    bank(&mut m, 2, 0, 20);
    bank(&mut m, 3, 0, 21);
    bank(&mut m, 4, 0, 22);
    bank(&mut m, 5, 0, 23);
    assert!(m.chr_bank_1k(0x0000) == 4 && m.chr_bank_1k(0x0400) == 5, "CHR R0 = 2 KB di $0000");
    assert!(m.chr_bank_1k(0x0800) == 10 && m.chr_bank_1k(0x0C00) == 11, "CHR R1 = 2 KB di $0800");
    assert!(m.chr_bank_1k(0x1000) == 20 && m.chr_bank_1k(0x1400) == 21, "CHR R2/R3 di $1000/$1400");
    assert!(m.chr_bank_1k(0x1800) == 22 && m.chr_bank_1k(0x1C00) == 23, "CHR R4/R5 di $1800/$1C00");
    assert!(m.chr_read(0x0000) == 4 && m.chr_read(0x03FF) == 4 && m.chr_read(0x1C00) == 23, "isi CHR dari bank yang benar");

    // Inversi A12 (bit 7): R2-R5 di $0000, R0 di $1000, R1 di $1800.
    m.cpu_write(0x8000, 0x80);
    assert!(m.chr_bank_1k(0x0000) == 20 && m.chr_bank_1k(0x0C00) == 23, "inversi: R2-R5 di $0000");
    assert!(m.chr_bank_1k(0x1000) == 4 && m.chr_bank_1k(0x1400) == 5, "inversi: R0 di $1000");
    assert!(m.chr_bank_1k(0x1800) == 10 && m.chr_bank_1k(0x1C00) == 11, "inversi: R1 di $1800");
}

#[test]
fn mmc3_mirroring() {
    // $A000 bit 0 (0 = vertikal, 1 = horizontal); four-screen dari header tak bisa diubah.
    let mut m = mmc3(0);
    m.cpu_write(0xA000, 0);
    assert_eq!(m.mirroring(), 1, "$A000 = 0 -> vertikal");
    m.cpu_write(0xA000, 1);
    assert_eq!(m.mirroring(), 0, "$A000 = 1 -> horizontal");
    let mut m = mmc3(2);
    m.cpu_write(0xA000, 1);
    assert_eq!(m.mirroring(), 2, "four-screen tidak bisa diubah");
}

#[test]
fn mmc3_irq() {
    let mut now = 0u64;

    // Latch 3 -> reload, lalu IRQ menyala pada clock ke-4 (penghitung mencapai 0).
    let mut m = mmc3(1);
    m.cpu_write(0xC000, 3);
    m.cpu_write(0xC001, 0);
    m.cpu_write(0xE001, 0);
    for _ in 0..3 {
        scanline(&mut m, &mut now);
    }
    assert!(!m.irq_pending(), "belum ada IRQ setelah 3 clock");
    scanline(&mut m, &mut now);
    assert!(m.irq_pending(), "IRQ menyala pada clock ke-4");
    assert!(m.irq_pending(), "IRQ level: tetap menyala sampai diakui");
    m.cpu_write(0xE000, 0); // akui + nonaktifkan
    assert!(!m.irq_pending() && !m.irq_enabled(), "$E000 mengakui dan menonaktifkan");
    // Penghitung memuat ulang latch saat mencapai 0, jadi periodenya latch + 1 clock.
    m.cpu_write(0xE001, 0);
    for _ in 0..3 {
        scanline(&mut m, &mut now);
    }
    assert!(!m.irq_pending(), "periode IRQ: belum setelah 3 clock");
    scanline(&mut m, &mut now);
    assert!(m.irq_pending(), "periode IRQ: menyala di clock ke-4 berikutnya");

    // IRQ nonaktif: penghitung tetap berjalan tapi IRQ tidak menyala.
    let mut m = mmc3(1);
    m.cpu_write(0xC000, 1);
    m.cpu_write(0xC001, 0);
    for _ in 0..6 {
        scanline(&mut m, &mut now);
    }
    assert!(!m.irq_pending(), "IRQ nonaktif tidak menyala");

    // Latch 0: setelah reload, tiap clock menghasilkan IRQ.
    let mut m = mmc3(1);
    m.cpu_write(0xC000, 0);
    m.cpu_write(0xC001, 0);
    m.cpu_write(0xE001, 0);
    scanline(&mut m, &mut now);
    assert!(m.irq_pending(), "latch 0: IRQ di clock pertama");
}

#[test]
fn mmc3_filter_a12() {
    // Tepi naik setelah A12 rendah terlalu singkat (< 9 dot) tidak meng-clock.
    let mut now = 0u64;
    let mut m = mmc3(1);
    m.cpu_write(0xC000, 5);
    m.cpu_write(0xC001, 0);
    m.cpu_write(0xE001, 0);
    now += 100;
    m.a12(true, now); // A12 mulai tinggi (belum rendah cukup lama sejak awal -> clock pertama)
    now += 20;
    m.a12(false, now);
    now += 8;
    m.a12(true, now); // rendah hanya 8 dot: diabaikan
    now += 20;
    m.a12(false, now);
    now += 9;
    m.a12(true, now); // rendah 9 dot: meng-clock
    m.cpu_write(0xC000, 0);
    m.cpu_write(0xC001, 0); // reload ke 0
    now += 20;
    m.a12(false, now);
    now += 8;
    m.a12(true, now); // diabaikan: tanpa clock, penghitung tetap 0 dan tidak ada IRQ baru
    assert!(!m.irq_pending(), "tepi naik yang terlalu singkat tidak meng-clock");
    now += 20;
    m.a12(false, now);
    now += 9;
    m.a12(true, now); // clock: reload 0 -> IRQ
    assert!(m.irq_pending(), "tepi naik yang cukup lama meng-clock");
    m.cpu_write(0xE000, 0);
}

#[test]
fn chr_ram_baca_tulis_dan_chr_rom_tidak_bisa_ditulis() {
    let mut m = mk(4, prg(), &[], 1);
    m.chr_write(0x0123, 0x77);
    assert_eq!(m.chr_read(0x0123), 0x77, "CHR-RAM baca-tulis");
    let c = chr();
    let mut m = mk(0, &prg()[..16384], &c[..8192], 1);
    m.chr_write(0x0123, 0x77);
    assert_eq!(m.chr_read(0x0123), c[0x123], "CHR-ROM tidak bisa ditulis");
}

// Mapper lain. PRG: tiap bank 8 KB berisi nomor banknya; CHR: bank 1 KB berisi nomornya, jadi bank
// 8 KB ke-n dimulai dengan byte 8n, bank 4 KB ke-n dimulai dengan 4n.

#[test]
fn cnrom_prg_tetap_chr_berganti() {
    let mut m = mk(3, &prg()[..32768], &chr()[..32768], 1);
    assert!(m.cpu_read(0x8000) == 0 && m.cpu_read(0xE000) == 3, "CNROM: PRG tetap");
    assert_eq!(m.chr_read(0x0000), 0, "CNROM: bank CHR awal 0");
    m.cpu_write(0x8000, 2);
    assert!(m.chr_read(0x0000) == 16 && m.chr_read(0x1FFF) == 23, "CNROM: bank CHR 2 = 1 KB nomor 16..23");
    m.cpu_write(0xFFFF, 5); // 4 bank saja: 5 % 4 = 1
    assert_eq!(m.chr_read(0x0000), 8, "CNROM: nomor bank di-modulo jumlah bank");
    assert_eq!(m.cpu_read(0x8000), 0, "CNROM: menulis tidak mengubah PRG");
}

#[test]
fn uxrom_8000_berganti_c000_tetap() {
    let mut m = mk(2, prg(), &[], 1);
    assert!(m.cpu_read(0x8000) == 0 && m.cpu_read(0xC000) == 30, "UxROM: awal");
    m.cpu_write(0x8000, 3); // bank 16 KB ke-3 = bank 8 KB nomor 6 dan 7
    assert!(m.cpu_read(0x8000) == 6 && m.cpu_read(0xA000) == 7, "UxROM: $8000 = bank 16 KB terpilih");
    assert!(m.cpu_read(0xC000) == 30 && m.cpu_read(0xFFFF) == 31, "UxROM: $C000 tetap");
    m.chr_write(0x0100, 0x5A);
    assert_eq!(m.chr_read(0x0100), 0x5A, "UxROM: CHR-RAM");
}

#[test]
fn axrom_32kb_berganti_dan_satu_layar() {
    let mut m = mk(7, prg(), &[], 0);
    assert_eq!(m.cpu_read(0x8000), 0, "AxROM: bank awal 0");
    m.cpu_write(0x8000, 0x13); // bank 3, nametable B
    assert!(m.cpu_read(0x8000) == 12 && m.cpu_read(0xFFFF) == 15, "AxROM: bank 32 KB ke-3");
    assert_eq!(m.mirroring(), 4, "AxROM: bit 4 = nametable B");
    m.cpu_write(0x8000, 0x02);
    assert!(m.mirroring() == 3 && m.cpu_read(0x8000) == 8, "AxROM: nametable A, bank 2");
}

#[test]
fn gxrom_color_dreams_dan_nina() {
    // GxROM (66): PRG = bit 4-5, CHR = bit 0-1.
    let mut m = mk(66, prg(), &chr()[..32768], 1);
    m.cpu_write(0x8000, 0x21); // PRG bank 2, CHR bank 1
    assert!(m.cpu_read(0x8000) == 8 && m.chr_read(0x0000) == 8, "GxROM: PRG bit 4-5, CHR bit 0-1");

    // Color Dreams (11): PRG = bit 0-1, CHR = bit 4-7.
    let mut m = mk(11, prg(), &chr()[..32768], 1);
    m.cpu_write(0x8000, 0x32); // PRG bank 2, CHR bank 3
    assert!(m.cpu_read(0x8000) == 8 && m.chr_read(0x0000) == 24, "Color Dreams: PRG bit 0-1, CHR bit 4-7");

    // NINA-003/006 (79): register di $4100-$5FFF (A8 = 1, A14-A13 bebas); PRG = bit 3, CHR = bit 0-2.
    let mut m = mk(79, prg(), &chr()[..32768], 1);
    m.cpu_write(0x4100, 0x09); // PRG bank 1, CHR bank 1
    assert!(m.cpu_read(0x8000) == 4 && m.chr_read(0x0000) == 8, "NINA: PRG bit 3, CHR bit 0-2");
    m.cpu_write(0x4000, 0x00); // A8 = 0: bukan register
    assert_eq!(m.cpu_read(0x8000), 4, "NINA: alamat tanpa A8 diabaikan");
    m.cpu_write(0x8000, 0x00); // tulis di $8000 bukan register mapper ini
    assert_eq!(m.cpu_read(0x8000), 4, "NINA: tulis di $8000 diabaikan");
}

#[test]
fn camerica_bank_16kb_lewat_c000() {
    let mut m = mk(71, prg(), &[], 1);
    m.cpu_write(0xC000, 2);
    assert!(m.cpu_read(0x8000) == 4 && m.cpu_read(0xC000) == 30, "Camerica: bank 16 KB di $8000");
    m.cpu_write(0x9000, 0x10);
    assert_eq!(m.mirroring(), 4, "Camerica: satu layar lewat $9000");
}

#[test]
fn mmc1_register_geser_prg_mirroring_dan_chr() {
    // Keadaan awal: mode PRG 3 ($8000 berganti, $C000 tetap terakhir).
    let mut m = mk(1, prg(), &chr()[..32768], 1);
    assert_eq!(m.cpu_read(0xC000), 30, "MMC1: awal, $C000 = bank 16 KB terakhir");
    mmc1(&mut m, 0xE000, 5);
    assert!(m.cpu_read(0x8000) == 10 && m.cpu_read(0xA000) == 11, "MMC1: PRG bank 5 (16 KB) di $8000");
    assert_eq!(m.cpu_read(0xC000), 30, "MMC1 mode 3: $C000 tetap");
    // Tulisan ke-5 menentukan register dari alamatnya; 4 tulisan pertama ke alamat lain tidak masalah.
    for _ in 0..4 {
        m.cpu_write(0x8000, 1);
    }
    m.cpu_write(0xE000, 0); // tulisan ke-5 di $E000: nilai 0b01111 = 15
    assert_eq!(m.cpu_read(0x8000), 30, "MMC1: lima bit 1,1,1,1,0 = nilai 15 -> bank 8 KB nomor 30");
    // Bit 7 mereset register geser: tulisan separuh jalan dibuang.
    m.cpu_write(0xE000, 1);
    m.cpu_write(0xE000, 1);
    m.cpu_write(0xE000, 0x80);
    mmc1(&mut m, 0xE000, 2);
    assert_eq!(m.cpu_read(0x8000), 4, "MMC1: bit 7 mereset register geser (PRG bank 2)");

    // Mode PRG 2 ($8000 tetap bank 0, $C000 berganti): kontrol = 0b01000 | mirroring 3 = 0x0B.
    mmc1(&mut m, 0x8000, 0x0B);
    mmc1(&mut m, 0xE000, 3);
    assert!(m.cpu_read(0x8000) == 0 && m.cpu_read(0xC000) == 6, "MMC1 mode 2: $8000 tetap, $C000 = bank 3");
    // Mode PRG 0/1 (32 KB): kontrol = mirroring 3 = 0x03; bit terendah bank diabaikan.
    mmc1(&mut m, 0x8000, 0x03);
    mmc1(&mut m, 0xE000, 5); // bank 5 -> 4 (genap)
    assert!(m.cpu_read(0x8000) == 8 && m.cpu_read(0xC000) == 10, "MMC1 mode 32 KB: bank 16 KB ke-4 dan ke-5");

    // Mirroring dari register kontrol: 0 satu layar A, 1 satu layar B, 2 vertikal, 3 horizontal.
    for (mode, want) in [(0u8, 3), (1, 4), (2, 1), (3, 0)] {
        mmc1(&mut m, 0x8000, mode);
        assert_eq!(m.mirroring(), want, "MMC1: mirroring dari kontrol {mode}");
    }

    // CHR: mode 4 KB (bit 4 kontrol) memakai dua register; mode 8 KB mengabaikan bit terendah.
    mmc1(&mut m, 0x8000, 0x1F); // kontrol: CHR 4 KB, PRG mode 3
    mmc1(&mut m, 0xA000, 6); // CHR0 = bank 4 KB 6
    mmc1(&mut m, 0xC000, 9); // CHR1 = bank 4 KB 9
    // CHR 32 KB = 8 bank 4 KB: CHR0 = 6 -> 1 KB nomor 24; CHR1 = 9 % 8 = 1 -> 1 KB nomor 4
    assert!(m.chr_read(0x0000) == 24 && m.chr_read(0x1000) == 4, "MMC1 CHR 4 KB: dua register (modulo jumlah bank)");
    mmc1(&mut m, 0x8000, 0x0F); // kontrol: CHR 8 KB
    mmc1(&mut m, 0xA000, 7); // 7 -> 6 (genap)
    assert!(m.chr_read(0x0000) == 24 && m.chr_read(0x1000) == 28, "MMC1 CHR 8 KB: bit terendah diabaikan");
}
