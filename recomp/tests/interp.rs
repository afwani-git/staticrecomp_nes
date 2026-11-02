//! Interpreter cadangan: harus cocok dengan nestest.log baris demi baris, baik berjalan sendirian
//! maupun sebagai cadangan bagi hasil recompile yang tidak punya target lompatan di TOML.
//! Dilewati bila berkas nestest tidak ada.

use recomp::config::{Config, EntryConfig};
use recomp::ines::InesHeader;

mod common;
use common::{build_game, gen_files, root, run_exe, CPU_DRIVER};

/// "C000 A:00 X:00 Y:00 P:24 SP:FD CYC:7" dari satu baris log atau trace.
fn normalize(line: &str) -> String {
    let field = |p: &str| line.split_whitespace().find(|t| t.starts_with(p)).unwrap_or_else(|| panic!("{p} di: {line}"));
    format!("{} {} {} {} {} {} {}", &line[..4], field("A:"), field("X:"), field("Y:"), field("P:"), field("SP:"), field("CYC:"))
}

/// Generate, bangun driver CPU dalam `mode`, jalankan, dan cocokkan trace dengan seluruh nestest.log.
fn run_against_log(name: &str, cfg: &Config, mode: &str) {
    let nes = root().join("roms/nestest/nestest.nes");
    let log = root().join("roms/nestest/nestest.log");
    if !nes.exists() || !log.exists() {
        eprintln!("nestest tidak ada, tes dilewati");
        return;
    }
    let data = std::fs::read(&nes).unwrap();
    let header = InesHeader::parse(&data).unwrap();

    let exe = build_game(name, &gen_files(&header, &data, cfg), CPU_DRIVER);
    let run = run_exe(&exe, &[mode]);
    let ours: Vec<&str> = run.stdout.lines().collect();
    let reference = std::fs::read_to_string(&log).unwrap();
    let reference: Vec<&str> = reference.lines().collect();
    assert_eq!(ours.len(), reference.len(), "jumlah baris trace; stderr: {}", run.stderr);
    for (i, (got, want)) in ours.iter().zip(&reference).enumerate() {
        assert_eq!(normalize(got), normalize(want), "baris {} beda\n  nestest.log: {want}\n  hasil kita : {got}", i + 1);
    }
    assert!(run.ok, "program keluar dengan error: {}", run.stderr);
}

#[test]
fn interpreter_murni_cocok_dengan_seluruh_nestest_log() {
    // Tidak ada fungsi hasil recompile yang ikut ditautkan: interpreter menjalankan semuanya.
    run_against_log("interp_murni", &Config::default(), "nestest-interp");
}

#[test]
fn hasil_recompile_tanpa_target_toml_tetap_cocok_berkat_interpreter() {
    // Hanya entri $C000. Tanpa target untuk JMP indirect, push+RTS, RTI sebagai lompatan, atau kode
    // RAM di $0300; semuanya jatuh ke interpreter.
    let cfg = Config { entry: vec![EntryConfig { addr: 0xC000 }], ..Config::default() };
    run_against_log("interp_campuran", &cfg, "nestest-mixed");
}
