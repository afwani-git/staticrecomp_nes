//! Runtime CPU 6502 untuk kode hasil recompile. Tidak ada logika hardware di sini:
//! semua akses memori lewat `Bus`, yang menentukan peta memori, waktu, dan interupsi.
//!
//! Kode hasil recompile berupa fungsi `fn(&mut Cpu<B>) -> u16` yang generik atas `B: Bus`.
//! Nilai kembaliannya adalah alamat tujuan RTS/RTI terakhir fungsi itu.

use crate::interp::{self, Op};

/// Peta memori dan hook waktu yang dipakai CPU. Semua fungsi menerima `Cpu` utuh karena
/// hardware perlu membaca siklus dan flag (mis. PPU dimajukan ke siklus sekarang).
pub trait Bus: Sized {
    fn read8(cpu: &mut Cpu<Self>, addr: u16) -> u8;
    fn write8(cpu: &mut Cpu<Self>, addr: u16, value: u8);

    /// Dipanggil di batas instruksi bila `cycles >= poll_at`. Hardware memakainya untuk memajukan
    /// PPU dan mengirim NMI/IRQ lewat `Cpu::nmi`/`Cpu::irq`. Bawaan: tidak ada.
    fn poll(_cpu: &mut Cpu<Self>) {}

    /// Dipanggil setelah instruksi yang bisa menurunkan flag I (CLI, PLP, RTI) supaya hardware
    /// memeriksa IRQ yang tertahan. `delay` = jumlah instruksi sebelum IRQ boleh diterima: CLI dan
    /// PLP menunda satu instruksi seperti CPU asli (delay 1), RTI tidak (delay 0).
    fn i_changed(_cpu: &mut Cpu<Self>, _delay: u8) {}

    /// Eksekusi sampai ke kode yang tidak dianalisis (BRK, opcode tak didukung, jalur di luar ROM).
    fn fault(_cpu: &mut Cpu<Self>, pc: u16) -> ! {
        panic!("recomp: eksekusi sampai kode yang tidak dianalisis di ${pc:04X}");
    }

    /// Kode hasil recompile menyerahkan kendali ke interpreter karena lompatan dinamis yang tidak
    /// terdaftar di TOML. Berguna untuk melengkapi TOML.
    fn interp_hook(_cpu: &mut Cpu<Self>, _from_pc: u16, _target: u16) {}
}

/// Tabel yang dihasilkan bersama kode recompile: titik masuk fungsi dan tabel opcode interpreter.
pub struct Game<B: Bus> {
    /// Bila `addr` adalah awal fungsi hasil recompile, jalankan fungsi itu dan kembalikan alamat
    /// tujuan RTS-nya. Bila bukan, None.
    pub call_entry: fn(&mut Cpu<B>, u16) -> Option<u16>,
    pub ops: &'static [Op; 256],
}

impl<B: Bus> Clone for Game<B> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<B: Bus> Copy for Game<B> {}

/// Satu baris trace (hanya dengan fitur `trace`): keadaan CPU sebelum instruksi di `pc`.
#[cfg(feature = "trace")]
#[derive(Clone, Copy, Debug)]
pub struct TraceEntry {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub p: u8,
    pub sp: u8,
    pub cycles: u64,
}

/// Register dan flag. Flag bernilai 0 atau 1. D disimpan tapi tidak dipakai ADC/SBC (NES).
pub struct Cpu<B: Bus> {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub n: u8,
    pub v: u8,
    pub d: u8,
    pub i: u8,
    pub z: u8,
    pub c: u8,
    /// Jumlah siklus CPU sejak reset.
    pub cycles: u64,
    /// `Bus::poll` dipanggil bila `cycles` sudah mencapai nilai ini.
    pub poll_at: u64,
    pub bus: B,
    pub game: Game<B>,
    #[cfg(feature = "trace")]
    pub trace_hook: Option<Box<dyn FnMut(TraceEntry)>>,
}

impl<B: Bus> Cpu<B> {
    pub fn new(bus: B, game: Game<B>) -> Cpu<B> {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            n: 0,
            v: 0,
            d: 0,
            i: 1,
            z: 0,
            c: 0,
            cycles: 7,
            poll_at: u64::MAX,
            bus,
            game,
            #[cfg(feature = "trace")]
            trace_hook: None,
        }
    }

    // ---- Akses memori dan hook ----

    #[inline(always)]
    pub fn read8(&mut self, addr: u16) -> u8 {
        B::read8(self, addr)
    }

    #[inline(always)]
    pub fn write8(&mut self, addr: u16, value: u8) {
        B::write8(self, addr, value)
    }

    /// Titik sela hardware, dipanggil di batas instruksi.
    #[inline(always)]
    pub fn poll(&mut self) {
        if self.cycles >= self.poll_at {
            B::poll(self);
        }
    }

    #[inline(always)]
    pub fn i_changed(&mut self, delay: u8) {
        B::i_changed(self, delay);
    }

    #[cold]
    pub fn fault(&mut self, pc: u16) -> ! {
        B::fault(self, pc)
    }

    /// Hook trace per instruksi; tanpa fitur `trace` kosong dan hilang saat kompilasi.
    #[inline(always)]
    pub fn trace(&mut self, _pc: u16) {
        #[cfg(feature = "trace")]
        if let Some(mut h) = self.trace_hook.take() {
            h(TraceEntry { pc: _pc, a: self.a, x: self.x, y: self.y, p: self.get_p(false), sp: self.sp, cycles: self.cycles });
            self.trace_hook = Some(h);
        }
    }

    // ---- Flag dan stack ----

    #[inline(always)]
    pub fn set_nz(&mut self, v: u8) {
        self.n = v >> 7;
        self.z = u8::from(v == 0);
    }

    /// Byte status. Bit 5 selalu 1; bit B (4) hanya 1 saat dipush oleh PHP/BRK.
    #[inline(always)]
    pub fn get_p(&self, b: bool) -> u8 {
        (self.n << 7)
            | (self.v << 6)
            | 0x20
            | if b { 0x10 } else { 0 }
            | (self.d << 3)
            | (self.i << 2)
            | (self.z << 1)
            | self.c
    }

    /// Bit B dan bit 5 diabaikan (tidak ada register fisiknya).
    #[inline(always)]
    pub fn set_p(&mut self, p: u8) {
        self.n = (p >> 7) & 1;
        self.v = (p >> 6) & 1;
        self.d = (p >> 3) & 1;
        self.i = (p >> 2) & 1;
        self.z = (p >> 1) & 1;
        self.c = p & 1;
    }

    #[inline(always)]
    pub fn push8(&mut self, v: u8) {
        self.write8(0x100 | u16::from(self.sp), v);
        self.sp = self.sp.wrapping_sub(1);
    }

    #[inline(always)]
    pub fn pull8(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.read8(0x100 | u16::from(self.sp))
    }

    /// Pointer 16-bit dari halaman nol; byte tinggi berputar di dalam halaman nol.
    #[inline(always)]
    pub fn rd16zp(&mut self, zp: u8) -> u16 {
        let lo = self.read8(u16::from(zp));
        let hi = self.read8(u16::from(zp.wrapping_add(1)));
        u16::from(lo) | (u16::from(hi) << 8)
    }

    // ---- ALU ----

    #[inline(always)]
    pub fn adc(&mut self, m: u8) {
        let s = u32::from(self.a) + u32::from(m) + u32::from(self.c);
        self.v = u8::from((!(self.a ^ m) & (self.a ^ s as u8) & 0x80) != 0);
        self.c = u8::from(s > 0xFF);
        self.a = s as u8;
        self.set_nz(self.a);
    }

    /// SBC = ADC dengan operand dibalik; carry berarti "tidak ada pinjaman".
    #[inline(always)]
    pub fn sbc(&mut self, m: u8) {
        self.adc(!m);
    }

    #[inline(always)]
    pub fn cmp(&mut self, r: u8, m: u8) {
        self.c = u8::from(r >= m);
        self.set_nz(r.wrapping_sub(m));
    }

    #[inline(always)]
    pub fn bit(&mut self, m: u8) {
        self.z = u8::from(self.a & m == 0);
        self.n = m >> 7;
        self.v = (m >> 6) & 1;
    }

    #[inline(always)]
    pub fn asl(&mut self, v: u8) -> u8 {
        self.c = v >> 7;
        let v = v << 1;
        self.set_nz(v);
        v
    }

    #[inline(always)]
    pub fn lsr(&mut self, v: u8) -> u8 {
        self.c = v & 1;
        let v = v >> 1;
        self.set_nz(v);
        v
    }

    #[inline(always)]
    pub fn rol(&mut self, v: u8) -> u8 {
        let c = self.c;
        self.c = v >> 7;
        let v = (v << 1) | c;
        self.set_nz(v);
        v
    }

    #[inline(always)]
    pub fn ror(&mut self, v: u8) -> u8 {
        let c = self.c;
        self.c = v & 1;
        let v = (v >> 1) | (c << 7);
        self.set_nz(v);
        v
    }

    // ---- Interpreter dan titik masuk CPU ----

    /// Jalankan mulai `pc` dengan interpreter cadangan; lihat `interp::interpret`.
    pub fn interpret(&mut self, pc: u16) -> u16 {
        interp::interpret(self, pc)
    }

    /// Sama, tetapi memberi tahu `Bus::interp_hook` dari mana lompatan itu datang.
    pub fn interpret_from(&mut self, from_pc: u16, target: u16) -> u16 {
        B::interp_hook(self, from_pc, target);
        interp::interpret(self, target)
    }

    fn enter_vector(&mut self, vec: u16) -> u16 {
        let lo = self.read8(vec);
        let hi = self.read8(vec.wrapping_add(1));
        let target = u16::from(lo) | (u16::from(hi) << 8);
        match (self.game.call_entry)(self, target) {
            Some(r) => r,
            None => self.interpret(target),
        }
    }

    /// RESET: vektor dibaca dari memori saat dipanggil, jadi bekerja untuk bank mana pun.
    /// Mengembalikan alamat tujuan RTS terakhirnya (lanjutkan dengan `interpret`).
    pub fn reset(&mut self) -> u16 {
        self.a = 0;
        self.x = 0;
        self.y = 0;
        self.sp = 0xFD;
        self.n = 0;
        self.v = 0;
        self.d = 0;
        self.i = 1;
        self.z = 0;
        self.c = 0;
        self.cycles = 7;
        self.enter_vector(0xFFFC)
    }

    pub fn nmi(&mut self) {
        self.push8(0x00);
        self.push8(0x00);
        let p = self.get_p(false);
        self.push8(p);
        self.i = 1;
        self.cycles += 7;
        let _ = self.enter_vector(0xFFFA);
    }

    pub fn irq(&mut self) {
        self.push8(0x00);
        self.push8(0x00);
        let p = self.get_p(false);
        self.push8(p);
        self.i = 1;
        self.cycles += 7;
        let _ = self.enter_vector(0xFFFE);
    }
}

/// Memori datar 64 KB tanpa hardware: untuk menguji CPU saja (tidak pernah memanggil poll).
pub struct FlatBus {
    pub mem: Box<[u8; 0x10000]>,
}

impl FlatBus {
    pub fn new() -> FlatBus {
        FlatBus { mem: Box::new([0; 0x10000]) }
    }

    /// Salin PRG-ROM ke alamat CPU-nya: 16 KB tampil di $8000 dan $C000, 32 KB di $8000.
    pub fn with_prg(prg: &[u8]) -> FlatBus {
        let mut b = FlatBus::new();
        b.mem[0x8000..0x8000 + prg.len()].copy_from_slice(prg);
        if prg.len() == 0x4000 {
            b.mem[0xC000..0xC000 + prg.len()].copy_from_slice(prg);
        }
        b
    }
}

impl Default for FlatBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for FlatBus {
    fn read8(cpu: &mut Cpu<Self>, addr: u16) -> u8 {
        cpu.bus.mem[addr as usize]
    }
    fn write8(cpu: &mut Cpu<Self>, addr: u16, value: u8) {
        cpu.bus.mem[addr as usize] = value;
    }
}
