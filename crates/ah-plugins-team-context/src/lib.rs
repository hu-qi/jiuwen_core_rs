//! # ah-plugins-team-context
//!
//! 真实团队会话上下文(1:1 对齐 `openjiuwen/agent_teams/context.py`):
//! - `set_session_id` / `get_session_id` / `reset_session_id`:session_id
//!   隔离原语(token 可逆);
//! - `LOG_DEFAULT_TRACE_ID` 占位语义。
//!
//! Rust 侧以进程内 `Mutex<Option<String>>` 存储当前 session_id(单执行上下文
//! 语义,与 Python contextvar 的单线程模型一致);token 携带前值用于 reset。

use std::sync::Mutex;

use ah_contracts::keys::TEAM_CONTEXT;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_context::{SessionToken, TeamContextError, TeamSessionContext};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实会话上下文实现。
pub struct TeamSessionContextImpl {
    current: Mutex<Option<String>>,
}

impl Default for TeamSessionContextImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamSessionContextImpl {
    /// 构造空上下文。
    pub fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }
}

impl Seam for TeamSessionContextImpl {}

impl TeamSessionContext for TeamSessionContextImpl {
    fn set_session_id(&self, session_id: &str) -> Result<SessionToken, TeamContextError> {
        let mut current = self.current.lock().expect("ctx lock");
        let previous = current.clone();
        *current = Some(session_id.to_string());
        Ok(SessionToken::new(previous))
    }

    fn get_session_id(&self) -> String {
        self.current
            .lock()
            .expect("ctx lock")
            .clone()
            .unwrap_or_default()
    }

    fn reset_session_id(&self, token: SessionToken) -> Result<(), TeamContextError> {
        let mut current = self.current.lock().expect("ctx lock");
        // reset 恢复前值;无前值 → 空(占位语义)。
        *current = token.previous.clone();
        Ok(())
    }
}

/// team-context 插件:注册 `team-context` seam。
pub struct TeamContextPlugin;

impl Plugin for TeamContextPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-context"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_CONTEXT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let service: std::sync::Arc<dyn TeamSessionContext> =
            std::sync::Arc::new(TeamSessionContextImpl::new());
        Ok(vec![ctx.register(TEAM_CONTEXT, service)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_CONTEXT;
    use ah_hub::plugin::DynPlugin;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![std::sync::Arc::new(TeamContextPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn seam_registered() {
        let (ctx, effects) = build_ctx();
        assert!(
            ctx.service::<dyn TeamSessionContext>(&TEAM_CONTEXT)
                .is_some()
        );
        drop(effects);
    }

    #[test]
    fn set_get_reset_roundtrip() {
        let (ctx, effects) = build_ctx();
        let svc = ctx
            .service::<dyn TeamSessionContext>(&TEAM_CONTEXT)
            .expect("ctx");
        // 初始为空。
        assert_eq!(svc.get_session_id(), "");
        // 设置。
        let token = svc.set_session_id("sess-1").expect("set");
        assert_eq!(svc.get_session_id(), "sess-1");
        // 覆盖设置(前值 sess-1)。
        let token2 = svc.set_session_id("sess-2").expect("set");
        assert_eq!(svc.get_session_id(), "sess-2");
        // reset 到 token2 前值。
        svc.reset_session_id(token2).expect("reset");
        assert_eq!(svc.get_session_id(), "sess-1");
        // reset 到 token 前值(空)。
        svc.reset_session_id(token).expect("reset");
        assert_eq!(svc.get_session_id(), "");
        drop(effects);
    }

    #[test]
    fn default_trace_id_placeholder_matches() {
        assert_eq!(
            ah_contracts::team_context::LOG_DEFAULT_TRACE_ID,
            "default_trace_id"
        );
    }
}
