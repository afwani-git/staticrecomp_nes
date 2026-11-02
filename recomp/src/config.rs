use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Isi file TOML: pengetahuan manual yang tidak bisa ditebak analisis.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Target lompatan dinamis (`JMP (indirect)` atau `RTS` dispatcher).
    #[serde(default)]
    pub jump: Vec<JumpConfig>,
    /// Entri fungsi tambahan.
    #[serde(default)]
    pub entry: Vec<EntryConfig>,
    /// Kode yang dijalankan dari RAM: byte yang dianggap ada di alamat itu.
    #[serde(default)]
    pub ram_code: Vec<RamCodeConfig>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JumpConfig {
    /// Alamat instruksi lompatan dinamis.
    pub at: u16,
    /// Alamat tujuan sebenarnya (untuk push+RTS: alamat rutin, bukan alamat-1).
    pub targets: Vec<u16>,
    /// Entri fungsi tempat lompatan ini berlaku. Kosong = semua fungsi yang
    /// memuat alamat itu. Untuk `RTS`, mengisinya memaksa `RTS` itu dinamis;
    /// tanpa ini `RTS` hanya dinamis bila pola push+RTS terdeteksi.
    #[serde(default)]
    pub function: Option<u16>,
    /// Bila diisi, target dipanggil sebagai subrutin yang kembali ke alamat ini, mis.
    /// `LDA #hi / PHA / LDA #lo / PHA / JMP (tabel)`: alamat kembali di-push manual lalu
    /// rutin target berakhir dengan RTS. Target menjadi fungsi sendiri, bukan label lokal.
    #[serde(default)]
    pub returns_to: Option<u16>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RamCodeConfig {
    /// Alamat RAM ($0000-$7FFF) tempat byte-byte ini berada saat dijalankan.
    pub addr: u16,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EntryConfig {
    pub addr: u16,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("file TOML tidak valid")
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("gagal membaca {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("konfigurasi {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_dan_desimal() {
        let c = Config::parse(
            "[[jump]]\nat = 0xC123\ntargets = [0xC200, 49680]\n\n[[entry]]\naddr = 0xC800\n",
        )
        .unwrap();
        assert_eq!(c.jump.len(), 1);
        assert_eq!(c.jump[0].at, 0xC123);
        assert_eq!(c.jump[0].targets, vec![0xC200, 0xC210]);
        assert_eq!(c.entry, vec![EntryConfig { addr: 0xC800 }]);
    }

    #[test]
    fn function_dan_ram_code() {
        let c = Config::parse(
            "[[jump]]\nat = 0xCE66\ntargets = [0xCE67]\nfunction = 0xC5F5\n\n[[ram_code]]\naddr = 0x0300\nbytes = [0xA9, 0xAA, 0x60]\n",
        )
        .unwrap();
        assert_eq!(c.jump[0].function, Some(0xC5F5));
        assert_eq!(c.jump[0].returns_to, None);
        assert_eq!(c.ram_code, vec![RamCodeConfig { addr: 0x300, bytes: vec![0xA9, 0xAA, 0x60] }]);
        // byte di luar 0-255 ditolak
        assert!(Config::parse("[[ram_code]]\naddr = 1\nbytes = [256]").is_err());
    }

    #[test]
    fn returns_to_dibaca() {
        let c = Config::parse("[[jump]]\nat = 0xC107\ntargets = [0xC72D]\nreturns_to = 0xC1DF\n").unwrap();
        assert_eq!(c.jump[0].returns_to, Some(0xC1DF));
    }

    #[test]
    fn kosong_valid() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn rusak_ditolak() {
        assert!(Config::parse("[[jump]\nat =").is_err());
        // alamat di luar 16-bit
        assert!(Config::parse("[[jump]]\nat = 0x10000\ntargets = []").is_err());
        // field salah ketik
        assert!(Config::parse("[[jump]]\nat = 1\ntarget = [2]").is_err());
        // field wajib hilang
        assert!(Config::parse("[[jump]]\nat = 1").is_err());
    }
}
