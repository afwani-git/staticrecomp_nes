//! APU NES. Tabel (panjang, duty, segitiga, periode noise, laju DMC) dan rumus mixer diambil
//! dari fogleman/nes (nes/apu.go), bukan diketik dari ingatan. Perilaku kanal mengikuti
//! dokumentasi APU di nesdev wiki. Penyederhanaan yang disengaja:
//!  - IRQ frame counter dan IRQ DMC tidak dikirim ke CPU;
//!  - pembacaan DMC tidak menghentikan CPU (stall);
//!  - downsampling memakai rata-rata per periode sampel ditambah filter penghilang DC.
//!
//! Tidak pasti: timer noise di sini diclock tiap siklus CPU dengan tabel dalam siklus CPU
//! (pemahaman saya dari nesdev); referensi di atas mengclock-nya tiap 2 siklus.
//!
//! Tidak bergantung pada CPU atau host: waktu diberikan lewat siklus CPU, dan DMC membaca memori
//! lewat fungsi yang diberikan pemanggil.

pub const MAX_SAMPLES: usize = 8192;

const CPU_HZ: f64 = 1_789_773.0;
const FRAME_STEP_CYCLES: u32 = 7457; // frame counter ~240 Hz

const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72,
    26, 16, 28, 32, 30,
];
const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];
const TRIANGLE_TABLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];
const NOISE_TABLE: [u16; 16] = [4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068];
/// Dalam siklus APU (= 2 siklus CPU).
const DMC_TABLE: [u8; 16] = [214, 190, 170, 160, 143, 127, 113, 107, 95, 80, 71, 64, 53, 42, 36, 27];

/// Pembaca memori CPU untuk DMC.
pub type ReadMem<'a> = &'a mut dyn FnMut(u16) -> u8;

#[derive(Default, Clone)]
struct Envelope {
    start: bool,
    looping: bool,
    constant: bool,
    period: u8,
    divider: u8,
    decay: u8,
    volume: u8,
}

impl Envelope {
    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.period;
        } else if self.divider != 0 {
            self.divider -= 1;
        } else {
            self.divider = self.period;
            if self.decay != 0 {
                self.decay -= 1;
            } else if self.looping {
                self.decay = 15;
            }
        }
    }

    fn volume(&self) -> u8 {
        if self.constant { self.volume } else { self.decay }
    }

    fn write(&mut self, value: u8) {
        self.looping = (value >> 5) & 1 != 0;
        self.constant = (value >> 4) & 1 != 0;
        self.period = value & 15;
        self.volume = value & 15;
    }
}

#[derive(Default, Clone)]
struct Pulse {
    enabled: bool,
    ones_complement: bool,
    length: u8,
    halt: bool,
    duty: u8,
    step: u8,
    period: u16,
    timer: u16,
    env: Envelope,
    sweep_enabled: bool,
    sweep_negate: bool,
    sweep_reload: bool,
    sweep_period: u8,
    sweep_shift: u8,
    sweep_divider: u8,
}

impl Pulse {
    fn sweep_target(&self) -> u16 {
        let delta = self.period >> self.sweep_shift;
        if !self.sweep_negate {
            return self.period.wrapping_add(delta);
        }
        if u32::from(delta) + u32::from(self.ones_complement) > u32::from(self.period) {
            return 0;
        }
        self.period - delta - u16::from(self.ones_complement)
    }

    fn muted(&self) -> bool {
        self.period < 8 || (!self.sweep_negate && self.sweep_target() > 0x7FF)
    }

    fn sweep_clock(&mut self) {
        if self.sweep_divider == 0 && self.sweep_enabled && self.sweep_shift != 0 && !self.muted() {
            self.period = self.sweep_target();
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.length == 0 || self.muted() || DUTY_TABLE[self.duty as usize][self.step as usize] == 0 {
            return 0;
        }
        self.env.volume()
    }

    fn write(&mut self, reg: u16, value: u8) {
        match reg {
            0 => {
                self.duty = value >> 6;
                self.halt = (value >> 5) & 1 != 0;
                self.env.write(value);
            }
            1 => {
                self.sweep_enabled = value >> 7 != 0;
                self.sweep_period = (value >> 4) & 7;
                self.sweep_negate = (value >> 3) & 1 != 0;
                self.sweep_shift = value & 7;
                self.sweep_reload = true;
            }
            2 => self.period = (self.period & 0x700) | u16::from(value),
            3 => {
                self.period = (self.period & 0xFF) | (u16::from(value & 7) << 8);
                if self.enabled {
                    self.length = LENGTH_TABLE[(value >> 3) as usize];
                }
                self.step = 0;
                self.env.start = true;
            }
            _ => {}
        }
    }
}

#[derive(Default, Clone)]
struct Triangle {
    enabled: bool,
    halt: bool,
    reload: bool,
    length: u8,
    linear: u8,
    linear_period: u8,
    step: u8,
    period: u16,
    timer: u16,
}

impl Triangle {
    fn linear_clock(&mut self) {
        if self.reload {
            self.linear = self.linear_period;
        } else if self.linear != 0 {
            self.linear -= 1;
        }
        if !self.halt {
            self.reload = false;
        }
    }
}

#[derive(Default, Clone)]
struct Noise {
    enabled: bool,
    halt: bool,
    mode: bool,
    length: u8,
    period: u16,
    timer: u16,
    shift: u16,
    env: Envelope,
}

impl Noise {
    fn timer_clock(&mut self) {
        if self.timer != 0 {
            self.timer -= 1;
            return;
        }
        self.timer = self.period.wrapping_sub(1);
        let feedback = (self.shift ^ (self.shift >> if self.mode { 6 } else { 1 })) & 1;
        self.shift = (self.shift >> 1) | (feedback << 14);
    }
}

#[derive(Default, Clone)]
struct Dmc {
    enabled: bool,
    looping: bool,
    level: u8,
    rate: u8,
    timer: u8,
    shift_reg: u8,
    bits: u8,
    buffer: u8,
    buffer_full: bool,
    sample_addr: u16,
    sample_len: u16,
    addr: u16,
    remaining: u16,
}

impl Dmc {
    fn restart(&mut self) {
        self.addr = self.sample_addr;
        self.remaining = self.sample_len;
    }

    fn fill_buffer(&mut self, read: ReadMem) {
        if self.buffer_full || self.remaining == 0 {
            return;
        }
        self.buffer = read(self.addr);
        self.buffer_full = true;
        self.addr = if self.addr == 0xFFFF { 0x8000 } else { self.addr + 1 };
        self.remaining -= 1;
        if self.remaining == 0 && self.looping {
            self.restart();
        }
    }

    fn timer_clock(&mut self, read: ReadMem) {
        self.fill_buffer(read);
        if self.timer != 0 {
            self.timer -= 1;
            return;
        }
        self.timer = self.rate.wrapping_sub(1);
        if self.bits == 0 {
            if !self.buffer_full {
                return; // diam: tidak ada data
            }
            self.shift_reg = self.buffer;
            self.buffer_full = false;
            self.bits = 8;
            self.fill_buffer(read);
        }
        if self.shift_reg & 1 != 0 {
            if self.level <= 125 {
                self.level += 2;
            }
        } else if self.level >= 2 {
            self.level -= 2;
        }
        self.shift_reg >>= 1;
        self.bits -= 1;
    }
}

pub struct Apu {
    /// Sampel hasil (-1..1). Pemakai membaca isinya lalu mengosongkannya (`clear`).
    pub samples: Vec<f32>,

    pulse: [Pulse; 2],
    tri: Triangle,
    noise: Noise,
    dmc: Dmc,
    five_step: bool,
    frame_cycle: u32,
    frame_index: u32,
    cycle: u64,

    sample_period: f64,
    sample_clock: f64,
    accum: f64,
    accum_n: u32,
    hp_prev_in: f32,
    hp_prev_out: f32,
}

impl Apu {
    pub fn new(sample_rate: u32) -> Apu {
        let mut pulse: [Pulse; 2] = Default::default();
        pulse[0].ones_complement = true; // pulse 1 mengurangi satu lagi saat sweep negatif
        let noise = Noise { shift: 1, period: NOISE_TABLE[0], ..Default::default() };
        let dmc = Dmc { rate: DMC_TABLE[0], ..Default::default() };
        Apu {
            samples: Vec::with_capacity(MAX_SAMPLES),
            pulse,
            tri: Triangle::default(),
            noise,
            dmc,
            five_step: false,
            frame_cycle: 0,
            frame_index: 0,
            cycle: 0,
            sample_period: CPU_HZ / f64::from(if sample_rate != 0 { sample_rate } else { 48000 }),
            sample_clock: 0.0,
            accum: 0.0,
            accum_n: 0,
            hp_prev_in: 0.0,
            hp_prev_out: 0.0,
        }
    }

    // ---- Frame counter ----

    fn quarter_frame(&mut self) {
        self.pulse[0].env.clock();
        self.pulse[1].env.clock();
        self.noise.env.clock();
        self.tri.linear_clock();
    }

    fn half_frame(&mut self) {
        for p in &mut self.pulse {
            if !p.halt && p.length != 0 {
                p.length -= 1;
            }
            p.sweep_clock();
        }
        if !self.tri.halt && self.tri.length != 0 {
            self.tri.length -= 1;
        }
        if !self.noise.halt && self.noise.length != 0 {
            self.noise.length -= 1;
        }
    }

    fn frame_counter_step(&mut self) {
        if !self.five_step {
            self.quarter_frame();
            if self.frame_index == 1 || self.frame_index == 3 {
                self.half_frame();
            }
            self.frame_index = (self.frame_index + 1) % 4;
        } else {
            if self.frame_index != 3 {
                self.quarter_frame(); // langkah ke-4 dari 5 kosong
            }
            if self.frame_index == 1 || self.frame_index == 4 {
                self.half_frame();
            }
            self.frame_index = (self.frame_index + 1) % 5;
        }
    }

    // ---- Pencampuran dan sampel ----

    fn mix(&self) -> f32 {
        let p = u32::from(self.pulse[0].output()) + u32::from(self.pulse[1].output());
        let t = if self.tri.length != 0 && self.tri.linear != 0 && self.tri.period >= 2 {
            u32::from(TRIANGLE_TABLE[self.tri.step as usize])
        } else {
            0
        };
        let n = if self.noise.length != 0 && self.noise.shift & 1 == 0 {
            u32::from(self.noise.env.volume())
        } else {
            0
        };
        let tnd = 3 * t + 2 * n + u32::from(self.dmc.level);
        let pulse_out = if p != 0 { 95.52f32 / (8128.0f32 / p as f32 + 100.0f32) } else { 0.0 };
        let tnd_out = if tnd != 0 { 163.67f32 / (24329.0f32 / tnd as f32 + 100.0f32) } else { 0.0 };
        pulse_out + tnd_out
    }

    fn emit_sample(&mut self, x: f32) {
        // Penghilang DC (high-pass ~ 30 Hz): sinyal APU selalu positif.
        let y = x - self.hp_prev_in + 0.996f32 * self.hp_prev_out;
        self.hp_prev_in = x;
        self.hp_prev_out = y;
        if self.samples.len() < MAX_SAMPLES {
            self.samples.push(y);
        }
    }

    fn step(&mut self, read: ReadMem) {
        // Triangle dan noise per siklus CPU; pulse dan DMC per siklus APU (2 siklus CPU).
        if self.tri.timer != 0 {
            self.tri.timer -= 1;
        } else {
            self.tri.timer = self.tri.period;
            if self.tri.length != 0 && self.tri.linear != 0 {
                self.tri.step = (self.tri.step + 1) & 31;
            }
        }
        self.noise.timer_clock();
        if self.cycle & 1 != 0 {
            for p in &mut self.pulse {
                if p.timer != 0 {
                    p.timer -= 1;
                } else {
                    p.timer = p.period;
                    p.step = (p.step + 1) & 7;
                }
            }
            self.dmc.timer_clock(read);
        }
        self.frame_cycle += 1;
        if self.frame_cycle >= FRAME_STEP_CYCLES {
            self.frame_cycle = 0;
            self.frame_counter_step();
        }

        self.accum += f64::from(self.mix());
        self.accum_n += 1;
        self.sample_clock += 1.0;
        if self.sample_clock >= self.sample_period {
            self.sample_clock -= self.sample_period;
            let x = (self.accum / f64::from(self.accum_n)) as f32;
            self.emit_sample(x);
            self.accum = 0.0;
            self.accum_n = 0;
        }
        self.cycle += 1;
    }

    /// Majukan APU sampai siklus CPU `cpu_cycle`, menghasilkan sampel di sepanjang jalan.
    pub fn sync(&mut self, cpu_cycle: u64, read: ReadMem) {
        while self.cycle < cpu_cycle {
            self.step(read);
        }
    }

    // ---- Register ----

    /// Tulis register $4000-$4013, $4015, $4017. Memajukan APU ke `cpu_cycle` lebih dulu.
    pub fn write(&mut self, addr: u16, value: u8, cpu_cycle: u64, read: ReadMem) {
        self.sync(cpu_cycle, read);
        if addr < 0x4008 {
            self.pulse[((addr >> 2) & 1) as usize].write(addr & 3, value);
            return;
        }
        match addr {
            0x4008 => {
                self.tri.halt = value >> 7 != 0;
                self.tri.linear_period = value & 0x7F;
            }
            0x400A => self.tri.period = (self.tri.period & 0x700) | u16::from(value),
            0x400B => {
                self.tri.period = (self.tri.period & 0xFF) | (u16::from(value & 7) << 8);
                if self.tri.enabled {
                    self.tri.length = LENGTH_TABLE[(value >> 3) as usize];
                }
                self.tri.reload = true;
            }
            0x400C => {
                self.noise.halt = (value >> 5) & 1 != 0;
                self.noise.env.write(value);
            }
            0x400E => {
                self.noise.mode = value >> 7 != 0;
                self.noise.period = NOISE_TABLE[(value & 15) as usize];
            }
            0x400F => {
                if self.noise.enabled {
                    self.noise.length = LENGTH_TABLE[(value >> 3) as usize];
                }
                self.noise.env.start = true;
            }
            0x4010 => {
                self.dmc.looping = (value >> 6) & 1 != 0;
                self.dmc.rate = DMC_TABLE[(value & 15) as usize];
            }
            0x4011 => self.dmc.level = value & 0x7F,
            0x4012 => self.dmc.sample_addr = 0xC000 | (u16::from(value) << 6),
            0x4013 => self.dmc.sample_len = (u16::from(value) << 4) | 1,
            0x4015 => {
                self.pulse[0].enabled = value & 1 != 0;
                self.pulse[1].enabled = (value >> 1) & 1 != 0;
                self.tri.enabled = (value >> 2) & 1 != 0;
                self.noise.enabled = (value >> 3) & 1 != 0;
                self.dmc.enabled = (value >> 4) & 1 != 0;
                if !self.pulse[0].enabled {
                    self.pulse[0].length = 0;
                }
                if !self.pulse[1].enabled {
                    self.pulse[1].length = 0;
                }
                if !self.tri.enabled {
                    self.tri.length = 0;
                }
                if !self.noise.enabled {
                    self.noise.length = 0;
                }
                if !self.dmc.enabled {
                    self.dmc.remaining = 0;
                } else if self.dmc.remaining == 0 {
                    self.dmc.restart();
                }
            }
            0x4017 => {
                self.five_step = value >> 7 != 0;
                self.frame_cycle = 0;
                self.frame_index = 0;
                if self.five_step {
                    // mode 5 langkah langsung mengclock semua unit
                    self.quarter_frame();
                    self.half_frame();
                }
            }
            _ => {}
        }
    }

    /// Baca $4015. Memajukan APU ke `cpu_cycle` lebih dulu.
    pub fn read_status(&mut self, cpu_cycle: u64, read: ReadMem) -> u8 {
        self.sync(cpu_cycle, read);
        u8::from(self.pulse[0].length != 0)
            | (u8::from(self.pulse[1].length != 0) << 1)
            | (u8::from(self.tri.length != 0) << 2)
            | (u8::from(self.noise.length != 0) << 3)
            | (u8::from(self.dmc.remaining != 0) << 4)
    }

    /// Pengamatan untuk tes.
    pub fn dmc_level(&self) -> u8 {
        self.dmc.level
    }
}
