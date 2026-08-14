//! 事件契约。

/// 事件契约:所有可分发事件都必须实现本 trait。
///
/// 事件是插件的扩展点:监听方注册在事件类型上,发布方按类型分发。
/// 要求 `Clone`:分发器按监听器数量克隆事件(对齐 DSH 的 emit/waterfall 语义)。
pub trait Event: Send + Sync + Clone + 'static {
    /// 稳定的事件标识,用于诊断、日志与配置。
    const ID: &'static str;
}
