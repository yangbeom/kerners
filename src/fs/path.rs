//! 경로 처리 유틸리티

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::{VfsError, VfsResult, VNode};

/// 경로 정규화
///
/// - 연속 슬래시 제거
/// - . 및 .. 처리
/// - 절대 경로로 변환
pub fn normalize(path: &str) -> VfsResult<String> {
    if path.is_empty() {
        return Err(VfsError::InvalidPath);
    }

    // 절대 경로가 아니면 에러 (현재는 절대 경로만 지원)
    if !path.starts_with('/') {
        return Err(VfsError::InvalidPath);
    }

    let mut components: Vec<&str> = Vec::new();

    for component in path.split('/') {
        match component {
            "" | "." => continue,
            ".." => {
                components.pop();
            }
            name => {
                components.push(name);
            }
        }
    }

    if components.is_empty() {
        Ok(String::from("/"))
    } else {
        let mut result = String::new();
        for comp in components {
            result.push('/');
            result.push_str(comp);
        }
        Ok(result)
    }
}

/// 경로 분리 (디렉토리, 파일명)
pub fn split(path: &str) -> (&str, &str) {
    if let Some(pos) = path.rfind('/') {
        if pos == 0 {
            ("/", &path[1..])
        } else {
            (&path[..pos], &path[pos + 1..])
        }
    } else {
        (".", path)
    }
}

/// 파일명 추출
pub fn basename(path: &str) -> &str {
    split(path).1
}

/// 디렉토리 경로 추출
pub fn dirname(path: &str) -> &str {
    split(path).0
}

/// 경로 결합
pub fn join(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", base, name)
    }
}

/// 경로 해석 (VNode 순회)
///
/// 루트 VNode에서 시작하여 경로를 따라 순회
pub fn resolve(root: &Arc<dyn VNode>, path: &str) -> VfsResult<Arc<dyn VNode>> {
    if path.is_empty() || path == "/" {
        return Ok(root.clone());
    }

    let normalized = normalize(path)?;
    let mut current = root.clone();

    for component in normalized.split('/') {
        if component.is_empty() {
            continue;
        }

        current = current.lookup(component)?;
    }

    Ok(current)
}

/// 부모 디렉토리 VNode 및 마지막 컴포넌트 반환
pub fn resolve_parent(root: &Arc<dyn VNode>, path: &str) -> VfsResult<(Arc<dyn VNode>, String)> {
    let normalized = normalize(path)?;
    let (parent_path, name) = split(&normalized);

    if name.is_empty() {
        return Err(VfsError::InvalidPath);
    }

    let parent = resolve(root, parent_path)?;

    Ok((parent, String::from(name)))
}

/// 경로가 절대 경로인지 확인
pub fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
}

/// 확장자 추출
pub fn extension(path: &str) -> Option<&str> {
    let name = basename(path);
    if let Some(pos) = name.rfind('.') {
        if pos > 0 {
            return Some(&name[pos + 1..]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_normalize() {
        assert_eq!(normalize("/").unwrap(), "/");
        assert_eq!(normalize("/a/b/c").unwrap(), "/a/b/c");
        assert_eq!(normalize("/a//b///c").unwrap(), "/a/b/c");
        assert_eq!(normalize("/a/./b/./c").unwrap(), "/a/b/c");
        assert_eq!(normalize("/a/b/../c").unwrap(), "/a/c");
        assert_eq!(normalize("/a/b/c/..").unwrap(), "/a/b");
        assert_eq!(normalize("/a/b/c/../..").unwrap(), "/a");
    }

    fn test_split() {
        assert_eq!(split("/a/b/c"), ("/a/b", "c"));
        assert_eq!(split("/a"), ("/", "a"));
        assert_eq!(split("a"), (".", "a"));
    }

    fn test_join() {
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(join("/a/b", "c"), "/a/b/c");
    }
}
