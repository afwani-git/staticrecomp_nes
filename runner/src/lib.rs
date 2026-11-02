//! Game hasil recompile + inti hardware, siap dijalankan.
//!
//! ROM yang dipakai ditentukan saat build (lihat build.rs). Untuk WebAssembly, modul `wasm`
//! mengekspor fungsi yang dipanggil `web/worker.js`; di platform lain crate ini menyediakan
//! `nes()` untuk membangun komputer NES-nya sendiri.

use hardware::nes::{Host, Nes};

#[allow(unused, unreachable_code, unused_assignments, unused_mut, unused_parens, non_snake_case, clippy::all)]
mod game {
    include!(concat!(env!("OUT_DIR"), "/game.rs"));
}

pub use game::ROM;

/// Bangun komputer NES untuk ROM ini. Err(nomor mapper) bila mapper tidak didukung.
pub fn nes(host: Box<dyn Host>, sample_rate: u32) -> Result<Nes, u8> {
    Nes::build(game::game(), game::ROM, host, sample_rate)
}

#[cfg(target_arch = "wasm32")]
mod wasm;
