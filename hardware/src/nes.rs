//! "Komputer" NES di sekitar hasil recompile: peta memori CPU, pengendali, DMA sprite, APU, mapper,
//! dan penyela yang memajukan PPU serta mengirim NMI dan IRQ. Mapper: lihat `mapper`.

use std::panic::{self, AssertUnwindSafe};

use crate::apu::Apu;
use crate::cpu::{Bus, Cpu, Game};
use crate::mapper::Mapper;
use crate::ppu::Ppu;

/// Antarmuka hardware ke dunia luar (browser, tes native).
pub trait Host {
    /// Status tombol pengendali 1: bit 0 A, 1 B, 2 Select, 3 Start, 4 Atas, 5 Bawah, 6 Kiri, 7 Kanan.
    fn buttons(&mut self) -> u32;

    /// Dipanggil sekali per frame (saat vblank dimulai) dengan layar RGBA 256x240.
    /// Boleh memblokir untuk mengatur kecepatan 60 fps.
    fn present(&mut self, rgba: &[u8]);

    /// Dipanggil sekali per frame, sebelum `present`, dengan sampel suara mono (-1..1)
    /// pada laju yang diberikan ke `Nes::new`.
    fn audio(&mut self, _samples: &[f32]) {}

    /// Program berhenti. kind: 0 = kode tak dianalisis, 1 = lompatan dinamis (target = tujuannya),
    /// 2 = mapper tidak didukung (target = nomor mapper).
    fn fault(&mut self, _pc: u32, _target: u32, _kind: u32) {}

    /// Dipanggil sesudah `present`. Mengembalikan true untuk menghentikan program (hanya berlaku
    /// bila dijalankan lewat `Nes::run_until_stop`). `peek` membaca RAM dan PRG-RAM tanpa efek samping.
    fn stop(&mut self, _peek: &dyn Fn(u16) -> u8) -> bool {
        false
    }

    /// Dipanggil tiap kali kode hasil recompile menyerahkan kendali ke interpreter karena lompatan
    /// dinamis yang tidak ada di TOML. `from_pc` = alamat lompatan, `target` = tujuannya.
    fn interp(&mut self, _from_pc: u16, _target: u16) {}
}

/// Isi cartridge. Nomor mapper dan mirroring bawaan header ikut dihasilkan bersama ROM.
#[derive(Clone, Copy)]
pub struct Rom {
    pub prg: &'static [u8],
    /// Kosong = CHR-RAM 8 KB.
    pub chr: &'static [u8],
    /// 0 horizontal, 1 vertikal, 2 four-screen.
    pub mirroring: u8,
    pub mapper: u8,
}

/// Penanda untuk menghentikan `Nes::run` dari dalam host (lihat `Nes::run_until_stop`).
pub struct Stop;

pub struct NesBus {
    ram: [u8; 0x800],
    prg_ram: [u8; 0x2000],
    pub ppu: Ppu,
    pub apu: Apu,
    pub mapper: Mapper,
    pub host: Box<dyn Host>,
    pad_latch: u8,
    pad_strobe: bool,
    in_nmi: bool,
    irq_count: u32,
    irq_scanline_sum: u32,
    irq_log: [u16; 64], // scanline tiap IRQ dikirim (ring)
    i_delay: u8,        // instruksi yang harus lewat sebelum IRQ boleh diterima (setelah CLI/PLP)
    sample_rate: u32,
}

/// CPU lengkap dengan hardware NES.
pub type Nes = Cpu<NesBus>;

/// Pengamatan tanpa efek samping: RAM dan PRG-RAM saja.
fn peek_mem(ram: &[u8; 0x800], prg_ram: &[u8; 0x2000], addr: u16) -> u8 {
    if addr < 0x2000 {
        ram[(addr & 0x7FF) as usize]
    } else if (0x6000..0x8000).contains(&addr) {
        prg_ram[(addr & 0x1FFF) as usize]
    } else {
        0
    }
}

/// Pembacaan memori oleh DMC: RAM, PRG-RAM, dan mapper.
fn dmc_read(ram: &[u8; 0x800], prg_ram: &[u8; 0x2000], mapper: &Mapper, addr: u16) -> u8 {
    if addr < 0x2000 {
        ram[(addr & 0x7FF) as usize]
    } else if addr >= 0x8000 {
        mapper.cpu_read(addr)
    } else if addr >= 0x6000 {
        prg_ram[(addr & 0x1FFF) as usize]
    } else {
        0
    }
}

impl Bus for NesBus {
    #[inline(never)]
    fn read8(s: &mut Cpu<Self>, addr: u16) -> u8 {
        if addr < 0x2000 {
            return s.bus.ram[(addr & 0x7FF) as usize];
        }
        if addr < 0x4000 {
            let cyc = s.cycles;
            let b = &mut s.bus;
            let v = b.ppu.read(&mut b.mapper, addr, cyc);
            if b.ppu.nmi_pending || (b.mapper.irq_pending() && s.i == 0) {
                s.poll_at = 0; // membaca PPU memajukannya; mungkin NMI/IRQ baru terpicu
            }
            return v;
        }
        if addr >= 0x8000 {
            return s.bus.mapper.cpu_read(addr);
        }
        if addr == 0x4015 {
            let cyc = s.cycles;
            let NesBus { apu, ram, prg_ram, mapper, .. } = &mut s.bus;
            return apu.read_status(cyc, &mut |a| dmc_read(ram, prg_ram, mapper, a));
        }
        if addr == 0x4016 {
            let b = &mut s.bus;
            let bit = if b.pad_strobe {
                (b.host.buttons() & 1) as u8
            } else {
                let bit = b.pad_latch & 1;
                b.pad_latch = (b.pad_latch >> 1) | 0x80; // sesudah 8 bit, pengendali membaca 1
                bit
            };
            return 0x40 | bit;
        }
        if addr >= 0x6000 {
            return s.bus.prg_ram[(addr & 0x1FFF) as usize];
        }
        0 // APU, pengendali 2, dan area kosong
    }

    #[inline(never)]
    fn write8(s: &mut Cpu<Self>, addr: u16, value: u8) {
        if addr < 0x2000 {
            s.bus.ram[(addr & 0x7FF) as usize] = value;
        } else if addr < 0x4000 {
            let cyc = s.cycles;
            let b = &mut s.bus;
            b.ppu.write(&mut b.mapper, addr, value, cyc);
            if b.ppu.nmi_pending || (b.mapper.irq_pending() && s.i == 0) {
                s.poll_at = 0; // kirim NMI/IRQ di instruksi berikutnya
            }
        } else if addr == 0x4014 {
            let mut page = [0u8; 256];
            for (i, p) in page.iter_mut().enumerate() {
                *p = s.read8((u16::from(value) << 8) | i as u16);
            }
            s.bus.ppu.oam_dma(&page);
            s.cycles += 513 + (s.cycles & 1); // CPU berhenti selama DMA
        } else if addr == 0x4016 {
            let b = &mut s.bus;
            b.pad_strobe = value & 1 != 0;
            b.pad_latch = b.host.buttons() as u8;
        } else if addr <= 0x4013 || addr == 0x4015 || addr == 0x4017 {
            let cyc = s.cycles;
            let NesBus { apu, ram, prg_ram, mapper, .. } = &mut s.bus;
            apu.write(addr, value, cyc, &mut |a| dmc_read(ram, prg_ram, mapper, a));
        } else if addr >= 0x8000 {
            s.bus.mapper.cpu_write(addr, value);
            if s.bus.mapper.irq_pending() && s.i == 0 {
                s.poll_at = 0;
            }
        } else if addr >= 0x6000 {
            s.bus.prg_ram[(addr & 0x1FFF) as usize] = value;
        } else if addr >= 0x4020 {
            s.bus.mapper.cpu_write(addr, value); // register di area ekspansi (mis. mapper 79)
        }
    }

    #[inline(never)]
    fn poll(s: &mut Cpu<Self>) {
        sync_ppu(s);
        if s.bus.ppu.take_frame() {
            let cyc = s.cycles;
            let NesBus { apu, ram, prg_ram, mapper, ppu, host, .. } = &mut s.bus;
            apu.sync(cyc, &mut |a| dmc_read(ram, prg_ram, mapper, a));
            host.audio(&apu.samples);
            apu.samples.clear();
            host.present(&ppu.framebuffer);
            let (ram, prg_ram) = (&*ram, &*prg_ram);
            if host.stop(&|a| peek_mem(ram, prg_ram, a)) {
                panic::resume_unwind(Box::new(Stop));
            }
        }
        if s.bus.ppu.nmi_pending && !s.bus.in_nmi {
            s.bus.ppu.nmi_pending = false;
            s.bus.in_nmi = true;
            s.nmi();
            s.bus.in_nmi = false;
            sync_ppu(s);
        }
        if s.bus.i_delay != 0 {
            s.bus.i_delay -= 1; // CLI/PLP menunda IRQ satu instruksi
        }
        // Seperti CPU asli, IRQ hanya ditahan oleh flag I (dan jeda satu instruksi setelah CLI/PLP),
        // bukan oleh "sedang di dalam penangan NMI": penangan NMI yang panjang sering melakukan CLI
        // agar IRQ split-screen tetap masuk. Penangan IRQ menyalakan I, jadi tidak ada masuk berulang.
        if s.bus.mapper.irq_pending() && s.i == 0 && s.bus.i_delay == 0 {
            let scanline = s.bus.ppu.scanline();
            let b = &mut s.bus;
            b.irq_count = b.irq_count.wrapping_add(1);
            b.irq_scanline_sum = b.irq_scanline_sum.wrapping_add(scanline as u32);
            b.irq_log[(b.irq_count & 63) as usize] = (scanline * 4) as u16;
            s.irq();
            sync_ppu(s);
        }
        // Bangun lagi saat peristiwa berikutnya: vblank, atau clock scanline bila IRQ mapper aktif.
        // Akses register PPU memajukan PPU sendiri, jadi tidak perlu bangun tiap scanline.
        let b = &s.bus;
        let mut next = s.cycles + b.ppu.cycles_to_vblank();
        if b.mapper.irq_enabled() {
            let n = s.cycles + b.ppu.cycles_to_scanline_clock(&b.mapper);
            if n < next {
                next = n;
            }
        }
        if b.mapper.irq_pending() && s.i != 0 && s.cycles + 114 < next {
            next = s.cycles + 114; // IRQ tertahan flag I: cek lagi sekitar tiap scanline
        }
        if b.ppu.nmi_pending || (b.mapper.irq_pending() && s.i == 0) || b.i_delay != 0 {
            next = s.cycles + 1;
        }
        s.poll_at = next;
    }

    #[inline(never)]
    fn i_changed(s: &mut Cpu<Self>, delay: u8) {
        s.bus.i_delay = delay;
        s.poll_at = 0;
    }

    fn fault(s: &mut Cpu<Self>, pc: u16) -> ! {
        s.bus.host.fault(u32::from(pc), 0, 0);
        panic!("recomp: eksekusi sampai kode yang tidak dianalisis di ${pc:04X}");
    }

    fn interp_hook(s: &mut Cpu<Self>, from_pc: u16, target: u16) {
        s.bus.host.interp(from_pc, target);
    }
}

fn sync_ppu(s: &mut Nes) {
    let cyc = s.cycles;
    let b = &mut s.bus;
    b.ppu.sync(&mut b.mapper, cyc);
}

impl Nes {
    /// Membangun komputer NES (`Cpu::new` hanya untuk bus lain). Err(nomor mapper) bila mapper
    /// tidak didukung.
    pub fn build(game: Game<NesBus>, rom: Rom, host: Box<dyn Host>, sample_rate: u32) -> Result<Nes, u8> {
        let mapper = Mapper::new(rom.mapper, rom.prg, rom.chr, rom.mirroring).ok_or(rom.mapper)?;
        let bus = NesBus {
            ram: [0; 0x800],
            prg_ram: [0; 0x2000],
            ppu: Ppu::new(),
            apu: Apu::new(sample_rate),
            mapper,
            host,
            pad_latch: 0,
            pad_strobe: false,
            in_nmi: false,
            irq_count: 0,
            irq_scanline_sum: 0,
            irq_log: [0; 64],
            i_delay: 0,
            sample_rate,
        };
        let mut nes = Cpu::new(bus, game);
        nes.poll_at = 0;
        Ok(nes)
    }

    /// Laju sampel suara; panggil sebelum `run` (bawaan 48000).
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.bus.sample_rate = rate;
        self.bus.apu = Apu::new(rate);
    }

    /// Titik masuk: berjalan terus sampai game berakhir (biasanya tidak pernah).
    /// Fungsi RESET hasil recompile bisa kembali (mis. lewat push+RTS ke alamat yang tidak terdaftar);
    /// CPU sungguhan tidak pernah berhenti, jadi eksekusi dilanjutkan oleh interpreter cadangan.
    pub fn run(&mut self) -> ! {
        self.poll_at = 0;
        let mut pc = self.reset();
        loop {
            pc = self.interpret(pc);
        }
    }

    /// Seperti `run`, tetapi kembali bila host menghentikan program lewat
    /// `std::panic::resume_unwind(Box::new(Stop))`. Panic lain dilempar ulang. Untuk tes native.
    pub fn run_until_stop(&mut self) {
        let r = panic::catch_unwind(AssertUnwindSafe(|| -> () { self.run() }));
        if let Err(payload) = r {
            if payload.downcast_ref::<Stop>().is_none() {
                panic::resume_unwind(payload);
            }
        }
    }

    /// Pengamatan tanpa efek samping (untuk tes dan debugging): RAM dan PRG-RAM saja.
    pub fn peek(&self, addr: u16) -> u8 {
        peek_mem(&self.bus.ram, &self.bus.prg_ram, addr)
    }

    /// Untuk debugging: jumlah IRQ yang dikirim, scanline saat tiap IRQ, dan jumlah scanline-nya.
    pub fn irq_count(&self) -> u32 {
        self.bus.irq_count
    }
    pub fn irq_log(&self, i: u32) -> u32 {
        u32::from(self.bus.irq_log[(i & 63) as usize])
    }
    pub fn irq_scanline_sum(&self) -> u32 {
        self.bus.irq_scanline_sum
    }

    /// Untuk debugging: nilai PPUCTRL/PPUMASK terakhir.
    pub fn ppu_ctrl(&self) -> u8 {
        self.bus.ppu.reg_ctrl()
    }
    pub fn ppu_mask(&self) -> u8 {
        self.bus.ppu.reg_mask()
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.bus.ppu.framebuffer
    }
}
