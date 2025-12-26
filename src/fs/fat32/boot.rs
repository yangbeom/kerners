//! FAT32 부트 섹터 파싱
//!
//! FAT32 파일시스템의 BPB (BIOS Parameter Block) 및 부트 섹터 구조

use alloc::string::String;

/// FAT32 부트 섹터 (바이트 배열에서 파싱)
#[derive(Debug, Clone, Copy)]
pub struct Fat32BootSector {
    /// 점프 명령 (3 bytes)
    pub jmp_boot: [u8; 3],
    /// OEM 이름 (8 bytes)
    pub oem_name: [u8; 8],
    /// 섹터 당 바이트 수 (512, 1024, 2048, 4096)
    pub bytes_per_sector: u16,
    /// 클러스터 당 섹터 수
    pub sectors_per_cluster: u8,
    /// 예약된 섹터 수 (부트 섹터 포함)
    pub reserved_sectors: u16,
    /// FAT 테이블 수 (보통 2)
    pub num_fats: u8,
    /// 루트 디렉토리 엔트리 수 (FAT32에서는 0)
    pub root_entry_count: u16,
    /// 총 섹터 수 (FAT32에서는 0)
    pub total_sectors_16: u16,
    /// 미디어 타입
    pub media_type: u8,
    /// FAT 크기 (FAT32에서는 0)
    pub fat_size_16: u16,
    /// 트랙 당 섹터 수
    pub sectors_per_track: u16,
    /// 헤드 수
    pub num_heads: u16,
    /// 숨겨진 섹터 수
    pub hidden_sectors: u32,
    /// 총 섹터 수 (FAT32)
    pub total_sectors_32: u32,
    // FAT32 확장 필드
    /// FAT 크기 (섹터 수)
    pub fat_size_32: u32,
    /// 확장 플래그
    pub ext_flags: u16,
    /// 파일시스템 버전
    pub fs_version: u16,
    /// 루트 디렉토리 클러스터 번호
    pub root_cluster: u32,
    /// FSInfo 섹터 번호
    pub fs_info: u16,
    /// 백업 부트 섹터 위치
    pub backup_boot_sector: u16,
    /// 드라이브 번호
    pub drive_number: u8,
    /// 부트 시그니처
    pub boot_sig: u8,
    /// 볼륨 시리얼 번호
    pub volume_id: u32,
    /// 볼륨 레이블 (11 bytes)
    pub volume_label: [u8; 11],
    /// 파일시스템 타입 문자열
    pub fs_type: [u8; 8],
}

impl Fat32BootSector {
    /// 바이트 배열에서 부트 섹터 파싱
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < 90 {
            return None;
        }

        // Helper function to read u16 from little-endian bytes
        fn read_u16(buf: &[u8], offset: usize) -> u16 {
            u16::from_le_bytes([buf[offset], buf[offset + 1]])
        }

        // Helper function to read u32 from little-endian bytes
        fn read_u32(buf: &[u8], offset: usize) -> u32 {
            u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
        }

        let mut jmp_boot = [0u8; 3];
        jmp_boot.copy_from_slice(&buf[0..3]);

        let mut oem_name = [0u8; 8];
        oem_name.copy_from_slice(&buf[3..11]);

        let mut volume_label = [0u8; 11];
        volume_label.copy_from_slice(&buf[71..82]);

        let mut fs_type = [0u8; 8];
        fs_type.copy_from_slice(&buf[82..90]);

        Some(Self {
            jmp_boot,
            oem_name,
            bytes_per_sector: read_u16(buf, 11),
            sectors_per_cluster: buf[13],
            reserved_sectors: read_u16(buf, 14),
            num_fats: buf[16],
            root_entry_count: read_u16(buf, 17),
            total_sectors_16: read_u16(buf, 19),
            media_type: buf[21],
            fat_size_16: read_u16(buf, 22),
            sectors_per_track: read_u16(buf, 24),
            num_heads: read_u16(buf, 26),
            hidden_sectors: read_u32(buf, 28),
            total_sectors_32: read_u32(buf, 32),
            fat_size_32: read_u32(buf, 36),
            ext_flags: read_u16(buf, 40),
            fs_version: read_u16(buf, 42),
            root_cluster: read_u32(buf, 44),
            fs_info: read_u16(buf, 48),
            backup_boot_sector: read_u16(buf, 50),
            drive_number: buf[64],
            boot_sig: buf[66],
            volume_id: read_u32(buf, 67),
            volume_label,
            fs_type,
        })
    }

    /// 부트 섹터 유효성 검사
    pub fn is_valid(&self) -> bool {
        // 점프 명령 확인
        if self.jmp_boot[0] != 0xEB && self.jmp_boot[0] != 0xE9 {
            return false;
        }
        // 바이트 수 확인
        let bps = self.bytes_per_sector;
        if bps != 512 && bps != 1024 && bps != 2048 && bps != 4096 {
            return false;
        }
        // 클러스터 당 섹터 확인
        let spc = self.sectors_per_cluster;
        if spc == 0 || (spc & (spc - 1)) != 0 {
            return false; // 2의 거듭제곱이어야 함
        }
        // FAT32인지 확인
        if self.root_entry_count != 0 || self.total_sectors_16 != 0 {
            return false; // FAT12/16
        }
        if self.fat_size_32 == 0 {
            return false;
        }
        true
    }

    /// FAT 영역 시작 섹터
    pub fn fat_start_sector(&self) -> u32 {
        self.reserved_sectors as u32
    }

    /// 데이터 영역 시작 섹터
    pub fn data_start_sector(&self) -> u32 {
        self.reserved_sectors as u32 + (self.num_fats as u32 * self.fat_size_32)
    }

    /// 클러스터 번호를 섹터 번호로 변환
    pub fn cluster_to_sector(&self, cluster: u32) -> u32 {
        // 클러스터 번호는 2부터 시작
        self.data_start_sector() + (cluster - 2) * self.sectors_per_cluster as u32
    }

    /// 총 클러스터 수
    pub fn total_clusters(&self) -> u32 {
        let data_sectors = self.total_sectors_32 - self.data_start_sector();
        data_sectors / self.sectors_per_cluster as u32
    }

    /// 볼륨 레이블 문자열
    pub fn volume_label_str(&self) -> String {
        let label = core::str::from_utf8(&self.volume_label)
            .unwrap_or("NO NAME")
            .trim();
        String::from(label)
    }

    /// OEM 이름 문자열
    pub fn oem_name_str(&self) -> String {
        let name = core::str::from_utf8(&self.oem_name)
            .unwrap_or("")
            .trim();
        String::from(name)
    }
}
