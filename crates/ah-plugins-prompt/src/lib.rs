//! # ah-plugins-prompt
//!
//! 真实文件后端 prompt 注册表(对应 openjiuwen/core 的 prompt_builder):
//! - 模板:{{var}} 占位符,渲染时替换;
//! - 版本:同名注册递增版本,最新版本生效,文件落盘(dir/{name}.json);
//! - 错误:缺失变量显式报错并列明缺失项,绝不静默替换。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use ah_contracts::keys::PROMPT;
use ah_contracts::prelude::Effect;
use ah_contracts::prompt::{PromptError, PromptRegistry, PromptTemplate, RenderedPrompt};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 提取模板中的变量名({{var}})。
fn variables(template: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        if let Some(end) = after.find("}}") {
            let name = after[..end].trim().to_string();
            if !name.is_empty() && !vars.contains(&name) {
                vars.push(name);
            }
            rest = &after[end + 2..];
        } else {
            break;
        }
    }
    vars
}

/// 真实文件后端 prompt 注册表。
pub struct FilePromptRegistry {
    dir: PathBuf,
    /// name → 最新版本模板;启动时从磁盘加载。
    index: Mutex<HashMap<String, PromptTemplate>>,
}

impl FilePromptRegistry {
    /// 打开注册表(加载 dir 下全部模板)。
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, PromptError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| PromptError(format!("create prompt dir failed: {e}")))?;
        let mut index = HashMap::new();
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| PromptError(format!("read prompt dir failed: {e}")))?
        {
            let entry = entry.map_err(|e| PromptError(format!("entry failed: {e}")))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(key) = name.strip_suffix(".json")
                && let Ok(text) = std::fs::read_to_string(entry.path())
                && let Ok(template) = serde_json::from_str::<PromptTemplate>(&text)
            {
                // 同名多版本时只保留最新。
                let keep = index
                    .get(key)
                    .map(|existing: &PromptTemplate| template.version > existing.version)
                    .unwrap_or(true);
                if keep {
                    index.insert(key.to_string(), template);
                }
            }
        }
        Ok(Self {
            dir,
            index: Mutex::new(index),
        })
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }
}

impl Seam for FilePromptRegistry {}

impl PromptRegistry for FilePromptRegistry {
    fn register(&self, template: PromptTemplate) -> Result<PromptTemplate, PromptError> {
        let mut index = self.index.lock().unwrap();
        let version = index
            .get(&template.name)
            .map(|existing| existing.version + 1)
            .unwrap_or(1);
        let stored = PromptTemplate {
            name: template.name.clone(),
            version,
            template: template.template,
            description: template.description,
        };
        let line = serde_json::to_string(&stored)
            .map_err(|e| PromptError(format!("serialize prompt: {e}")))?;
        std::fs::write(self.path_for(&stored.name), line)
            .map_err(|e| PromptError(format!("write prompt: {e}")))?;
        index.insert(stored.name.clone(), stored.clone());
        Ok(stored)
    }

    fn get(&self, name: &str) -> Option<PromptTemplate> {
        self.index.lock().unwrap().get(name).cloned()
    }

    fn list(&self) -> Vec<PromptTemplate> {
        let mut templates: Vec<PromptTemplate> =
            self.index.lock().unwrap().values().cloned().collect();
        templates.sort_by(|a, b| a.name.cmp(&b.name));
        templates
    }

    fn render(
        &self,
        name: &str,
        vars: &HashMap<String, String>,
    ) -> Result<RenderedPrompt, PromptError> {
        let template = self
            .get(name)
            .ok_or_else(|| PromptError(format!("prompt not found: {name}")))?;
        let used = variables(&template.template);
        let missing: Vec<&String> = used.iter().filter(|v| !vars.contains_key(*v)).collect();
        if !missing.is_empty() {
            return Err(PromptError(format!(
                "missing variables for {name}: {}",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let mut content = template.template.clone();
        for var in &used {
            content = content.replace(
                &format!("{{{{{var}}}}}"),
                vars.get(var).expect("checked above"),
            );
        }
        Ok(RenderedPrompt {
            name: template.name.clone(),
            version: template.version,
            content,
            variables: used,
        })
    }
}

/// prompt 插件:提供版本化模板注册表。
pub struct PromptPlugin {
    dir: PathBuf,
}

impl PromptPlugin {
    /// 以模板目录创建插件。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for PromptPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-prompt"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![PROMPT]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let registry =
            FilePromptRegistry::open(self.dir.clone()).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?;
        let registry: std::sync::Arc<dyn PromptRegistry> = std::sync::Arc::new(registry);
        Ok(vec![ctx.register(PROMPT, registry)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::PROMPT;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(PromptPlugin::new(root.join("prompts")))];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn greeting() -> PromptTemplate {
        PromptTemplate {
            name: "greet".to_string(),
            version: 0, // register 会重写版本
            template: "Hello {{name}}, you are the {{role}}.".to_string(),
            description: "greeting".to_string(),
        }
    }

    #[test]
    fn register_render_with_variables() {
        let root = std::env::temp_dir().join(format!("ah-prompt-render-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn PromptRegistry>(&PROMPT).expect("prompt");

        let stored = registry.register(greeting()).expect("register");
        assert_eq!(stored.version, 1, "first registration is v1");

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("role".to_string(), "developer".to_string());
        let rendered = registry.render("greet", &vars).expect("render");
        assert_eq!(rendered.content, "Hello Alice, you are the developer.");
        assert_eq!(
            rendered.variables,
            vec!["name".to_string(), "role".to_string()]
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_variable_errors_explicitly() {
        let root = std::env::temp_dir().join(format!("ah-prompt-miss-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn PromptRegistry>(&PROMPT).expect("prompt");
        registry.register(greeting()).expect("register");

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Bob".to_string());
        let err = registry
            .render("greet", &vars)
            .expect_err("missing role must error");
        assert!(
            err.0.contains("missing variables"),
            "explicit error listing missing"
        );
        assert!(err.0.contains("role"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn versioning_and_persistence_across_reopen() {
        let root = std::env::temp_dir().join(format!("ah-prompt-ver-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn PromptRegistry>(&PROMPT).expect("prompt");

        let v1 = registry.register(greeting()).expect("v1");
        let mut upgraded = greeting();
        upgraded.template = "Hello {{name}} (v2), you are the {{role}}.".to_string();
        let v2 = registry.register(upgraded).expect("v2");
        assert_eq!(v2.version, 2);
        assert!(v1.version < v2.version);
        assert_eq!(
            registry.get("greet").expect("get").version,
            2,
            "latest wins"
        );

        drop(effects);

        // 重开:从磁盘加载,最新版本生效。
        let reopened = FilePromptRegistry::open(root.join("prompts")).expect("reopen");
        assert_eq!(reopened.get("greet").expect("get").version, 2);
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Cara".to_string());
        vars.insert("role".to_string(), "reviewer".to_string());
        let rendered = reopened.render("greet", &vars).expect("render");
        assert!(rendered.content.contains("(v2)"));
        assert_eq!(reopened.list().len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }
}
