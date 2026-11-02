//! Tes unit PPU. Nilai harapan dihitung dari aturan register di nesdev wiki.

use hardware::mapper::Mapper;
use hardware::palette::NES_PALETTE;
use hardware::ppu::{Ppu, H, W};

const BLACK: u8 = 0x0F;
const RED: u8 = 0x16;
const GREEN: u8 = 0x1A;
const LBLUE: u8 = 0x21;
const WHITE: u8 = 0x30;
const SPR1: u8 = 0x24;
const SPR2: u8 = 0x27;
const SPR_P1: u8 = 0x2A;

const FF: [u8; 8] = [0xFF; 8];
const ZERO: [u8; 8] = [0; 8];

struct T {
    ppu: Ppu,
    m: Mapper,
    cyc: u64,
}

impl T {
    /// PPU baru dengan CHR-RAM 8 KB. Penggambaran masih mati.
    fn raw(mirror: u8) -> T {
        static PRG: [u8; 16384] = [0; 16384];
        T { ppu: Ppu::new(), m: Mapper::new(0, &PRG, &[], mirror).unwrap(), cyc: 0 }
    }

    /// PPU baru dengan pola ubin dan palet siap pakai.
    fn fresh(mirror: u8) -> T {
        let mut t = T::raw(mirror);
        t.pattern(1, &FF, &ZERO); // warna 1 penuh
        t.pattern(2, &ZERO, &FF); // warna 2 penuh
        t.pattern(3, &[0x80, 0, 0, 0, 0, 0, 0, 0], &ZERO); // satu piksel di kiri atas
        t.pattern(4, &FF, &ZERO); // sprite 8x16: bagian atas
        t.pattern(5, &ZERO, &FF); // sprite 8x16: bagian bawah
        t.vw(0x3F00, BLACK);
        t.vw(0x3F01, RED);
        t.vw(0x3F02, GREEN);
        t.vw(0x3F05, LBLUE);
        t.vw(0x3F0D, WHITE);
        t.vw(0x3F11, SPR1);
        t.vw(0x3F12, SPR2);
        t.vw(0x3F15, SPR_P1);
        t.set_addr(0);
        t.w(0x2005, 0);
        t.w(0x2005, 0);
        t
    }

    fn w(&mut self, reg: u16, v: u8) {
        self.ppu.write(&mut self.m, reg, v, self.cyc);
    }

    fn r(&mut self, reg: u16) -> u8 {
        self.ppu.read(&mut self.m, reg, self.cyc)
    }

    fn set_addr(&mut self, a: u16) {
        self.w(0x2006, (a >> 8) as u8);
        self.w(0x2006, a as u8);
    }

    fn vw(&mut self, a: u16, v: u8) {
        self.set_addr(a);
        self.w(0x2007, v);
    }

    fn peek(&self, a: u16) -> u8 {
        self.ppu.vram_peek(&self.m, a)
    }

    fn pattern(&mut self, tile: u16, lo: &[u8; 8], hi: &[u8; 8]) {
        for i in 0..8u16 {
            self.vw(tile * 16 + i, lo[i as usize]);
            self.vw(tile * 16 + 8 + i, hi[i as usize]);
        }
    }

    fn sync(&mut self, cyc: u64) {
        self.ppu.sync(&mut self.m, cyc);
    }

    /// Jalankan sampai satu frame penuh selesai (vblank dimulai).
    fn run_frame(&mut self) {
        self.ppu.take_frame();
        while !self.ppu.take_frame() {
            self.cyc += 1;
            self.ppu.sync(&mut self.m, self.cyc);
        }
    }

    fn pixel_is(&self, x: usize, y: usize, color_index: u8) -> bool {
        assert!(x < W && y < H);
        let p = &self.ppu.framebuffer[(y * W + x) * 4..][..4];
        let want = NES_PALETTE[color_index as usize];
        p[0] == want[0] && p[1] == want[1] && p[2] == want[2] && p[3] == 255
    }

    fn put_sprite(&mut self, idx: u8, y: u8, tile: u8, attr: u8, x: u8) {
        self.w(0x2003, idx.wrapping_mul(4));
        self.w(0x2004, y);
        self.w(0x2004, tile);
        self.w(0x2004, attr);
        self.w(0x2004, x);
    }

    fn hide_all_sprites(&mut self) {
        for i in 0..64 {
            self.put_sprite(i, 0xFF, 0, 0, 0);
        }
    }
}

#[test]
fn baca_tulis_2007_lewat_buffer() {
    let mut t = T::fresh(1);
    t.set_addr(0x2000);
    t.w(0x2007, 0xAA);
    t.w(0x2007, 0xBB);
    t.set_addr(0x2000);
    let _ = t.r(0x2007); // pembacaan pertama basi
    let a = t.r(0x2007);
    let b = t.r(0x2007);
    assert!(a == 0xAA && b == 0xBB, "baca $2007 berurutan lewat buffer");

    // Pembacaan palet langsung, tanpa penundaan buffer.
    t.vw(0x3F00, 0x15);
    t.set_addr(0x3F00);
    assert_eq!(t.r(0x2007), 0x15, "baca palet langsung");

    // Penambahan 32 bila PPUCTRL bit 2 menyala.
    t.w(0x2000, 0x04);
    t.set_addr(0x2000);
    t.w(0x2007, 1);
    t.w(0x2007, 2);
    t.w(0x2007, 3);
    assert!(t.peek(0x2000) == 1 && t.peek(0x2020) == 2 && t.peek(0x2040) == 3, "penambahan alamat 32");
    t.w(0x2000, 0x00);

    // Cermin palet: $3F10 = $3F00, tetapi $3F11 mandiri.
    t.vw(0x3F10, 0x2A);
    assert_eq!(t.peek(0x3F00), 0x2A, "$3F10 mencerminkan $3F00");
    t.vw(0x3F14, 0x04);
    assert_eq!(t.peek(0x3F04), 0x04, "$3F14 mencerminkan $3F04");
    t.vw(0x3F11, 0x11);
    assert!(t.peek(0x3F01) != 0x11 && t.peek(0x3F11) == 0x11, "$3F11 mandiri");
}

#[test]
fn register_scroll_t_v_x() {
    // $2005 dua kali lalu $2000 (nesdev: t = yyy NN YYYYY XXXXX).
    let mut t = T::fresh(1);
    t.w(0x2005, 0x7D); // X: kasar 0x0F, halus 5
    t.w(0x2005, 0x5E); // Y: kasar 0x0B, halus 6
    assert!(t.ppu.reg_t() == 0x616F && t.ppu.reg_x() == 5, "t dan x setelah $2005 dua kali");
    t.w(0x2000, 0x03);
    assert_eq!(t.ppu.reg_t(), 0x6D6F, "bit nametable dari $2000 masuk ke t");
    t.w(0x2006, 0x3F);
    assert_eq!(t.ppu.reg_t(), 0x3F6F, "$2006 pertama: byte tinggi, bit 14 dihapus");
    t.w(0x2006, 0x00);
    assert!(t.ppu.reg_t() == 0x3F00 && t.ppu.reg_v() == 0x3F00, "$2006 kedua menyalin t ke v");

    // Membaca $2002 mengembalikan latch tulis ke keadaan awal.
    let mut t = T::fresh(1);
    t.w(0x2005, 0x08); // tulis pertama
    let _ = t.r(0x2002);
    t.w(0x2005, 0x10); // sekarang dianggap tulis pertama lagi
    assert!(t.ppu.reg_x() == 0 && (t.ppu.reg_t() & 0x1F) == 2, "$2002 mereset latch tulis");
}

#[test]
fn oam_baca_tulis_dan_dma() {
    // Alamat bertambah, atribut tak terpakai terbaca nol (bit 2-4).
    let mut t = T::fresh(1);
    t.w(0x2003, 0x10);
    t.w(0x2004, 1);
    t.w(0x2004, 2);
    t.w(0x2004, 0xFF);
    t.w(0x2004, 4);
    t.w(0x2003, 0x10);
    let a = t.r(0x2004);
    t.w(0x2003, 0x12);
    let c = t.r(0x2004);
    assert!(a == 1 && c == 0xE3, "OAM baca-tulis dan topeng atribut");

    // DMA OAM menyalin 256 byte mulai OAMADDR.
    let mut page = [0u8; 256];
    for (i, p) in page.iter_mut().enumerate() {
        *p = i as u8;
    }
    let mut t = T::fresh(1);
    t.w(0x2003, 0);
    t.ppu.oam_dma(&page);
    t.w(0x2003, 0x07);
    assert_eq!(t.r(0x2004), 7, "DMA OAM menyalin urut");
    t.w(0x2003, 0x80);
    assert_eq!(t.r(0x2004), 0x80, "DMA OAM sampai akhir");
}

#[test]
fn mirroring() {
    let mut t = T::fresh(0); // horizontal
    t.vw(0x2000, 0x11);
    t.vw(0x2800, 0x22);
    assert!(t.peek(0x2400) == 0x11 && t.peek(0x2C00) == 0x22 && t.peek(0x2800) == 0x22, "mirroring horizontal");
    assert_eq!(t.peek(0x3000), 0x11, "$3000 mencerminkan $2000");

    let mut t = T::fresh(1); // vertikal
    t.vw(0x2000, 0x11);
    t.vw(0x2400, 0x22);
    assert!(t.peek(0x2800) == 0x11 && t.peek(0x2C00) == 0x22 && t.peek(0x2400) == 0x22, "mirroring vertikal");

    let mut t = T::fresh(2); // four-screen
    t.vw(0x2000, 1);
    t.vw(0x2400, 2);
    t.vw(0x2800, 3);
    t.vw(0x2C00, 4);
    assert!(
        t.peek(0x2000) == 1 && t.peek(0x2400) == 2 && t.peek(0x2800) == 3 && t.peek(0x2C00) == 4,
        "four-screen punya empat nametable"
    );
}

#[test]
fn waktu_vblank_dan_nmi() {
    // Vblank menyala saat dot (241,1) selesai diproses: total dot 241*341+1+1 = 82183.
    // 3 dot per siklus CPU: 27394 siklus = 82182 dot (belum), 27395 siklus = 82185 dot (sudah).
    let mut t = T::raw(1);
    assert_eq!(t.ppu.read(&mut t.m, 0x2002, 27394) & 0x80, 0, "vblank belum menyala di siklus 27394");
    let mut t = T::raw(1);
    assert_ne!(t.ppu.read(&mut t.m, 0x2002, 27395) & 0x80, 0, "vblank menyala di siklus 27395");
    assert_eq!(t.ppu.read(&mut t.m, 0x2002, 27395) & 0x80, 0, "membaca $2002 menghapus vblank");

    // Posisi setelah satu frame penuh (89342 dot) + 1 dot lebih.
    let mut t = T::raw(1);
    t.sync(29781); // 89343 dot
    assert!(t.ppu.scanline() == 0 && t.ppu.dot() == 1, "posisi setelah satu frame");

    // Vblank dihapus di pre-render (261, 1).
    let mut t = T::raw(1);
    t.sync(29000); // sudah di dalam vblank
    t.sync(261 * 341 / 3 + 5);
    assert_eq!(t.ppu.scanline(), 261, "sampai pre-render");
    assert_eq!(t.ppu.read(&mut t.m, 0x2002, 261 * 341 / 3 + 5) & 0x80, 0, "vblank dihapus di pre-render");

    // NMI naik tepi: aktif sebelum vblank.
    let mut t = T::raw(1);
    t.ppu.write(&mut t.m, 0x2000, 0x80, 0);
    t.sync(27394);
    assert!(!t.ppu.nmi_pending, "NMI belum terpicu sebelum vblank");
    t.sync(27395);
    assert!(t.ppu.nmi_pending, "NMI terpicu saat vblank dimulai");

    // NMI dimatikan lalu dinyalakan lagi saat vblank masih menyala: tepi baru.
    t.ppu.nmi_pending = false;
    t.ppu.write(&mut t.m, 0x2000, 0x00, 27396);
    t.ppu.write(&mut t.m, 0x2000, 0x80, 27397);
    assert!(t.ppu.nmi_pending, "menyalakan NMI di dalam vblank memicu NMI");

    // NMI mati: tidak terpicu.
    let mut t = T::raw(1);
    t.sync(27400);
    assert!(!t.ppu.nmi_pending, "tanpa NMI enable tidak ada NMI");
    assert!(t.ppu.take_frame() && !t.ppu.take_frame(), "satu sinyal frame per vblank");

    // cycles_to_vblank konsisten dengan waktu vblank.
    let mut t = T::raw(1);
    let n = t.ppu.cycles_to_vblank();
    t.sync(n - 1);
    assert_eq!(t.ppu.read(&mut t.m, 0x2002, n - 1) & 0x80, 0, "sebelum perkiraan vblank");
    let mut t = T::raw(1);
    assert_ne!(t.ppu.read(&mut t.m, 0x2002, n) & 0x80, 0, "pada perkiraan vblank");
}

#[test]
fn latar_ubin_dan_warna() {
    // Ubin warna 1 di pojok kiri atas, latar hitam.
    let mut t = T::fresh(1);
    t.vw(0x2000, 1);
    t.set_addr(0);
    t.w(0x2001, 0x0A);
    t.run_frame();
    assert!(t.pixel_is(0, 0, RED) && t.pixel_is(7, 7, RED), "ubin 8x8 di kiri atas merah");
    assert!(t.pixel_is(8, 0, BLACK) && t.pixel_is(0, 8, BLACK) && t.pixel_is(255, 239, BLACK), "sisanya latar hitam");

    // Penggambaran mati: layar berisi warna latar universal.
    let mut t = T::fresh(1);
    t.vw(0x2000, 1);
    t.set_addr(0);
    t.w(0x2001, 0x00);
    t.run_frame();
    assert!(t.pixel_is(0, 0, BLACK) && t.pixel_is(100, 100, BLACK), "penggambaran mati menampilkan latar");
}

#[test]
fn latar_atribut() {
    // Kuadran kanan-bawah (bit 6-7) memilih palet 3.
    let mut t = T::fresh(1);
    t.vw(0x2000 + 2 * 32 + 2, 1); // kolom 2, baris 2
    t.vw(0x23C0, 0xC0);
    t.vw(0x2000, 1);
    t.set_addr(0);
    t.w(0x2001, 0x0A);
    t.run_frame();
    assert!(t.pixel_is(16, 16, WHITE) && t.pixel_is(23, 23, WHITE), "atribut kuadran kanan-bawah");
    assert!(t.pixel_is(0, 0, RED), "ubin lain tetap palet 0");

    // Atribut kuadran kiri-atas dan kanan-atas.
    let mut t = T::fresh(1);
    t.vw(0x2000, 1); // kolom 0, baris 0
    t.vw(0x2000 + 2, 1); // kolom 2, baris 0
    t.vw(0x23C0, 0x04); // bit 2-3 = kanan-atas -> palet 1
    t.set_addr(0);
    t.w(0x2001, 0x0A);
    t.run_frame();
    assert!(t.pixel_is(0, 0, RED) && t.pixel_is(16, 0, LBLUE), "atribut kuadran kanan-atas");
}

#[test]
fn latar_tabel_pola_dan_scroll() {
    // Pilihan tabel pola latar lewat PPUCTRL bit 4: ubin 1 di $1000 berwarna 2.
    let mut t = T::fresh(1);
    for i in 0..8u16 {
        t.vw(0x1010 + i, ZERO[i as usize]);
        t.vw(0x1018 + i, FF[i as usize]);
    }
    t.vw(0x2000, 1);
    t.set_addr(0);
    t.w(0x2000, 0x10);
    t.w(0x2001, 0x0A);
    t.run_frame();
    assert!(t.pixel_is(0, 0, GREEN), "tabel pola latar di $1000");

    // Scroll halus X = 4: ubin merah hanya tersisa 4 piksel di kiri.
    let mut t = T::fresh(1);
    t.vw(0x2000, 1);
    t.set_addr(0);
    t.w(0x2005, 4);
    t.w(0x2005, 0);
    t.w(0x2001, 0x0A);
    t.run_frame();
    assert!(t.pixel_is(0, 0, RED) && t.pixel_is(3, 0, RED) && t.pixel_is(4, 0, BLACK), "scroll halus X");

    // Scroll kasar melewati batas nametable (mirroring vertikal: $2400 berbeda).
    let mut t = T::fresh(1);
    t.vw(0x2000 + 31, 1); // NT0 kolom 31 -> warna 1
    t.vw(0x2400, 2); // NT1 kolom 0 -> warna 2
    t.set_addr(0);
    t.w(0x2005, 252); // kasar 31, halus 4
    t.w(0x2005, 0);
    t.w(0x2001, 0x0A);
    t.run_frame();
    t.run_frame(); // t disalin ke v di pre-render, jadi berlaku penuh di frame kedua
    assert!(t.pixel_is(0, 0, RED) && t.pixel_is(3, 0, RED), "kolom 31 NT0 tampil di kiri");
    assert!(
        t.pixel_is(4, 0, GREEN) && t.pixel_is(11, 0, GREEN) && t.pixel_is(12, 0, BLACK),
        "NT1 kolom 0 menyambung"
    );

    // Scroll Y: baris ubin 1 (y=8..15) naik ke atas bila scroll Y = 8.
    let mut t = T::fresh(1);
    t.vw(0x2000 + 32, 1); // kolom 0, baris 1
    t.set_addr(0);
    t.w(0x2005, 0);
    t.w(0x2005, 8);
    t.w(0x2001, 0x0A);
    t.run_frame();
    t.run_frame();
    assert!(t.pixel_is(0, 0, RED) && t.pixel_is(7, 7, RED) && t.pixel_is(0, 8, BLACK), "scroll Y");

    // Klip 8 piksel kiri: tanpa bit 1 PPUMASK, latar x<8 tersembunyi.
    let mut t = T::fresh(1);
    t.vw(0x2000, 1);
    t.vw(0x2001, 1);
    t.set_addr(0);
    t.w(0x2001, 0x08);
    t.run_frame();
    assert!(t.pixel_is(0, 0, BLACK) && t.pixel_is(7, 0, BLACK), "klip kiri latar");
    assert!(t.pixel_is(8, 0, RED), "ubin kedua di x=8 tetap tampil");
}

#[test]
fn sprite_posisi_prioritas_dan_palet() {
    // Sprite 8x8: tampil mulai scanline y+1.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    t.put_sprite(0, 15, 2, 0, 20);
    t.set_addr(0);
    t.w(0x2001, 0x1E);
    t.run_frame();
    assert!(t.pixel_is(20, 16, SPR2) && t.pixel_is(27, 23, SPR2), "sprite 8x8 tampil di y+1");
    assert!(t.pixel_is(28, 16, BLACK) && t.pixel_is(19, 16, BLACK), "di luar sprite (x)");
    assert!(t.pixel_is(20, 15, BLACK) && t.pixel_is(20, 24, BLACK), "di luar sprite (y)");

    // Prioritas: sprite di belakang latar buram tertutup, di luar latar tetap tampak.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    t.vw(0x2000 + 2 * 32 + 2, 1); // latar merah di x16..23, y16..23
    t.put_sprite(0, 15, 2, 0x20, 20);
    t.set_addr(0);
    t.w(0x2001, 0x1E);
    t.run_frame();
    assert!(t.pixel_is(20, 16, RED), "sprite di belakang latar buram tertutup");
    assert!(t.pixel_is(24, 16, SPR2), "sprite di belakang tetap tampak di atas latar transparan");

    // Sprite di depan latar, dan palet sprite 1 lewat atribut.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    t.vw(0x2000 + 2 * 32 + 2, 1);
    t.put_sprite(0, 15, 1, 0x01, 20); // palet sprite 1, warna 1
    t.set_addr(0);
    t.w(0x2001, 0x1E);
    t.run_frame();
    assert!(t.pixel_is(20, 16, SPR_P1), "sprite di depan latar memakai palet sprite 1");
}

#[test]
fn sprite_balik_8x16_klip_dan_batas() {
    // Pembalikan horizontal dan vertikal: piksel hanya ada di sudut kiri atas ubin.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    t.put_sprite(0, 49, 3, 0x00, 100);
    t.put_sprite(1, 49, 3, 0x40, 120);
    t.put_sprite(2, 49, 3, 0x80, 140);
    t.put_sprite(3, 49, 3, 0xC0, 160);
    t.set_addr(0);
    t.w(0x2001, 0x1E);
    t.run_frame();
    assert!(t.pixel_is(100, 50, SPR1) && t.pixel_is(101, 50, BLACK) && t.pixel_is(100, 51, BLACK), "tanpa balik");
    assert!(t.pixel_is(127, 50, SPR1) && t.pixel_is(120, 50, BLACK), "balik horizontal: sudut kanan atas");
    assert!(t.pixel_is(140, 57, SPR1) && t.pixel_is(140, 50, BLACK), "balik vertikal: sudut kiri bawah");
    assert!(t.pixel_is(167, 57, SPR1) && t.pixel_is(160, 50, BLACK), "balik keduanya: sudut kanan bawah");

    // Sprite 8x16: ubin genap = atas, ubin berikutnya = bawah.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    t.put_sprite(0, 79, 4, 0x00, 50);
    t.put_sprite(1, 79, 4, 0x80, 80); // balik vertikal: atas dan bawah bertukar
    t.set_addr(0);
    t.w(0x2000, 0x20);
    t.w(0x2001, 0x1E);
    t.run_frame();
    assert!(t.pixel_is(50, 80, SPR1) && t.pixel_is(50, 87, SPR1), "sprite 8x16 bagian atas");
    assert!(t.pixel_is(50, 88, SPR2) && t.pixel_is(50, 95, SPR2), "sprite 8x16 bagian bawah");
    assert!(t.pixel_is(50, 96, BLACK), "di bawah sprite 8x16");
    assert!(t.pixel_is(80, 80, SPR2) && t.pixel_is(80, 95, SPR1), "sprite 8x16 dibalik vertikal");

    // Klip kiri sprite: tanpa bit 2 PPUMASK, sprite di x<8 tersembunyi.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    t.put_sprite(0, 15, 2, 0, 4);
    t.set_addr(0);
    t.w(0x2001, 0x18);
    t.run_frame();
    assert!(t.pixel_is(4, 16, BLACK) && t.pixel_is(7, 16, BLACK) && t.pixel_is(8, 16, SPR2), "klip kiri sprite");

    // Batas 8 sprite per scanline: sprite ke-9 tidak tampil dan overflow menyala.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    for i in 0..9u8 {
        t.put_sprite(i, 15, 2, 0, i * 16);
    }
    t.set_addr(0);
    t.w(0x2001, 0x1E);
    t.run_frame();
    assert!(t.pixel_is(7 * 16, 16, SPR2) && t.pixel_is(8 * 16, 16, BLACK), "maksimal 8 sprite per scanline");
    assert_ne!(t.r(0x2002) & 0x20, 0, "bendera overflow sprite menyala");
}

#[test]
fn sprite_0_hit() {
    // Latar merah di x16..23,y16..23; sprite 0 di x=20,y=15 -> piksel pertama tumpang tindih
    // adalah (20,16), jadi bendera menyala saat dot 21 di scanline 16 diproses.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    t.vw(0x2000 + 2 * 32 + 2, 1);
    t.put_sprite(0, 15, 2, 0, 20);
    t.set_addr(0);
    t.w(0x2001, 0x1E);
    // scanline 16, dot 19 = total 16*341+19 = 5475 dot = siklus 1825 (belum); siklus 1826 = dot 22 (sudah)
    t.sync(1825);
    assert!(t.ppu.scanline() == 16 && t.r(0x2002) & 0x40 == 0, "sprite 0 hit belum menyala sebelum dot 21");
    t.sync(1826);
    assert_ne!(t.r(0x2002) & 0x40, 0, "sprite 0 hit menyala setelah dot 21");

    // Bendera tetap menyala sepanjang frame, dihapus di pre-render.
    t.cyc = 241 * 341 / 3 + 10;
    assert_ne!(t.r(0x2002) & 0x40, 0, "bendera bertahan sampai vblank");
    t.cyc = 261 * 341 / 3 + 5;
    assert_eq!(t.r(0x2002) & 0x40, 0, "bendera dihapus di pre-render");

    // Tanpa tumpang tindih tidak ada hit: sprite di area latar transparan.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    t.vw(0x2000 + 2 * 32 + 2, 1);
    t.put_sprite(0, 15, 2, 0, 100);
    t.set_addr(0);
    t.w(0x2001, 0x1E);
    t.run_frame();
    assert_eq!(t.r(0x2002) & 0x40, 0, "tanpa tumpang tindih tidak ada hit");

    // Sprite selain indeks 0 tidak memicu hit.
    let mut t = T::fresh(1);
    t.hide_all_sprites();
    t.vw(0x2000 + 2 * 32 + 2, 1);
    t.put_sprite(1, 15, 2, 0, 20);
    t.set_addr(0);
    t.w(0x2001, 0x1E);
    t.run_frame();
    assert_eq!(t.r(0x2002) & 0x40, 0, "hanya sprite 0 yang memicu hit");
}
