// Driver tes hardware lengkap (bus, PPU, APU, mapper) tanpa browser. Mode lewat argumen pertama:
//   mem <frame> [alamat-hex ...]
//       jalankan sampai frame ke-N, cetak "mem AAAA=VV ..."
//   render <frame> <snap,...> <tombol:dari-sampai,...>
//       jalankan N frame dengan tombol ditahan pada rentang frame (inklusif); cetak "HASH n h"
//       (16 hex pertama SHA-256 layar RGBA) untuk frame di daftar snap, "INTERP pc->target ...",
//       dan "OK frames=N colors=K"
//   blargg <frame-maks>
//       jalankan ROM tes blargg: $6000 = status, $6001-$6003 = DE B0 61, teks mulai $6004
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use hardware::nes::{Host, Nes};

// ---- SHA-256 (hanya untuk mengunci hasil render) ----

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---- Host ----

#[derive(Default)]
struct State {
    frame: u32,
    limit: u32,
    snaps: BTreeSet<u32>,
    /// (bit tombol, dari frame, sampai frame)
    presses: Vec<(u8, u32, u32)>,
    hashes: Vec<String>,
    interp: BTreeSet<String>,
    last_fb: Vec<u8>,
    blargg: bool,
    result: Option<(u8, String)>,
    fault: Option<(u32, u32, u32)>,
}

struct H(Rc<RefCell<State>>);

impl Host for H {
    fn buttons(&mut self) -> u32 {
        let s = self.0.borrow();
        s.presses.iter().fold(0, |m, &(bit, from, to)| if s.frame >= from && s.frame <= to { m | (1 << bit) } else { m })
    }

    fn present(&mut self, rgba: &[u8]) {
        let mut s = self.0.borrow_mut();
        s.frame += 1;
        if s.snaps.contains(&s.frame) {
            let h = hex(&sha256(rgba))[..16].to_string();
            let line = format!("HASH {} {h}", s.frame);
            s.hashes.push(line);
        }
        s.last_fb = rgba.to_vec();
    }

    fn stop(&mut self, peek: &dyn Fn(u16) -> u8) -> bool {
        let mut s = self.0.borrow_mut();
        if s.blargg {
            let status = peek(0x6000);
            if peek(0x6001) == 0xDE && peek(0x6002) == 0xB0 && peek(0x6003) == 0x61 && status != 0x80 {
                let mut text = String::new();
                for a in 0x6004u16..0x7000 {
                    let c = peek(a);
                    if c == 0 {
                        break;
                    }
                    text.push(c as char);
                }
                s.result = Some((status, text.trim().to_string()));
                return true;
            }
        }
        s.frame >= s.limit
    }

    fn interp(&mut self, from_pc: u16, target: u16) {
        self.0.borrow_mut().interp.insert(format!("{from_pc:X}->{target:X}"));
    }

    fn fault(&mut self, pc: u32, target: u32, kind: u32) {
        self.0.borrow_mut().fault = Some((pc, target, kind));
        eprintln!("fault pc={pc:04X} target={target:04X} kind={kind}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("mem");
    let state = Rc::new(RefCell::new(State::default()));
    let names = ["a", "b", "select", "start", "up", "down", "left", "right"];
    let (rate, peeks): (u32, Vec<u16>);
    match mode {
        "mem" => {
            state.borrow_mut().limit = args.get(2).map_or(5, |a| a.parse().unwrap());
            peeks = args.iter().skip(3).map(|a| u16::from_str_radix(a, 16).unwrap()).collect();
            rate = 48000;
        }
        "render" => {
            let mut s = state.borrow_mut();
            s.limit = args[2].parse().unwrap();
            s.snaps = args[3].split(',').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect();
            for p in args[4].split(',').filter(|x| !x.is_empty()) {
                let (name, range) = p.split_once(':').unwrap();
                let (from, to) = range.split_once('-').unwrap_or((range, range));
                let bit = names.iter().position(|n| *n == name).expect("tombol tidak dikenal") as u8;
                s.presses.push((bit, from.parse().unwrap(), to.parse().unwrap()));
            }
            peeks = vec![];
            rate = 44100;
        }
        "blargg" => {
            let mut s = state.borrow_mut();
            s.limit = args[2].parse().unwrap();
            s.blargg = true;
            peeks = vec![];
            rate = 44100;
        }
        other => panic!("mode tidak dikenal: {other}"),
    }

    let mut nes = match Nes::build(game::game(), game::ROM, Box::new(H(state.clone())), rate) {
        Ok(n) => n,
        Err(m) => {
            eprintln!("fault pc=0000 target={m:04X} kind=2");
            std::process::exit(2);
        }
    };
    nes.run_until_stop();

    let s = state.borrow();
    match mode {
        "mem" => {
            print!("mem");
            for a in peeks {
                print!(" {a:04X}={:02X}", nes.peek(a));
            }
            println!();
        }
        "render" => {
            for h in &s.hashes {
                println!("{h}");
            }
            let mut colors = BTreeSet::new();
            for px in s.last_fb.chunks(4) {
                colors.insert((px[0], px[1], px[2]));
            }
            if !s.interp.is_empty() {
                println!("INTERP {}", s.interp.iter().cloned().collect::<Vec<_>>().join(" "));
            }
            println!("OK frames={} colors={}", s.frame, colors.len());
        }
        "blargg" => match &s.result {
            Some((status, text)) => {
                println!("STATUS {status:02x} frames={}", s.frame);
                println!("{text}");
            }
            None => {
                println!("TIMEOUT frames={} status={:02x}", s.frame, nes.peek(0x6000));
                std::process::exit(3);
            }
        },
        _ => unreachable!(),
    }
}
