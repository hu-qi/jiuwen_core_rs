//! # ah-plugins-bridge-compose
//!
//! 真实 bridge avatar 文本组装(1:1 对齐
//! `agent_teams/agent/bridge_inbound_compose.py` + `bridge_outbound_wrap.py`):
//! - `compose_bridge_inbound`:入站消息 + 远程执行结果 → bridge LLM 上下文;
//! - `wrap_outbound_to_remote`:PASSTHROUGH / REPHRASE 两种出站格式。
//!
//! 纯函数、无 IO、无 LLM;委托契约层实现。

use std::sync::Arc;

use ah_contracts::bridge_compose::{
    BridgeCompose, BridgeMailboxInjectMode, TeamRole, compose_bridge_inbound,
    wrap_outbound_to_remote,
};
use ah_contracts::keys::BRIDGE_COMPOSE;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实实现:委托契约层纯函数。
pub struct BridgeComposeImpl;

impl Seam for BridgeComposeImpl {}

impl BridgeCompose for BridgeComposeImpl {
    fn compose_bridge_inbound(
        &self,
        original_sender: &str,
        original_body: &str,
        remote_reply: &str,
        language: &str,
        time_info: Option<&str>,
    ) -> String {
        compose_bridge_inbound(
            original_sender,
            original_body,
            remote_reply,
            language,
            time_info,
        )
    }

    fn wrap_outbound_to_remote(
        &self,
        sender: &str,
        sender_display_name: Option<&str>,
        sender_role: Option<TeamRole>,
        sender_desc: Option<&str>,
        body: &str,
        broadcast: bool,
        task_hint: Option<&str>,
        mode: BridgeMailboxInjectMode,
        language: &str,
    ) -> String {
        wrap_outbound_to_remote(
            sender,
            sender_display_name,
            sender_role,
            sender_desc,
            body,
            broadcast,
            task_hint,
            mode,
            language,
        )
    }
}

/// bridge-compose 插件:注册 `bridge-compose` seam。
pub struct BridgeComposePlugin;

impl Plugin for BridgeComposePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-bridge-compose"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![BRIDGE_COMPOSE]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let svc: Arc<dyn BridgeCompose> = Arc::new(BridgeComposeImpl);
        Ok(vec![ctx.register(BRIDGE_COMPOSE, svc)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::BRIDGE_COMPOSE;
    use ah_hub::plugin::DynPlugin;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(BridgeComposePlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn seam_registered() {
        let (ctx, effects) = build_ctx();
        assert!(ctx.service::<dyn BridgeCompose>(&BRIDGE_COMPOSE).is_some());
        drop(effects);
    }

    #[test]
    fn compose_via_seam() {
        let (ctx, effects) = build_ctx();
        let svc = ctx
            .service::<dyn BridgeCompose>(&BRIDGE_COMPOSE)
            .expect("svc");
        let inbound = svc.compose_bridge_inbound("alice", "hi", "ok", "cn", None);
        assert!(inbound.contains("[来自团队成员 alice 的消息]"));
        let outbound = svc.wrap_outbound_to_remote(
            "bob",
            None,
            None,
            None,
            "hello",
            false,
            None,
            BridgeMailboxInjectMode::Passthrough,
            "en",
        );
        assert_eq!(outbound, "[from bob] hello");
        drop(effects);
    }
}
