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
use crate::tools::{ListDirTool, ReadFileTool, ShellTool, WriteFileTool};

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
            vec!["list_dir", "read_file", "run_shell", "write_file"]
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
}
