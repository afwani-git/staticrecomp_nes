//! PPU NES sederhana. Register, pemetaan memori video, dan penggunaan register scroll
//! (v, t, x, w) mengikuti dokumentasi nesdev wiki. Penyederhanaan yang disengaja:
//!  - scanline digambar utuh pada dot 0 memakai keadaan register saat itu (perubahan scroll
//!    di tengah scanline baru berlaku di scanline berikutnya);
//!  - sprite diambil dari OAM saat scanline dimulai, bukan lewat evaluasi dot demi dot;
//!  - tidak ada bug perangkat keras langka (balapan $2002 di dot vblank, overflow sprite
//!    yang "salah hitung", emphasis warna).
//!
//! Tidak bergantung pada CPU atau host: waktu diberikan lewat siklus CPU. CHR, mirroring, dan
//! jalur A12 dilayani mapper yang diberikan pemanggil.

use crate::mapper::Mapper;
use crate::palette::NES_PALETTE;

pub const W: usize = 256;
pub const H: usize = 240;

const DOTS_PER_LINE: i32 = 341;
const LINES_PER_FRAME: i32 = 262;
const PRE_RENDER: i32 = 261;
const VBLANK_LINE: i32 = 241;

pub struct Ppu {
    /// Layar RGBA (4 byte per piksel), diisi per scanline.
    pub framebuffer: Vec<u8>,
    /// True bila NMI baru terpicu dan belum diambil hardware.
    pub nmi_pending: bool,

    vram: [u8; 4096],
    palette: [u8; 32],
    oam: [u8; 256],

    ctrl: u8,
    mask: u8,
    status: u8,
    oam_addr: u8,
    read_buffer: u8,
    bus_latch: u8,
    w_latch: u8,
    vreg: u16,
    treg: u16,
    fine_x: u8,

    scanline: i32,
    dot: i32,
    dots_done: u64,
    odd_frame: bool,
    frame_ready: bool,
    nmi_line: bool,
    s0_hit_dot: i32,
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppu {
    pub fn new() -> Ppu {
        Ppu {
            framebuffer: vec![0; W * H * 4],
            nmi_pending: false,
            vram: [0; 4096],
            palette: [0; 32],
            oam: [0; 256],
            ctrl: 0,
            mask: 0,
            status: 0,
            oam_addr: 0,
            read_buffer: 0,
            bus_latch: 0,
            w_latch: 0,
            vreg: 0,
            treg: 0,
            fine_x: 0,
            scanline: 0,
            dot: 0,
            dots_done: 0,
            odd_frame: false,
            frame_ready: false,
            nmi_line: false,
            s0_hit_dot: 0,
        }
    }

    // ---- Memori video ----

    fn nametable_index(m: &Mapper, addr: u16) -> usize {
        let t = u32::from((addr >> 10) & 3);
        let page = match m.mirroring() {
            0 => t >> 1, // horizontal: $2000=$2400, $2800=$2C00
            1 => t & 1,  // vertikal:   $2000=$2800, $2400=$2C00
            3 => 0,      // satu layar: nametable A
            4 => 1,      // satu layar: nametable B
            _ => t,      // four-screen
        };
        (page * 0x400 + u32::from(addr & 0x3FF)) as usize
    }

    fn palette_index(addr: u16) -> usize {
        let mut i = addr & 0x1F;
        if i & 0x13 == 0x10 {
            i &= !0x10; // $3F10/14/18/1C = $3F00/04/08/0C
        }
        i as usize
    }

    fn mem_read(&self, m: &Mapper, addr: u16) -> u8 {
        let addr = addr & 0x3FFF;
        if addr < 0x2000 {
            m.chr_read(addr)
        } else if addr < 0x3F00 {
            self.vram[Self::nametable_index(m, addr & 0x2FFF)]
        } else {
            self.palette[Self::palette_index(addr)]
        }
    }

    fn mem_write(&mut self, m: &mut Mapper, addr: u16, value: u8) {
        let addr = addr & 0x3FFF;
        if addr < 0x2000 {
            m.chr_write(addr, value);
        } else if addr < 0x3F00 {
            self.vram[Self::nametable_index(m, addr & 0x2FFF)] = value;
        } else {
            self.palette[Self::palette_index(addr)] = value & 0x3F;
        }
    }

    pub fn vram_peek(&self, m: &Mapper, addr: u16) -> u8 {
        self.mem_read(m, addr)
    }

    // ---- Register ----

    fn update_nmi(&mut self) {
        let line = (self.status & 0x80) != 0 && (self.ctrl & 0x80) != 0;
        if line && !self.nmi_line {
            self.nmi_pending = true;
        }
        self.nmi_line = line;
    }

    fn oam_mask(index: u8, value: u8) -> u8 {
        if index & 3 == 2 { value & 0xE3 } else { value } // bit 2-4 atribut tidak ada
    }

    /// Saat penggambaran mati, alamat VRAM (v) muncul di jalur alamat PPU: A12 = bit 12.
    fn bus_a12(&self, m: &mut Mapper) {
        if m.has_a12() && self.mask & 0x18 == 0 {
            m.a12((self.vreg >> 12) & 1 != 0, self.dots_done);
        }
    }

    /// Baca register $2000-$3FFF (dicerminkan tiap 8). Memajukan PPU ke `cpu_cycle` lebih dulu.
    pub fn read(&mut self, m: &mut Mapper, reg: u16, cpu_cycle: u64) -> u8 {
        let mut result = self.bus_latch;
        self.sync(m, cpu_cycle);
        match reg & 7 {
            2 => {
                result = (self.status & 0xE0) | (self.bus_latch & 0x1F);
                self.status &= !0x80;
                self.w_latch = 0;
                self.update_nmi();
            }
            4 => result = self.oam[self.oam_addr as usize],
            7 => {
                if self.vreg & 0x3FFF >= 0x3F00 {
                    result = self.mem_read(m, self.vreg);
                    self.read_buffer = self.mem_read(m, self.vreg & 0x2FFF); // nametable di "bawah" palet
                } else {
                    result = self.read_buffer;
                    self.read_buffer = self.mem_read(m, self.vreg);
                }
                self.vreg = self.vreg.wrapping_add(if self.ctrl & 4 != 0 { 32 } else { 1 });
                self.bus_a12(m); // alamat yang sudah bertambah muncul di jalur PPU
            }
            _ => {} // register tulis-saja membaca bus terakhir
        }
        self.bus_latch = result;
        result
    }

    /// Tulis register $2000-$3FFF. Memajukan PPU ke `cpu_cycle` lebih dulu.
    pub fn write(&mut self, m: &mut Mapper, reg: u16, value: u8, cpu_cycle: u64) {
        self.sync(m, cpu_cycle);
        self.bus_latch = value;
        match reg & 7 {
            0 => {
                self.ctrl = value;
                self.treg = (self.treg & 0xF3FF) | (u16::from(value & 3) << 10);
                self.update_nmi();
            }
            1 => self.mask = value,
            3 => self.oam_addr = value,
            4 => {
                self.oam[self.oam_addr as usize] = Self::oam_mask(self.oam_addr, value);
                self.oam_addr = self.oam_addr.wrapping_add(1);
            }
            5 => {
                if self.w_latch == 0 {
                    self.treg = (self.treg & 0xFFE0) | u16::from(value >> 3);
                    self.fine_x = value & 7;
                } else {
                    self.treg =
                        (self.treg & 0x8C1F) | (u16::from(value & 7) << 12) | (u16::from(value >> 3) << 5);
                }
                self.w_latch ^= 1;
            }
            6 => {
                if self.w_latch == 0 {
                    self.treg = (self.treg & 0x00FF) | (u16::from(value & 0x3F) << 8);
                } else {
                    self.treg = (self.treg & 0xFF00) | u16::from(value);
                    self.vreg = self.treg;
                    self.bus_a12(m);
                }
                self.w_latch ^= 1;
            }
            7 => {
                self.mem_write(m, self.vreg, value);
                self.vreg = self.vreg.wrapping_add(if self.ctrl & 4 != 0 { 32 } else { 1 });
                self.bus_a12(m); // alamat yang sudah bertambah muncul di jalur PPU
            }
            _ => {} // $2002 tidak bisa ditulis
        }
    }

    /// Salin 256 byte ke OAM mulai dari OAMADDR (DMA $4014).
    pub fn oam_dma(&mut self, page: &[u8; 256]) {
        for (i, &b) in page.iter().enumerate() {
            let idx = self.oam_addr.wrapping_add(i as u8);
            self.oam[idx as usize] = Self::oam_mask(idx, b);
        }
    }

    // ---- Penggambaran ----

    fn put_pixel(&mut self, x: usize, y: usize, color_index: u8) {
        let rgb = NES_PALETTE[(color_index & if self.mask & 1 != 0 { 0x30 } else { 0x3F }) as usize];
        let p = (y * W + x) * 4;
        self.framebuffer[p] = rgb[0];
        self.framebuffer[p + 1] = rgb[1];
        self.framebuffer[p + 2] = rgb[2];
        self.framebuffer[p + 3] = 255;
    }

    fn render_scanline(&mut self, m: &Mapper, y: i32) {
        let mut bg_pix = [0u8; W];
        let mut bg_pal = [0u8; W];
        let mut spr_pix = [0u8; W];
        let mut spr_attr = [0u8; W];
        let mut spr_is0 = [false; W];
        let bg_on = self.mask & 0x08 != 0;
        let spr_on = self.mask & 0x10 != 0;

        self.s0_hit_dot = 0;
        if !(bg_on || spr_on) {
            for px in 0..W {
                self.put_pixel(px, y as usize, self.palette[0]);
            }
            return;
        }

        // Background: 33 ubin mulai dari kolom kasar v, digeser fine_x.
        {
            let fy = i32::from((self.vreg >> 12) & 7);
            let cy = i32::from((self.vreg >> 5) & 31);
            let nt = u32::from((self.vreg >> 10) & 3);
            let cx = i32::from(self.vreg & 31);
            let pat_base: u32 = if self.ctrl & 0x10 != 0 { 0x1000 } else { 0 };
            for k in 0..33 {
                let mut col = cx + k;
                let ntx = nt ^ (((col >> 5) & 1) as u32);
                let base = (0x2000 | (ntx << 10)) as u16;
                col &= 31;
                let tile = self.mem_read(m, base.wrapping_add((cy * 32 + col) as u16));
                let attr = self.mem_read(m, base.wrapping_add((0x3C0 + (cy >> 2) * 8 + (col >> 2)) as u16));
                let shift = (((cy & 2) << 1) | (col & 2)) as u32;
                let pal = (attr >> shift) & 3;
                let addr = (pat_base + u32::from(tile) * 16 + fy as u32) as u16;
                let lo = self.mem_read(m, addr);
                let hi = self.mem_read(m, addr.wrapping_add(8));
                for j in 0..8 {
                    let px = k * 8 + j - i32::from(self.fine_x);
                    if px < 0 || px >= W as i32 {
                        continue;
                    }
                    let bit = 7 - j;
                    bg_pix[px as usize] = ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1);
                    bg_pal[px as usize] = pal;
                }
            }
            if !bg_on {
                bg_pix.fill(0);
            }
            if self.mask & 2 == 0 {
                bg_pix[..8].fill(0);
            }
        }

        // Sprite: maksimal 8 per scanline; yang indeks OAM-nya lebih kecil menang.
        if spr_on {
            let h: i32 = if self.ctrl & 0x20 != 0 { 16 } else { 8 };
            let mut list = [0usize; 8];
            let mut n = 0usize;
            for i in 0..64usize {
                let row = y - (i32::from(self.oam[i * 4]) + 1);
                if row < 0 || row >= h {
                    continue;
                }
                if n < 8 {
                    list[n] = i;
                    n += 1;
                } else {
                    self.status |= 0x20;
                    break;
                }
            }
            for k in (0..n).rev() {
                let i = list[k];
                let mut row = y - (i32::from(self.oam[i * 4]) + 1);
                let mut tile = self.oam[i * 4 + 1];
                let attr = self.oam[i * 4 + 2];
                let sx = i32::from(self.oam[i * 4 + 3]);
                let base: u32;
                if attr & 0x80 != 0 {
                    row = h - 1 - row;
                }
                if h == 8 {
                    base = if self.ctrl & 8 != 0 { 0x1000 } else { 0 };
                } else {
                    base = if tile & 1 != 0 { 0x1000 } else { 0 };
                    tile &= 0xFE;
                    if row >= 8 {
                        tile = tile.wrapping_add(1);
                        row -= 8;
                    }
                }
                let addr = (base + u32::from(tile) * 16 + row as u32) as u16;
                let lo = self.mem_read(m, addr);
                let hi = self.mem_read(m, addr.wrapping_add(8));
                for j in 0..8 {
                    let bit = if attr & 0x40 != 0 { j } else { 7 - j };
                    let pix = ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1);
                    let px = sx + j;
                    if px >= W as i32 {
                        break;
                    }
                    if pix == 0 || (px < 8 && self.mask & 4 == 0) {
                        continue;
                    }
                    spr_pix[px as usize] = pix;
                    spr_attr[px as usize] = attr;
                    spr_is0[px as usize] = i == 0;
                }
            }
        }

        for px in 0..W {
            if spr_is0[px] && bg_pix[px] != 0 && px != 255 && self.s0_hit_dot == 0 {
                self.s0_hit_dot = px as i32 + 1;
            }
            let color = if spr_pix[px] != 0 && (bg_pix[px] == 0 || spr_attr[px] & 0x20 == 0) {
                self.palette[(0x10 | ((spr_attr[px] & 3) << 2) | spr_pix[px]) as usize]
            } else if bg_pix[px] != 0 {
                self.palette[((bg_pal[px] << 2) | bg_pix[px]) as usize]
            } else {
                self.palette[0]
            };
            self.put_pixel(px, y as usize, color);
        }
    }

    // ---- Waktu ----

    fn increment_y(&mut self) {
        if self.vreg & 0x7000 != 0x7000 {
            self.vreg = self.vreg.wrapping_add(0x1000);
        } else {
            let mut y = (self.vreg & 0x03E0) >> 5;
            self.vreg &= !0x7000;
            if y == 29 {
                y = 0;
                self.vreg ^= 0x0800;
            } else if y == 31 {
                y = 0;
            } else {
                y += 1;
            }
            self.vreg = (self.vreg & !0x03E0) | (y << 5);
        }
    }

    /// Dot tempat A12 naik sekali per scanline: sprite di $1000 dan latar di $0000 -> dot 260 (ambil
    /// sprite); sebaliknya -> dot 324 (ambil ubin latar scanline berikutnya); sama -> tidak ada tepi.
    /// Sprite 8x16 dianggap memakai $1000 (umum). None bila tidak ada tepi.
    fn a12_edge_dot(&self) -> Option<i32> {
        let bg_high = self.ctrl & 0x10 != 0;
        let spr_high = if self.ctrl & 0x20 != 0 { true } else { self.ctrl & 0x08 != 0 };
        if spr_high && !bg_high {
            Some(260)
        } else if !spr_high && bg_high {
            Some(324)
        } else {
            None
        }
    }

    fn step(&mut self, m: &mut Mapper) {
        let rendering = self.mask & 0x18 != 0;

        if self.scanline < 240 {
            if self.dot == 0 {
                self.render_scanline(m, self.scanline);
            }
            if self.s0_hit_dot != 0 && self.dot == self.s0_hit_dot {
                self.status |= 0x40;
            }
            if rendering {
                if self.dot == 256 {
                    self.increment_y();
                }
                if self.dot == 257 {
                    self.vreg = (self.vreg & !0x041F) | (self.treg & 0x041F);
                }
            }
        } else if self.scanline == VBLANK_LINE && self.dot == 1 {
            self.status |= 0x80;
            self.frame_ready = true;
            self.update_nmi();
        } else if self.scanline == PRE_RENDER {
            if self.dot == 1 {
                self.status &= !0xE0;
                self.update_nmi();
            }
            if rendering {
                if self.dot == 256 {
                    self.increment_y();
                }
                if self.dot == 257 {
                    self.vreg = (self.vreg & !0x041F) | (self.treg & 0x041F);
                }
                if (280..=304).contains(&self.dot) {
                    self.vreg = (self.vreg & !0x7BE0) | (self.treg & 0x7BE0);
                }
                if self.dot == 339 && self.odd_frame {
                    self.dot += 1; // frame ganjil lebih pendek satu dot
                }
            }
        }

        // Penggambaran: A12 rendah dulu (filter mapper butuh ~3 siklus CPU rendah), lalu naik.
        if rendering && m.has_a12() && (self.scanline < 240 || self.scanline == PRE_RENDER) {
            if let Some(edge) = self.a12_edge_dot() {
                if self.dot == edge - 12 {
                    m.a12(false, self.dots_done);
                }
                if self.dot == edge {
                    m.a12(true, self.dots_done);
                }
            }
        }

        self.dot += 1;
        if self.dot >= DOTS_PER_LINE {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline >= LINES_PER_FRAME {
                self.scanline = 0;
                self.odd_frame = !self.odd_frame;
            }
        }
    }

    /// Majukan PPU sampai siklus CPU `cpu_cycle` (3 dot per siklus CPU).
    pub fn sync(&mut self, m: &mut Mapper, cpu_cycle: u64) {
        let target = cpu_cycle * 3;
        while self.dots_done < target {
            self.step(m);
            self.dots_done += 1;
        }
    }

    /// True sekali per frame, saat vblank dimulai (layar siap ditampilkan).
    pub fn take_frame(&mut self) -> bool {
        std::mem::take(&mut self.frame_ready)
    }

    /// Berapa siklus CPU lagi (dari posisi PPU sekarang) sampai vblank berikutnya dimulai.
    pub fn cycles_to_vblank(&self) -> u64 {
        let mut delta = VBLANK_LINE * DOTS_PER_LINE + 1 - (self.scanline * DOTS_PER_LINE + self.dot);
        if delta < 0 {
            delta += LINES_PER_FRAME * DOTS_PER_LINE;
        }
        (delta + 1 + 2) as u64 / 3 // butuh delta+1 dot, dibulatkan ke atas
    }

    /// Berapa siklus CPU lagi sampai tepi naik A12 berikutnya dari penggambaran (sangat besar
    /// bila tidak ada).
    pub fn cycles_to_scanline_clock(&self, m: &Mapper) -> u64 {
        let edge = match self.a12_edge_dot() {
            Some(e) if m.has_a12() && self.mask & 0x18 != 0 => e,
            _ => return 1_000_000,
        };
        // Cari scanline berikutnya (mungkin yang sekarang) yang menghasilkan tepi.
        let mut line = self.scanline;
        for _ in 0..LINES_PER_FRAME + 1 {
            let this = line;
            line = (line + 1) % LINES_PER_FRAME;
            if !(this < 240 || this == PRE_RENDER) {
                continue;
            }
            let mut delta = (this - self.scanline) * DOTS_PER_LINE + edge - self.dot;
            if this == self.scanline && delta < 0 {
                continue;
            }
            if delta < 0 {
                delta += LINES_PER_FRAME * DOTS_PER_LINE;
            }
            return (delta + 1 + 2) as u64 / 3;
        }
        1_000_000
    }

    // Pengamatan untuk tes dan penyelaras waktu.
    pub fn scanline(&self) -> i32 {
        self.scanline
    }
    pub fn dot(&self) -> i32 {
        self.dot
    }
    pub fn reg_t(&self) -> u16 {
        self.treg
    }
    pub fn reg_v(&self) -> u16 {
        self.vreg
    }
    pub fn reg_x(&self) -> u8 {
        self.fine_x
    }
    pub fn reg_ctrl(&self) -> u8 {
        self.ctrl
    }
    pub fn reg_mask(&self) -> u8 {
        self.mask
    }
}
