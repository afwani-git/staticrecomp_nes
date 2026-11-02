//! Jembatan WebAssembly: impor `env.*` (dipenuhi `web/worker.js`) dan ekspor `nes_*`.

use hardware::nes::{Host, Nes};

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Status tombol pengendali 1 (bit 0 A, 1 B, 2 Select, 3 Start, 4 Atas, 5 Bawah, 6 Kiri, 7 Kanan).
    fn buttons() -> u32;
    /// Layar RGBA 256x240 siap ditampilkan; boleh memblokir untuk mengatur 60 fps.
    fn present(rgba: *const u8);
    fn audio(samples: *const f32, count: u32);
    fn fault(pc: u32, target: u32, kind: u32);
    fn interp(from_pc: u32, target: u32);
}

struct WasmHost;

impl Host for WasmHost {
    fn buttons(&mut self) -> u32 {
        unsafe { buttons() }
    }
    fn present(&mut self, rgba: &[u8]) {
        unsafe { present(rgba.as_ptr()) }
    }
    fn audio(&mut self, samples: &[f32]) {
        unsafe { audio(samples.as_ptr(), samples.len() as u32) }
    }
    fn fault(&mut self, pc: u32, target: u32, kind: u32) {
        unsafe { fault(pc, target, kind) }
    }
    fn interp(&mut self, from_pc: u16, target: u16) {
        unsafe { interp(u32::from(from_pc), u32::from(target)) }
    }
}

// WebAssembly di sini satu thread; `nes_run` tidak pernah kembali, jadi fungsi pengamatan di bawah
// membaca keadaan yang sedang berjalan (dipakai untuk debugging dan tes).
static mut NES: Option<Nes> = None;
static mut SAMPLE_RATE: u32 = 48000;

fn state() -> Option<&'static mut Nes> {
    unsafe { (*(&raw mut NES)).as_mut() }
}

/// Laju sampel suara; panggil sebelum `nes_run` (bawaan 48000).
#[unsafe(no_mangle)]
pub extern "C" fn nes_set_sample_rate(rate: u32) {
    unsafe { SAMPLE_RATE = rate }
}

/// Titik masuk: berjalan terus sampai game berakhir (biasanya tidak pernah). Berhenti lewat
/// pengecualian dari impor `present` (dilempar `worker.js` saat diminta berhenti) atau trap.
#[unsafe(no_mangle)]
pub extern "C" fn nes_run() {
    let rate = unsafe { SAMPLE_RATE };
    match crate::nes(Box::new(WasmHost), rate) {
        Ok(nes) => unsafe {
            *(&raw mut NES) = Some(nes);
            state().unwrap().run()
        },
        Err(mapper) => {
            unsafe { fault(0, u32::from(mapper), 2) }; // mapper tidak didukung
            // Trap supaya host menerima pengecualian dan melaporkan fault.
            core::arch::wasm32::unreachable()
        }
    }
}

/// Pengamatan tanpa efek samping: RAM dan PRG-RAM saja.
#[unsafe(no_mangle)]
pub extern "C" fn nes_read(addr: u32) -> u32 {
    state().map_or(0, |n| u32::from(n.peek(addr as u16)))
}

#[unsafe(no_mangle)]
pub extern "C" fn nes_framebuffer() -> *const u8 {
    state().map_or(core::ptr::null(), |n| n.framebuffer().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn nes_irq_count() -> u32 {
    state().map_or(0, |n| n.irq_count())
}

#[unsafe(no_mangle)]
pub extern "C" fn nes_irq_log(i: u32) -> u32 {
    state().map_or(0, |n| n.irq_log(i))
}

#[unsafe(no_mangle)]
pub extern "C" fn nes_irq_scanline_sum() -> u32 {
    state().map_or(0, |n| n.irq_scanline_sum())
}

#[unsafe(no_mangle)]
pub extern "C" fn nes_ppu_ctrl() -> u32 {
    state().map_or(0, |n| u32::from(n.ppu_ctrl()))
}

#[unsafe(no_mangle)]
pub extern "C" fn nes_ppu_mask() -> u32 {
    state().map_or(0, |n| u32::from(n.ppu_mask()))
}
