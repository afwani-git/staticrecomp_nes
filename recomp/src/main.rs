use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use recomp::analysis::{analyze, render};
use recomp::codegen::generate;
use recomp::config::Config;
use recomp::disasm::disassemble;
use recomp::ines::InesHeader;

mod serve;

#[derive(Parser)]
#[command(name = "recomp", about = "Static recompiler NES (6502) ke Rust dan WebAssembly")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Tampilkan informasi header iNES dari sebuah ROM
    Info {
        /// Path ke file .nes
        rom: PathBuf,
    },
    /// Disassemble kode 6502 dari PRG-ROM (sapuan linear)
    Disasm {
        /// Path ke file .nes
        rom: PathBuf,
        /// Alamat CPU awal, mis. 0xC000 atau C000
        #[arg(long, value_parser = parse_addr)]
        from: u16,
        /// Jumlah instruksi
        #[arg(long, default_value_t = 50)]
        count: usize,
    },
    /// Telusuri alur kode dari vektor RESET/NMI/IRQ dan laporkan hasilnya
    Analyze {
        /// Path ke file .nes
        rom: PathBuf,
        /// File TOML berisi target lompatan dinamis dan entri tambahan
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Hasilkan kode Rust (folder out/) dari ROM
    Generate {
        /// Path ke file .nes
        rom: PathBuf,
        /// File TOML berisi target lompatan dinamis dan entri tambahan
        #[arg(long)]
        config: Option<PathBuf>,
        /// Folder keluaran
        #[arg(long, default_value = "out")]
        out: PathBuf,
    },
    /// Bangun nes.wasm dari ROM: recompile ke Rust lalu `cargo build --target wasm32-unknown-unknown`
    Build {
        /// Path ke file .nes
        rom: PathBuf,
        /// File TOML berisi target lompatan dinamis dan entri tambahan
        #[arg(long)]
        config: Option<PathBuf>,
        /// Berkas keluaran
        #[arg(long, default_value = "web/nes.wasm")]
        out: PathBuf,
    },
    /// Layani folder web/ untuk dicoba di browser (menambahkan header COOP/COEP yang dibutuhkan
    /// SharedArrayBuffer)
    Serve {
        #[arg(long, default_value_t = 8000)]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Folder yang dilayani
        #[arg(long, default_value = "web")]
        dir: PathBuf,
    },
}

fn parse_addr(s: &str) -> Result<u16, String> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X").trim_start_matches('$');
    u16::from_str_radix(t, 16).map_err(|e| format!("alamat heksadesimal tidak valid '{s}': {e}"))
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { rom } => info(&rom),
        Command::Disasm { rom, from, count } => disasm(&rom, from, count),
        Command::Analyze { rom, config } => analyze_cmd(&rom, config.as_deref()),
        Command::Generate { rom, config, out } => generate_cmd(&rom, config.as_deref(), &out),
        Command::Build { rom, config, out } => build_cmd(&rom, config.as_deref(), &out),
        Command::Serve { port, host, dir } => serve::run(&host, port, &dir),
    }
}

fn info(path: &PathBuf) -> Result<()> {
    let data = std::fs::read(path).with_context(|| format!("gagal membaca {}", path.display()))?;
    let h = InesHeader::parse(&data)
        .with_context(|| format!("header tidak valid: {}", path.display()))?;

    println!("File      : {}", path.display());
    println!("PRG-ROM   : {} bank x 16 KB = {} KB", h.prg_banks, h.prg_len() / 1024);
    if h.chr_banks == 0 {
        println!("CHR       : tidak ada (CHR-RAM)");
    } else {
        println!("CHR-ROM   : {} bank x 8 KB = {} KB", h.chr_banks, h.chr_len() / 1024);
    }
    println!("Mapper    : {}", h.mapper);
    if h.nes2 {
        println!("Format    : NES 2.0 (submapper {})", h.submapper);
        if h.prg_ram_size > 0 {
            println!("PRG-RAM   : {} KB", h.prg_ram_size.div_ceil(1024));
        }
        if h.chr_ram_size > 0 {
            println!("CHR-RAM   : {} KB", h.chr_ram_size.div_ceil(1024));
        }
        if h.chr_ram_size > 8192 {
            println!("Catatan   : CHR-RAM lebih dari 8 KB belum didukung hardware (hanya 8 KB)");
        }
    } else {
        println!("Format    : iNES 1.0");
    }
    println!("Mirroring : {}", h.mirroring);
    println!("Baterai   : {}", if h.battery { "ya" } else { "tidak" });
    println!("Trainer   : {}", if h.trainer { "ya" } else { "tidak" });
    Ok(())
}

fn disasm(path: &PathBuf, from: u16, count: usize) -> Result<()> {
    let data = std::fs::read(path).with_context(|| format!("gagal membaca {}", path.display()))?;
    let h = InesHeader::parse(&data)
        .with_context(|| format!("header tidak valid: {}", path.display()))?;
    if !h.mapper_supported() {
        anyhow::bail!("{}", recomp::ines::unsupported_mapper_message(h.mapper));
    }
    let listing = disassemble(|a| h.read_prg(&data, a), from, count);
    for line in &listing.lines {
        println!("{line}");
    }
    if let Some(e) = listing.error {
        anyhow::bail!("{e}");
    }
    Ok(())
}

fn analyze_cmd(path: &Path, config: Option<&Path>) -> Result<()> {
    let data = std::fs::read(path).with_context(|| format!("gagal membaca {}", path.display()))?;
    let h = InesHeader::parse(&data)
        .with_context(|| format!("header tidak valid: {}", path.display()))?;
    let cfg = match config {
        Some(p) => Config::load(p)?,
        None => Config::default(),
    };
    let an = analyze(&h, &data, &cfg)?;
    print!("{}", render(&an, &h));
    Ok(())
}

fn generate_cmd(path: &Path, config: Option<&Path>, out: &Path) -> Result<()> {
    let data = std::fs::read(path).with_context(|| format!("gagal membaca {}", path.display()))?;
    let h = InesHeader::parse(&data)
        .with_context(|| format!("header tidak valid: {}", path.display()))?;
    let cfg = match config {
        Some(p) => Config::load(p)?,
        None => Config::default(),
    };
    let an = analyze(&h, &data, &cfg)?;
    std::fs::create_dir_all(out).with_context(|| format!("gagal membuat {}", out.display()))?;
    for f in generate(&an, &h, &data) {
        let p = out.join(f.name);
        std::fs::write(&p, f.content).with_context(|| format!("gagal menulis {}", p.display()))?;
    }
    println!(
        "{} fungsi, {} instruksi ditulis ke {}/ ({} peringatan analisis; lihat `recomp analyze`)",
        an.functions.len(),
        an.instructions.len(),
        out.display(),
        an.warnings.len()
    );
    Ok(())
}

/// Akar workspace (folder yang berisi Cargo.toml utama), diketahui saat kompilasi.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn build_cmd(rom: &Path, config: Option<&Path>, out: &Path) -> Result<()> {
    // Periksa ROM dan config lebih dulu supaya galatnya jelas, bukan galat dari build.rs.
    let data = std::fs::read(rom).with_context(|| format!("gagal membaca {}", rom.display()))?;
    let h = InesHeader::parse(&data).with_context(|| format!("header tidak valid: {}", rom.display()))?;
    if !h.mapper_supported() {
        anyhow::bail!("{}", recomp::ines::unsupported_mapper_message(h.mapper));
    }
    let abs = |p: &Path| std::fs::canonicalize(p).with_context(|| format!("tidak ditemukan: {}", p.display()));
    let rom_abs = abs(rom)?;
    let config_abs = config.map(abs).transpose()?;
    let out_abs = std::env::current_dir()?.join(out);

    let root = workspace_root();
    let mut cmd = std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.current_dir(&root)
        .args(["build", "--release", "--target", "wasm32-unknown-unknown", "-p", "nes-wasm"])
        .env("NES_ROM", &rom_abs);
    match &config_abs {
        Some(c) => cmd.env("NES_CONFIG", c),
        None => cmd.env_remove("NES_CONFIG"),
    };
    let status = cmd.status().context("gagal menjalankan cargo")?;
    if !status.success() {
        anyhow::bail!(
            "build WASM gagal. Bila pesannya menyebut target tidak ditemukan, jalankan: rustup target add wasm32-unknown-unknown"
        );
    }

    let built = root.join("target/wasm32-unknown-unknown/release/nes_wasm.wasm");
    if let Some(dir) = out_abs.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(&built, &out_abs).with_context(|| format!("gagal menyalin ke {}", out_abs.display()))?;
    println!("ok: {} ({} byte)", out.display(), std::fs::metadata(&out_abs)?.len());
    Ok(())
}
