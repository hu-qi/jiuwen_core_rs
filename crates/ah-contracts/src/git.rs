//! git seam:本地 git 操作(auto_harness 基建;远端 fork/PR/GitCode API 需网络与
//! 凭据,留待后续,文档注明)。全部经真实 git 子进程执行。

use std::path::Path;

use crate::seam::Seam;

/// 一次提交。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GitCommit {
    pub hash: String,
    pub subject: String,
    pub author: String,
}

/// 一条状态条目(porcelain v1 解析)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GitStatusEntry {
    /// porcelain 状态码(如 "??"、"M ")。
    pub state: String,
    pub path: String,
}

/// git 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError(pub String);

impl core::fmt::Display for GitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

/// git Seam(Service Definition):本地仓库操作。
pub trait GitProvider: Seam {
    /// dir 是否为 git 仓库。
    fn is_repo(&self, dir: &Path) -> bool;

    /// 在 dir 初始化仓库(已是仓库则幂等成功)。
    fn init(&self, dir: &Path) -> Result<(), GitError>;

    /// 工作区状态(porcelain v1)。
    fn status(&self, dir: &Path) -> Result<Vec<GitStatusEntry>, GitError>;

    /// 暂存指定路径(空列表 = 全部)。
    fn add(&self, dir: &Path, paths: &[&str]) -> Result<(), GitError>;

    /// 提交(以显式身份,不依赖全局配置),返回提交记录。
    fn commit(
        &self,
        dir: &Path,
        message: &str,
        author_name: &str,
        author_email: &str,
    ) -> Result<GitCommit, GitError>;

    /// 最近 n 条提交(新→旧)。
    fn log(&self, dir: &Path, n: usize) -> Result<Vec<GitCommit>, GitError>;

    /// 未提交改动涉及的路径数(diff --stat 解析)。
    fn diff_stat(&self, dir: &Path) -> Result<usize, GitError>;

    /// 新建并切换到分支。
    fn branch(&self, dir: &Path, name: &str) -> Result<(), GitError>;
}
