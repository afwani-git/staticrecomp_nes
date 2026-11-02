//! Meng-recompile ROM menjadi kode Rust saat build. ROM dipilih lewat variabel lingkungan
//! `NES_ROM` (dan `NES_CONFIG` untuk TOML target lompatan), yang diisi oleh `recomp build`.
//! Tanpa `NES_ROM`, dipakai nestest (ROM tes bebas) supaya `cargo build` biasa tetap jalan.

use std::env;
use std::path::PathBuf;

use recomp::analysis::analyze;
use recomp::codegen::generate;
use recomp::config::Config;
use recomp::ines::InesHeader;

fn main() {
    println!("cargo:rerun-if-env-changed=NES_ROM");
    println!("cargo:rerun-if-env-changed=NES_CONFIG");
    println!("cargo:rerun-if-changed=build.rs");

    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let (rom, config) = match env::var_os("NES_ROM") {
        Some(rom) => (PathBuf::from(rom), env::var_os("NES_CONFIG").map(PathBuf::from)),
        None => (root.join("roms/nestest/nestest.nes"), Some(root.join("builder/configs/nestest_ui.toml"))),
    };
    println!("cargo:rerun-if-changed={}", rom.display());
    if let Some(c) = &config {
        println!("cargo:rerun-if-changed={}", c.display());
    }

    let data = std::fs::read(&rom).unwrap_or_else(|e| panic!("gagal membaca ROM {}: {e}", rom.display()));
    let header = InesHeader::parse(&data).unwrap_or_else(|e| panic!("header tidak valid di {}: {e:#}", rom.display()));
    let cfg = match &config {
        Some(p) => Config::load(p).unwrap_or_else(|e| panic!("gagal membaca config {}: {e:#}", p.display())),
        None => Config::default(),
    };
    let an = analyze(&header, &data, &cfg).unwrap_or_else(|e| panic!("analisis gagal: {e:#}"));

    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for f in generate(&an, &header, &data) {
        std::fs::write(out.join(f.name), &f.content).unwrap();
    }
}
