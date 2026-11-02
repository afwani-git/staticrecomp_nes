//! Hardware end-to-end: menu nestest (ROM tes bebas) dijalankan lewat hardware lengkap dengan tombol
//! Start ditekan, dan layar akhirnya dibandingkan dengan hash SHA-256 (16 hex pertama).
//!
//! Hash dikunci setelah layarnya diperiksa dengan mata: frame 30 = menu nestest, frame 900 =
//! menu yang sama dengan "OK" di depan tiap tes (penilaian nestest sendiri bahwa semua lulus).
//! Nilainya sama dengan yang dikunci saat hardware masih berupa C di WebAssembly, jadi tes ini
//! sekaligus membuktikan keluaran PPU hasil port ke Rust identik bit demi bit.
//! Bila PPU berubah dengan sengaja, perbarui hash. Dilewati bila berkas nestest tidak ada.

use recomp::config::Config;
use recomp::ines::InesHeader;

const HASH_MENU: &str = "05a14fb880377acf";
const HASH_ALL_OK: &str = "ab449105fc88cf76";

mod common;
use common::{build_game, gen_files, root, run_exe, NES_DRIVER};

/// Jalankan 900 frame dengan Start ditekan di frame 40-43, kembalikan stdout driver.
fn render(name: &str, cfg: &Config) -> Option<String> {
    let nes = root().join("roms/nestest/nestest.nes");
    if !nes.exists() {
        eprintln!("nestest tidak ada, tes dilewati");
        return None;
    }
    let data = std::fs::read(&nes).unwrap();
    let header = InesHeader::parse(&data).unwrap();
    let exe = build_game(name, &gen_files(&header, &data, cfg), NES_DRIVER);
    let run = run_exe(&exe, &["render", "900", "30,900", "start:40-43"]);
    assert!(run.ok && run.stdout.contains("OK frames=900"), "stdout: {}\nstderr: {}", run.stdout, run.stderr);
    Some(run.stdout)
}

#[test]
fn menu_nestest_tampil_dan_semua_tes_ok() {
    let cfg = Config::load(&root().join("builder/configs/nestest_ui.toml")).unwrap();
    let Some(out) = render("render_ui", &cfg) else { return };
    assert!(out.contains(&format!("HASH 30 {HASH_MENU}")), "layar menu berubah:\n{out}");
    assert!(out.contains(&format!("HASH 900 {HASH_ALL_OK}")), "layar akhir berubah:\n{out}");
}

#[test]
fn menu_nestest_tanpa_target_toml_identik_berkat_interpreter() {
    // Hanya entri $C000: tidak ada target lompatan, jadi semuanya jatuh ke interpreter.
    let cfg = Config {
        entry: vec![recomp::config::EntryConfig { addr: 0xC000 }],
        ..Config::default()
    };
    let Some(out) = render("render_kosong", &cfg) else { return };
    // Layar dan hasil sama persis dengan versi yang dikonfigurasi lengkap: cycle-exact sampai PPU.
    assert!(out.contains(&format!("HASH 30 {HASH_MENU}")), "layar menu beda:\n{out}");
    assert!(out.contains(&format!("HASH 900 {HASH_ALL_OK}")), "layar akhir beda:\n{out}");
    // Dan hardware melaporkan lompatan yang jatuh ke interpreter (bahan untuk melengkapi TOML).
    assert!(out.contains("INTERP ") && out.contains("C107->C72D") && out.contains("DB7B->DB7E"), "{out}");
}
