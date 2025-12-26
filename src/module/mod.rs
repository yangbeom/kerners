//! 커널 모듈 시스템
//!
//! ELF64 relocatable object 로딩 및 동적 모듈 관리
//! - ELF64 파서
//! - 심볼 테이블 관리
//! - 재배치 처리
//! - 모듈 라이프사이클

pub mod elf;
pub mod loader;
pub mod symbol;

pub use elf::{Elf64, Elf64Error};
pub use loader::{LoadedModule, Module, ModuleError, ModuleInfo, ModuleLoader, ModuleRef, ModuleState};
pub use symbol::{lookup_symbol, register_symbol, KernelSymbol};
