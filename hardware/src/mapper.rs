//! Mapper cartridge: 0 NROM, 1 MMC1, 2 UxROM, 3 CNROM, 4 MMC3, 7 AxROM, 11 Color Dreams,
//! 66 GxROM, 71 Camerica, 79 NINA-003/006. Register dan aturan bank mengikuti halaman mapper di
//! nesdev wiki. Penyederhanaan yang disengaja:
//!  - bus conflict (nilai tulis di-AND dengan byte ROM) tidak ditiru, seperti kebanyakan emulator;
//!  - MMC1: tulisan beruntun di siklus CPU berurutan tidak diabaikan;
//!  - MMC3: saat penggambaran, tepi A12 dibuat sekali per scanline di dot tetap; tepi dari akses
//!    $2006/$2007 dilaporkan apa adanya; proteksi PRG-RAM diabaikan; mengikuti varian "baru";
//!  - keadaan awal bank saat daya menyala tidak terdefinisi di perangkat keras; di sini bank 0 untuk
//!    mapper yang menukar PRG (AxROM, GxROM, Color Dreams, NINA, BNROM-like).
//!
//! Tidak bergantung pada CPU atau PPU.

/// Nomor mapper iNES yang didukung.
pub const SUPPORTED: [u8; 10] = [0, 1, 2, 3, 4, 7, 11, 66, 71, 79];

/// MMC3 meng-clock penghitung IRQ pada tepi naik A12 bila A12 sebelumnya rendah sedikitnya
/// sekian dot PPU (~3 siklus CPU).
pub const A12_FILTER: u64 = 9;

pub struct Mapper {
    id: u8,
    prg: &'static [u8],
    prg_size: u32,
    prg_banks8: u32,
    prg_banks16: u32,
    prg_banks32: u32,
    chr: &'static [u8],
    chr_size: u32,
    chr_banks1k: u32,
    chr_banks4k: u32,
    chr_banks8k: u32,
    chr_ram: [u8; 8192],
    header_mirroring: u8,
    mirroring: u8,

    // Bank sederhana: PRG 16 KB (UxROM, Camerica), PRG 32 KB, CHR 8 KB.
    prg16_sel: u32,
    prg32_sel: u32,
    chr8_sel: u32,

    // MMC1
    mmc1_shift: u8,
    mmc1_count: u8,
    mmc1_ctrl: u8,
    mmc1_chr0: u8,
    mmc1_chr1: u8,
    mmc1_prg: u8,

    // MMC3
    bank_select: u8,
    regs: [u8; 8],
    prg_mode: u8,
    chr_invert: u8,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
    a12: bool,
    a12_low_since: u64,
}

impl Mapper {
    /// None bila mapper tidak didukung. `chr` kosong = CHR-RAM 8 KB.
    /// `mirroring` bawaan header: 0 horizontal, 1 vertikal, 2 four-screen.
    pub fn new(id: u8, prg: &'static [u8], chr: &'static [u8], mirroring: u8) -> Option<Mapper> {
        if !SUPPORTED.contains(&id) {
            return None;
        }
        let prg_size = prg.len() as u32;
        let chr_size = chr.len() as u32;
        let mut regs = [0u8; 8];
        // Keadaan daya-nyala yang umum: R6/R7 = bank 0/1, supaya $8000-$BFFF langsung bermakna.
        regs[7] = 1;
        regs[3] = 1;
        regs[4] = 2;
        regs[5] = 3;
        regs[1] = 2;
        Some(Mapper {
            id,
            prg,
            prg_size,
            prg_banks8: (prg_size / 8192).max(1),
            prg_banks16: (prg_size / 16384).max(1),
            prg_banks32: (prg_size / 32768).max(1),
            chr,
            chr_size,
            chr_banks1k: (chr_size / 1024).max(1),
            chr_banks4k: (chr_size / 4096).max(1),
            chr_banks8k: (chr_size / 8192).max(1),
            chr_ram: [0; 8192],
            header_mirroring: mirroring,
            mirroring,
            prg16_sel: 0,
            prg32_sel: 0,
            chr8_sel: 0,
            mmc1_shift: 0,
            mmc1_count: 0,
            mmc1_ctrl: 0x0C, // mode PRG 3: bank terakhir tetap di $C000
            mmc1_chr0: 0,
            mmc1_chr1: 0,
            mmc1_prg: 0,
            bank_select: 0,
            regs,
            prg_mode: 0,
            chr_invert: 0,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
            a12: false,
            a12_low_since: 0,
        })
    }

    /// Hanya MMC3 yang memakai tepi A12; PPU tidak perlu menghasilkannya untuk mapper lain.
    pub fn has_a12(&self) -> bool {
        self.id == 4
    }

    // ---- PRG ----

    fn prg_at(&self, off: u32) -> u8 {
        self.prg[(off % self.prg_size) as usize]
    }

    /// Bank 16 KB yang terlihat di `addr` untuk MMC1 menurut mode PRG.
    fn mmc1_prg_bank16(&self, addr: u16) -> u32 {
        let last = self.prg_banks16 - 1;
        let b = match (self.mmc1_ctrl >> 2) & 3 {
            // mode 32 KB: bit terendah diabaikan
            0 | 1 => u32::from(self.mmc1_prg & 0x0E) + u32::from(addr >= 0xC000),
            // $8000 tetap ke bank 0, $C000 berganti
            2 => {
                if addr < 0xC000 {
                    0
                } else {
                    u32::from(self.mmc1_prg)
                }
            }
            // $8000 berganti, $C000 tetap ke bank terakhir
            _ => {
                if addr < 0xC000 {
                    u32::from(self.mmc1_prg)
                } else {
                    last
                }
            }
        };
        b % self.prg_banks16
    }

    /// Nomor bank 8 KB yang terlihat di `addr` (MMC3).
    pub fn prg_bank_8k(&self, addr: u16) -> u32 {
        let last = self.prg_banks8 - 1;
        let second = if self.prg_banks8 >= 2 { self.prg_banks8 - 2 } else { 0 };
        let r6 = u32::from(self.regs[6] & 0x3F);
        let b = match (addr >> 13) & 3 {
            0 => {
                if self.prg_mode != 0 {
                    second
                } else {
                    r6
                }
            }
            1 => u32::from(self.regs[7] & 0x3F),
            2 => {
                if self.prg_mode != 0 {
                    r6
                } else {
                    second
                }
            }
            _ => last,
        };
        b % self.prg_banks8
    }

    /// CPU membaca $8000-$FFFF.
    pub fn cpu_read(&self, addr: u16) -> u8 {
        match self.id {
            1 => self.prg_at(self.mmc1_prg_bank16(addr) * 16384 + u32::from(addr & 0x3FFF)),
            2 | 71 => {
                if addr < 0xC000 {
                    self.prg_at((self.prg16_sel % self.prg_banks16) * 16384 + u32::from(addr & 0x3FFF))
                } else {
                    self.prg_at((self.prg_banks16 - 1) * 16384 + u32::from(addr & 0x3FFF))
                }
            }
            4 => self.prg[(self.prg_bank_8k(addr) * 8192 + u32::from(addr & 0x1FFF)) as usize],
            7 | 11 | 66 | 79 => {
                self.prg_at((self.prg32_sel % self.prg_banks32) * 32768 + u32::from(addr & 0x7FFF))
            }
            // 0 dan 3: PRG tetap
            _ => self.prg[(u32::from(addr).wrapping_sub(0x8000) % self.prg_size) as usize],
        }
    }

    fn mmc1_write(&mut self, addr: u16, v: u8) {
        if v & 0x80 != 0 {
            // reset register geser dan kunci mode PRG 3
            self.mmc1_shift = 0;
            self.mmc1_count = 0;
            self.mmc1_ctrl |= 0x0C;
            return;
        }
        self.mmc1_shift |= (v & 1) << self.mmc1_count;
        self.mmc1_count += 1;
        if self.mmc1_count < 5 {
            return;
        }
        match (addr >> 13) & 3 {
            // register dipilih oleh alamat tulisan ke-5
            0 => self.mmc1_ctrl = self.mmc1_shift,
            1 => self.mmc1_chr0 = self.mmc1_shift,
            2 => self.mmc1_chr1 = self.mmc1_shift,
            _ => self.mmc1_prg = self.mmc1_shift & 0x0F,
        }
        self.mmc1_shift = 0;
        self.mmc1_count = 0;
    }

    fn mmc3_write(&mut self, addr: u16, v: u8) {
        match (addr & 0xE000) | (addr & 1) {
            0x8000 => {
                self.bank_select = v & 7;
                self.prg_mode = (v >> 6) & 1;
                self.chr_invert = (v >> 7) & 1;
            }
            0x8001 => self.regs[self.bank_select as usize] = v,
            0xA000 => {
                if self.header_mirroring != 2 {
                    // bit 0: 0 = vertikal, 1 = horizontal
                    self.mirroring = if v & 1 != 0 { 0 } else { 1 };
                }
            }
            0xC000 => self.irq_latch = v,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {} // $A001: proteksi PRG-RAM, diabaikan
        }
    }

    /// CPU menulis $4020-$FFFF (kebanyakan mapper hanya $8000+).
    pub fn cpu_write(&mut self, addr: u16, v: u8) {
        match self.id {
            1 => {
                if addr >= 0x8000 {
                    self.mmc1_write(addr, v);
                }
            }
            2 => {
                if addr >= 0x8000 {
                    self.prg16_sel = u32::from(v);
                }
            }
            3 => {
                if addr >= 0x8000 {
                    self.chr8_sel = u32::from(v);
                }
            }
            4 => {
                if addr >= 0x8000 {
                    self.mmc3_write(addr, v);
                }
            }
            7 => {
                if addr >= 0x8000 {
                    self.prg32_sel = u32::from(v & 7);
                    self.mirroring = if v & 0x10 != 0 { 4 } else { 3 }; // satu layar: nametable A atau B
                }
            }
            11 => {
                if addr >= 0x8000 {
                    self.prg32_sel = u32::from(v & 3);
                    self.chr8_sel = u32::from(v >> 4) & 0xF;
                }
            }
            66 => {
                if addr >= 0x8000 {
                    self.prg32_sel = u32::from(v >> 4) & 3;
                    self.chr8_sel = u32::from(v & 3);
                }
            }
            71 => {
                if addr >= 0xC000 {
                    self.prg16_sel = u32::from(v & 0xF);
                } else if (0x9000..0xA000).contains(&addr) {
                    self.mirroring = if v & 0x10 != 0 { 4 } else { 3 }; // hanya Fire Hawk
                }
            }
            79 => {
                if (0x4100..0x6000).contains(&addr) && (addr & 0xE100) == 0x4100 {
                    self.prg32_sel = u32::from(v >> 3) & 1;
                    self.chr8_sel = u32::from(v & 7);
                }
            }
            _ => {}
        }
    }

    // ---- CHR ----

    /// Nomor bank 1 KB yang terlihat di `addr` (MMC3).
    pub fn chr_bank_1k(&self, addr: u16) -> u32 {
        let slot = u32::from((addr >> 10) & 7);
        let s = if self.chr_invert != 0 { slot ^ 4 } else { slot }; // inversi menukar dua setengah
        let b = if s < 2 {
            u32::from(self.regs[0] & 0xFE) + (s & 1)
        } else if s < 4 {
            u32::from(self.regs[1] & 0xFE) + (s & 1)
        } else {
            u32::from(self.regs[(s - 2) as usize])
        };
        b % self.chr_banks1k
    }

    /// PPU membaca $0000-$1FFF.
    pub fn chr_read(&self, addr: u16) -> u8 {
        let addr = addr & 0x1FFF;
        if self.chr_size == 0 {
            return self.chr_ram[addr as usize];
        }
        match self.id {
            1 => {
                let b = if self.mmc1_ctrl & 0x10 != 0 {
                    // dua bank 4 KB
                    u32::from(if addr < 0x1000 { self.mmc1_chr0 } else { self.mmc1_chr1 }) % self.chr_banks4k
                } else {
                    // satu bank 8 KB: bit terendah diabaikan
                    (u32::from(self.mmc1_chr0 & 0x1E) + u32::from(addr >= 0x1000)) % self.chr_banks4k
                };
                self.chr[(b * 4096 + u32::from(addr & 0xFFF)) as usize]
            }
            4 => self.chr[(self.chr_bank_1k(addr) * 1024 + u32::from(addr & 0x3FF)) as usize],
            3 | 11 | 66 | 79 => {
                self.chr[((self.chr8_sel % self.chr_banks8k) * 8192 + u32::from(addr)) as usize]
            }
            _ => self.chr[(u32::from(addr) % self.chr_size) as usize],
        }
    }

    pub fn chr_write(&mut self, addr: u16, v: u8) {
        if self.chr_size == 0 {
            self.chr_ram[(addr & 0x1FFF) as usize] = v;
        }
    }

    /// Mirroring nametable saat ini: 0 horizontal, 1 vertikal, 2 four-screen, 3 satu layar
    /// (nametable A), 4 satu layar (nametable B).
    pub fn mirroring(&self) -> u8 {
        if self.id == 1 {
            return match self.mmc1_ctrl & 3 {
                0 => 3, // satu layar, nametable A
                1 => 4, // satu layar, nametable B
                2 => 1, // vertikal
                _ => 0, // horizontal
            };
        }
        self.mirroring
    }

    // ---- IRQ (MMC3) ----

    fn clock_counter(&mut self) {
        if self.irq_counter == 0 || self.irq_reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 && self.irq_enabled {
            self.irq_pending = true;
        }
    }

    /// Perubahan jalur A12 alamat PPU, dengan waktu dalam dot PPU sejak awal.
    pub fn a12(&mut self, level: bool, dot_time: u64) {
        if self.id != 4 {
            return;
        }
        if level {
            if !self.a12 && dot_time.wrapping_sub(self.a12_low_since) >= A12_FILTER {
                self.clock_counter();
            }
            self.a12 = true;
        } else {
            if self.a12 {
                self.a12_low_since = dot_time;
            }
            self.a12 = false;
        }
    }

    /// Garis IRQ mapper (level): tetap menyala sampai game mengakuinya.
    pub fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    pub fn irq_enabled(&self) -> bool {
        self.irq_enabled
    }
}
