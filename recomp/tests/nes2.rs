//! Header NES 2.0 harus menghasilkan recompile yang identik dengan header iNES 1.0 untuk ROM yang sama.
//! Memakai nestest (ROM tes gratis): header iNES 1.0-nya diubah menjadi NES 2.0 tanpa menyentuh isi.
//! Dilewati bila berkas nestest tidak ada.

use std::path::{Path, PathBuf};

use recomp::analysis::analyze;
use recomp::codegen::generate;
use recomp::config::Config;
use recomp::ines::InesHeader;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn nestest_nes20_menghasilkan_kode_identik_dengan_ines1() {
    let nes = root().join("roms/nestest/nestest.nes");
    if !nes.exists() {
        eprintln!("nestest tidak ada, tes dilewati");
        return;
    }
    let ines1 = std::fs::read(&nes).unwrap();
    let h1 = InesHeader::parse(&ines1).unwrap();
    assert!(!h1.nes2);

    // Ubah header: tandai NES 2.0 (byte 7 bit 2-3 = 10), ukuran lewat byte 4/5 dan MSB di byte 9 = 0.
    let mut nes2 = ines1.clone();
    nes2[7] = (nes2[7] & 0xF3) | 0x08;
    nes2[8..16].fill(0);
    let h2 = InesHeader::parse(&nes2).unwrap();
    assert!(h2.nes2);
    assert_eq!((h2.prg_banks, h2.chr_banks, h2.mapper), (h1.prg_banks, h1.chr_banks, h1.mapper));
    assert_eq!(h2.mirroring, h1.mirroring);

    let cfg = Config::load(&root().join("builder/configs/nestest.toml")).unwrap();
    let out1 = generate(&analyze(&h1, &ines1, &cfg).unwrap(), &h1, &ines1);
    let out2 = generate(&analyze(&h2, &nes2, &cfg).unwrap(), &h2, &nes2);
    assert_eq!(out1.len(), out2.len());
    for (a, b) in out1.iter().zip(&out2) {
        assert_eq!(a.name, b.name);
        assert!(a.content == b.content, "{} berbeda antara iNES 1.0 dan NES 2.0", a.name);
    }
}
