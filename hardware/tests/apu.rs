//! Tes unit APU. Frekuensi dihitung dari zero crossing dan dibandingkan dengan rumus nesdev:
//! pulse f = CPU/(16*(t+1)), triangle f = CPU/(32*(t+1)).

use hardware::apu::Apu;

const CPU_HZ: f64 = 1_789_773.0;
const RATE: u32 = 48000;

struct Stats {
    crossings: i32,
    min: f32,
    max: f32,
    sumsq: f64,
    n: i64,
}

impl Stats {
    fn rms(&self) -> f64 {
        (self.sumsq / self.n as f64).sqrt()
    }
}

struct T {
    apu: Apu,
    cyc: u64,
}

impl T {
    fn fresh() -> T {
        T { apu: Apu::new(RATE), cyc: 0 }
    }

    fn w(&mut self, addr: u16, v: u8) {
        self.apu.write(addr, v, self.cyc, &mut |_| 0xFF);
    }

    fn status(&mut self) -> u8 {
        self.apu.read_status(self.cyc, &mut |_| 0xFF)
    }

    /// Jalankan `seconds` detik; statistik hanya dihitung setelah `skip` detik pertama.
    fn run(&mut self, seconds: f64, skip: f64) -> Stats {
        let mut s = Stats { crossings: 0, min: 1e9, max: -1e9, sumsq: 0.0, n: 0 };
        let mut total = 0i64;
        let skip_n = (skip * f64::from(RATE)) as i64;
        let mut prev = 0f32;
        let end = self.cyc + (seconds * CPU_HZ) as u64;
        while self.cyc < end {
            self.cyc = (self.cyc + 4000).min(end);
            self.apu.sync(self.cyc, &mut |_| 0xFF);
            for &x in &self.apu.samples {
                if total >= skip_n {
                    if prev < 0.0 && x >= 0.0 {
                        s.crossings += 1;
                    }
                    s.min = s.min.min(x);
                    s.max = s.max.max(x);
                    s.sumsq += f64::from(x) * f64::from(x);
                    s.n += 1;
                }
                prev = x;
                total += 1;
            }
            self.apu.samples.clear();
        }
        s
    }
}

#[test]
fn hening_bila_semua_kanal_mati() {
    let mut t = T::fresh();
    let s = t.run(0.5, 0.1);
    assert!(s.max < 1e-4 && s.min > -1e-4, "hening bila semua kanal mati");
}

#[test]
fn pulse_1_frekuensi_dan_amplitudo() {
    // Duty 50%, volume tetap 15, periode 253 -> 440.4 Hz.
    let mut t = T::fresh();
    t.w(0x4015, 0x01);
    t.w(0x4000, 0xBF);
    t.w(0x4002, 0xFD);
    t.w(0x4003, 0x00);
    let s = t.run(1.2, 0.2);
    assert!((s.crossings - 440).abs() <= 3, "frekuensi pulse 440 Hz: {}", s.crossings);
    // Mixer: lompatan A = 95.52 / (8128/15 + 100) = 0.1488. Filter DC (0.996 per sampel) membuat
    // gelombang kotak melorot, sehingga puncak-ke-puncak = 2A / (1 + 0.996^N), N = sampel per
    // setengah periode.
    let a = 95.52 / (8128.0 / 15.0 + 100.0);
    let f = CPU_HZ / (16.0 * 254.0);
    let pp = 2.0 * a / (1.0 + 0.996f64.powf(f64::from(RATE) / (2.0 * f)));
    let got = f64::from(s.max - s.min);
    assert!((got - pp).abs() < 0.05 * pp, "amplitudo pulse volume 15 sesuai rumus mixer: {got} vs {pp}");
}

#[test]
fn pulse_2_bekerja_sama() {
    let mut t = T::fresh();
    t.w(0x4015, 0x02);
    t.w(0x4004, 0xBF);
    t.w(0x4006, 0xFD);
    t.w(0x4007, 0x00);
    let s = t.run(1.2, 0.2);
    assert!((s.crossings - 440).abs() <= 3, "frekuensi pulse 2: {}", s.crossings);
}

#[test]
fn triangle_frekuensi() {
    // Linear counter ditahan, periode 253 -> 220.2 Hz.
    let mut t = T::fresh();
    t.w(0x4015, 0x04);
    t.w(0x4008, 0xFF);
    t.w(0x400A, 0xFD);
    t.w(0x400B, 0x00);
    let s = t.run(1.2, 0.2);
    assert!((s.crossings - 220).abs() <= 3, "frekuensi triangle 220 Hz: {}", s.crossings);
}

#[test]
fn noise_berbunyi() {
    let mut t = T::fresh();
    t.w(0x4015, 0x08);
    t.w(0x400C, 0x3F);
    t.w(0x400E, 0x04);
    t.w(0x400F, 0x00);
    let s = t.run(0.5, 0.1);
    assert!(s.rms() > 0.01, "noise berbunyi: rms {}", s.rms());
}

#[test]
fn periode_pulse_di_bawah_8_dibisukan() {
    let mut t = T::fresh();
    t.w(0x4015, 0x01);
    t.w(0x4000, 0xBF);
    t.w(0x4002, 0x05);
    t.w(0x4003, 0x00);
    let s = t.run(0.5, 0.1);
    assert!(s.max < 1e-3 && s.min > -1e-3, "periode pulse < 8 hening");
}

#[test]
fn length_counter_habis_sekitar_83_ms() {
    // Indeks 0 = 10 setengah-frame (120 Hz).
    let mut t = T::fresh();
    t.w(0x4015, 0x01);
    t.w(0x4000, 0x1F); // halt mati
    t.w(0x4002, 0xFD);
    t.w(0x4003, 0x00);
    assert_eq!(t.status() & 1, 1, "status pulse menyala setelah dimuat");
    t.run(0.05, 0.0);
    assert_eq!(t.status() & 1, 1, "pulse masih berbunyi di 50 ms");
    t.run(0.07, 0.0);
    assert_eq!(t.status() & 1, 0, "length counter habis setelah ~83 ms");
}

#[test]
fn mematikan_kanal_menghapus_length() {
    let mut t = T::fresh();
    t.w(0x4015, 0x01);
    t.w(0x4003, 0x08);
    t.w(0x4015, 0x00);
    assert_eq!(t.status() & 1, 0, "$4015 mematikan kanal");
    t.w(0x4003, 0x08);
    assert_eq!(t.status() & 1, 0, "kanal mati tidak memuat length");
}

#[test]
fn dmc_membaca_memori_dan_menaikkan_level() {
    // 17 byte $FF dari $C000 -> level naik 2 per bit sampai 126, lalu selesai.
    let mut t = T::fresh();
    t.w(0x4011, 0x00);
    t.w(0x4010, 0x0F);
    t.w(0x4012, 0x00);
    t.w(0x4013, 0x01);
    t.w(0x4015, 0x10);
    assert_ne!(t.status() & 0x10, 0, "DMC aktif setelah dimulai");
    t.run(0.01, 0.0);
    assert_eq!(t.apu.dmc_level(), 126, "level DMC naik sampai 126");
    assert_eq!(t.status() & 0x10, 0, "DMC selesai");
}
