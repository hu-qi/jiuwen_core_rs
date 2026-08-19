//! worktree seam:团队托管 worktree 的确定性命名 / 成员状态判定 / 会话作用域。
//!
//! 对齐 `openjiuwen/agent_teams/worktree/{naming,member_state,session_scope}.py`
//! 的确定性部分:
//! - naming:`build_teammate_worktree_name`(slug 化 + sha256 摘要 + slug 校验);
//! - member_state:`MemberWorktreeInfo` 模型 + `matches_scope` / `info_matches_scope`
//!   / `info_from_options`(DB 元数据与 owner scope 的归属判定);
//! - session_scope:`WorktreeOwnerScope` 模型(team/member/session/project 归属)。
//!
//! git 生命周期(create/remove/贡献分类)依赖真实 git worktree 命令,由插件
//! 通过注入的命令执行器实现;本契约只定义纯类型与判定逻辑。

use crate::seam::Seam;

/// team 段长度上限(对齐 `naming._TEAM_PART_LENGTH = 14`)。
pub const TEAM_PART_LENGTH: usize = 14;
/// member 段长度上限(对齐 `naming._MEMBER_PART_LENGTH = 16`)。
pub const MEMBER_PART_LENGTH: usize = 16;
/// slug 总长度上限(对齐 `slug.MAX_SLUG_LENGTH = 64`)。
pub const MAX_SLUG_LENGTH: usize = 64;

/// worktree 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeError(pub String);

impl core::fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WorktreeError {}

/// 一个被托管 teammate worktree 的解析后 owner 身份。
///
/// 对齐 `session_scope.WorktreeOwnerScope`(frozen dataclass)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorktreeOwnerScope {
    pub team_name: String,
    pub member_name: String,
    pub session_id: String,
    pub project_dir: String,
    pub project_hash: String,
    pub managed_root: String,
    pub worktree_name: String,
}

/// Leader 宿主内存中一个 teammate worktree 的元数据。
///
/// 对齐 `member_state.MemberWorktreeInfo`(pydantic BaseModel)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemberWorktreeInfo {
    pub worktree_path: String,
    pub worktree_name: String,
    #[serde(default)]
    pub worktree_branch: Option<String>,
    #[serde(default)]
    pub head_commit: Option<String>,
    #[serde(default)]
    pub hook_based: bool,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub project_hash: Option<String>,
    #[serde(default)]
    pub managed_root: Option<String>,
}

impl MemberWorktreeInfo {
    /// 构造一个基础元数据(worktree_path + worktree_name)。
    pub fn new(worktree_path: impl Into<String>, worktree_name: impl Into<String>) -> Self {
        Self {
            worktree_path: worktree_path.into(),
            worktree_name: worktree_name.into(),
            worktree_branch: None,
            head_commit: None,
            hook_based: false,
            session_id: None,
            project_hash: None,
            managed_root: None,
        }
    }
}

/// 校验 worktree slug 的安全性(对齐 `slug.validate_slug`)。
///
/// 拒绝:超长、`.` / `..` 路径段、非 `[a-zA-Z0-9._-]` 字符。
pub fn validate_slug(slug: &str) -> Result<(), WorktreeError> {
    if slug.len() > MAX_SLUG_LENGTH {
        return Err(WorktreeError(format!(
            "Invalid worktree name: must be {MAX_SLUG_LENGTH} characters or fewer (got {})",
            slug.len()
        )));
    }
    for segment in slug.split('/') {
        if segment == "." || segment == ".." {
            return Err(WorktreeError(format!(
                "Invalid worktree name \"{slug}\": must not contain \".\" or \"..\" path segments"
            )));
        }
        if segment.is_empty()
            || !segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(WorktreeError(format!(
                "Invalid worktree name \"{slug}\": each segment must be non-empty and contain only letters, digits, dots, underscores, and dashes"
            )));
        }
    }
    Ok(())
}

/// 单段 slug 化(对齐 `naming._slug_part`)。
///
/// 小写、非 `[a-z0-9._-]` 替换为 `-`、strip `._-`、截断、空回退。
pub fn slug_part(value: &str, fallback: &str, max_length: usize) -> String {
    let raw = value.trim().to_lowercase();
    let mut slug: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    slug = slug
        .trim_matches(|c| c == '.' || c == '_' || c == '-')
        .to_string();
    if slug.is_empty() {
        return fallback.to_string();
    }
    let truncated: String = slug.chars().take(max_length).collect();
    let trimmed = truncated
        .trim_matches(|c| c == '.' || c == '_' || c == '-')
        .to_string();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed
    }
}

/// 构建 session 作用域 team 成员的确定性 worktree slug。
///
/// 对齐 `naming.build_teammate_worktree_name`:
/// `agent-{team}-{member}-{sha256(team:member:session:project)[:10]}`。
pub fn build_teammate_worktree_name(
    team_name: &str,
    member_name: &str,
    session_id: &str,
    project_hash: &str,
) -> Result<String, WorktreeError> {
    let team = slug_part(team_name, "team", TEAM_PART_LENGTH);
    let member = slug_part(member_name, "member", MEMBER_PART_LENGTH);
    let seed = format!("{team_name}:{member_name}:{session_id}:{project_hash}");
    let digest = sha256_hex_prefix(&seed, 10);
    let slug = format!("agent-{team}-{member}-{digest}");
    validate_slug(&slug)?;
    Ok(slug)
}

/// DB/缓存元数据是否属于 owner scope(对齐 `member_state.matches_scope`)。
pub fn matches_scope(
    session_id: &str,
    project_hash: &str,
    managed_root: &str,
    scope: &WorktreeOwnerScope,
) -> bool {
    session_id == scope.session_id
        && project_hash == scope.project_hash
        && managed_root == scope.managed_root
}

/// 宿主缓存元数据是否属于 owner scope(对齐 `member_state.info_matches_scope`)。
pub fn info_matches_scope(info: &MemberWorktreeInfo, scope: &WorktreeOwnerScope) -> bool {
    info.session_id.as_deref() == Some(scope.session_id.as_str())
        && info.project_hash.as_deref() == Some(scope.project_hash.as_str())
        && info.managed_root.as_deref() == Some(scope.managed_root.as_str())
        && !info.worktree_path.is_empty()
}

/// 从持久化 DB 成员选项构建宿主 worktree 元数据。
///
/// 对齐 `member_state.info_from_options`:四个归属字段齐全 + matches_scope 才
/// 返回 Some;否则 None(调用方保留旧 worktree 不动)。
#[allow(clippy::too_many_arguments)]
pub fn info_from_options(
    worktree_path: &str,
    worktree_branch: Option<&str>,
    head_commit: Option<&str>,
    session_id: Option<&str>,
    project_hash: Option<&str>,
    managed_root: Option<&str>,
    scope: &WorktreeOwnerScope,
) -> Option<MemberWorktreeInfo> {
    if worktree_path.is_empty()
        || session_id.is_none()
        || project_hash.is_none()
        || managed_root.is_none()
    {
        return None;
    }
    if !matches_scope(
        session_id.unwrap_or(""),
        project_hash.unwrap_or(""),
        managed_root.unwrap_or(""),
        scope,
    ) {
        return None;
    }
    Some(MemberWorktreeInfo {
        worktree_path: worktree_path.to_string(),
        worktree_name: scope.worktree_name.clone(),
        worktree_branch: worktree_branch.map(str::to_string),
        head_commit: head_commit.map(str::to_string),
        hook_based: false,
        session_id: session_id.map(str::to_string),
        project_hash: project_hash.map(str::to_string),
        managed_root: managed_root.map(str::to_string),
    })
}

/// 简化 FIPS 180-4 SHA-256,输出 hex 前缀。
///
/// 纯函数、无外部依赖,用于 worktree 名称摘要(与 Python hashlib.sha256 对齐)。
pub fn sha256_hex_prefix(input: &str, prefix_len: usize) -> String {
    let digest = sha256(input.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex.chars().take(prefix_len).collect()
}

/// FIPS 180-4 SHA-256 实现(自包含,无依赖)。
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// worktree 命名 seam(Service Definition):确定性命名与校验。
pub trait WorktreeNaming: Seam {
    /// 构建 session 作用域 team 成员的 worktree slug。
    fn build_teammate_worktree_name(
        &self,
        team_name: &str,
        member_name: &str,
        session_id: &str,
        project_hash: &str,
    ) -> Result<String, WorktreeError>;

    /// 校验 slug 安全性。
    fn validate_slug(&self, slug: &str) -> Result<(), WorktreeError>;
}

/// worktree 成员状态 seam(Service Definition):归属判定。
pub trait WorktreeMemberState: Seam {
    /// DB/缓存元数据是否属于 owner scope。
    fn matches_scope(&self, info: &MemberWorktreeInfo, scope: &WorktreeOwnerScope) -> bool;

    /// 从持久化选项构建宿主元数据。
    ///
    /// 参数镜像 Python `info_from_options` 签名(1:1),故允许超 7 参。
    #[allow(clippy::too_many_arguments)]
    fn info_from_options(
        &self,
        worktree_path: &str,
        worktree_branch: Option<&str>,
        head_commit: Option<&str>,
        session_id: Option<&str>,
        project_hash: Option<&str>,
        managed_root: Option<&str>,
        scope: &WorktreeOwnerScope,
    ) -> Option<MemberWorktreeInfo>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        // 空输入。
        let digest = sha256(b"");
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // "abc"。
        let digest = sha256(b"abc");
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn validate_slug_accepts_team_pattern() {
        assert!(validate_slug("agent-my-team-bob-abc123def0").is_ok());
    }

    #[test]
    fn validate_slug_rejects_path_traversal_and_chars() {
        assert!(validate_slug("..").is_err());
        // `/` 是合法分段分隔符(仅 `.`/`..` 段被拒)。
        assert!(validate_slug("a/b").is_ok());
        assert!(validate_slug("bad!slug").is_err());
        let long = "a".repeat(MAX_SLUG_LENGTH + 1);
        assert!(validate_slug(&long).is_err());
    }

    #[test]
    fn slug_part_normalizes_and_truncates() {
        assert_eq!(
            slug_part("  My Team! ", "team", TEAM_PART_LENGTH),
            "my-team"
        );
        assert_eq!(slug_part("", "team", TEAM_PART_LENGTH), "team");
        let long = "a".repeat(30);
        assert_eq!(slug_part(&long, "team", TEAM_PART_LENGTH), "a".repeat(14));
    }

    #[test]
    fn build_name_is_deterministic_and_validated() {
        let name =
            build_teammate_worktree_name("TeamA", "Bob", "sess1", "hash1234").expect("build");
        let again =
            build_teammate_worktree_name("TeamA", "Bob", "sess1", "hash1234").expect("build");
        assert_eq!(name, again);
        // slug_part("TeamA") → "teama";slug_part("Bob") → "bob"。
        assert!(name.starts_with("agent-teama-bob-"));
        // agent-(6) + teama(5) + -(1) + bob(3) + -(1) + digest(10) = 26。
        assert_eq!(name.len(), 26);
    }

    #[test]
    fn scope_matching_is_exact() {
        let scope = WorktreeOwnerScope {
            team_name: "t".into(),
            member_name: "m".into(),
            session_id: "s1".into(),
            project_dir: "/p".into(),
            project_hash: "h1".into(),
            managed_root: "/r".into(),
            worktree_name: "agent-t-m-abc".into(),
        };
        assert!(matches_scope("s1", "h1", "/r", &scope));
        assert!(!matches_scope("s2", "h1", "/r", &scope));
        assert!(!matches_scope("s1", "h2", "/r", &scope));
        assert!(!matches_scope("s1", "h1", "/other", &scope));
    }

    #[test]
    fn info_from_options_requires_full_scope() {
        let scope = WorktreeOwnerScope {
            team_name: "t".into(),
            member_name: "m".into(),
            session_id: "s1".into(),
            project_dir: "/p".into(),
            project_hash: "h1".into(),
            managed_root: "/r".into(),
            worktree_name: "agent-t-m-abc".into(),
        };
        // 字段齐全 + 匹配 → Some。
        let info = info_from_options(
            "/w",
            Some("branch"),
            Some("deadbeef"),
            Some("s1"),
            Some("h1"),
            Some("/r"),
            &scope,
        )
        .expect("info");
        assert_eq!(info.worktree_name, "agent-t-m-abc");
        assert_eq!(info.worktree_branch.as_deref(), Some("branch"));

        // 缺失 session_id → None。
        assert!(
            info_from_options("/w", None, None, None, Some("h1"), Some("/r"), &scope).is_none()
        );
        // 不匹配 session → None。
        assert!(
            info_from_options("/w", None, None, Some("s9"), Some("h1"), Some("/r"), &scope)
                .is_none()
        );
        // 空 path → None。
        assert!(
            info_from_options("", None, None, Some("s1"), Some("h1"), Some("/r"), &scope).is_none()
        );
    }

    #[test]
    fn info_matches_scope_checks_all_fields() {
        let scope = WorktreeOwnerScope {
            team_name: "t".into(),
            member_name: "m".into(),
            session_id: "s1".into(),
            project_dir: "/p".into(),
            project_hash: "h1".into(),
            managed_root: "/r".into(),
            worktree_name: "agent-t-m-abc".into(),
        };
        let info = MemberWorktreeInfo {
            worktree_path: "/w".into(),
            worktree_name: "agent-t-m-abc".into(),
            session_id: Some("s1".into()),
            project_hash: Some("h1".into()),
            managed_root: Some("/r".into()),
            ..Default::default()
        };
        assert!(info_matches_scope(&info, &scope));
        let mut foreign = info.clone();
        foreign.session_id = Some("s9".into());
        assert!(!info_matches_scope(&foreign, &scope));
        let mut no_path = info.clone();
        no_path.worktree_path = String::new();
        assert!(!info_matches_scope(&no_path, &scope));
    }
}
