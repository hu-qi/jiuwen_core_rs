//! # ah-contracts
//!
//! 契约层:只声明接口(Seam trait)与纯类型,零实现。
//!
//! 设计规则(对齐 DSH/Cordis):
//! - 本 crate 不允许出现 Mock / Unsupported / fallback 实现;
//!   生产路径的实现必须来自插件 crate;
//! - 每个 Seam 由三角构成:Service Definition(接口)、Service Provider(实现)、
//!   Consumer(消费方,通常是模型可见工具);
//! - 插件之间不允许直接依赖彼此的具体类型,只允许依赖本 crate 的契约;
//! - 机制类型([`Effect`]、[`ServiceKey`])也定义在此层,供 seam 接口使用。

pub mod effect;
pub mod event;
pub mod fs;
pub mod keys;
pub mod llm;
pub mod seam;
pub mod service;
pub mod shell;
pub mod tools;

pub use effect::Effect;

pub mod prelude {
    pub use crate::effect::Effect;
    pub use crate::event::Event;
    pub use crate::fs::{FsError, FsProvider};
    pub use crate::keys::{AGENT_LOOP, FS, LLM, SHELL, TOOLS};
    pub use crate::llm::{
        ChatMessage, ChatRole, ModelError, ModelProvider, ModelRequest, ModelResponse, ToolCall,
        ToolSchema,
    };
    pub use crate::seam::Seam;
    pub use crate::service::ServiceKey;
    pub use crate::shell::{ShellError, ShellOutput, ShellProvider};
    pub use crate::tools::{Tool, ToolError, ToolRegistry};
}
