//! # ah-plugins-team-join-descriptor
//!
//! 真实的外部团队加入描述符实现(对齐 openjiuwen/agent_teams/external/descriptor.py):
//! `TeamJoinDescriptor` 的 JSON 序列化 / 环境变量打包 / 解析与必填字段校验。
//! 纯数据转换,无 IO、无状态;非法输入一律显式 `Err`,无静默 fallback。

use std::collections::BTreeMap;
use std::sync::Arc;

use ah_contracts::keys::TEAM_JOIN_DESCRIPTOR;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_join_descriptor::{
    DescriptorError, TEAM_JOIN_ENV, TeamJoinDescriptor, TeamJoinDescriptorApi,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实描述符编解码实现(纯函数,无 IO/状态)。
pub struct JoinDescriptorCodec;

impl Seam for JoinDescriptorCodec {}

impl TeamJoinDescriptorApi for JoinDescriptorCodec {
    fn to_json(&self, d: &TeamJoinDescriptor) -> String {
        serde_json::to_string(d)
            .expect("TeamJoinDescriptor serialization is total: all fields are JSON-safe")
    }

    fn to_env(&self, d: &TeamJoinDescriptor) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert(TEAM_JOIN_ENV.to_string(), self.to_json(d));
        env
    }

    fn from_json(&self, raw: &str) -> Result<TeamJoinDescriptor, DescriptorError> {
        let value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| DescriptorError(format!("malformed team join descriptor: {e}")))?;
        // 必填字段校验:session_id / team_name / member_name 必须为非空字符串。
        for field in ["session_id", "team_name", "member_name"] {
            let ok = matches!(
                value.get(field).and_then(serde_json::Value::as_str),
                Some(s) if !s.trim().is_empty()
            );
            if !ok {
                return Err(DescriptorError(format!(
                    "team join descriptor missing required field `{field}`"
                )));
            }
        }
        serde_json::from_value(value)
            .map_err(|e| DescriptorError(format!("invalid team join descriptor: {e}")))
    }

    fn from_env(
        &self,
        env: Option<&BTreeMap<String, String>>,
    ) -> Result<TeamJoinDescriptor, DescriptorError> {
        let raw = match env {
            Some(map) => map.get(TEAM_JOIN_ENV).cloned(),
            None => std::env::var(TEAM_JOIN_ENV).ok(),
        };
        let raw = raw.ok_or_else(|| {
            DescriptorError(format!(
                "environment variable {TEAM_JOIN_ENV} is not set; cannot join team"
            ))
        })?;
        self.from_json(&raw)
    }
}

/// team-join-descriptor 插件:注册 `team-join-descriptor` seam。
pub struct TeamJoinDescriptorPlugin;

impl Plugin for TeamJoinDescriptorPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-join-descriptor"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_JOIN_DESCRIPTOR]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let api: Arc<dyn TeamJoinDescriptorApi> = Arc::new(JoinDescriptorCodec);
        Ok(vec![ctx.register(TEAM_JOIN_DESCRIPTOR, api)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::team_join_descriptor::{
        DispatchMode, JoinDbConfig, JoinTransportConfig, Scope, TeammateMode,
    };
    use ah_hub::plugin::DynPlugin;

    fn codec() -> JoinDescriptorCodec {
        JoinDescriptorCodec
    }

    fn full_descriptor() -> TeamJoinDescriptor {
        TeamJoinDescriptor {
            session_id: "sess-7".to_string(),
            team_name: "alpha-team".to_string(),
            member_name: "builder-01".to_string(),
            role: "leader".to_string(),
            scope: Scope::Member,
            language: "en".to_string(),
            dispatch_mode: DispatchMode::Scheduled,
            teammate_mode: TeammateMode::PlanMode,
            db_config: JoinDbConfig {
                connection_string: "sqlite:///tmp/alpha.db".to_string(),
            },
            transport_config: JoinTransportConfig {
                external_publish_url: "wss://gw.example.com/events".to_string(),
            },
            workspace_path: Some("/srv/workspace/alpha".to_string()),
        }
    }

    #[test]
    fn roundtrip_full_fields() {
        let c = codec();
        let d = full_descriptor();
        let json = c.to_json(&d);
        // 枚举按 snake_case 序列化。
        assert!(json.contains("\"scope\":\"member\""), "json: {json}");
        assert!(
            json.contains("\"dispatch_mode\":\"scheduled\""),
            "json: {json}"
        );
        assert!(
            json.contains("\"teammate_mode\":\"plan_mode\""),
            "json: {json}"
        );
        assert!(
            json.contains("\"workspace_path\":\"/srv/workspace/alpha\""),
            "json: {json}"
        );
        assert_eq!(c.from_json(&json).expect("valid full descriptor"), d);
    }

    #[test]
    fn roundtrip_minimal_applies_defaults() {
        let c = codec();
        let d = c
            .from_json(r#"{"session_id":"s1","team_name":"t1","member_name":"m1"}"#)
            .expect("minimal descriptor");
        assert_eq!(d.role, "teammate");
        assert_eq!(d.scope, Scope::Operator);
        assert_eq!(d.language, "cn");
        assert_eq!(d.dispatch_mode, DispatchMode::Autonomous);
        assert_eq!(d.teammate_mode, TeammateMode::BuildMode);
        assert_eq!(d.db_config, JoinDbConfig::default());
        assert_eq!(d.transport_config, JoinTransportConfig::default());
        assert_eq!(d.workspace_path, None);
        // 补默认后再序列化,往返结果稳定。
        let json = c.to_json(&d);
        assert_eq!(c.from_json(&json).expect("re-roundtrip"), d);
    }

    #[test]
    fn missing_required_field_errors() {
        let c = codec();
        let err = c
            .from_json(r#"{"session_id":"s1","team_name":"t1"}"#)
            .expect_err("member_name missing");
        assert!(err.0.contains("member_name"), "err: {}", err.0);
        let err = c
            .from_json(r#"{"team_name":"t1","member_name":"m1"}"#)
            .expect_err("session_id missing");
        assert!(err.0.contains("session_id"), "err: {}", err.0);
    }

    #[test]
    fn empty_required_field_errors() {
        let c = codec();
        let err = c
            .from_json(r#"{"session_id":"s1","team_name":"","member_name":"m1"}"#)
            .expect_err("team_name empty");
        assert!(err.0.contains("team_name"), "err: {}", err.0);
    }

    #[test]
    fn invalid_scope_value_errors() {
        let c = codec();
        let err = c
            .from_json(
                r#"{"session_id":"s1","team_name":"t1","member_name":"m1","scope":"superuser"}"#,
            )
            .expect_err("invalid scope");
        assert!(
            err.0.contains("invalid team join descriptor"),
            "err: {}",
            err.0
        );
        assert!(
            err.0.contains("unknown variant `superuser`"),
            "err: {}",
            err.0
        );
    }

    #[test]
    fn invalid_dispatch_mode_errors() {
        let c = codec();
        let err = c
            .from_json(
                r#"{"session_id":"s1","team_name":"t1","member_name":"m1","dispatch_mode":"burst"}"#,
            )
            .expect_err("invalid dispatch_mode");
        assert!(
            err.0.contains("invalid team join descriptor"),
            "err: {}",
            err.0
        );
        assert!(err.0.contains("unknown variant `burst`"), "err: {}", err.0);
    }

    #[test]
    fn invalid_teammate_mode_errors() {
        let c = codec();
        let err = c
            .from_json(
                r#"{"session_id":"s1","team_name":"t1","member_name":"m1","teammate_mode":"chaos"}"#,
            )
            .expect_err("invalid teammate_mode");
        assert!(
            err.0.contains("invalid team join descriptor"),
            "err: {}",
            err.0
        );
        assert!(err.0.contains("unknown variant `chaos`"), "err: {}", err.0);
    }

    #[test]
    fn invalid_json_errors() {
        let c = codec();
        let err = c.from_json("{not json").expect_err("not json");
        assert!(
            err.0.contains("malformed team join descriptor"),
            "err: {}",
            err.0
        );
        let err = c
            .from_json(r#"{"session_id":42,"team_name":"t1","member_name":"m1"}"#)
            .expect_err("wrong field type");
        assert!(err.0.contains("session_id"), "err: {}", err.0);
    }

    #[test]
    fn to_env_single_key() {
        let c = codec();
        let d = full_descriptor();
        let env = c.to_env(&d);
        assert_eq!(env.len(), 1);
        let json = env.get(TEAM_JOIN_ENV).expect("env key present");
        assert_eq!(c.from_json(json).expect("parse env value"), d);
    }

    #[test]
    fn from_env_ok() {
        let c = codec();
        let d = full_descriptor();
        let mut env = BTreeMap::new();
        env.insert(TEAM_JOIN_ENV.to_string(), c.to_json(&d));
        assert_eq!(c.from_env(Some(&env)).expect("valid env"), d);
    }

    #[test]
    fn from_env_missing_key_errors() {
        let c = codec();
        let env = BTreeMap::new();
        let err = c.from_env(Some(&env)).expect_err("missing env key");
        assert_eq!(
            err.0,
            "environment variable OPENJIUWEN_TEAM_JOIN is not set; cannot join team"
        );
    }

    #[test]
    fn from_env_bad_json_errors() {
        let c = codec();
        let mut env = BTreeMap::new();
        env.insert(TEAM_JOIN_ENV.to_string(), "{not json".to_string());
        let err = c.from_env(Some(&env)).expect_err("bad json in env");
        assert!(
            err.0.contains("malformed team join descriptor"),
            "err: {}",
            err.0
        );
    }

    #[test]
    fn workspace_path_none_and_some_roundtrip() {
        let c = codec();
        let none_d = TeamJoinDescriptor {
            workspace_path: None,
            ..full_descriptor()
        };
        let parsed = c.from_json(&c.to_json(&none_d)).expect("none roundtrip");
        assert_eq!(parsed.workspace_path, None);
        let some_d = full_descriptor();
        let parsed = c.from_json(&c.to_json(&some_d)).expect("some roundtrip");
        assert_eq!(
            parsed.workspace_path.as_deref(),
            Some("/srv/workspace/alpha")
        );
    }

    #[test]
    fn plugin_registers_descriptor_seam() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamJoinDescriptorPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let api = ctx
            .service::<dyn TeamJoinDescriptorApi>(&TEAM_JOIN_DESCRIPTOR)
            .expect("team-join-descriptor seam");
        let d = full_descriptor();
        let parsed = api.from_json(&api.to_json(&d)).expect("roundtrip via seam");
        assert_eq!(parsed, d);
        assert!(api.to_env(&d).contains_key(TEAM_JOIN_ENV));
        drop(effects);
        assert!(!ctx.has_service(&TEAM_JOIN_DESCRIPTOR));
    }
}
