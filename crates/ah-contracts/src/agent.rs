//! agent 域共享类型与事件。

use crate::event::Event;

/// agent 每轮(step)事件:emit 模式,供遥测/日志监听。
///
/// 定义在契约层(跨插件共享):生产者是 agent 循环插件,
/// 消费者是遥测/日志/UI 等任意插件。
#[derive(Clone, Debug)]
pub struct AgentStep {
    /// 第几轮(0 起)。
    pub iteration: usize,
    /// 本轮模型请求的工具调用数。
    pub tool_calls: usize,
    /// 是否已得到最终回答(循环结束)。
    pub done: bool,
}

impl Event for AgentStep {
    const ID: &'static str = "agent/step";
}
