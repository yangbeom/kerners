//! FAT32 디렉토리 엔트리
//!
//! 8.3 파일명 및 LFN (Long File Name) 지원

use alloc::string::String;
use alloc::vec::Vec;

/// 디렉토리 엔트리 (32 bytes)
#[derive(Debug, Clone, Copy)]
pub struct DirEntry {
    /// 파일명 (8 bytes, 공백 패딩)
    pub name: [u8; 8],
    /// 확장자 (3 bytes, 공백 패딩)
    pub ext: [u8; 3],
    /// 속성
    pub attr: u8,
    /// NT 예약
    pub nt_res: u8,
    /// 생성 시간 (1/10초)
    pub crt_time_tenth: u8,
    /// 생성 시간
    pub crt_time: u16,
    /// 생성 날짜
    pub crt_date: u16,
    /// 마지막 접근 날짜
    pub lst_acc_date: u16,
    /// 클러스터 번호 상위 16비트
    pub fst_clus_hi: u16,
    /// 수정 시간
    pub wrt_time: u16,
    /// 수정 날짜
    pub wrt_date: u16,
    /// 클러스터 번호 하위 16비트
    pub fst_clus_lo: u16,
    /// 파일 크기 (바이트)
    pub file_size: u32,
}

/// 디렉토리 엔트리 속성
pub mod attr {
    pub const READ_ONLY: u8 = 0x01;
    pub const HIDDEN: u8 = 0x02;
    pub const SYSTEM: u8 = 0x04;
    pub const VOLUME_ID: u8 = 0x08;
    pub const DIRECTORY: u8 = 0x10;
    pub const ARCHIVE: u8 = 0x20;
    pub const LONG_NAME: u8 = READ_ONLY | HIDDEN | SYSTEM | VOLUME_ID;
    pub const LONG_NAME_MASK: u8 = READ_ONLY | HIDDEN | SYSTEM | VOLUME_ID | DIRECTORY | ARCHIVE;
}

impl DirEntry {
    /// 디렉토리 엔트리 크기
    pub const SIZE: usize = 32;

    /// 빈 엔트리 생성
    pub fn empty() -> Self {
        Self {
            name: [0; 8],
            ext: [0; 3],
            attr: 0,
            nt_res: 0,
            crt_time_tenth: 0,
            crt_time: 0,
            crt_date: 0,
            lst_acc_date: 0,
            fst_clus_hi: 0,
            wrt_time: 0,
            wrt_date: 0,
            fst_clus_lo: 0,
            file_size: 0,
        }
    }

    /// 파일 엔트리 생성
    pub fn new_file(name: &str, cluster: u32, size: u32) -> Self {
        let mut entry = Self::empty();
        entry.set_name(name);
        entry.set_cluster(cluster);
        entry.file_size = size;
        entry.attr = attr::ARCHIVE;
        entry
    }

    /// 디렉토리 엔트리 생성
    pub fn new_dir(name: &str, cluster: u32) -> Self {
        let mut entry = Self::empty();
        entry.set_name(name);
        entry.set_cluster(cluster);
        entry.attr = attr::DIRECTORY;
        entry
    }

    /// 빈 엔트리인지 확인
    pub fn is_empty(&self) -> bool {
        self.name[0] == 0x00
    }

    /// 삭제된 엔트리인지 확인
    pub fn is_deleted(&self) -> bool {
        self.name[0] == 0xE5
    }

    /// 유효한 엔트리인지 확인
    pub fn is_valid(&self) -> bool {
        !self.is_empty() && !self.is_deleted()
    }

    /// LFN 엔트리인지 확인
    pub fn is_lfn(&self) -> bool {
        (self.attr & attr::LONG_NAME_MASK) == attr::LONG_NAME
    }

    /// 디렉토리인지 확인
    pub fn is_dir(&self) -> bool {
        (self.attr & attr::DIRECTORY) != 0
    }

    /// 볼륨 레이블인지 확인
    pub fn is_volume_label(&self) -> bool {
        (self.attr & attr::VOLUME_ID) != 0 && !self.is_lfn()
    }

    /// 클러스터 번호 가져오기
    pub fn cluster(&self) -> u32 {
        ((self.fst_clus_hi as u32) << 16) | (self.fst_clus_lo as u32)
    }

    /// 클러스터 번호 설정
    pub fn set_cluster(&mut self, cluster: u32) {
        self.fst_clus_hi = (cluster >> 16) as u16;
        self.fst_clus_lo = (cluster & 0xFFFF) as u16;
    }

    /// 8.3 파일명 가져오기
    pub fn short_name(&self) -> String {
        // 이름 부분 (공백 제거)
        let name_end = self.name.iter().position(|&c| c == b' ' || c == 0).unwrap_or(8);
        let name = &self.name[..name_end];

        // 확장자 부분
        let ext_end = self.ext.iter().position(|&c| c == b' ' || c == 0).unwrap_or(3);
        let ext = &self.ext[..ext_end];

        if ext.is_empty() {
            String::from_utf8_lossy(name).into_owned()
        } else {
            let mut result = String::from_utf8_lossy(name).into_owned();
            result.push('.');
            result.push_str(&String::from_utf8_lossy(ext));
            result
        }
    }

    /// 8.3 파일명 설정
    pub fn set_name(&mut self, name: &str) {
        // 이름과 확장자 분리
        let (base, ext) = if let Some(dot_pos) = name.rfind('.') {
            (&name[..dot_pos], &name[dot_pos + 1..])
        } else {
            (name, "")
        };

        // 이름 설정 (8문자, 대문자, 공백 패딩)
        self.name = [b' '; 8];
        for (i, c) in base.chars().take(8).enumerate() {
            self.name[i] = c.to_ascii_uppercase() as u8;
        }

        // 확장자 설정 (3문자, 대문자, 공백 패딩)
        self.ext = [b' '; 3];
        for (i, c) in ext.chars().take(3).enumerate() {
            self.ext[i] = c.to_ascii_uppercase() as u8;
        }
    }

    /// "." 엔트리인지 확인
    pub fn is_dot(&self) -> bool {
        self.name[0] == b'.' && self.name[1] == b' '
    }

    /// ".." 엔트리인지 확인
    pub fn is_dotdot(&self) -> bool {
        self.name[0] == b'.' && self.name[1] == b'.'
    }

    /// "." 엔트리 생성
    pub fn dot_entry(cluster: u32) -> Self {
        let mut entry = Self::empty();
        entry.name[0] = b'.';
        for i in 1..8 {
            entry.name[i] = b' ';
        }
        entry.ext = [b' '; 3];
        entry.attr = attr::DIRECTORY;
        entry.set_cluster(cluster);
        entry
    }

    /// ".." 엔트리 생성
    pub fn dotdot_entry(parent_cluster: u32) -> Self {
        let mut entry = Self::empty();
        entry.name[0] = b'.';
        entry.name[1] = b'.';
        for i in 2..8 {
            entry.name[i] = b' ';
        }
        entry.ext = [b' '; 3];
        entry.attr = attr::DIRECTORY;
        entry.set_cluster(parent_cluster);
        entry
    }

    /// 엔트리를 삭제됨으로 마킹
    pub fn mark_deleted(&mut self) {
        self.name[0] = 0xE5;
    }

    /// 바이트 배열로 변환
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut buf = [0u8; 32];
        buf[0..8].copy_from_slice(&self.name);
        buf[8..11].copy_from_slice(&self.ext);
        buf[11] = self.attr;
        buf[12] = self.nt_res;
        buf[13] = self.crt_time_tenth;
        buf[14..16].copy_from_slice(&self.crt_time.to_le_bytes());
        buf[16..18].copy_from_slice(&self.crt_date.to_le_bytes());
        buf[18..20].copy_from_slice(&self.lst_acc_date.to_le_bytes());
        buf[20..22].copy_from_slice(&self.fst_clus_hi.to_le_bytes());
        buf[22..24].copy_from_slice(&self.wrt_time.to_le_bytes());
        buf[24..26].copy_from_slice(&self.wrt_date.to_le_bytes());
        buf[26..28].copy_from_slice(&self.fst_clus_lo.to_le_bytes());
        buf[28..32].copy_from_slice(&self.file_size.to_le_bytes());
        buf
    }

    /// 바이트 배열에서 읽기
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 32 {
            return None;
        }

        fn read_u16(buf: &[u8], offset: usize) -> u16 {
            u16::from_le_bytes([buf[offset], buf[offset + 1]])
        }

        fn read_u32(buf: &[u8], offset: usize) -> u32 {
            u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
        }

        let mut name = [0u8; 8];
        name.copy_from_slice(&bytes[0..8]);

        let mut ext = [0u8; 3];
        ext.copy_from_slice(&bytes[8..11]);

        Some(Self {
            name,
            ext,
            attr: bytes[11],
            nt_res: bytes[12],
            crt_time_tenth: bytes[13],
            crt_time: read_u16(bytes, 14),
            crt_date: read_u16(bytes, 16),
            lst_acc_date: read_u16(bytes, 18),
            fst_clus_hi: read_u16(bytes, 20),
            wrt_time: read_u16(bytes, 22),
            wrt_date: read_u16(bytes, 24),
            fst_clus_lo: read_u16(bytes, 26),
            file_size: read_u32(bytes, 28),
        })
    }
}

/// LFN (Long File Name) 엔트리 (32 bytes)
#[derive(Debug, Clone, Copy)]
pub struct LfnEntry {
    /// 순서 (마지막 엔트리는 0x40 OR)
    pub order: u8,
    /// 이름 파트 1 (5 UCS-2 문자)
    pub name1: [u16; 5],
    /// 속성 (항상 LONG_NAME)
    pub attr: u8,
    /// 타입 (항상 0)
    pub entry_type: u8,
    /// 체크섬
    pub checksum: u8,
    /// 이름 파트 2 (6 UCS-2 문자)
    pub name2: [u16; 6],
    /// 예약 (항상 0)
    pub fst_clus_lo: u16,
    /// 이름 파트 3 (2 UCS-2 문자)
    pub name3: [u16; 2],
}

impl LfnEntry {
    /// 마지막 LFN 엔트리 마커
    pub const LAST_ENTRY: u8 = 0x40;

    /// 바이트 배열에서 읽기
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 32 {
            return None;
        }

        fn read_u16(buf: &[u8], offset: usize) -> u16 {
            u16::from_le_bytes([buf[offset], buf[offset + 1]])
        }

        let mut name1 = [0u16; 5];
        for i in 0..5 {
            name1[i] = read_u16(bytes, 1 + i * 2);
        }

        let mut name2 = [0u16; 6];
        for i in 0..6 {
            name2[i] = read_u16(bytes, 14 + i * 2);
        }

        let mut name3 = [0u16; 2];
        for i in 0..2 {
            name3[i] = read_u16(bytes, 28 + i * 2);
        }

        Some(Self {
            order: bytes[0],
            name1,
            attr: bytes[11],
            entry_type: bytes[12],
            checksum: bytes[13],
            name2,
            fst_clus_lo: read_u16(bytes, 26),
            name3,
        })
    }

    /// 8.3 이름의 체크섬 계산
    pub fn checksum(short_name: &[u8; 11]) -> u8 {
        let mut sum: u8 = 0;
        for &byte in short_name {
            sum = sum.wrapping_shr(1).wrapping_add(sum.wrapping_shl(7)).wrapping_add(byte);
        }
        sum
    }

    /// 이 엔트리가 마지막 LFN인지 확인
    pub fn is_last(&self) -> bool {
        (self.order & Self::LAST_ENTRY) != 0
    }

    /// 순서 번호 (1-based)
    pub fn sequence(&self) -> u8 {
        self.order & 0x1F
    }

    /// UCS-2 문자들을 문자열로 변환
    pub fn get_name_part(&self) -> String {
        let mut chars: Vec<u16> = Vec::new();

        for c in self.name1 {
            if c == 0 || c == 0xFFFF {
                break;
            }
            chars.push(c);
        }
        for c in self.name2 {
            if c == 0 || c == 0xFFFF {
                break;
            }
            chars.push(c);
        }
        for c in self.name3 {
            if c == 0 || c == 0xFFFF {
                break;
            }
            chars.push(c);
        }

        String::from_utf16_lossy(&chars)
    }
}

/// LFN 엔트리들에서 전체 이름 추출
pub fn extract_lfn_name(lfn_entries: &[LfnEntry]) -> String {
    let mut parts: Vec<(u8, String)> = Vec::new();

    for entry in lfn_entries {
        let seq = entry.sequence();
        let part = entry.get_name_part();
        parts.push((seq, part));
    }

    // 순서대로 정렬
    parts.sort_by_key(|(seq, _)| *seq);

    // 조합
    let mut result = String::new();
    for (_, part) in parts {
        result.push_str(&part);
    }

    result
}
