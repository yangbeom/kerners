//! 보드 모듈 레지스트리
//!
//! 런타임 보드 감지 및 선택을 위한 레지스트리를 제공합니다.
//! DTB의 compatible 속성과 매칭하여 적절한 보드 모듈을 선택합니다.

use super::board_module::BoardModuleInfo;
use crate::sync::Spinlock;

/// 최대 등록 가능한 보드 수
const MAX_BOARDS: usize = 16;

/// 보드 레지스트리 엔트리
struct BoardEntry {
    /// 보드 모듈 정보
    info: Option<&'static BoardModuleInfo>,
    /// 빌트인 보드 여부
    is_builtin: bool,
}

impl BoardEntry {
    const fn empty() -> Self {
        Self {
            info: None,
            is_builtin: false,
        }
    }
}

/// 보드 레지스트리
struct BoardRegistry {
    /// 등록된 보드 목록
    boards: [BoardEntry; MAX_BOARDS],
    /// 등록된 보드 수
    count: usize,
}

impl BoardRegistry {
    const fn new() -> Self {
        const EMPTY: BoardEntry = BoardEntry::empty();
        Self {
            boards: [EMPTY; MAX_BOARDS],
            count: 0,
        }
    }
}

/// 전역 보드 레지스트리
static REGISTRY: Spinlock<BoardRegistry> = Spinlock::new(BoardRegistry::new());

/// 현재 활성 보드
static ACTIVE_BOARD: Spinlock<Option<&'static BoardModuleInfo>> = Spinlock::new(None);

/// 보드 레지스트리 에러
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardError {
    /// 레지스트리가 가득 참
    RegistryFull,
    /// 보드를 찾을 수 없음
    NotFound,
    /// 이미 등록된 보드
    AlreadyRegistered,
}

/// 빌트인 보드 모듈 등록
///
/// 커널에 컴파일된 보드 모듈을 등록합니다.
pub fn register_builtin_board(info: &'static BoardModuleInfo) -> Result<(), BoardError> {
    let mut registry = REGISTRY.lock();

    // 중복 체크
    for i in 0..registry.count {
        if let Some(existing) = registry.boards[i].info {
            if existing.name == info.name {
                return Err(BoardError::AlreadyRegistered);
            }
        }
    }

    if registry.count >= MAX_BOARDS {
        return Err(BoardError::RegistryFull);
    }

    let idx = registry.count;
    registry.boards[idx] = BoardEntry {
        info: Some(info),
        is_builtin: true,
    };
    registry.count += 1;

    Ok(())
}

/// 외부 모듈에서 로드한 보드 등록
///
/// ELF 모듈에서 로드한 보드를 등록합니다.
#[allow(dead_code)]
pub fn register_board_from_module(info: &'static BoardModuleInfo) -> Result<(), BoardError> {
    let mut registry = REGISTRY.lock();

    // 중복 체크
    let count = registry.count;
    for i in 0..count {
        if let Some(existing) = registry.boards[i].info {
            if existing.name == info.name {
                return Err(BoardError::AlreadyRegistered);
            }
        }
    }

    if count >= MAX_BOARDS {
        return Err(BoardError::RegistryFull);
    }

    registry.boards[count] = BoardEntry {
        info: Some(info),
        is_builtin: false,
    };
    registry.count += 1;

    Ok(())
}

/// 보드 모듈 등록 해제
#[allow(dead_code)]
pub fn unregister_board(name: &str) -> Result<(), BoardError> {
    let mut registry = REGISTRY.lock();

    let count = registry.count;
    for i in 0..count {
        if let Some(info) = registry.boards[i].info {
            if info.name == name {
                // 마지막 엔트리와 교체하여 제거
                let last_idx = count - 1;
                let last_entry = core::mem::replace(&mut registry.boards[last_idx], BoardEntry::empty());
                registry.boards[i] = last_entry;
                registry.count -= 1;
                return Ok(());
            }
        }
    }

    Err(BoardError::NotFound)
}

/// DTB compatible 문자열로 보드 찾기
///
/// 정확히 일치하는 첫 번째 보드를 반환합니다.
pub fn find_board_by_compatible(compat: &str) -> Option<&'static BoardModuleInfo> {
    let registry = REGISTRY.lock();

    for i in 0..registry.count {
        if let Some(info) = registry.boards[i].info {
            if info.matches_compatible(compat) {
                return Some(info);
            }
        }
    }

    None
}

/// DTB compatible 목록으로 가장 적합한 보드 찾기
///
/// compatible 목록의 첫 번째 항목부터 순서대로 매칭을 시도합니다.
/// DTB에서 첫 번째 compatible이 가장 구체적이므로 이 순서를 따릅니다.
pub fn find_best_board_by_compatibles(compats: &[&str]) -> Option<&'static BoardModuleInfo> {
    // 각 compatible에 대해 순서대로 매칭 시도
    for compat in compats {
        if let Some(board) = find_board_by_compatible(compat) {
            return Some(board);
        }
    }

    None
}

/// 현재 활성 보드 가져오기
pub fn active_board() -> Option<&'static BoardModuleInfo> {
    *ACTIVE_BOARD.lock()
}

/// 활성 보드 설정
pub fn set_active_board(info: &'static BoardModuleInfo) {
    *ACTIVE_BOARD.lock() = Some(info);
}

/// 활성 보드 해제
#[allow(dead_code)]
pub fn clear_active_board() {
    *ACTIVE_BOARD.lock() = None;
}

/// 등록된 보드 수
pub fn board_count() -> usize {
    REGISTRY.lock().count
}

/// 등록된 모든 보드 이름 순회
///
/// 콜백 함수에 각 보드의 이름과 활성 여부를 전달합니다.
pub fn for_each_board<F>(mut f: F)
where
    F: FnMut(&str, bool),
{
    let registry = REGISTRY.lock();
    let active = *ACTIVE_BOARD.lock();

    for i in 0..registry.count {
        if let Some(info) = registry.boards[i].info {
            let is_active = active.map_or(false, |a| core::ptr::eq(a, info));
            f(info.name, is_active);
        }
    }
}

/// 등록된 모든 보드 정보 순회
pub fn for_each_board_info<F>(mut f: F)
where
    F: FnMut(&'static BoardModuleInfo, bool),
{
    let registry = REGISTRY.lock();
    let active = *ACTIVE_BOARD.lock();

    for i in 0..registry.count {
        if let Some(info) = registry.boards[i].info {
            let is_active = active.map_or(false, |a| core::ptr::eq(a, info));
            f(info, is_active);
        }
    }
}
