//! Modul WebAssembly dan runner web.
//!  - `recomp build` harus menghasilkan wasm dengan antarmuka yang dipakai web/worker.js.
//!  - Bila Node ada: worker.js (pengatur 60 fps, tombol lewat SharedArrayBuffer, suara, berhenti atas
//!    permintaan) dan sintaks skrip index.html dijalankan di Node. Browser sungguhan tidak diuji di sini.
//!
//! Dilewati bila target `wasm32-unknown-unknown` belum terpasang (`rustup target add
//! wasm32-unknown-unknown`) atau berkas nestest tidak ada.

use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::root;

fn wasm_target_installed() -> bool {
    let sysroot = Command::new("rustc").args(["--print", "sysroot"]).output().ok();
    sysroot
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .is_some_and(|s| Path::new(&s).join("lib/rustlib/wasm32-unknown-unknown").exists())
}

/// `recomp build` untuk nestest (dengan menu), hasilnya di folder kerja tes. None bila tes dilewati.
fn build_nestest_wasm(name: &str) -> Option<PathBuf> {
    let nes = root().join("roms/nestest/nestest.nes");
    if !nes.exists() || !wasm_target_installed() {
        eprintln!("nestest atau target wasm32-unknown-unknown tidak ada, tes dilewati");
        return None;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wasm = dir.join("nes.wasm");
    let build = Command::new(env!("CARGO_BIN_EXE_recomp"))
        .arg("build")
        .arg(&nes)
        .arg("--config")
        .arg(root().join("builder/configs/nestest_ui.toml"))
        .arg("--out")
        .arg(&wasm)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "recomp build gagal:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    Some(wasm)
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn recomp_build_menghasilkan_wasm_dengan_antarmuka_untuk_worker() {
    let Some(wasm) = build_nestest_wasm("web_build") else { return };
    let bytes = std::fs::read(&wasm).unwrap();
    assert_eq!(&bytes[..4], b"\0asm", "bukan berkas WebAssembly");
    // Nama impor dan ekspor tersimpan sebagai teks biasa di dalam berkas.
    for export in ["memory", "nes_run", "nes_set_sample_rate", "nes_framebuffer", "nes_read"] {
        assert!(contains(&bytes, export), "ekspor {export} tidak ada");
    }
    for import in ["buttons", "present", "audio", "fault", "interp"] {
        assert!(contains(&bytes, import), "impor {import} tidak ada");
    }
}

#[test]
fn worker_mengatur_kecepatan_membaca_input_dan_bisa_dihentikan() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("node tidak ada, tes dilewati");
        return;
    }
    let Some(wasm) = build_nestest_wasm("web_smoke") else { return };
    let run = Command::new("node")
        .arg(root().join("recomp/tests/web/web_smoke.mjs"))
        .arg(&wasm)
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&run.stdout);
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success() && out.contains("OK fps="), "stdout: {out}\nstderr: {err}");
}
