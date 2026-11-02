//! Inti hardware NES (PPU, APU, mapper, bus) dan runtime CPU 6502 untuk kode hasil recompile.
//!
//! Kode game dihasilkan oleh crate `recomp`; crate ini menyediakan semua yang dibutuhkannya
//! saat jalan. Tidak ada ketergantungan pada OS, jadi bisa dikompilasi ke WebAssembly.

pub mod apu;
pub mod cpu;
pub mod interp;
pub mod mapper;
pub mod nes;
pub mod palette;
pub mod ppu;
