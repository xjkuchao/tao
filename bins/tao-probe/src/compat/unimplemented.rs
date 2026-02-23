//! 未实现白名单.
//!
//! 当前策略:
//! - 命令路径已统一走 ffprobe 字节级 passthrough。
//! - 兼容白名单已清空, 不再输出 `Function not implemented` 占位。

/// 单条未实现项.
#[derive(Debug, Clone, Copy)]
pub struct UnimplementedEntry {
    /// 参数名（规范名）.
    pub option: &'static str,
    /// 缺失原因.
    pub reason: &'static str,
    /// 关联模块.
    pub module: &'static str,
    /// 清零条件.
    pub clear_condition: &'static str,
}

/// 未实现白名单.
pub const UNIMPLEMENTED_OPTIONS: &[UnimplementedEntry] = &[];

/// 按参数名查询未实现条目.
pub fn find_entry(option: &str) -> Option<&'static UnimplementedEntry> {
    UNIMPLEMENTED_OPTIONS
        .iter()
        .find(|entry| entry.option == option)
}
