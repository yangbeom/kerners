//! 드라이버 프레임워크
//!
//! DTB 기반 디바이스 탐색 및 드라이버 매칭

pub mod config;
pub mod probe;

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::dtb::DeviceInfo;
use crate::sync::RwLock;

/// 드라이버 에러
#[derive(Debug, Clone, Copy)]
pub enum DriverError {
    /// 디바이스를 찾을 수 없음
    DeviceNotFound,
    /// 초기화 실패
    InitFailed,
    /// 이미 초기화됨
    AlreadyInitialized,
    /// 지원하지 않는 디바이스
    NotSupported,
    /// 리소스 부족
    OutOfResources,
}

/// 드라이버 결과 타입
pub type DriverResult<T> = Result<T, DriverError>;

/// 드라이버 trait
///
/// 각 드라이버는 이 trait을 구현하여 DTB 기반 자동 탐색/초기화 지원
pub trait Driver: Send + Sync {
    /// 드라이버 이름
    fn name(&self) -> &str;

    /// 지원하는 compatible 문자열들
    fn compatible(&self) -> &[&str];

    /// 디바이스 탐색 및 초기화
    ///
    /// DTB에서 찾은 디바이스 정보로 드라이버 초기화
    fn probe(&self, info: &DeviceInfo) -> DriverResult<()>;
}

/// 등록된 드라이버 정보
struct RegisteredDriver {
    driver: Arc<dyn Driver>,
    probed: bool,
}

/// 드라이버 레지스트리
struct DriverRegistry {
    drivers: Vec<RegisteredDriver>,
}

impl DriverRegistry {
    const fn new() -> Self {
        Self {
            drivers: Vec::new(),
        }
    }
}

/// 전역 드라이버 레지스트리
static DRIVER_REGISTRY: RwLock<DriverRegistry> = RwLock::new(DriverRegistry::new());

/// 드라이버 등록
pub fn register_driver(driver: Arc<dyn Driver>) {
    let mut registry = DRIVER_REGISTRY.write();
    crate::kprintln!("[drivers] Registering driver: {}", driver.name());
    registry.drivers.push(RegisteredDriver {
        driver,
        probed: false,
    });
}

/// 모든 드라이버 probe 실행
///
/// DTB를 순회하며 등록된 드라이버와 매칭되는 디바이스를 찾아 초기화
pub fn probe_all() {
    let dt = match crate::dtb::get() {
        Some(dt) => dt,
        None => {
            crate::kprintln!("[drivers] Warning: DTB not available, skipping probe");
            return;
        }
    };

    crate::kprintln!("[drivers] Starting device probe...");

    let mut registry = DRIVER_REGISTRY.write();

    for reg_driver in registry.drivers.iter_mut() {
        if reg_driver.probed {
            continue;
        }

        let driver = &reg_driver.driver;
        let compatible_list = driver.compatible();

        for compat in compatible_list {
            let devices = dt.find_compatible(compat);

            for info in devices {
                crate::kprintln!(
                    "[drivers] Found {} for driver {} (compatible: {})",
                    info.name,
                    driver.name(),
                    compat
                );

                match driver.probe(&info) {
                    Ok(()) => {
                        crate::kprintln!(
                            "[drivers] {} probed successfully @ {:#x}",
                            driver.name(),
                            info.reg_base
                        );
                        reg_driver.probed = true;
                    }
                    Err(e) => {
                        crate::kprintln!(
                            "[drivers] {} probe failed: {:?}",
                            driver.name(),
                            e
                        );
                    }
                }
            }
        }
    }

    crate::kprintln!("[drivers] Device probe complete");
}

/// 등록된 드라이버 목록 출력
pub fn list_drivers() {
    let registry = DRIVER_REGISTRY.read();
    crate::kprintln!("[drivers] Registered drivers:");
    for reg_driver in registry.drivers.iter() {
        let status = if reg_driver.probed { "probed" } else { "pending" };
        crate::kprintln!("  - {} [{}]", reg_driver.driver.name(), status);
    }
}

/// 특정 compatible로 디바이스 정보 찾기
pub fn find_device(compatible: &str) -> Option<DeviceInfo> {
    let dt = crate::dtb::get()?;
    let devices = dt.find_compatible(compatible);
    devices.into_iter().next()
}

/// 모든 compatible로 디바이스 정보들 찾기
pub fn find_devices(compatible: &str) -> Vec<DeviceInfo> {
    match crate::dtb::get() {
        Some(dt) => dt.find_compatible(compatible),
        None => Vec::new(),
    }
}
