# NES Static Recompiler → WebAssembly


```
ROM .nes ──► recomp (Rust) ──► kode Rust ──┐
                                           ├─► cargo build ──► nes.wasm ──► browser
hardware/ (Rust: CPU, bus, PPU, APU) ──────┘
```

**Lisensi:** MIT, © 2026 Afwani Shofiyulloh. Lihat [LICENSE](LICENSE).

## Struktur

| Folder | Isi |
|---|---|
| `recomp/` | Compiler: parser iNES/NES 2.0, decoder, analisis alur, generator kode Rust, perintah `build` dan `serve`, dan tes integrasi |
| `hardware/` | Inti hardware dalam Rust: runtime CPU 6502, interpreter cadangan, bus, PPU, APU, mapper, beserta tes unitnya |
| `runner/` | Crate `nes-wasm`: meng-recompile satu ROM saat build (`build.rs`) lalu digabung dengan `hardware` menjadi modul WebAssembly |
| `builder/configs/` | `*.toml` per game: target lompatan dinamis yang tidak bisa ditemukan analisis statis |
| `web/` | Halaman browser: `index.html`, worker, AudioWorklet |
| `roms/` | Tempat ROM. Hanya ROM uji gratis yang dilacak git; ROM lain diabaikan lewat `.gitignore` |

## Kebutuhan

- Rust (cargo) terbaru, dengan target WebAssembly: `rustup target add wasm32-unknown-unknown`
- Browser modern (Chrome/Firefox/Edge)

Tidak butuh Node.js, zig, atau kompiler C.

## Build dan tes

```bash
cargo build --release          # membangun compiler `recomp`
cargo test                     # menjalankan seluruh tes
```

Tes integrasi meng-generate kode Rust dari ROM uji lalu mengompilasinya sebagai crate sementara,
jadi pemakaian pertama lebih lama (hardware dikompilasi sekali, lalu dipakai bersama). Tes yang
memerlukan ROM uji atau target WebAssembly dilewati bila tidak tersedia.

Perintah compiler:

```bash
cargo run -p recomp -- info     "roms/GAME.nes"     # header, mapper, ukuran
cargo run -p recomp -- disasm   "roms/GAME.nes" --from C000
cargo run -p recomp -- analyze  "roms/GAME.nes" [--config builder/configs/GAME.toml]
cargo run -p recomp -- generate "roms/GAME.nes" [--config ...] --out out/   # kode Rust hasil recompile
```

## Build WASM

```bash
cargo wasm "roms/GAME.nes" [--config builder/configs/GAME.toml] [--out web/nes.wasm]
```

`cargo wasm` adalah alias untuk `recomp build` (lihat `.cargo/config.toml`). Ia menjalankan
`cargo build --release --target wasm32-unknown-unknown -p nes-wasm`; ROM dan config diteruskan ke
`runner/build.rs` lewat variabel lingkungan `NES_ROM` dan `NES_CONFIG`. Hasilnya disalin ke
`web/nes.wasm` secara bawaan, **menimpa berkas yang ada**.

`--config` (TOML) opsional. Isinya target lompatan dinamis untuk game tertentu; tanpa config,
bagian itu dijalankan oleh interpreter cadangan. Contoh yang tersedia ada di
[builder/configs/](builder/configs/).

Cek dulu `recomp info` bila build gagal dengan pesan mapper: ia menampilkan nomor mapper
dan daftar yang didukung.

**Mapper didukung:** 0, 1, 2, 3, 4, 7, 11, 66, 71, 79. Header iNES 1.0 dan NES 2.0 dibaca.

## Cara run

```bash
cargo serve                    # http://127.0.0.1:8000
```

Buka alamat itu di browser. Server ini wajib dipakai (jangan buka `index.html` langsung)
karena `SharedArrayBuffer` butuh header COOP/COEP, yang ditambahkan server ini.
Opsi: `cargo serve --port 9000 --host 0.0.0.0 --dir web`.

Kontrol di desktop:

| Tombol NES | Keyboard |
|---|---|
| D-pad | W A S D (atau panah) |
| A | K |
| B | J |
| Start | Enter |
| Select | Shift kanan |

Gamepad standar juga didukung. Di layar sentuh, D-pad, A, B, Start, dan Select tampil di bawah
layar game (mendukung multi-touch). Suara aktif setelah menekan tombol apa pun (aturan autoplay
browser).

Dari ponsel lewat jaringan lokal, `SharedArrayBuffer` baru tersedia bila halaman disajikan lewat
HTTPS atau `localhost`, jadi `http://192.168.x.x:8000` biasanya tidak jalan; pakai tunnel HTTPS atau
hosting yang mengirim header COOP/COEP.

## Tes dan validasi

- `nestest` (CPU, termasuk opcode tak resmi) dicocokkan per baris dengan log referensi, baik lewat
  kode hasil recompile, interpreter murni, maupun campuran keduanya; waktu PPU dicocokkan dengan
  kolom `PPU:` di log yang sama
- ROM uji blargg `mmc3_test_2`
- Hash layar (SHA-256) untuk regresi PPU
- Tes unit PPU, APU, mapper, dan helper CPU di `hardware/tests/`
- Modul WebAssembly: antarmukanya diperiksa, dan bila Node.js ada, `web/worker.js` dijalankan di
  Node (kecepatan 60 fps, tombol, suara, berhenti atas permintaan)

ROM uji gratis diletakkan di `roms/nestest/` dan `roms/mmc3_test_2/`.

## Batasan

- PPU dirender per scanline; timing IRQ MMC3 tingkat dot belum tepat
- Bus conflict dan beberapa keanehan mapper tidak ditiru
- Mapper lain (MMC2/4/5, VRC, dst.) belum didukung
- CHR-RAM di atas 8 KB belum didukung
- Belum diuji dengan banyak game; perilaku di browser (kecepatan, audio) bisa berbeda antar perangkat

## Catatan hukum

Proyek ini tidak menyertakan ROM komersial. Gunakan hanya ROM yang kamu miliki secara sah
atau ROM homebrew/uji yang bebas.
