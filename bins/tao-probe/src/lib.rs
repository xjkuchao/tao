//! tao-probe 入口库.
//!
//! 提供统一 `run(argv)` 调度, 固定为 `tao-probe` 单入口语义.

pub mod app;
mod cli;
mod compat;
mod core;
mod model;
mod writer;

/// 执行探测命令.
///
/// 返回值为进程退出码.
pub fn run(argv: Vec<String>) -> i32 {
    app::run(argv)
}
