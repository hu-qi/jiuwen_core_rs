//! 真实本地文件系统 provider(受限 workspace)。

use std::path::PathBuf;

use ah_contracts::fs::{FsError, FsProvider};
use ah_contracts::seam::Seam;

/// 真实本地文件系统:所有操作被约束在 workspace root 内。
pub struct LocalFsProvider {
    root: PathBuf,
}

impl LocalFsProvider {
    /// 以 workspace root 创建 provider;root 不存在则创建。
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, FsError> {
        let root = root.into();
        if !root.exists() {
            std::fs::create_dir_all(&root)
                .map_err(|e| FsError(format!("create workspace root failed: {e}")))?;
        }
        Ok(Self { root })
    }

    /// 解析已存在的路径,并校验不逃逸 root。
    fn resolve_existing(&self, rel: &str) -> Result<PathBuf, FsError> {
        let root = self
            .root
            .canonicalize()
            .map_err(|e| FsError(format!("workspace root unavailable: {e}")))?;
        let candidate = self.root.join(rel);
        let canonical = candidate
            .canonicalize()
            .map_err(|e| FsError(format!("path not accessible: {rel}: {e}")))?;
        if !canonical.starts_with(&root) {
            return Err(FsError(format!("path escapes workspace root: {rel}")));
        }
        Ok(canonical)
    }

    /// 解析可写路径(自动创建父目录),并校验不逃逸 root。
    fn resolve_writable(&self, rel: &str) -> Result<PathBuf, FsError> {
        let root = self
            .root
            .canonicalize()
            .map_err(|e| FsError(format!("workspace root unavailable: {e}")))?;
        let candidate = self.root.join(rel);
        let parent = candidate
            .parent()
            .ok_or_else(|| FsError("path has no parent".to_string()))?;
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| FsError(format!("create parent failed: {e}")))?;
        }
        let canonical_parent = parent
            .canonicalize()
            .map_err(|e| FsError(format!("parent not accessible: {e}")))?;
        if !canonical_parent.starts_with(&root) {
            return Err(FsError(format!("path escapes workspace root: {rel}")));
        }
        if std::fs::symlink_metadata(&candidate).is_ok() {
            let canonical = candidate
                .canonicalize()
                .map_err(|e| FsError(format!("path not accessible: {rel}: {e}")))?;
            if !canonical.starts_with(&root) {
                return Err(FsError(format!("path escapes workspace root: {rel}")));
            }
        }
        Ok(candidate)
    }

    fn canonical_root(&self) -> Result<PathBuf, FsError> {
        self.root
            .canonicalize()
            .map_err(|e| FsError(format!("workspace root unavailable: {e}")))
    }

    /// 测试辅助:临时 workspace(真实目录,测试后清理)。
    #[cfg(test)]
    pub fn temp_workspace(tag: &str) -> (Self, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ah-sysop-{tag}-{}", std::process::id()));
        let provider = Self::new(&dir).expect("temp workspace");
        (provider, dir)
    }
}

impl Seam for LocalFsProvider {}

impl FsProvider for LocalFsProvider {
    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn read(&self, rel: &str) -> Result<Vec<u8>, FsError> {
        let path = self.resolve_existing(rel)?;
        if !path.is_file() {
            return Err(FsError(format!("not a file: {rel}")));
        }
        std::fs::read(&path).map_err(|e| FsError(format!("read failed: {e}")))
    }

    fn write(&self, rel: &str, content: &[u8]) -> Result<(), FsError> {
        let path = self.resolve_writable(rel)?;
        std::fs::write(&path, content).map_err(|e| FsError(format!("write failed: {e}")))
    }

    fn list(&self, rel: &str) -> Result<Vec<String>, FsError> {
        let path = self.resolve_existing(rel)?;
        let mut entries: Vec<String> = std::fs::read_dir(&path)
            .map_err(|e| FsError(format!("list failed: {e}")))?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        entries.sort();
        Ok(entries)
    }

    fn remove(&self, rel: &str) -> Result<(), FsError> {
        let path = self.resolve_existing(rel)?;
        if path == self.canonical_root()? {
            return Err(FsError("cannot remove workspace root".to_string()));
        }
        if path.is_dir() {
            std::fs::remove_dir(&path)
        } else {
            std::fs::remove_file(&path)
        }
        .map_err(|e| FsError(format!("remove failed: {e}")))
    }

    fn exists(&self, rel: &str) -> bool {
        self.resolve_existing(rel).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_workspace<F: FnOnce(LocalFsProvider, PathBuf)>(tag: &str, f: F) {
        let (provider, dir) = LocalFsProvider::temp_workspace(tag);
        f(provider, dir.clone());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_read_list_remove_roundtrip() {
        with_workspace("roundtrip", |fs, _dir| {
            fs.write("a/b.txt", b"hello real fs").expect("write");
            assert_eq!(fs.read("a/b.txt").unwrap(), b"hello real fs");
            assert_eq!(fs.list("a").unwrap(), vec!["b.txt".to_string()]);
            assert!(fs.exists("a/b.txt"));
            fs.remove("a/b.txt").expect("remove");
            assert!(!fs.exists("a/b.txt"));
        });
    }

    #[test]
    fn rejects_path_escaping_workspace() {
        with_workspace("escape", |fs, _dir| {
            fs.write("ok.txt", b"x").expect("write inside");
            // 绝对路径逃逸(root.join 会替换为绝对路径):必须报 escapes。
            let absolute = fs.read("/etc/passwd");
            assert!(matches!(absolute, Err(FsError(message)) if message.contains("escapes")));
            let absolute_write = fs.write("/tmp/ah-escape-attempt", b"x");
            assert!(matches!(absolute_write, Err(FsError(message)) if message.contains("escapes")));
            // 相对路径向上逃逸:必须失败(不存在则不可访问,存在则逃逸)。
            let upward = fs.read("../../etc/passwd");
            assert!(upward.is_err(), "escape must not succeed");
        });
    }

    #[cfg(unix)]
    #[test]
    fn rejects_writing_through_symlinked_file() {
        use std::os::unix::fs::symlink;

        with_workspace("symlink-file", |fs, dir| {
            let outside = dir
                .parent()
                .unwrap()
                .join(format!("ah-sysop-outside-{}-symlink", std::process::id()));
            std::fs::write(&outside, b"outside").expect("outside file");
            symlink(&outside, dir.join("link.txt")).expect("symlink");
            let result = fs.write("link.txt", b"must not escape");
            assert!(matches!(result, Err(FsError(message)) if message.contains("escapes")));
            assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
            let _ = std::fs::remove_file(outside);
        });
    }

    #[test]
    fn read_missing_file_errors() {
        with_workspace("missing", |fs, _dir| {
            assert!(matches!(fs.read("nope.txt"), Err(FsError(_))));
        });
    }
}
