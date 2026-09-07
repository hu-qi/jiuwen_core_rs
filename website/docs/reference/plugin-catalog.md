# Catalog 与 Profile API

插件开发完成后，宿主还需要把插件对象和配置名称接入运行时。

## Catalog

`ah-app::plugin_catalog` 返回稳定名称到 `DynPlugin` 的映射：

```rust
pub fn plugin_catalog(
    workspace_root: &Path,
    session_path: &PathBuf,
    session_dir: &PathBuf,
    memory_dir: &PathBuf,
    retrieval_dir: &PathBuf,
    telemetry_dir: &PathBuf,
) -> Vec<(&'static str, DynPlugin)>
```

应用通过名称选择插件：

```rust
let catalog = ah_app::plugin_catalog(
    &workspace_root,
    &session_path,
    &session_dir,
    &memory_dir,
    &retrieval_dir,
    &telemetry_dir,
);

let plugin = catalog
    .iter()
    .find(|(name, _)| *name == "ah-plugins-my")
    .ok_or("plugin is not registered in catalog")?
    .1
    .clone();

let effects = ctx.mount_all(vec![plugin])?;
```

Catalog 是宿主装配层，插件实现不应反向依赖 `ah-app`。新增插件必须同时加入 Workspace、Catalog 和适用 Profile。

## `Profile`

定义位置：`crates/ah-hub/src/profile.rs`。

```rust
pub struct Profile {
    pub name: String,
    pub bundles: Vec<Bundle>,
}

pub struct Bundle {
    pub id: String,
    pub plugins: Vec<String>,
    pub config: Option<toml::Value>,
}
```

### 解析和展开

```rust
let profile = Profile::from_toml(input)?;
let profile = Profile::load("profiles/dev.toml")?;
let names: Vec<String> = profile.plugin_names();
```

| API | 语义 |
| --- | --- |
| `Profile::from_toml(input)` | 从 TOML 文本解析；语法错误返回 `ProfileError::Parse` |
| `Profile::load(path)` | 读取文件后解析；读取失败返回 `ProfileError::Io` |
| `profile.plugin_names()` | 按 Bundle 顺序展开插件名并去重 |
| `Bundle.config` | 保留给宿主组合器的可选 TOML 数据；不会自动变成插件状态 |

### 配置示例

```toml
name = "dev"

[[bundles]]
id = "boot"
plugins = ["ah-plugins-mock", "ah-plugins-tools"]

[bundles.config]
permission_mode = "strict"
```

Profile 只声明组合，不改变依赖规则：挂载时仍由 `provides()` 和 `inject()` 决定拓扑顺序，并执行重复 Provider、缺失依赖和依赖环检查。

## 从 Profile 到 Context

```text
Profile::load
  → plugin_names
  → Catalog 按名称解析 DynPlugin
  → Context::mount_all
  → Context::service::<dyn Seam>
```

完整宿主示例见 `crates/ah-app/src/lib.rs`，组合测试见 `crates/ah-app/tests/stage2_plugins.rs`。
