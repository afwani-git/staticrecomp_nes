//! Pencocokan waktu PPU dengan kolom "PPU:scanline,dot" di nestest.log. (Tes unit PPU, APU, dan mapper
//! ada di hardware/tests/.) Dilewati bila berkas nestest tidak ada.

use recomp::config::Config;
use recomp::ines::InesHeader;

mod common;
use common::{build_game, gen_files, root, run_exe, CPU_DRIVER};

/// "PPU:  0, 21" -> (0, 21)
fn log_ppu(line: &str) -> (i32, i32) {
    let rest = &line[line.find("PPU:").expect("tidak ada PPU:") + 4..];
    let rest = &rest[..rest.find("CYC:").expect("tidak ada CYC:")];
    let (a, b) = rest.split_once(',').unwrap();
    (a.trim().parse().unwrap(), b.trim().parse().unwrap())
}

#[test]
fn waktu_ppu_cocok_dengan_kolom_ppu_di_nestest_log() {
    let nes = root().join("roms/nestest/nestest.nes");
    let log = root().join("roms/nestest/nestest.log");
    if !nes.exists() || !log.exists() {
        eprintln!("nestest tidak ada, tes dilewati");
        return;
    }
    let data = std::fs::read(&nes).unwrap();
    let header = InesHeader::parse(&data).unwrap();
    let cfg = Config::load(&root().join("builder/configs/nestest.toml")).unwrap();

    let exe = build_game("nestest_ppu", &gen_files(&header, &data, &cfg), CPU_DRIVER);
    let run = run_exe(&exe, &["nestest-ppu"]);
    let ours: Vec<&str> = run.stdout.lines().collect();
    let reference = std::fs::read_to_string(&log).unwrap();
    let reference: Vec<&str> = reference.lines().collect();
    assert_eq!(ours.len(), reference.len(), "jumlah baris trace; stderr: {}", run.stderr);

    for (i, (got, want)) in ours.iter().zip(&reference).enumerate() {
        let f: Vec<&str> = got.split(' ').collect();
        let got_ppu = (f[1].parse::<i32>().unwrap(), f[2].parse::<i32>().unwrap());
        assert_eq!(got_ppu, log_ppu(want), "baris {} beda: PPU kita {:?}\n  nestest.log: {want}", i + 1, got_ppu);
    }
}
