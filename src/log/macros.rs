//! 커널 로그 매크로
//!
//! log_error!, log_warn!, log_info!, log_debug!, log_trace!

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::LogLevel::Error, core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::LogLevel::Warn, core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::LogLevel::Info, core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::LogLevel::Debug, core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::LogLevel::Trace, core::format_args!($($arg)*))
    };
}
