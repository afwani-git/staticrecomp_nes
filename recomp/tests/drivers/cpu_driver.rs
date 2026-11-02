// Driver tes CPU: memori datar 64 KB (FlatBus), tanpa hardware. Dipilih lewat argumen pertama:
//   state [n]        jalankan RESET (dan NMI dua kali bila "n"), cetak register dan memori
//   nestest-flat     nestest dari $C000 lewat hasil recompile, cetak trace per instruksi
//   nestest-interp   nestest MURNI lewat interpreter (tidak ada fungsi hasil recompile)
//   nestest-mixed    hasil recompile + interpreter cadangan, berhenti setelah LOG_LINES instruksi
//   nestest-ppu      cetak "PC scanline dot cycles" dengan PPU dimajukan di tiap instruksi
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use hardware::cpu::{Cpu, FlatBus, Game, TraceEntry};
use hardware::mapper::Mapper;
use hardware::ppu::Ppu;

const LOG_LINES: u32 = 8991;

fn fresh(game: Game<FlatBus>) -> Cpu<FlatBus> {
    let mut cpu = Cpu::new(FlatBus::with_prg(game::ROM.prg), game);
    // Mulai di $C000 seperti mode otomatis nestest.
    cpu.a = 0;
    cpu.x = 0;
    cpu.y = 0;
    cpu.sp = 0xFD;
    cpu.n = 0;
    cpu.v = 0;
    cpu.d = 0;
    cpu.i = 1;
    cpu.z = 0;
    cpu.c = 0;
    cpu.cycles = 7;
    cpu
}

/// Jalankan fungsi hasil recompile di $C000 (harus ada di konfigurasi); kembalikan tujuan RTS-nya.
fn entry_c000(cpu: &mut Cpu<FlatBus>) -> u16 {
    game::call_entry(cpu, 0xC000).expect("tidak ada fungsi hasil recompile di $C000")
}

fn line(t: &TraceEntry) -> String {
    format!("{:04X} A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} CYC:{}", t.pc, t.a, t.x, t.y, t.p, t.sp, t.cycles)
}

/// Pasang hook yang mengumpulkan baris trace dan menghitung instruksi.
fn trace_lines(cpu: &mut Cpu<FlatBus>) -> (Rc<RefCell<Vec<String>>>, Rc<Cell<u32>>) {
    let lines = Rc::new(RefCell::new(Vec::new()));
    let count = Rc::new(Cell::new(0u32));
    let (l, c) = (lines.clone(), count.clone());
    cpu.trace_hook = Some(Box::new(move |t| {
        c.set(c.get() + 1);
        l.borrow_mut().push(line(&t));
    }));
    (lines, count)
}

fn print_all(lines: &Rc<RefCell<Vec<String>>>) {
    for l in lines.borrow().iter() {
        println!("{l}");
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "state".into());
    match mode.as_str() {
        "state" => {
            let mut cpu = Cpu::new(FlatBus::with_prg(game::ROM.prg), game::game());
            let _ = cpu.reset();
            if std::env::args().nth(2).is_some_and(|a| a.starts_with('n')) {
                cpu.nmi();
                cpu.nmi();
            }
            println!(
                "A={:02X} X={:02X} Y={:02X} SP={:02X} N={} V={} D={} I={} Z={} C={} cycles={}",
                cpu.a, cpu.x, cpu.y, cpu.sp, cpu.n, cpu.v, cpu.d, cpu.i, cpu.z, cpu.c, cpu.cycles
            );
            let watch = [0x08u16, 0x10, 0x11, 0x12, 0x13, 0x20, 0x21, 0x30, 0x31, 0x32, 0x33, 0x50, 0x0200, 0x0201];
            print!("mem");
            for a in watch {
                print!(" {a:04X}={:02X}", cpu.read8(a));
            }
            println!();
        }
        "nestest-flat" => {
            let mut cpu = fresh(game::game());
            let (lines, _) = trace_lines(&mut cpu);
            entry_c000(&mut cpu);
            print_all(&lines);
            println!("RESULT 0002={:02X} 0003={:02X}", cpu.bus.mem[2], cpu.bus.mem[3]);
        }
        "nestest-interp" => {
            // Tidak ada fungsi hasil recompile: interpreter menjalankan semuanya. Bila RTS menuju
            // alamat yang bukan milik interpreter, ia kembali; CPU sungguhan lanjut di sana.
            let game = Game { call_entry: |_, _| None, ops: &game::OPS };
            let mut cpu = fresh(game);
            let (lines, count) = trace_lines(&mut cpu);
            let mut pc = 0xC000;
            while count.get() < LOG_LINES {
                pc = cpu.interpret(pc);
            }
            print_all(&lines);
        }
        "nestest-mixed" => {
            let mut cpu = fresh(game::game());
            let (lines, count) = trace_lines(&mut cpu);
            let mut r = entry_c000(&mut cpu);
            while count.get() < LOG_LINES {
                r = cpu.interpret(r);
            }
            print_all(&lines);
        }
        "nestest-ppu" => {
            let mut cpu = fresh(game::game());
            let mut ppu = Ppu::new();
            let mut mapper = Mapper::new(0, game::ROM.prg, &[], 0).unwrap();
            let lines = Rc::new(RefCell::new(Vec::new()));
            let l = lines.clone();
            cpu.trace_hook = Some(Box::new(move |t| {
                ppu.sync(&mut mapper, t.cycles);
                l.borrow_mut().push(format!("{:04X} {} {} {}", t.pc, ppu.scanline(), ppu.dot(), t.cycles));
            }));
            entry_c000(&mut cpu);
            print_all(&lines);
        }
        other => panic!("mode tidak dikenal: {other}"),
    }
}
