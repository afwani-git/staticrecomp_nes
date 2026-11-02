//! Perlengkapan bersama tes hardware: perakit mini 6502 dan jalankan-ROM lewat hardware native.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use recomp::analysis::analyze;
use recomp::codegen::{generate, GeneratedFile};
use recomp::config::Config;
use recomp::ines::InesHeader;

pub const CPU_DRIVER: &str = include_str!("../drivers/cpu_driver.rs");
pub const NES_DRIVER: &str = include_str!("../drivers/nes_driver.rs");

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Perakit mini: cukup untuk program uji (alamat dihitung dari posisi sekarang).
pub struct Asm {
    pub base: u16,
    pub bytes: Vec<u8>,
}

impl Asm {
    pub fn new(base: u16) -> Self {
        Asm { base, bytes: vec![] }
    }
    pub fn here(&self) -> u16 {
        self.base + self.bytes.len() as u16
    }
    pub fn op(&mut self, b: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(b);
        self
    }
    pub fn lda(&mut self, v: u8) -> &mut Self {
        self.op(&[0xA9, v])
    }
    pub fn lda_abs(&mut self, a: u16) -> &mut Self {
        self.op(&[0xAD, a as u8, (a >> 8) as u8])
    }
    pub fn sta(&mut self, a: u16) -> &mut Self {
        self.op(&[0x8D, a as u8, (a >> 8) as u8])
    }
    pub fn jsr(&mut self, a: u16) -> &mut Self {
        self.op(&[0x20, a as u8, (a >> 8) as u8])
    }
    pub fn jmp(&mut self, a: u16) -> &mut Self {
        self.op(&[0x4C, a as u8, (a >> 8) as u8])
    }
    /// Pilih register bank MMC3 lalu isi nilainya.
    pub fn bank(&mut self, reg: u8, value: u8) -> &mut Self {
        self.lda(reg).sta(0x8000).lda(value).sta(0x8001)
    }
    /// Atur alamat PPU.
    pub fn ppu_addr(&mut self, a: u16) -> &mut Self {
        self.lda((a >> 8) as u8).sta(0x2006).lda(a as u8).sta(0x2006)
    }
}

/// Hasil menjalankan sebuah program uji.
pub struct Run {
    pub stdout: String,
    pub stderr: String,
    pub ok: bool,
}

/// Bangun crate Rust sementara berisi kode hasil generator + `main_rs` (yang boleh memakai modul
/// `game`), lalu kembalikan path programnya. Semua crate berbagi satu target dir supaya `hardware`
/// hanya dikompilasi sekali. Memakai fitur `trace` milik `hardware`.
pub fn build_game(name: &str, files: &[GeneratedFile], main_rs: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let pkg = format!("gen_{}", name.replace(|c: char| !c.is_ascii_alphanumeric(), "_"));
    let hw = root().join("hardware");
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{pkg}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nhardware = {{ path = {:?}, features = [\"trace\"] }}\n\n\
             [workspace]\n\n[profile.dev]\nopt-level = 1\ndebug = false\n",
            hw.canonicalize().unwrap().to_str().unwrap()
        ),
    )
    .unwrap();
    for f in files {
        std::fs::write(dir.join("src").join(f.name), &f.content).unwrap();
    }
    std::fs::write(
        dir.join("src/main.rs"),
        format!(
            "#![allow(unused, unreachable_code, unused_assignments, unused_mut, unused_parens, non_snake_case)]\n\
             mod game {{ include!(\"game.rs\"); }}\n{main_rs}"
        ),
    )
    .unwrap();

    let target = root().join("target/gen");
    let out = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "--manifest-path"])
        .arg(dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("cargo gagal dijalankan");
    assert!(out.status.success(), "kompilasi gagal:\n{}", String::from_utf8_lossy(&out.stderr));
    target.join("debug").join(&pkg)
}

pub fn run_exe(exe: &Path, args: &[&str]) -> Run {
    let out = Command::new(exe).args(args).output().expect("program gagal dijalankan");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        ok: out.status.success(),
    }
}

/// Generate dari ROM + konfigurasi.
pub fn gen_files(h: &InesHeader, d: &[u8], cfg: &Config) -> Vec<GeneratedFile> {
    let an = analyze(h, d, cfg).unwrap();
    generate(&an, h, d)
}

/// Generate, bangun, dan jalankan driver hardware lengkap dalam mode `mem`: berhenti di frame
/// `args[0]`, lalu mencetak isi memori di alamat `args[1..]` (heksadesimal). Kembalikan stdout.
pub fn build_and_run(name: &str, h: &InesHeader, d: &[u8], args: &[&str]) -> String {
    let exe = build_game(name, &gen_files(h, d, &Config::default()), NES_DRIVER);
    let mut a = vec!["mem"];
    a.extend_from_slice(args);
    let run = run_exe(&exe, &a);
    assert!(run.ok, "program gagal: {}\n{}", run.stderr, run.stdout);
    run.stdout
}

pub fn peek(text: &str, a: u16) -> u32 {
    let key = format!("{a:04X}=");
    let i = text.find(&key).unwrap_or_else(|| panic!("tidak ada {key} di: {text}")) + key.len();
    u32::from_str_radix(&text[i..i + 2], 16).unwrap()
}
