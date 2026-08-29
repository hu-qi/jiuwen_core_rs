//! # ah-plugins-sysop
//!
//! 真实本地系统操作:受限文件系统 + 受限 shell 执行,并把真实工具
//! (read_file / write_file / list_dir / run_shell)注册到 tools seam。
//!
//! 本插件没有 mock:所有行为都是真实副作用(真建文件、真跑进程),
//! 测试用真实临时目录与真实命令验证。

pub mod fs;
pub mod shell;
pub mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::fs::FsProvider;
use ah_contracts::keys::{FS, SHELL, TOOLS};
use ah_contracts::prelude::Effect;
use ah_contracts::service::ServiceKey;
use ah_contracts::shell::ShellProvider;
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

use crate::fs::LocalFsProvider;
use crate::shell::LocalShellProvider;
use crate::tools::{
    EditFileTool, GlobTool, GrepTool, ListDirTool, ReadFileTool, ShellTool, WriteFileTool,
};

/// 真实系统操作插件:提供 fs / shell seam,并把真实工具注入 tools seam。
pub struct SysopPlugin {
    workspace_root: PathBuf,
}

impl SysopPlugin {
    /// 以 workspace root 创建插件。
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }
}

impl Plugin for SysopPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-sysop"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![FS, SHELL]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let fs = Arc::new(
            LocalFsProvider::new(self.workspace_root.clone()).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?,
        );
        let shell = Arc::new(
            LocalShellProvider::new(self.workspace_root.clone()).map_err(|e| {
                PluginError::Apply {
                    plugin: self.name(),
                    message: e.0,
                }
            })?,
        );

        let mut effects = vec![
            ctx.register(FS, fs.clone() as Arc<dyn FsProvider>),
            ctx.register(SHELL, shell.clone() as Arc<dyn ShellProvider>),
        ];

        // 把真实工具注入 tools seam(依赖 TOOLS,挂载顺序由拓扑保证)。
        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".to_string(),
                })?;
        effects.push(registry.register(Arc::new(ReadFileTool::new(fs.clone()))));
        effects.push(registry.register(Arc::new(WriteFileTool::new(fs.clone()))));
        effects.push(registry.register(Arc::new(ListDirTool::new(fs.clone()))));
        effects.push(registry.register(Arc::new(EditFileTool::new(fs.clone()))));
        effects.push(registry.register(Arc::new(GlobTool::new(fs.clone()))));
        effects.push(registry.register(Arc::new(GrepTool::new(fs.clone()))));
        effects.push(registry.register(Arc::new(ShellTool::new(shell.clone()))));

        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TOOLS;
    use ah_hub::plugin::DynPlugin;

    #[tokio::test]
    async fn sysop_plugin_registers_real_seams_and_tools() {
        let root = std::env::temp_dir().join(format!("ah-sysop-plugin-{}", std::process::id()));
        let ctx = Context::new();
        // tools 先挂载(sysop 依赖它)。
        let tools_plugin: DynPlugin = Arc::new(ah_plugins_tools::ToolsPlugin);
        let sysop_plugin: DynPlugin = Arc::new(SysopPlugin::new(&root));
        let effects = ctx
            .mount_all(vec![tools_plugin, sysop_plugin])
            .expect("mount");

        // 真实 fs seam:真实写文件再读回。
        let fs: Arc<dyn FsProvider> = ctx.service(&FS).expect("fs seam");
        fs.write("notes.md", b"real content").expect("write");
        assert_eq!(fs.read("notes.md").unwrap(), b"real content");

        // 真实 shell seam:真实执行命令。
        let shell: Arc<dyn ShellProvider> = ctx.service(&SHELL).expect("shell seam");
        let output = shell
            .run(
                "echo",
                &["from sysop".to_string()],
                std::time::Duration::from_secs(5),
            )
            .await
            .expect("echo");
        assert!(output.stdout.contains("from sysop"));

        // 真实工具:经 tools seam 调用 write_file -> read_file 往返(真实文件)。
        let registry: Arc<dyn ToolRegistry> = ctx.service(&TOOLS).expect("tools seam");
        let mut names = registry.names();
        names.sort();
        assert_eq!(
            names,
            vec![
                "edit",
                "glob",
                "grep",
                "list_dir",
                "read_file",
                "run_shell",
                "write_file"
            ]
        );
        let _ = registry
            .invoke(
                "write_file",
                serde_json::json!({ "path": "a.txt", "content": "via tool" }),
            )
            .await
            .expect("write_file tool");
        let read = registry
            .invoke("read_file", serde_json::json!({ "path": "a.txt" }))
            .await
            .expect("read_file tool");
        assert_eq!(read["content"], "via tool");

        drop(effects);
        assert!(!ctx.has_service(&FS));
        assert!(!ctx.has_service(&SHELL));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// P2-01:edit/glob/grep 工具的成功、错误与结构化输出。
    #[tokio::test]
    async fn edit_glob_grep_tools_work_end_to_end() {
        use serde_json::json;

        let root = std::env::temp_dir().join(format!("ah-sysop-p201-{}", std::process::id()));
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                Arc::new(ah_plugins_tools::ToolsPlugin) as DynPlugin,
                Arc::new(SysopPlugin::new(&root)) as DynPlugin,
            ])
            .expect("mount");
        let registry: Arc<dyn ToolRegistry> = ctx.service(&TOOLS).expect("tools seam");
        let fs: Arc<dyn FsProvider> = ctx.service(&FS).expect("fs seam");

        // 造一个小文件树。
        fs.write("src/main.rs", b"fn main() { println!(\"hello\"); }")
            .expect("write main");
        fs.write("src/util.rs", b"// hello from util")
            .expect("write util");
        fs.write("README.md", b"# hello project")
            .expect("write readme");

        // edit:替换成功(结构化输出 replaced)。
        let edited = registry
            .invoke(
                "edit",
                json!({ "path": "src/main.rs", "old_string": "hello", "new_string": "world" }),
            )
            .await
            .expect("edit");
        assert_eq!(edited["replaced"], 1);
        let content = String::from_utf8_lossy(&fs.read("src/main.rs").unwrap()).into_owned();
        assert!(content.contains("world") && !content.contains("hello"));
        // edit 错误:old_string 未命中 → 显式报错且不落盘。
        let before = fs.read("src/util.rs").unwrap();
        let error = registry
            .invoke(
                "edit",
                json!({ "path": "src/util.rs", "old_string": "absent", "new_string": "x" }),
            )
            .await
            .expect_err("edit must fail when old_string missing");
        assert!(error.0.contains("old_string not found"));
        assert_eq!(fs.read("src/util.rs").unwrap(), before, "未命中不落盘");

        // glob:递归匹配 *。
        let globbed = registry
            .invoke("glob", json!({ "pattern": "*.rs" }))
            .await
            .expect("glob");
        let matches: Vec<&str> = globbed["matches"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.as_str())
            .collect();
        assert!(matches.contains(&"src/main.rs"), "{matches:?}");
        assert!(matches.contains(&"src/util.rs"), "{matches:?}");
        assert!(!matches.iter().any(|m| m.contains("README")));

        // grep:正则命中 + 行号;非法正则显式报错。
        let grepped = registry
            .invoke("grep", json!({ "pattern": "hello" }))
            .await
            .expect("grep");
        assert!(grepped["match_count"].as_u64().unwrap() >= 2);
        let line_numbers: Vec<_> = grepped["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["path"].as_str().unwrap(),
                    m["line_number"].as_u64().unwrap(),
                )
            })
            .collect();
        assert!(line_numbers.contains(&("README.md", 1)), "{line_numbers:?}");
        let bad = registry
            .invoke("grep", json!({ "pattern": "(" }))
            .await
            .expect_err("invalid regex");
        assert!(bad.0.contains("invalid regex"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
