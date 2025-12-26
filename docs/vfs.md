# Virtual File System (VFS)

가상 파일시스템 문서

## Overview

`src/fs/` 모듈은 다양한 파일시스템을 통합하는 VFS 레이어를 제공합니다.

## Architecture

```
┌─────────────────────────────────────────┐
│              User Space                  │
├─────────────────────────────────────────┤
│           System Calls (syscall/fs.rs)   │
├─────────────────────────────────────────┤
│              VFS Layer (fs/mod.rs)       │
├──────────┬──────────┬───────────────────┤
│  RamFS   │  DevFS   │      FAT32        │
├──────────┴──────────┴───────────────────┤
│            Block Device Layer            │
└─────────────────────────────────────────┘
```

## Core Traits

### VNode

파일/디렉토리 추상화. 모든 파일시스템 객체가 구현해야 하는 인터페이스.

```rust
pub trait VNode: Send + Sync {
    fn node_type(&self) -> VNodeType;
    fn read(&self, offset: usize, buf: &mut [u8]) -> VfsResult<usize>;
    fn write(&self, offset: usize, buf: &[u8]) -> VfsResult<usize>;
    fn lookup(&self, name: &str) -> VfsResult<Arc<dyn VNode>>;
    fn create(&self, name: &str, node_type: VNodeType, mode: FileMode) -> VfsResult<Arc<dyn VNode>>;
    fn readdir(&self) -> VfsResult<Vec<DirEntry>>;
    fn stat(&self) -> VfsResult<Stat>;
    // ... 기타 메서드
}
```

### FileSystem

파일시스템 추상화.

```rust
pub trait FileSystem: Send + Sync {
    fn name(&self) -> &str;
    fn root(&self) -> Arc<dyn VNode>;
    fn sync(&self) -> VfsResult<()>;
    fn statfs(&self) -> VfsResult<FsStats>;
}
```

## VNode Types

```rust
pub enum VNodeType {
    File,           // 일반 파일
    Directory,      // 디렉토리
    Symlink,        // 심볼릭 링크
    BlockDevice,    // 블록 디바이스
    CharDevice,     // 캐릭터 디바이스
    Fifo,           // 파이프
    Socket,         // 소켓
}
```

## Filesystem Implementations

### RamFS

메모리 기반 파일시스템. 부팅 시 루트 파일시스템으로 사용.

```rust
use crate::fs::ramfs::RamFs;

let ramfs = RamFs::new();
fs::set_root_fs(Arc::new(ramfs));
```

**특징:**
- 휘발성 (재부팅 시 데이터 손실)
- 빠른 접근 속도
- 동적 크기 조절

### DevFS

장치 파일시스템. `/dev` 아래에 마운트.

```rust
use crate::fs::devfs::DevFs;

let devfs = DevFs::new();
fs::mount("/dev", Arc::new(devfs))?;
```

**장치 파일:**
- `/dev/null` - 모든 입력을 버림
- `/dev/zero` - 무한한 0 바이트 제공
- `/dev/console` - 콘솔 디바이스

### FAT32

FAT32 파일시스템. 블록 디바이스에서 마운트.

```rust
use crate::fs::fat32::Fat32;
use crate::block::get_device;

if let Some(device) = get_device("vda") {
    let fat32 = Fat32::mount(device)?;
    fs::mount("/mnt", Arc::new(fat32))?;
}
```

**특징:**
- 영구 저장소 지원
- 호환성 높음
- 읽기/쓰기 지원

## Mount System

```rust
// 루트 파일시스템 설정
fs::set_root_fs(Arc::new(ramfs));

// 추가 파일시스템 마운트
fs::mount("/dev", Arc::new(devfs))?;
fs::mount("/mnt", Arc::new(fat32))?;

// 마운트 해제
fs::unmount("/mnt")?;

// 마운트 목록 조회
let mounts = fs::list_mounts();
```

## Path Resolution

`fs/path.rs`에서 경로 파싱 및 정규화 처리.

```rust
use crate::fs::path;

// 경로 정규화
let normalized = path::normalize("/foo/../bar/./baz")?;
// 결과: "/bar/baz"

// VNode 검색
let node = fs::lookup_path("/etc/config")?;
```

## File Descriptors

`fs/fd.rs`에서 파일 디스크립터 테이블 관리.

```rust
// 파일 열기
let fd = fd::open("/hello.txt", OpenFlags::RDONLY)?;

// 읽기
let mut buf = [0u8; 1024];
let n = fd::read(fd, &mut buf)?;

// 닫기
fd::close(fd)?;
```

## Error Handling

```rust
pub enum VfsError {
    NotFound,           // 파일 없음
    PermissionDenied,   // 권한 없음
    AlreadyExists,      // 이미 존재
    NotADirectory,      // 디렉토리 아님
    IsADirectory,       // 디렉토리임
    InvalidPath,        // 잘못된 경로
    IoError,            // I/O 에러
    NoSpace,            // 공간 부족
    ReadOnly,           // 읽기 전용
    // ...
}
```

## Adding a New Filesystem

1. `src/fs/` 하위에 새 모듈 생성
2. `VNode` trait 구현
3. `FileSystem` trait 구현
4. `fs/mod.rs`에 모듈 추가
5. `docs/vfs.md` 문서 업데이트
