//! # ah-plugins-worktree
//!
//! 真实团队 worktree 确定性部分(1:1 对齐 `openjiuwen/agent_teams/worktree/`
//! 的 naming + member_state 纯逻辑):
//! - naming:`build_teammate_worktree_name` / `validate_slug`(委托契约层纯函数);
//! - member_state:`matches_scope` / `info_from_options`(DB 元数据归属判定)。
//!
//! git 生命周期(create/remove/贡献分类)依赖真实 git worktree 子进程,由
//! ah-plugins-git 的扩展提供,不在本 crate 硬编码。

use std::sync::Arc;

use ah_contracts::keys::{WORKTREE_MEMBER_STATE, WORKTREE_NAMING};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::worktree::{
    MemberWorktreeInfo, WorktreeError, WorktreeMemberState, WorktreeNaming, WorktreeOwnerScope,
    build_teammate_worktree_name, info_from_options, validate_slug,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实命名实现:委托契约层纯函数。
pub struct WorktreeNamingImpl;

impl Seam for WorktreeNamingImpl {}

impl WorktreeNaming for WorktreeNamingImpl {
    fn build_teammate_worktree_name(
        &self,
        team_name: &str,
        member_name: &str,
        session_id: &str,
        project_hash: &str,
    ) -> Result<String, WorktreeError> {
        build_teammate_worktree_name(team_name, member_name, session_id, project_hash)
    }

    fn validate_slug(&self, slug: &str) -> Result<(), WorktreeError> {
        validate_slug(slug)
    }
}

/// 真实成员状态实现:委托契约层纯函数。
pub struct WorktreeMemberStateImpl;

impl Seam for WorktreeMemberStateImpl {}

impl WorktreeMemberState for WorktreeMemberStateImpl {
    fn matches_scope(&self, info: &MemberWorktreeInfo, scope: &WorktreeOwnerScope) -> bool {
        info.session_id.as_deref() == Some(scope.session_id.as_str())
            && info.project_hash.as_deref() == Some(scope.project_hash.as_str())
            && info.managed_root.as_deref() == Some(scope.managed_root.as_str())
    }

    fn info_from_options(
        &self,
        worktree_path: &str,
        worktree_branch: Option<&str>,
        head_commit: Option<&str>,
        session_id: Option<&str>,
        project_hash: Option<&str>,
        managed_root: Option<&str>,
        scope: &WorktreeOwnerScope,
    ) -> Option<MemberWorktreeInfo> {
        info_from_options(
            worktree_path,
            worktree_branch,
            head_commit,
            session_id,
            project_hash,
            managed_root,
            scope,
        )
    }
}

/// worktree 插件:注册命名 + 成员状态两个 seam。
pub struct WorktreePlugin;

impl Plugin for WorktreePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-worktree"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![WORKTREE_NAMING, WORKTREE_MEMBER_STATE]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let naming: Arc<dyn WorktreeNaming> = Arc::new(WorktreeNamingImpl);
        let state: Arc<dyn WorktreeMemberState> = Arc::new(WorktreeMemberStateImpl);
        Ok(vec![
            ctx.register(WORKTREE_NAMING, naming),
            ctx.register(WORKTREE_MEMBER_STATE, state),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::worktree::WorktreeOwnerScope;
    use ah_hub::plugin::DynPlugin;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(WorktreePlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn sample_scope() -> WorktreeOwnerScope {
        WorktreeOwnerScope {
            team_name: "TeamA".to_string(),
            member_name: "Bob".to_string(),
            session_id: "sess-1".to_string(),
            project_dir: "/tmp/proj".to_string(),
            project_hash: "abc123".to_string(),
            managed_root: "/tmp/teams/TeamA/sessions/sess-1/worktrees".to_string(),
            worktree_name: "agent-team-a-bob-xyz".to_string(),
        }
    }

    #[test]
    fn naming_seam_builds_deterministic_slug() {
        let (ctx, effects) = build_ctx();
        let naming = ctx
            .service::<dyn WorktreeNaming>(&WORKTREE_NAMING)
            .expect("naming");
        let a = naming
            .build_teammate_worktree_name("TeamA", "Bob", "sess-1", "abc123")
            .expect("build");
        let b = naming
            .build_teammate_worktree_name("TeamA", "Bob", "sess-1", "abc123")
            .expect("build");
        assert_eq!(a, b);
        assert!(a.starts_with("agent-teama-bob-"));
        drop(effects);
    }

    #[test]
    fn naming_seam_rejects_bad_slug() {
        let (ctx, effects) = build_ctx();
        let naming = ctx
            .service::<dyn WorktreeNaming>(&WORKTREE_NAMING)
            .expect("naming");
        assert!(naming.validate_slug("agent-team-a-bob-abc123def0").is_ok());
        assert!(naming.validate_slug("..").is_err());
        drop(effects);
    }

    #[test]
    fn member_state_seam_matches_and_builds() {
        let (ctx, effects) = build_ctx();
        let state = ctx
            .service::<dyn WorktreeMemberState>(&WORKTREE_MEMBER_STATE)
            .expect("state");
        let scope = sample_scope();
        let info = state
            .info_from_options(
                "/tmp/wt",
                Some("feature/x"),
                Some("deadbeef"),
                Some("sess-1"),
                Some("abc123"),
                Some("/tmp/teams/TeamA/sessions/sess-1/worktrees"),
                &scope,
            )
            .expect("info");
        assert_eq!(info.worktree_name, "agent-team-a-bob-xyz");
        assert!(state.matches_scope(&info, &scope));
        let mut foreign = info.clone();
        foreign.session_id = Some("sess-2".to_string());
        assert!(!state.matches_scope(&foreign, &scope));
        drop(effects);
    }
}
