//! Validasi dengan nestest: jalankan hasil recompile dari $C000 dan bandingkan keadaan CPU
//! di tiap instruksi dengan nestest.log (kolom PC, A, X, Y, P, SP, CYC; PPU diabaikan).
//!
//! Seluruh 8991 baris dibandingkan: baris 1-5003 menguji opcode resmi, baris 5004 dan
//! seterusnya opcode tak resmi. Dilewati bila file nestest tidak ada.

use recomp::config::Config;
use recomp::ines::InesHeader;

mod common;
use common::{build_game, gen_files, root, run_exe, CPU_DRIVER};

const LOG_LINES: usize = 8991;

/// "C000 A:00 X:00 Y:00 P:24 SP:FD CYC:7" dari satu baris nestest.log atau trace kita.
fn normalize_log(line: &str) -> String {
    let field = |prefix: &str| {
        line.split_whitespace()
            .find(|t| t.starts_with(prefix))
            .unwrap_or_else(|| panic!("tidak ada {prefix} di: {line}"))
    };
    format!(
        "{} {} {} {} {} {} {}",
        &line[..4],
        field("A:"),
        field("X:"),
        field("Y:"),
        field("P:"),
        field("SP:"),
        field("CYC:")
    )
}

#[test]
fn cocok_dengan_nestest_log_untuk_opcode_resmi() {
    let nes = root().join("roms/nestest/nestest.nes");
    let log = root().join("roms/nestest/nestest.log");
    if !nes.exists() || !log.exists() {
        eprintln!("nestest tidak ada, tes dilewati");
        return;
    }

    let data = std::fs::read(&nes).unwrap();
    let header = InesHeader::parse(&data).unwrap();
    let cfg = Config::load(&root().join("builder/configs/nestest.toml")).unwrap();

    let exe = build_game("nestest", &gen_files(&header, &data, &cfg), CPU_DRIVER);
    let run = run_exe(&exe, &["nestest-flat"]);
    let stdout = run.stdout;
    let stderr = run.stderr;
    let ours: Vec<&str> = stdout.lines().filter(|l| !l.starts_with("RESULT")).collect();
    let result = stdout.lines().find(|l| l.starts_with("RESULT")).unwrap_or("(tanpa RESULT)");

    let reference = std::fs::read_to_string(&log).unwrap();
    let reference: Vec<&str> = reference.lines().collect();
    assert_eq!(reference.len(), LOG_LINES);

    for (i, want) in reference.iter().enumerate() {
        let Some(got) = ours.get(i) else {
            panic!(
                "trace berhenti di baris {} dari {LOG_LINES}; berikutnya seharusnya:\n  {want}\n{stderr}",
                i + 1
            );
        };
        assert_eq!(
            normalize_log(got),
            normalize_log(want),
            "baris {} beda\n  nestest.log: {want}\n  hasil kita : {got}",
            i + 1
        );
    }

    // Program selesai sendiri (RTS terakhir nestest), bukan berhenti karena error.
    assert_eq!(ours.len(), LOG_LINES, "jumlah baris trace");
    assert!(run.ok, "program berhenti dengan error: {stderr}");
    // nestest sendiri menyatakan semua tes lulus: $0002 (opcode resmi) dan $0003 (tak resmi).
    assert!(result.contains("0002=00 0003=00"), "kode hasil nestest: {result}");
}
