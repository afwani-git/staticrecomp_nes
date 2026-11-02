//! ROM tes MMC3 gratis dari blargg (mmc3_test_2), dijalankan lewat hardware lengkap.
//! Protokol: $6000 = status, $6004... = teks.
//!
//! Hasil yang diharapkan (dikunci supaya perubahan terdeteksi):
//!  - 1-clocking, 2-details, 3-A12_clocking, 5-MMC3: LULUS.
//!  - 4-scanline_timing: GAGAL di tes #2. Tes ini menuntut IRQ tepat ke satu dot PPU; hardware ini
//!    menerima IRQ di batas instruksi dan menggambar per scanline, jadi tidak bisa seteliti itu.
//!  - 6-MMC3_alt: GAGAL di tes #2, memang diharapkan. Ia menguji varian chip MMC3 lama yang
//!    berlawanan dengan 5-MMC3; hardware ini mengikuti varian baru yang dipakai kebanyakan game.
//! Dilewati bila berkas ROM tes tidak ada.

use recomp::config::Config;
use recomp::ines::InesHeader;

mod common;
use common::{build_game, gen_files, root, run_exe, NES_DRIVER};

/// (nama berkas, status yang diharapkan di $6000)
const CASES: [(&str, u8); 6] = [
    ("1-clocking", 0x00),
    ("2-details", 0x00),
    ("3-A12_clocking", 0x00),
    ("4-scanline_timing", 0x02),
    ("5-MMC3", 0x00),
    ("6-MMC3_alt", 0x02),
];

#[test]
fn rom_tes_mmc3_blargg_sesuai_hasil_yang_diharapkan() {
    let dir_roms = root().join("roms/mmc3_test_2");
    if !dir_roms.exists() {
        eprintln!("ROM tes tidak ada, tes dilewati");
        return;
    }
    for (name, expected) in CASES {
        let data = std::fs::read(dir_roms.join(format!("{name}.nes"))).unwrap();
        let header = InesHeader::parse(&data).unwrap();
        let exe = build_game(
            &format!("blargg_{name}"),
            &gen_files(&header, &data, &Config::default()),
            NES_DRIVER,
        );
        let run = run_exe(&exe, &["blargg", "2400"]);
        let want = format!("STATUS {expected:02x} ");
        assert!(run.stdout.starts_with(&want), "{name}: diharapkan '{want}', hasilnya:\n{}\n{}", run.stdout, run.stderr);
    }
}
