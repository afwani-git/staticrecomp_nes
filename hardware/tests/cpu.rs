//! Helper ALU, flag, dan stack di `Cpu` dengan nilai batas. Memori datar 64 KB, tanpa hardware.

use hardware::cpu::{Cpu, FlatBus, Game};
use hardware::interp::NO_OP;

static OPS: [hardware::interp::Op; 256] = [NO_OP; 256];

fn cpu() -> Cpu<FlatBus> {
    Cpu::new(FlatBus::new(), Game { call_entry: |_, _| None, ops: &OPS })
}

/// Cek flag N V Z C sekaligus.
fn flags(c: &Cpu<FlatBus>, n: u8, v: u8, z: u8, cy: u8, what: &str) {
    assert_eq!((c.n, c.v, c.z, c.c), (n, v, z, cy), "{what}");
}

#[test]
fn adc() {
    let mut c = cpu();
    c.a = 0x50;
    c.c = 0;
    c.adc(0x50);
    assert_eq!(c.a, 0xA0, "adc 50+50 hasil");
    flags(&c, 1, 1, 0, 0, "adc 50+50 flag");
    c.a = 0xFF;
    c.c = 0;
    c.adc(0x01);
    assert_eq!(c.a, 0x00, "adc FF+1 hasil");
    flags(&c, 0, 0, 1, 1, "adc FF+1 flag");
    c.a = 0x7F;
    c.c = 1;
    c.adc(0x00);
    assert_eq!(c.a, 0x80, "adc 7F+0+c hasil");
    flags(&c, 1, 1, 0, 0, "adc 7F+0+c flag");
    c.a = 0xD0;
    c.c = 0;
    c.adc(0x90);
    assert_eq!(c.a, 0x60, "adc D0+90 hasil");
    flags(&c, 0, 1, 0, 1, "adc D0+90 flag");
}

#[test]
fn sbc_carry_berarti_tidak_ada_pinjaman() {
    let mut c = cpu();
    c.a = 0x50;
    c.c = 1;
    c.sbc(0xF0);
    assert_eq!(c.a, 0x60, "sbc 50-F0 hasil");
    flags(&c, 0, 0, 0, 0, "sbc 50-F0 flag");
    c.a = 0x80;
    c.c = 1;
    c.sbc(0x01);
    assert_eq!(c.a, 0x7F, "sbc 80-1 hasil");
    flags(&c, 0, 1, 0, 1, "sbc 80-1 flag");
    c.a = 0x00;
    c.c = 0;
    c.sbc(0x00);
    assert_eq!(c.a, 0xFF, "sbc 0-0-pinjam hasil");
    flags(&c, 1, 0, 0, 0, "sbc 0-0-pinjam flag");
    c.a = 0x05;
    c.c = 1;
    c.sbc(0x05);
    assert_eq!(c.a, 0x00, "sbc 5-5 hasil");
    flags(&c, 0, 0, 1, 1, "sbc 5-5 flag");
}

#[test]
fn cmp_dan_bit() {
    let mut c = cpu();
    c.c = 0;
    c.z = 0;
    c.n = 0;
    let v = c.v;
    c.cmp(0x40, 0x40);
    flags(&c, 0, v, 1, 1, "cmp sama");
    c.cmp(0x30, 0x40);
    flags(&c, 1, v, 0, 0, "cmp lebih kecil");
    c.cmp(0x50, 0x40);
    flags(&c, 0, v, 0, 1, "cmp lebih besar");

    c.a = 0x0F;
    c.bit(0xC0);
    assert_eq!((c.z, c.n, c.v), (1, 1, 1), "bit 0F & C0");
    assert_eq!(c.a, 0x0F, "bit tidak mengubah A");
    c.a = 0xFF;
    c.bit(0x40);
    assert_eq!((c.z, c.n, c.v), (0, 0, 1), "bit FF & 40");
}

#[test]
fn geser_dan_rotasi() {
    let mut c = cpu();
    c.c = 0;
    let r = c.asl(0x81);
    assert_eq!((r, c.c, c.n, c.z), (0x02, 1, 0, 0), "asl 81");
    c.c = 0;
    let r = c.lsr(0x01);
    assert_eq!((r, c.c, c.z), (0x00, 1, 1), "lsr 01");
    c.c = 1;
    let r = c.rol(0x80);
    assert_eq!((r, c.c), (0x01, 1), "rol 80 dengan carry");
    c.c = 1;
    let r = c.ror(0x01);
    assert_eq!((r, c.c, c.n), (0x80, 1, 1), "ror 01 dengan carry");
    c.c = 0;
    let r = c.ror(0x02);
    assert_eq!((r, c.c), (0x01, 0), "ror 02 tanpa carry");
}

#[test]
fn byte_status() {
    let mut c = cpu();
    (c.n, c.v, c.d, c.i, c.z, c.c) = (1, 0, 0, 1, 0, 1);
    assert_eq!(c.get_p(true), 0xB5, "get_p dengan B");
    assert_eq!(c.get_p(false), 0xA5, "get_p tanpa B");
    c.set_p(0xFF);
    assert_eq!((c.n, c.v, c.d, c.i, c.z, c.c), (1, 1, 1, 1, 1, 1), "set_p FF menyalakan semua");
    assert_eq!(c.get_p(false), 0xEF, "set_p mengabaikan bit B dan bit 5");
    c.set_p(0x00);
    assert_eq!((c.n, c.v, c.d, c.i, c.z, c.c), (0, 0, 0, 0, 0, 0), "set_p 00 mematikan semua");
}

#[test]
fn stack_dan_pointer_halaman_nol() {
    let mut c = cpu();
    c.sp = 0xFF;
    c.push8(0xAB);
    assert_eq!((c.sp, c.bus.mem[0x1FF]), (0xFE, 0xAB), "push8");
    assert_eq!((c.pull8(), c.sp), (0xAB, 0xFF), "pull8");
    c.sp = 0x00;
    c.push8(0x11);
    assert_eq!((c.sp, c.bus.mem[0x100]), (0xFF, 0x11), "push8 berputar di halaman 1");

    c.bus.mem[0xFF] = 0x34;
    c.bus.mem[0x00] = 0x12;
    assert_eq!(c.rd16zp(0xFF), 0x1234, "rd16zp berputar di halaman nol");
}
