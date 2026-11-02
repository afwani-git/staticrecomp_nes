use anyhow::{Result, bail};

const MAGIC: [u8; 4] = *b"NES\x1A";

/// Mapper iNES yang dikenali hardware: 0 NROM, 1 MMC1, 2 UxROM, 3 CNROM, 4 MMC3, 7 AxROM,
/// 11 Color Dreams, 66 GxROM, 71 Camerica, 79 NINA-003/006.
pub const SUPPORTED_MAPPERS: [u16; 10] = [0, 1, 2, 3, 4, 7, 11, 66, 71, 79];

/// Pesan kesalahan untuk mapper yang tidak didukung.
pub fn unsupported_mapper_message(mapper: u16) -> String {
    let list: Vec<String> = SUPPORTED_MAPPERS.iter().map(|m| m.to_string()).collect();
    format!("mapper {mapper} belum didukung (didukung: {})", list.join(", "))
}
const HEADER_LEN: usize = 16;
const TRAINER_LEN: usize = 512;
const PRG_BANK_LEN: usize = 16 * 1024;
const CHR_BANK_LEN: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    FourScreen,
}

impl std::fmt::Display for Mirroring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Mirroring::Horizontal => "horizontal",
            Mirroring::Vertical => "vertical",
            Mirroring::FourScreen => "four-screen",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InesHeader {
    /// Jumlah bank PRG-ROM (satuan 16 KB).
    pub prg_banks: u16,
    /// Jumlah bank CHR-ROM (satuan 8 KB). 0 berarti cartridge memakai CHR-RAM.
    pub chr_banks: u16,
    /// Nomor mapper: iNES 1.0 sampai 255, NES 2.0 sampai 4095.
    pub mapper: u16,
    /// Submapper NES 2.0 (0 untuk iNES 1.0). Dicatat tetapi belum memengaruhi perilaku hardware.
    pub submapper: u8,
    pub nes2: bool,
    pub mirroring: Mirroring,
    pub battery: bool,
    pub trainer: bool,
    /// Ukuran PRG-RAM dalam byte (volatil + berbaterai) bila dinyatakan header NES 2.0; selain itu 0.
    pub prg_ram_size: u32,
    /// Ukuran CHR-RAM dalam byte (volatil + berbaterai) bila dinyatakan header NES 2.0; selain itu 0.
    pub chr_ram_size: u32,
}

/// Ukuran PRG/CHR dalam satuan bank dari header NES 2.0: byte LSB (4 atau 5) dan nibble MSB (byte 9).
/// Nibble MSB = 0xF berarti notasi eksponen-pengali: ukuran = 2^E * (2*M + 1) byte, dengan
/// byte LSB = EEEEEEMM.
fn nes2_banks(lsb: u8, msb: u8, unit: usize, what: &str) -> Result<u16> {
    if msb != 0x0F {
        return Ok(((msb as u16) << 8) | lsb as u16);
    }
    let bytes = (1u128 << (lsb >> 2)) * ((lsb & 3) as u128 * 2 + 1);
    if bytes % unit as u128 != 0 {
        bail!("ukuran {what} NES 2.0 (notasi eksponen) {bytes} byte bukan kelipatan {unit} byte, belum didukung");
    }
    u16::try_from(bytes / unit as u128).map_err(|_| anyhow::anyhow!("ukuran {what} NES 2.0 terlalu besar"))
}

/// Ukuran RAM dari nibble "jumlah geser" NES 2.0: 0 = tidak ada, selain itu 64 << nibble byte.
fn nes2_ram(shift: u8) -> u32 {
    if shift == 0 { 0 } else { 64u32 << shift }
}

impl InesHeader {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_LEN {
            bail!(
                "file terlalu pendek: {} byte, header iNES butuh {HEADER_LEN}",
                data.len()
            );
        }
        if data[0..4] != MAGIC {
            bail!("magic salah: ini bukan file iNES (harus diawali \"NES\\x1A\")");
        }
        let flags6 = data[6];
        let flags7 = data[7];
        let nes2 = flags7 & 0x0C == 0x08;

        let mirroring = if flags6 & 0x08 != 0 {
            Mirroring::FourScreen
        } else if flags6 & 0x01 != 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };

        let mut header = InesHeader {
            prg_banks: data[4] as u16,
            chr_banks: data[5] as u16,
            mapper: 0,
            submapper: 0,
            nes2,
            mirroring,
            battery: flags6 & 0x02 != 0,
            trainer: flags6 & 0x04 != 0,
            prg_ram_size: 0,
            chr_ram_size: 0,
        };
        if nes2 {
            header.prg_banks = nes2_banks(data[4], data[9] & 0x0F, PRG_BANK_LEN, "PRG")?;
            header.chr_banks = nes2_banks(data[5], data[9] >> 4, CHR_BANK_LEN, "CHR")?;
            header.mapper = ((data[8] & 0x0F) as u16) << 8 | (flags7 & 0xF0) as u16 | (flags6 >> 4) as u16;
            header.submapper = data[8] >> 4;
            header.prg_ram_size = nes2_ram(data[10] & 0x0F) + nes2_ram(data[10] >> 4);
            header.chr_ram_size = nes2_ram(data[11] & 0x0F) + nes2_ram(data[11] >> 4);
        } else {
            // Byte 8-15 yang kotor (mis. teks "DiskDude!" dari alat lama) merusak nibble atas mapper:
            // abaikan bila byte 12-15 bukan nol, seperti anjuran nesdev.
            let dirty = data[12..16].iter().any(|&b| b != 0);
            let high = if dirty { 0 } else { flags7 & 0xF0 };
            header.mapper = (high | (flags6 >> 4)) as u16;
        }

        let expected = header.prg_offset() + header.prg_len() + header.chr_len();
        if data.len() < expected {
            bail!(
                "file terpotong: header mengklaim {expected} byte, file hanya {} byte",
                data.len()
            );
        }
        Ok(header)
    }

    /// Posisi byte pertama PRG-ROM di dalam file.
    pub fn prg_offset(&self) -> usize {
        HEADER_LEN + if self.trainer { TRAINER_LEN } else { 0 }
    }

    pub fn prg_len(&self) -> usize {
        self.prg_banks as usize * PRG_BANK_LEN
    }

    pub fn chr_len(&self) -> usize {
        self.chr_banks as usize * CHR_BANK_LEN
    }

    /// Mapper yang didukung hardware.
    pub fn mapper_supported(&self) -> bool {
        SUPPORTED_MAPPERS.contains(&self.mapper)
    }

    /// Indeks di dalam PRG-ROM untuk alamat CPU `addr`, hanya untuk alamat yang SELALU berisi
    /// byte yang sama (jadi bisa dianalisis statis).
    ///  - Mapper 0 dan 3 (NROM, CNROM): PRG tidak berganti. 16 KB tampil dua kali ($8000 dan
    ///    $C000); 32 KB langsung.
    ///  - Mapper 2 dan 71 (UxROM, Camerica): bank 16 KB terakhir tetap di $C000-$FFFF.
    ///  - Mapper 4 (MMC3): bank 8 KB terakhir tetap di $E000-$FFFF.
    ///  - Mapper lain (1, 7, 11, 66, 79): tidak ada alamat yang pasti tetap. Bank terakhir bisa
    ///    berpindah (MMC1 tergantung mode) atau seluruh 32 KB berganti, jadi semuanya dijalankan
    ///    interpreter.
    pub fn prg_index(&self, addr: u16) -> Option<usize> {
        match self.mapper {
            0 | 3 if addr >= 0x8000 => {
                let mut off = (addr - 0x8000) as usize;
                if self.prg_banks == 1 {
                    off &= PRG_BANK_LEN - 1;
                }
                (off < self.prg_len()).then_some(off)
            }
            2 | 71 if addr >= 0xC000 && self.prg_len() >= 0x4000 => {
                Some(self.prg_len() - 0x4000 + (addr - 0xC000) as usize)
            }
            4 if addr >= 0xE000 && self.prg_len() >= 0x2000 => {
                Some(self.prg_len() - 0x2000 + (addr - 0xE000) as usize)
            }
            _ => None,
        }
    }

    /// Alamat CPU tempat byte pertama wilayah yang dianalisis ditampilkan.
    pub fn prg_base(&self) -> u16 {
        match self.mapper {
            4 => 0xE000,
            2 | 71 => 0xC000,
            _ if self.prg_banks == 1 => 0xC000,
            _ => 0x8000,
        }
    }

    /// (indeks awal, panjang) wilayah PRG yang bisa dianalisis statis; panjang 0 = tidak ada.
    pub fn analysis_region(&self) -> (usize, usize) {
        match self.mapper {
            0 | 3 => (0, self.prg_len()),
            2 | 71 => (self.prg_len().saturating_sub(0x4000), self.prg_len().min(0x4000)),
            4 => (self.prg_len().saturating_sub(0x2000), self.prg_len().min(0x2000)),
            _ => (0, 0),
        }
    }

    /// Byte PRG-ROM di alamat CPU `addr`.
    pub fn read_prg(&self, data: &[u8], addr: u16) -> Option<u8> {
        let off = self.prg_index(addr)?;
        data.get(self.prg_offset() + off).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Membuat ROM palsu dengan header sesuai argumen dan isi nol.
    fn rom(prg: u8, chr: u8, flags6: u8, flags7: u8) -> Vec<u8> {
        let mut v = vec![0u8; HEADER_LEN];
        v[0..4].copy_from_slice(&MAGIC);
        v[4] = prg;
        v[5] = chr;
        v[6] = flags6;
        v[7] = flags7;
        let trainer = if flags6 & 0x04 != 0 { TRAINER_LEN } else { 0 };
        v.resize(
            HEADER_LEN + trainer + prg as usize * PRG_BANK_LEN + chr as usize * CHR_BANK_LEN,
            0,
        );
        v
    }

    #[test]
    fn header_valid_terbaca() {
        let h = InesHeader::parse(&rom(1, 1, 0x01, 0x00)).unwrap();
        assert_eq!(h.prg_banks, 1);
        assert_eq!(h.chr_banks, 1);
        assert_eq!(h.mapper, 0);
        assert_eq!(h.mirroring, Mirroring::Vertical);
        assert!(!h.battery && !h.trainer);
        assert_eq!(h.prg_len(), 16 * 1024);
        assert_eq!(h.chr_len(), 8 * 1024);
    }

    #[test]
    fn magic_salah_ditolak() {
        let mut d = rom(1, 1, 0, 0);
        d[3] = 0x00;
        let err = InesHeader::parse(&d).unwrap_err().to_string();
        assert!(err.contains("magic"));
    }

    #[test]
    fn file_terlalu_pendek_ditolak() {
        assert!(InesHeader::parse(b"NES\x1A").is_err());
    }

    #[test]
    fn mapper_gabungan_nibble_flags6_dan_flags7() {
        // nibble bawah mapper di flags6 bit 4-7, nibble atas di flags7 bit 4-7
        let h = InesHeader::parse(&rom(1, 0, 0x10, 0x20)).unwrap();
        assert_eq!(h.mapper, 0x21);
    }

    #[test]
    fn mirroring_semua_jenis() {
        let m = |f6| InesHeader::parse(&rom(1, 0, f6, 0)).unwrap().mirroring;
        assert_eq!(m(0x00), Mirroring::Horizontal);
        assert_eq!(m(0x01), Mirroring::Vertical);
        assert_eq!(m(0x08), Mirroring::FourScreen);
        // four-screen menang atas bit vertikal
        assert_eq!(m(0x09), Mirroring::FourScreen);
    }

    #[test]
    fn trainer_menggeser_offset_prg() {
        let h = InesHeader::parse(&rom(1, 0, 0x04, 0)).unwrap();
        assert!(h.trainer);
        assert_eq!(h.prg_offset(), 16 + 512);
        let h = InesHeader::parse(&rom(1, 0, 0x00, 0)).unwrap();
        assert_eq!(h.prg_offset(), 16);
    }

    #[test]
    fn file_terpotong_ditolak() {
        let mut d = rom(2, 1, 0, 0);
        d.truncate(d.len() - 1);
        let err = InesHeader::parse(&d).unwrap_err().to_string();
        assert!(err.contains("terpotong"));
    }

    #[test]
    fn read_prg_16kb_di_mirror() {
        let mut d = rom(1, 0, 0, 0);
        d[16] = 0xAA; // awal PRG
        d[16 + 0x3FFF] = 0xBB; // akhir PRG
        let h = InesHeader::parse(&d).unwrap();
        assert_eq!(h.read_prg(&d, 0x8000), Some(0xAA));
        assert_eq!(h.read_prg(&d, 0xC000), Some(0xAA));
        assert_eq!(h.read_prg(&d, 0xBFFF), Some(0xBB));
        assert_eq!(h.read_prg(&d, 0xFFFF), Some(0xBB));
        assert_eq!(h.read_prg(&d, 0x7FFF), None);
    }

    #[test]
    fn read_prg_32kb_langsung_dan_mapper_lain_ditolak() {
        let mut d = rom(2, 0, 0, 0);
        d[16 + 0x4000] = 0xCC; // awal bank kedua = $C000
        let h = InesHeader::parse(&d).unwrap();
        assert_eq!(h.read_prg(&d, 0xC000), Some(0xCC));
        assert_eq!(h.read_prg(&d, 0x8000), Some(0));
        let d = rom(1, 0, 0x10, 0);
        let h = InesHeader::parse(&d).unwrap();
        assert_eq!(h.read_prg(&d, 0x8000), None);
    }

    #[test]
    fn mmc3_hanya_bank_terakhir_yang_tetap() {
        // 8 bank 16 KB = 16 bank 8 KB; bank terakhir = 8 KB terakhir PRG.
        let mut d = rom(8, 16, 0x40, 0x00); // mapper 4 (nibble bawah di flags6 bit 4-7)
        let last = 16 + 8 * 16 * 1024 - 8192;
        d[last] = 0xAA;
        d[last + 0x1FFF] = 0xBB;
        let h = InesHeader::parse(&d).unwrap();
        assert_eq!(h.mapper, 4);
        assert!(h.mapper_supported());
        assert_eq!(h.read_prg(&d, 0xE000), Some(0xAA));
        assert_eq!(h.read_prg(&d, 0xFFFF), Some(0xBB));
        // $8000-$DFFF bisa berganti bank: tidak ada jawaban statis
        assert_eq!(h.read_prg(&d, 0x8000), None);
        assert_eq!(h.read_prg(&d, 0xDFFF), None);
        assert_eq!(h.prg_base(), 0xE000);
        assert_eq!(h.analysis_region(), (8 * 16 * 1024 - 8192, 8192));
    }

    #[test]
    fn wilayah_tetap_per_mapper() {
        // 4 bank 16 KB = 64 KB PRG
        let make = |flags6: u8, flags7: u8| {
            let d = rom(4, 1, flags6, flags7);
            (InesHeader::parse(&d).unwrap(), d)
        };
        let n = 64 * 1024;
        // CNROM (3): seperti NROM, PRG tidak berganti
        let (h, _) = make(0x30, 0x00);
        assert_eq!((h.mapper, h.analysis_region()), (3, (0, n)));
        assert_eq!(h.prg_index(0x8000), Some(0));
        // UxROM (2) dan Camerica (71): hanya 16 KB terakhir di $C000
        for (f6, f7, id) in [(0x20, 0x00, 2), (0x70, 0x40, 71)] {
            let (h, _) = make(f6, f7);
            assert_eq!(h.mapper, id);
            assert_eq!(h.analysis_region(), (n - 0x4000, 0x4000));
            assert_eq!(h.prg_index(0xC000), Some(n - 0x4000));
            assert_eq!(h.prg_index(0xFFFF), Some(n - 1));
            assert_eq!(h.prg_index(0x8000), None);
            assert_eq!(h.prg_base(), 0xC000);
        }
        // MMC1 (1), AxROM (7), Color Dreams (11), GxROM (66), NINA (79): tidak ada wilayah tetap
        for (f6, f7, id) in [(0x10, 0x00, 1), (0x70, 0x00, 7), (0xB0, 0x00, 11), (0x20, 0x40, 66), (0xF0, 0x40, 79)] {
            let (h, _) = make(f6, f7);
            assert_eq!(h.mapper, id);
            assert!(h.mapper_supported(), "mapper {id}");
            assert_eq!(h.analysis_region(), (0, 0), "mapper {id}");
            assert_eq!(h.prg_index(0xFFFF), None);
        }
        // Mapper yang tidak didukung
        let (h, _) = make(0x50, 0x00);
        assert!(!h.mapper_supported());
        assert!(unsupported_mapper_message(5).contains("didukung: 0, 1, 2, 3, 4, 7, 11, 66, 71, 79"));
    }

    /// Header NES 2.0 dengan isi nol sebanyak yang diklaim. `b` = byte 4..=15 yang diubah.
    fn nes2(b: &[(usize, u8)], prg_len: usize, chr_len: usize) -> Vec<u8> {
        let mut v = vec![0u8; HEADER_LEN];
        v[0..4].copy_from_slice(&MAGIC);
        v[7] = 0x08;
        for &(i, x) in b {
            v[i] |= x;
        }
        v.resize(HEADER_LEN + prg_len + chr_len, 0);
        v
    }

    #[test]
    fn nes20_dasar_terbaca_seperti_ines() {
        let d = nes2(&[(4, 1), (5, 1), (6, 0x01)], 16 * 1024, 8 * 1024);
        let h = InesHeader::parse(&d).unwrap();
        assert!(h.nes2);
        assert_eq!((h.prg_banks, h.chr_banks, h.mapper, h.submapper), (1, 1, 0, 0));
        assert_eq!(h.mirroring, Mirroring::Vertical);
        assert_eq!((h.prg_len(), h.chr_len()), (16 * 1024, 8 * 1024));
    }

    #[test]
    fn nes20_mapper_12_bit_dan_submapper() {
        // mapper 0x104 = 260: nibble bawah di byte 6, nibble tengah di byte 7, nibble atas di byte 8;
        // submapper di nibble atas byte 8
        let d = nes2(&[(4, 1), (6, 0x40), (8, 0x31)], 16 * 1024, 0);
        let h = InesHeader::parse(&d).unwrap();
        assert_eq!((h.mapper, h.submapper), (0x104, 3));
        assert!(!h.mapper_supported());
        assert!(unsupported_mapper_message(h.mapper).contains("mapper 260"));
        // mapper kecil lewat NES 2.0 tetap sama dengan iNES: mapper 4 submapper 1 (MMC6)
        let d = nes2(&[(4, 1), (6, 0x40), (8, 0x10)], 16 * 1024, 0);
        let h = InesHeader::parse(&d).unwrap();
        assert_eq!((h.mapper, h.submapper), (4, 1));
        assert!(h.mapper_supported());
    }

    #[test]
    fn nes20_ukuran_dengan_nibble_msb() {
        // PRG 0x102 bank = 258 bank 16 KB; CHR 0x201 bank = 513 bank 8 KB (nibble di byte 9)
        let d = nes2(&[(4, 0x02), (5, 0x01), (9, 0x21)], 258 * 16 * 1024, 513 * 8 * 1024);
        let h = InesHeader::parse(&d).unwrap();
        assert_eq!((h.prg_banks, h.chr_banks), (0x102, 0x201));
    }

    #[test]
    fn nes20_notasi_eksponen() {
        // PRG: E=14, M=1 -> 2^14 * 3 = 48 KB = 3 bank. CHR: E=13, M=0 -> 8 KB = 1 bank.
        let d = nes2(&[(4, (14 << 2) | 1), (5, 13 << 2), (9, 0xFF)], 48 * 1024, 8 * 1024);
        let h = InesHeader::parse(&d).unwrap();
        assert_eq!((h.prg_banks, h.chr_banks), (3, 1));
        // E=10 -> 1 KB: bukan kelipatan 16 KB, ditolak dengan pesan jelas
        let d = nes2(&[(4, 10 << 2), (9, 0x0F)], 1024, 0);
        let err = InesHeader::parse(&d).unwrap_err().to_string();
        assert!(err.contains("bukan kelipatan"), "{err}");
    }

    #[test]
    fn nes20_ukuran_ram() {
        // byte 10: PRG-RAM volatil geser 7 (8 KB) + berbaterai geser 5 (2 KB); byte 11: CHR-RAM 8 KB
        let d = nes2(&[(4, 1), (10, 0x57), (11, 0x07)], 16 * 1024, 0);
        let h = InesHeader::parse(&d).unwrap();
        assert_eq!(h.prg_ram_size, 8192 + 2048);
        assert_eq!(h.chr_ram_size, 8192);
        // tidak dinyatakan = 0
        let h = InesHeader::parse(&nes2(&[(4, 1)], 16 * 1024, 0)).unwrap();
        assert_eq!((h.prg_ram_size, h.chr_ram_size), (0, 0));
    }

    #[test]
    fn nes20_terpotong_tetap_ditolak() {
        let mut d = nes2(&[(4, 2)], 2 * 16 * 1024, 0);
        d.pop();
        assert!(InesHeader::parse(&d).unwrap_err().to_string().contains("terpotong"));
    }

    #[test]
    fn ines1_dengan_byte_kotor_mengabaikan_nibble_atas_mapper() {
        // "DiskDude!" di byte 7-15: byte 12-15 bukan nol, jadi nibble atas mapper tidak dipercaya
        let mut d = rom(1, 0, 0x40, 0x00);
        d[7] = b'D'; // 0x44: nibble atas 4 akan menjadi mapper 0x44 kalau dipercaya
        d[12..16].copy_from_slice(b"ude!");
        let h = InesHeader::parse(&d).unwrap();
        assert!(!h.nes2);
        assert_eq!(h.mapper, 4);
    }
}
