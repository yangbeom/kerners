//! Syscall user-memory access helpers.
//!
//! On riscv64, user pointer dereference in M-mode requires temporary
//! `mstatus` adjustment (MPRV + SUM + MPP=S). Helpers in this module
//! apply that mode per access so syscall code remains safe even when
//! the current thread yields/schedules during a syscall.

use super::errno;
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(target_arch = "riscv64")]
const MSTATUS_MPP_MASK: u64 = 0x1800;
#[cfg(target_arch = "riscv64")]
const MSTATUS_MPP_S: u64 = 0x0800;
#[cfg(target_arch = "riscv64")]
const MSTATUS_MPRV: u64 = 1 << 17;
#[cfg(target_arch = "riscv64")]
const MSTATUS_SUM: u64 = 1 << 18;

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const MIN_USER_VADDR: usize = 0x1000;
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const MAX_USER_VADDR_EXCLUSIVE: usize = crate::proc::user::USER_STACK_BASE;

#[inline]
fn with_user_access_mode<R>(f: impl FnOnce() -> R) -> R {
    #[cfg(target_arch = "riscv64")]
    {
        let mut saved_mstatus: u64 = 0;
        unsafe {
            // SAFETY: syscall/trap path in M-mode에서 현재 mstatus를 저장한다.
            core::arch::asm!(
                "csrr {saved}, mstatus",
                saved = out(reg) saved_mstatus,
                options(nomem, nostack)
            );
        }

        let updated =
            (saved_mstatus & !MSTATUS_MPP_MASK) | MSTATUS_MPP_S | MSTATUS_MPRV | MSTATUS_SUM;
        unsafe {
            // SAFETY: 데이터 접근 모드만 임시로 사용자 주소 변환 허용 상태로 전환한다.
            core::arch::asm!(
                "csrw mstatus, {value}",
                value = in(reg) updated,
                options(nomem, nostack)
            );
        }

        let result = f();

        unsafe {
            // SAFETY: 접근 완료 즉시 원래 mstatus를 복원한다.
            core::arch::asm!(
                "csrw mstatus, {value}",
                value = in(reg) saved_mstatus,
                options(nomem, nostack)
            );
        }

        result
    }

    #[cfg(not(target_arch = "riscv64"))]
    {
        f()
    }
}

#[inline]
pub fn user_pointer_in_range(ptr: usize, len: usize) -> bool {
    if ptr == 0 {
        return false;
    }
    if len == 0 {
        return true;
    }

    let Some(end) = ptr.checked_add(len) else {
        return false;
    };

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        ptr >= MIN_USER_VADDR && end <= MAX_USER_VADDR_EXCLUSIVE
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        ptr >= 0x1000 && end > ptr
    }
}

#[inline]
pub fn validate_user_pointer(ptr: usize, len: usize) -> Result<(), isize> {
    if user_pointer_in_range(ptr, len) {
        Ok(())
    } else {
        Err(errno::EFAULT)
    }
}

#[inline]
pub fn read_unaligned<T: Copy>(ptr: *const T) -> Result<T, isize> {
    validate_user_pointer(ptr as usize, core::mem::size_of::<T>())?;
    Ok(with_user_access_mode(|| unsafe {
        // SAFETY: 사용자 포인터 범위 검증 후 동일 주소를 비정렬 읽기한다.
        core::ptr::read_unaligned(ptr)
    }))
}

#[inline]
pub fn write_unaligned<T: Copy>(ptr: *mut T, value: T) -> Result<(), isize> {
    validate_user_pointer(ptr as usize, core::mem::size_of::<T>())?;
    with_user_access_mode(|| unsafe {
        // SAFETY: 사용자 포인터 범위 검증 후 동일 주소에 비정렬 쓰기를 수행한다.
        core::ptr::write_unaligned(ptr, value);
    });
    Ok(())
}

#[inline]
pub fn read_byte(ptr: *const u8) -> Result<u8, isize> {
    validate_user_pointer(ptr as usize, 1)?;
    Ok(with_user_access_mode(|| unsafe {
        // SAFETY: 1바이트 사용자 포인터 범위를 검증한 뒤 읽는다.
        core::ptr::read(ptr)
    }))
}

#[inline]
pub fn write_byte(ptr: *mut u8, value: u8) -> Result<(), isize> {
    validate_user_pointer(ptr as usize, 1)?;
    with_user_access_mode(|| unsafe {
        // SAFETY: 1바이트 사용자 포인터 범위를 검증한 뒤 기록한다.
        core::ptr::write(ptr, value);
    });
    Ok(())
}

#[inline]
pub fn copy_from_user(dst: &mut [u8], src: *const u8) -> Result<(), isize> {
    if dst.is_empty() {
        return Ok(());
    }
    validate_user_pointer(src as usize, dst.len())?;
    with_user_access_mode(|| unsafe {
        // SAFETY: src/dst 길이를 동일하게 맞추고 src 범위를 검증한 뒤 복사한다.
        core::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), dst.len());
    });
    Ok(())
}

#[inline]
pub fn copy_to_user(dst: *mut u8, src: &[u8]) -> Result<(), isize> {
    if src.is_empty() {
        return Ok(());
    }
    validate_user_pointer(dst as usize, src.len())?;
    with_user_access_mode(|| unsafe {
        // SAFETY: dst/src 길이를 동일하게 맞추고 dst 범위를 검증한 뒤 복사한다.
        core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
    });
    Ok(())
}

pub fn read_c_string(ptr: *const u8, max_len: usize) -> Result<String, isize> {
    if ptr.is_null() {
        return Err(errno::EFAULT);
    }

    let mut len = 0usize;
    loop {
        if len > max_len {
            return Err(errno::E2BIG);
        }

        let byte_addr = match (ptr as usize).checked_add(len) {
            Some(v) => v,
            None => return Err(errno::EFAULT),
        };
        let byte = read_byte(byte_addr as *const u8)?;
        if byte == 0 {
            break;
        }
        len += 1;
    }

    if len == 0 {
        return Ok(String::new());
    }

    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(len).is_err() {
        return Err(errno::ENOMEM);
    }
    bytes.resize(len, 0);
    copy_from_user(&mut bytes, ptr)?;

    let s = core::str::from_utf8(&bytes).map_err(|_| errno::EINVAL)?;
    Ok(String::from(s))
}
