//! # ah-plugins-a2a
//!
//! 真实 A2A(Agent2Agent)协议适配(1:1 对齐
//! `extensions/a2a/{a2a_transformer,a2a_agentcard_adapter,a2a_client,
//! a2a_server}.py` 的确定性部分):
//! - transformer:openjiuwen payload ↔ A2A payload(request/part/artifact/
//!   状态映射);
//! - agent-card 适配:openjiuwen AgentCard ↔ A2A AgentCard(描述拼接 + 接口列表);
//! - 客户端聚合:session id 解析 + AgentResult 跨事件合并;
//! - server 归一化:JSON-RPC 路由/接口 URL 尾斜杠 + 传输协议解析。
//!
//! 纯函数、无 IO、无 LLM;委托契约层实现。HTTP/SSE 传输走 transport seam。

use std::sync::Arc;

use ah_contracts::a2a::{
    A2aError, AgentResultView, merge_agent_results, message_to_payload,
    normalize_jsonrpc_interface_url, normalize_jsonrpc_route_path, resolve_session_id,
    resolve_transport_protocols, to_a2a_agent_card, to_a2a_request, with_session_id,
};
use ah_contracts::keys::A2A;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// A2A 适配 Seam(Service Definition):openjiuwen ↔ A2A 协议面。
pub trait A2AAdapter: Seam {
    /// openjiuwen request dict → A2A SendMessageRequest 形状。
    fn to_a2a_request(&self, request: &Value, message_id: &str) -> Result<Value, A2aError>;

    /// A2A Message → openjiuwen payload dict。
    fn message_to_payload(&self, message: &Value) -> Value;

    /// 解析输入 dict 的 session id(conversation_id 优先,其次 sessionId)。
    fn resolve_session_id(&self, inputs: &Value) -> Option<String>;

    /// A2A Task → AgentResult 视图。
    fn task_to_result(&self, task: &Value) -> AgentResultView;

    /// 跨事件聚合 AgentResult(artifacts 拼接 / metadata 合并 / 状态优先级)。
    fn merge_results(&self, base: &AgentResultView, update: &AgentResultView) -> AgentResultView;

    /// 聚合后回填 session id。
    fn with_session_id(
        &self,
        result: AgentResultView,
        session_id: Option<String>,
    ) -> AgentResultView;

    /// openjiuwen AgentCard → A2A AgentCard 形状。
    #[allow(clippy::too_many_arguments)]
    fn to_a2a_card(
        &self,
        name: &str,
        description: &str,
        input_params: Option<&Value>,
        output_params: Option<&Value>,
        interface_url: Option<&str>,
        protocol_binding: &str,
        protocol_version: &str,
        tenant: Option<&str>,
        supported_interfaces: &[Value],
    ) -> Value;

    /// 归一化 JSON-RPC 路由路径(尾斜杠)。
    fn normalize_route_path(&self, rpc_url: &str) -> String;

    /// 归一化 JSON-RPC 接口 URL(路径尾斜杠)。
    fn normalize_interface_url(&self, interface_url: Option<&str>) -> Option<String>;

    /// 解析传输协议集合(gRPC 显式拒绝)。
    fn resolve_transport_protocols(
        &self,
        supported_interfaces: &[Value],
    ) -> Result<Vec<String>, A2aError>;
}

/// 纯函数适配实现:委托契约层。
pub struct A2AAdapterImpl;

impl Seam for A2AAdapterImpl {}

impl A2AAdapter for A2AAdapterImpl {
    fn to_a2a_request(&self, request: &Value, message_id: &str) -> Result<Value, A2aError> {
        to_a2a_request(request, message_id)
    }

    fn message_to_payload(&self, message: &Value) -> Value {
        message_to_payload(message)
    }

    fn resolve_session_id(&self, inputs: &Value) -> Option<String> {
        resolve_session_id(inputs)
    }

    fn task_to_result(&self, task: &Value) -> AgentResultView {
        ah_contracts::a2a::a2a_task_to_result(Some(task))
    }

    fn merge_results(&self, base: &AgentResultView, update: &AgentResultView) -> AgentResultView {
        merge_agent_results(base, update)
    }

    fn with_session_id(
        &self,
        result: AgentResultView,
        session_id: Option<String>,
    ) -> AgentResultView {
        with_session_id(result, session_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn to_a2a_card(
        &self,
        name: &str,
        description: &str,
        input_params: Option<&Value>,
        output_params: Option<&Value>,
        interface_url: Option<&str>,
        protocol_binding: &str,
        protocol_version: &str,
        tenant: Option<&str>,
        supported_interfaces: &[Value],
    ) -> Value {
        to_a2a_agent_card(
            name,
            description,
            input_params,
            output_params,
            interface_url,
            protocol_binding,
            protocol_version,
            tenant,
            supported_interfaces,
        )
    }

    fn normalize_route_path(&self, rpc_url: &str) -> String {
        normalize_jsonrpc_route_path(rpc_url)
    }

    fn normalize_interface_url(&self, interface_url: Option<&str>) -> Option<String> {
        normalize_jsonrpc_interface_url(interface_url)
    }

    fn resolve_transport_protocols(
        &self,
        supported_interfaces: &[Value],
    ) -> Result<Vec<String>, A2aError> {
        resolve_transport_protocols(supported_interfaces)
    }
}

/// a2a 插件:注册 `a2a` seam(纯函数适配)。
pub struct A2APlugin;

impl Plugin for A2APlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-a2a"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![A2A]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let adapter: Arc<dyn A2AAdapter> = Arc::new(A2AAdapterImpl);
        Ok(vec![ctx.register(A2A, adapter)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::a2a::{ArtifactView, build_agent_result};
    use ah_contracts::controller::TaskStatus;
    use ah_contracts::keys::A2A;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(A2APlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn seam_registered() {
        let (ctx, effects) = build_ctx();
        assert!(ctx.service::<dyn A2AAdapter>(&A2A).is_some());
        drop(effects);
    }

    #[test]
    fn request_and_payload_roundtrip_via_seam() {
        let (ctx, effects) = build_ctx();
        let adapter = ctx.service::<dyn A2AAdapter>(&A2A).expect("a2a");
        let req = json!({"query": "hi", "sessionId": "s1", "tag": "x"});
        let a2a_req = adapter.to_a2a_request(&req, "mid").expect("ok");
        assert_eq!(a2a_req["message"]["context_id"], "s1");
        assert_eq!(a2a_req["message"]["metadata"]["tag"], "x");
        // A2A Message → payload。
        let payload = adapter.message_to_payload(&a2a_req["message"]);
        assert_eq!(payload["query"], "hi");
        assert_eq!(payload["sessionId"], "s1");
        assert_eq!(payload["tag"], "x");
        assert_eq!(
            adapter.resolve_session_id(&json!({"conversation_id": "a"})),
            Some("a".to_string())
        );
        drop(effects);
    }

    #[test]
    fn task_to_result_and_merge_via_seam() {
        let (ctx, effects) = build_ctx();
        let adapter = ctx.service::<dyn A2AAdapter>(&A2A).expect("a2a");
        let task = json!({
            "id": "t1",
            "status": {"state": "TASK_STATE_WORKING"},
            "artifacts": [{"artifact_id": "a1", "parts": [{"text": "x"}]}]
        });
        let result = adapter.task_to_result(&task);
        assert_eq!(result.status, Some(TaskStatus::Working));
        assert_eq!(result.artifacts.len(), 1);

        let update = build_agent_result(
            Some("t1".to_string()),
            None,
            Some(TaskStatus::Completed),
            vec![],
            Default::default(),
        );
        let merged = adapter.merge_results(&result, &update);
        assert_eq!(merged.status, Some(TaskStatus::Completed));
        let with_session = adapter.with_session_id(merged, Some("s9".to_string()));
        assert_eq!(with_session.session_id.as_deref(), Some("s9"));
        drop(effects);
    }

    #[test]
    fn card_and_normalization_via_seam() {
        let (ctx, effects) = build_ctx();
        let adapter = ctx.service::<dyn A2AAdapter>(&A2A).expect("a2a");
        let card = adapter.to_a2a_card(
            "agent",
            "desc",
            None,
            None,
            Some("http://h/a2a/jsonrpc"),
            "JSONRPC",
            "1.0",
            None,
            &[],
        );
        assert_eq!(card["name"], "agent");
        assert_eq!(card["capabilities"]["streaming"], true);
        assert_eq!(
            card["supported_interfaces"][0]["url"],
            "http://h/a2a/jsonrpc"
        );
        assert_eq!(adapter.normalize_route_path("a2a/jsonrpc"), "/a2a/jsonrpc/");
        assert_eq!(
            adapter.normalize_interface_url(Some("http://h/a2a/jsonrpc")),
            Some("http://h/a2a/jsonrpc/".to_string())
        );
        let protocols = adapter
            .resolve_transport_protocols(&[json!({"protocol_binding": "JSONRPC"})])
            .expect("ok");
        assert_eq!(protocols, vec!["JSONRPC".to_string()]);
        let _ = ArtifactView {
            artifact_id: None,
            name: None,
            description: None,
            parts: vec![],
            metadata: Default::default(),
        };
        drop(effects);
    }
}
