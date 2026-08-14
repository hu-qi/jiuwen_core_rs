//! # ah-plugins-git
//!
//! 真实本地 git 操作(auto_harness 基建):init/status/add/commit/log/diff_stat/branch
//! 全部经真实 git 子进程执行,输出解析为结构化结果;命令失败与 git 缺失显式报错。

use std::path::Path;
use std::process::Command;

use ah_contracts::git::{GitCommit, GitError, GitProvider, GitStatusEntry};
use ah_contracts::keys::GIT;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实 git provider(git 子进程)。
pub struct LocalGitProvider;

impl LocalGitProvider {
    fn run(dir: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Result<String, GitError> {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE");
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let output = command
            .output()
            .map_err(|e| GitError(format!("git unavailable: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(GitError(format!(
                "git {} failed: {}",
                args.first().copied().unwrap_or(""),
                stderr.trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Seam for LocalGitProvider {}

impl GitProvider for LocalGitProvider {
    fn is_repo(&self, dir: &Path) -> bool {
        Self::run(dir, &["rev-parse", "--is-inside-work-tree"], &[]).is_ok()
    }

    fn init(&self, dir: &Path) -> Result<(), GitError> {
        if self.is_repo(dir) {
            return Ok(());
        }
        Self::run(dir, &["init"], &[]).map(|_| ())
    }

    fn status(&self, dir: &Path) -> Result<Vec<GitStatusEntry>, GitError> {
        let output = Self::run(dir, &["status", "--porcelain=v1"], &[])?;
        Ok(output
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let state = line.get(..2).unwrap_or("").to_string();
                let path = line.get(3..).unwrap_or("").to_string();
                GitStatusEntry { state, path }
            })
            .collect())
    }

    fn add(&self, dir: &Path, paths: &[&str]) -> Result<(), GitError> {
        if paths.is_empty() {
            Self::run(dir, &["add", "-A"], &[]).map(|_| ())
        } else {
            let mut args = vec!["add"];
            args.extend_from_slice(paths);
            Self::run(dir, &args, &[]).map(|_| ())
        }
    }

    fn commit(
        &self,
        dir: &Path,
        message: &str,
        author_name: &str,
        author_email: &str,
    ) -> Result<GitCommit, GitError> {
        // 显式身份,不依赖全局/本地 user 配置。
        let env = [
            ("GIT_AUTHOR_NAME", author_name),
            ("GIT_AUTHOR_EMAIL", author_email),
            ("GIT_COMMITTER_NAME", author_name),
            ("GIT_COMMITTER_EMAIL", author_email),
        ];
        let _ = Self::run(dir, &["commit", "-m", message], &env)?;
        let output = Self::run(dir, &["log", "-1", "--format=%H%n%s%n%an"], &[])?;
        let mut lines = output.lines();
        let hash = lines.next().unwrap_or_default().to_string();
        let subject = lines.next().unwrap_or_default().to_string();
        let author = lines.next().unwrap_or_default().to_string();
        Ok(GitCommit {
            hash,
            subject,
            author,
        })
    }

    fn log(&self, dir: &Path, n: usize) -> Result<Vec<GitCommit>, GitError> {
        let n_str = n.to_string();
        let format = "--format=%H%n%s%n%an".to_string();
        let args = ["log", "-n", &n_str, &format];
        let output = Self::run(dir, &args, &[])?;
        let mut commits = Vec::new();
        let mut lines = output.lines();
        while let (Some(hash), Some(subject), Some(author)) =
            (lines.next(), lines.next(), lines.next())
        {
            commits.push(GitCommit {
                hash: hash.to_string(),
                subject: subject.to_string(),
                author: author.to_string(),
            });
        }
        Ok(commits)
    }

    fn diff_stat(&self, dir: &Path) -> Result<usize, GitError> {
        let output = Self::run(dir, &["diff", "--stat", "HEAD"], &[])?;
        // 最后一行 "N files changed, ..." 或空。
        let last = output.lines().last().unwrap_or_default();
        let count = last
            .split_whitespace()
            .next()
            .and_then(|w| w.parse::<usize>().ok())
            .unwrap_or(0);
        Ok(count)
    }

    fn branch(&self, dir: &Path, name: &str) -> Result<(), GitError> {
        Self::run(dir, &["checkout", "-b", name], &[]).map(|_| ())
    }
}

/// git 插件:提供真实本地 git 操作。
pub struct GitPlugin;

impl Plugin for GitPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-git"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![GIT]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: std::sync::Arc<dyn GitProvider> = std::sync::Arc::new(LocalGitProvider);
        Ok(vec![ctx.register(GIT, provider)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::GIT;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(GitPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn init_add_commit_log_roundtrip() {
        let root = std::env::temp_dir().join(format!("ah-git-rw-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("mkdir");
        let (ctx, effects) = build_ctx();
        let git = ctx.service::<dyn GitProvider>(&GIT).expect("git");

        assert!(!git.is_repo(&root), "fresh dir is not a repo");
        git.init(&root).expect("init");
        assert!(git.is_repo(&root));

        std::fs::write(root.join("a.txt"), "hello").expect("write");
        git.add(&root, &[]).expect("add all");
        let commit = git
            .commit(&root, "feat: add a.txt", "Alice", "alice@example.com")
            .expect("commit");
        assert!(!commit.hash.is_empty());
        assert_eq!(commit.subject, "feat: add a.txt");
        assert_eq!(commit.author, "Alice");

        let log = git.log(&root, 3).expect("log");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].hash, commit.hash);

        // 提交后工作区干净。
        assert!(git.status(&root).expect("status").is_empty());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn diff_stat_counts_uncommitted_changes() {
        let root = std::env::temp_dir().join(format!("ah-git-diff-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("mkdir");
        let (ctx, effects) = build_ctx();
        let git = ctx.service::<dyn GitProvider>(&GIT).expect("git");
        git.init(&root).expect("init");
        std::fs::write(root.join("a.txt"), "x").expect("write");
        git.add(&root, &[]).expect("add");
        git.commit(&root, "first", "A", "a@x").expect("commit");

        std::fs::write(root.join("a.txt"), "yyy").expect("modify");
        std::fs::write(root.join("b.txt"), "z").expect("new file");
        let count = git.diff_stat(&root).expect("diff stat");
        assert!(count >= 1, "uncommitted changes detected");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn branch_switches_workspace() {
        let root = std::env::temp_dir().join(format!("ah-git-br-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("mkdir");
        let (ctx, effects) = build_ctx();
        let git = ctx.service::<dyn GitProvider>(&GIT).expect("git");
        git.init(&root).expect("init");
        std::fs::write(root.join("a.txt"), "x").expect("write");
        git.add(&root, &[]).expect("add");
        git.commit(&root, "base", "A", "a@x").expect("commit");
        git.branch(&root, "feature").expect("branch");
        // 切到新分支后仍可提交。
        std::fs::write(root.join("f.txt"), "f").expect("write");
        git.add(&root, &[]).expect("add");
        git.commit(&root, "feat: on feature", "A", "a@x")
            .expect("commit");
        assert_eq!(
            git.log(&root, 1).expect("log")[0].subject,
            "feat: on feature"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn non_repo_operations_error_explicitly() {
        let root = std::env::temp_dir().join(format!("ah-git-err-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("mkdir");
        let (ctx, effects) = build_ctx();
        let git = ctx.service::<dyn GitProvider>(&GIT).expect("git");
        assert!(git.status(&root).is_err(), "status on non-repo must error");
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
