//! Tes end-to-end: ROM buatan sendiri -> analisis -> kode Rust -> `cargo build` -> jalankan.

use recomp::config::{Config, JumpConfig, RamCodeConfig};
use recomp::ines::InesHeader;

mod common;
use common::{build_game, gen_files, run_exe, CPU_DRIVER};

/// ROM NROM-128 palsu: `code` di alamat masing-masing, sisanya $FF.
fn rom(code: &[(u16, &[u8])], reset: u16, nmi: u16) -> (InesHeader, Vec<u8>) {
    let mut d = vec![0xFFu8; 16 + 0x4000 + 0x2000];
    d[0..4].copy_from_slice(b"NES\x1A");
    d[4] = 1;
    d[5] = 1;
    d[6..16].fill(0);
    let put = |d: &mut Vec<u8>, addr: u16, bytes: &[u8]| {
        let o = 16 + (addr as usize - 0xC000);
        d[o..o + bytes.len()].copy_from_slice(bytes);
    };
    for (a, b) in code {
        put(&mut d, *a, b);
    }
    put(&mut d, 0xFFFA, &nmi.to_le_bytes());
    put(&mut d, 0xFFFC, &reset.to_le_bytes());
    put(&mut d, 0xFFFE, &reset.to_le_bytes());
    (InesHeader::parse(&d).unwrap(), d)
}

/// Generate, bangun, jalankan driver CPU (mode `state`), kembalikan stdout.
fn run_program(name: &str, h: &InesHeader, d: &[u8], cfg: &Config, arg: &str) -> String {
    let exe = build_game(name, &gen_files(h, d, cfg), CPU_DRIVER);
    let run = run_exe(&exe, &["state", arg]);
    assert!(run.ok, "program gagal: {}", run.stderr);
    run.stdout
}

fn check(out: &str, state: &str, mem: &[&str]) {
    let mut lines = out.lines();
    assert_eq!(lines.next().unwrap(), state, "keadaan CPU\n{out}");
    let memline = lines.next().unwrap();
    for m in mem {
        assert!(memline.split(' ').any(|t| t == *m), "memori {m} tidak ada di: {memline}");
    }
}

#[test]
fn aritmetika_cabang_dan_jsr() {
    let (h, d) = rom(
        &[
            (
                0xC000,
                &[
                    0xA9, 0x50, 0x69, 0x50, 0x85, 0x10, // LDA #$50; ADC #$50; STA $10
                    0x38, 0xE9, 0x01, 0x85, 0x11, // SEC; SBC #$01; STA $11
                    0x20, 0x20, 0xC0, // JSR $C020
                    0xA2, 0x03, 0xCA, 0xD0, 0xFD, // LDX #3; DEX; BNE -3
                    0x86, 0x12, 0x60, // STX $12; RTS
                ],
            ),
            (0xC020, &[0xA9, 0x07, 0x85, 0x13, 0x60]),
        ],
        0xC000,
        0xC000,
    );
    let out = run_program("prog_a", &h, &d, &Config::default(), "");
    check(
        &out,
        "A=07 X=00 Y=00 SP=FF N=0 V=0 D=0 I=1 Z=1 C=1 cycles=63",
        &["0010=A0", "0011=9F", "0012=00", "0013=07"],
    );
}

#[test]
fn jmp_indirect_dan_push_rts_dengan_toml() {
    let (h, d) = rom(
        &[
            (
                0xC000,
                &[
                    0xA9, 0x30, 0x85, 0x20, 0xA9, 0xC0, 0x85, 0x21, // pointer $0020 = $C030
                    0x6C, 0x20, 0x00, // JMP ($0020)
                ],
            ),
            // push alamat ($C050 - 1) lalu RTS
            (0xC030, &[0xA9, 0xC0, 0x48, 0xA9, 0x4F, 0x48, 0x60]),
            (0xC050, &[0xA2, 0x99, 0x8E, 0x00, 0x02, 0x60]), // LDX #$99; STX $0200; RTS
        ],
        0xC000,
        0xC000,
    );
    let cfg = Config {
        jump: vec![
            JumpConfig { at: 0xC008, targets: vec![0xC030], function: None, returns_to: None },
            JumpConfig { at: 0xC036, targets: vec![0xC050], function: None, returns_to: None },
        ], ..Config::default()
    };
    let out = run_program("prog_b", &h, &d, &cfg, "");
    check(
        &out,
        "A=4F X=99 Y=00 SP=FF N=1 V=0 D=0 I=1 Z=0 C=0 cycles=50",
        &["0020=30", "0021=C0", "0200=99"],
    );
}

#[test]
fn mode_pengalamatan_dan_siklus_lintas_halaman() {
    let (h, d) = rom(
        &[
            (
                0xC000,
                &[
                    0xA9, 0x77, 0x85, 0x08, // LDA #$77; STA $08
                    0xA2, 0x10, // LDX #$10
                    0xB5, 0xF8, 0x85, 0x30, // LDA $F8,X (berputar ke $08); STA $30
                    0xBD, 0xF5, 0xC0, 0x85, 0x31, // LDA $C0F5,X (lintas halaman); STA $31
                    0xA0, 0x02, // LDY #$02
                    0xA9, 0xFF, 0x85, 0x40, 0xA9, 0x01, 0x85, 0x41, // pointer $0040 = $01FF
                    0xA9, 0x3C, 0x8D, 0x01, 0x02, // LDA #$3C; STA $0201
                    0xB1, 0x40, 0x85, 0x32, // LDA ($40),Y (lintas halaman); STA $32
                    0x4C, 0xF0, 0xC0, // JMP $C0F0
                ],
            ),
            (0xC0F0, &[0xA9, 0x01, 0xD0, 0x0C]), // LDA #1; BNE $C100 (beda halaman)
            (0xC100, &[0x85, 0x33, 0x60]),       // STA $33; RTS
            (0xC105, &[0x5A]),                    // data yang dibaca LDA $C0F5,X
        ],
        0xC000,
        0xC000,
    );
    let out = run_program("prog_c", &h, &d, &Config::default(), "");
    check(
        &out,
        "A=01 X=10 Y=02 SP=FF N=0 V=0 D=0 I=1 Z=0 C=0 cycles=74",
        &["0008=77", "0030=77", "0031=5A", "0032=3C", "0033=01", "0201=3C"],
    );
}

#[test]
fn interrupt_nmi_dan_rti() {
    let (h, d) = rom(
        &[(0xC000, &[0x60]), (0xC080, &[0xE6, 0x50, 0x40])], // handler: INC $50; RTI
        0xC000,
        0xC080,
    );
    let out = run_program("prog_d", &h, &d, &Config::default(), "n");
    check(
        &out,
        "A=00 X=00 Y=00 SP=FF N=0 V=0 D=0 I=1 Z=0 C=0 cycles=49",
        &["0050=02"],
    );
}

/// Program yang menulis kode ke RAM lalu menjalankannya lewat `JMP ($02FF)`.
/// `byte1` = nilai yang ditulis ke $0301 (TOML menyatakan $AA).
fn ram_program(byte1: u8) -> (InesHeader, Vec<u8>, Config) {
    let (h, d) = rom(
        &[
            (
                0xC000,
                &[
                    0xA9, 0xA9, 0x8D, 0x00, 0x03, // LDA #$A9; STA $0300
                    0xA9, byte1, 0x8D, 0x01, 0x03, // LDA #byte1; STA $0301
                    0xA9, 0x60, 0x8D, 0x02, 0x03, // LDA #$60; STA $0302
                    0xA9, 0x00, 0x8D, 0xFF, 0x02, // pointer lo di $02FF = $00
                    0xA9, 0x03, 0x8D, 0x00, 0x02, // pointer hi di $0200 = $03 (bug halaman)
                    0x20, 0x20, 0xC0, 0x60, // JSR $C020; RTS
                ],
            ),
            (0xC020, &[0x6C, 0xFF, 0x02]), // JMP ($02FF)
        ],
        0xC000,
        0xC000,
    );
    let cfg = Config {
        jump: vec![JumpConfig { at: 0xC020, targets: vec![0x0300], function: None, returns_to: None }],
        ram_code: vec![RamCodeConfig { addr: 0x0300, bytes: vec![0xA9, 0xAA, 0x60] }],
        ..Config::default()
    };
    (h, d, cfg)
}

#[test]
fn kode_ram_berjalan_dan_jmp_indirect_meniru_bug_halaman() {
    let (h, d, cfg) = ram_program(0xAA);
    let out = run_program("prog_e", &h, &d, &cfg, "");
    check(
        &out,
        "A=AA X=00 Y=00 SP=FF N=1 V=0 D=0 I=1 Z=0 C=0 cycles=62",
        &["0200=03"],
    );
}

#[test]
fn kode_ram_yang_berubah_dijalankan_oleh_interpreter() {
    // RAM berisi `LDA #$AB` padahal TOML menyatakan `LDA #$AA`: pemeriksaan di kode hasil
    // recompile gagal, dan interpreter menjalankan isi RAM yang sebenarnya.
    let (h, d, cfg) = ram_program(0xAB);
    let out = run_program("prog_e_berubah", &h, &d, &cfg, "");
    check(&out, "A=AB X=00 Y=00 SP=FF N=1 V=0 D=0 I=1 Z=0 C=0 cycles=62", &["0200=03"]);
}

#[test]
fn rti_sebagai_lompatan_dengan_toml() {
    let (h, d) = rom(
        &[
            (
                0xC000,
                &[
                    0xA9, 0xC0, 0x48, // push byte tinggi tujuan
                    0xA9, 0x10, 0x48, // push byte rendah tujuan
                    0xA9, 0x83, 0x48, // push flag: N, Z, C menyala; I mati
                    0x40, // RTI -> $C010
                ],
            ),
            (0xC010, &[0xA2, 0x05, 0x60]), // LDX #5; RTS
        ],
        0xC000,
        0xC000,
    );
    let cfg = Config {
        jump: vec![JumpConfig { at: 0xC009, targets: vec![0xC010], function: None, returns_to: None }],
        ..Config::default()
    };
    let out = run_program("prog_rti", &h, &d, &cfg, "");
    check(&out, "A=83 X=05 Y=00 SP=FF N=0 V=0 D=0 I=0 Z=0 C=1 cycles=36", &[]);
}

#[test]
fn rts_membongkar_beberapa_tingkat_dan_pla_pla_membuang_pemanggil() {
    // C010 memanggil C020; C020 membuang alamat kembali ke C010 (PLA PLA) lalu RTS,
    // sehingga RTS itu kembali langsung ke pemanggil C010 dan STA $30 dilewati.
    let (h, d) = rom(
        &[
            (0xC000, &[0x20, 0x10, 0xC0, 0xA2, 0x07, 0x60]), // JSR $C010; LDX #7; RTS
            (0xC010, &[0x20, 0x20, 0xC0, 0xA9, 0x11, 0x85, 0x30, 0x60]),
            (0xC020, &[0x68, 0x68, 0x60]), // PLA; PLA; RTS
        ],
        0xC000,
        0xC000,
    );
    let out = run_program("prog_unwind", &h, &d, &Config::default(), "");
    check(
        &out,
        "A=C0 X=07 Y=00 SP=FF N=0 V=0 D=0 I=1 Z=0 C=0 cycles=41",
        &["0030=00"],
    );
}

#[test]
fn opcode_tak_resmi_menghasilkan_register_flag_dan_siklus_yang_benar() {
    // Dihitung tangan: tiap langkah mengubah A, flag, dan [$10]; lihat komentar.
    let (h, d) = rom(
        &[(
            0xC000,
            &[
                0xA9, 0x0F, 0x85, 0x10, // LDA #$0F; STA $10
                0xA7, 0x10, // LAX $10        A=X=0F
                0x87, 0x11, // SAX $11        [11]=0F
                0xA9, 0x05, // LDA #$05
                0xC7, 0x10, // DCP $10        [10]=0E; 05 vs 0E: C=0 N=1
                0xE7, 0x10, // ISB $10        [10]=0F; A=05-0F-1=F5
                0x07, 0x10, // SLO $10        [10]=1E; A=F5|1E=FF
                0x27, 0x10, // RLA $10        [10]=3C; A=FF&3C=3C
                0x47, 0x10, // SRE $10        [10]=1E; A=3C^1E=22
                0x67, 0x10, // RRA $10        [10]=0F; A=22+0F=31
                0x1A, 0x04, 0x10, 0x0C, 0x00, 0x02, // NOP, NOP zp, NOP abs
                0xEB, 0x01, // SBC #$01 (tak resmi); A=31-01-1=2F, C=1
                0x60,
            ],
        )],
        0xC000,
        0xC000,
    );
    let out = run_program("prog_unofficial", &h, &d, &Config::default(), "");
    check(
        &out,
        "A=2F X=0F Y=00 SP=FF N=0 V=0 D=0 I=1 Z=0 C=1 cycles=67",
        &["0010=0F", "0011=0F"],
    );
}

#[test]
fn opcode_tak_resmi_menambah_siklus_lintas_halaman() {
    let (h, d) = rom(
        &[
            (
                0xC000,
                &[
                    0xA0, 0x10, // LDY #$10
                    0xBF, 0xF5, 0xC0, // LAX $C0F5,Y  -> $C105 (lintas halaman, 5 siklus)
                    0x1C, 0xF5, 0xC0, // NOP $C0F5,X  -> $C14F (lintas halaman, 5 siklus)
                    0x60,
                ],
            ),
            (0xC105, &[0x5A]),
        ],
        0xC000,
        0xC000,
    );
    let out = run_program("prog_unofficial_cross", &h, &d, &Config::default(), "");
    check(&out, "A=5A X=5A Y=10 SP=FF N=0 V=0 D=0 I=1 Z=0 C=0 cycles=25", &[]);
}

#[test]
fn panggilan_lewat_pointer_dengan_alamat_kembali_manual() {
    // Pola trampolin: push alamat kembali ($C010 + 1 = $C011), isi pointer di $00/$01, lalu
    // JMP ($0000). Rutin target ($C020) berakhir dengan RTS yang kembali ke $C011.
    let (h, d) = rom(
        &[
            (
                0xC000,
                &[
                    0xA9, 0xC0, 0x48, // push byte tinggi ($C0)
                    0xA9, 0x10, 0x48, // push byte rendah ($10)
                    0xA9, 0x20, 0x85, 0x00, // pointer rendah = $20
                    0xA9, 0xC0, 0x85, 0x01, // pointer tinggi = $C0
                    0x6C, 0x00, 0x00, // JMP ($0000) di $C00E
                    0xA2, 0x07, 0x60, // $C011: LDX #7; RTS
                ],
            ),
            (0xC020, &[0xA9, 0x55, 0x8D, 0x00, 0x02, 0x60]), // LDA #$55; STA $0200; RTS
        ],
        0xC000,
        0xC000,
    );
    let cfg = Config {
        jump: vec![JumpConfig {
            at: 0xC00E,
            targets: vec![0xC020],
            function: None,
            returns_to: Some(0xC011),
        }],
        ..Config::default()
    };
    let out = run_program("prog_trampoline", &h, &d, &cfg, "");
    check(&out, "A=55 X=07 Y=00 SP=FF N=0 V=0 D=0 I=1 Z=0 C=0 cycles=52", &["0200=55"]);
}

#[test]
fn lompatan_dinamis_tanpa_toml_dijalankan_interpreter() {
    // JMP ($0000) ke $C020 tanpa target di TOML: kode di $C020 tidak dianalisis sama sekali.
    // Cadangan: 7 + (2+3+2+3) + 5 (JMP) + 2 + 4 + 6 (RTS) = 34 siklus.
    let (h, d) = rom(
        &[
            (0xC000, &[0xA9, 0x20, 0x85, 0x00, 0xA9, 0xC0, 0x85, 0x01, 0x6C, 0x00, 0x00]),
            (0xC020, &[0xA9, 0x77, 0x8D, 0x00, 0x02, 0x60]), // LDA #$77; STA $0200; RTS
        ],
        0xC000,
        0xC000,
    );
    let out = run_program("prog_interp_jmp", &h, &d, &Config::default(), "");
    check(&out, "A=77 X=00 Y=00 SP=FF N=0 V=0 D=0 I=1 Z=0 C=0 cycles=34", &["0200=77"]);
}

#[test]
fn jsr_ke_kode_ram_yang_tidak_dianalisis_kembali_dengan_benar() {
    // Program menulis `LDA #$AA; RTS` ke RAM lalu JSR $0300 tanpa ram_code di TOML.
    // 7 + 3x(2+4) + 6 (JSR) + 2 + 6 (RTS RAM) + 6 (RTS) = 45 siklus.
    let (h, d) = rom(
        &[(
            0xC000,
            &[
                0xA9, 0xA9, 0x8D, 0x00, 0x03, // LDA #$A9; STA $0300
                0xA9, 0xAA, 0x8D, 0x01, 0x03, // LDA #$AA; STA $0301
                0xA9, 0x60, 0x8D, 0x02, 0x03, // LDA #$60; STA $0302
                0x20, 0x00, 0x03, 0x60, // JSR $0300; RTS
            ],
        )],
        0xC000,
        0xC000,
    );
    let out = run_program("prog_interp_jsr", &h, &d, &Config::default(), "");
    check(&out, "A=AA X=00 Y=00 SP=FF N=1 V=0 D=0 I=1 Z=0 C=0 cycles=45", &[]);
}
