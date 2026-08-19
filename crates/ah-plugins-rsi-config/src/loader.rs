//! # rsi 配置加载器(对齐 `openjiuwen/rsi/config/loader.py`)
//!
//! - `DEFAULT_CONFIG_TEMPLATE`:嵌入默认编排配置模板(与
//!   `rsi/resource/orchestrating.default.yaml` 逐字一致);
//! - `load_auto_coordinating_harness_config`:路径不存在时写入默认模板引导,
//!   仍不存在 → 显式 FileNotFoundError;非 mapping → 显式 ValueError;
//!   `from_dict` 解析;`team_spec_config_ref` 相对路径 → 相对配置目录绝对化。
//!
//! 文件 IO 经注入的闭包(测试用内存后端);YAML 解析用 serde_yaml。

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::auto::AutoCoordinatingHarnessConfig;

/// 默认编排配置模板(对齐 orchestrating.default.yaml,逐字一致)。
pub const DEFAULT_CONFIG_TEMPLATE: &str = r#"# Default auto-coordinating harness configuration template.
# Written by config.loader when the requested config path is missing.

workspace_dir: .local/rsi/tui/workspace
max_epochs: 1
freeze_team_skill: false
freeze_team_members: false

data_loader:
  file_pattern: "*.json"
  batch_size: 2
  batch_balance_keys:
    - dimension
    - difficulty
    - source
    - task_type

dataset_curation:
  enabled: true
  score_threshold: 1.0
  require_judgeable_reference: true
  output_filename: replay_cases.json
  report_filename: curation_report.yaml
  targeted_seed_filename: targeted_dataset_seed.json
  source_label: trace_replay

dataset_generator:
  model_config_ref: mock-model
  min_cases: 2
  coverage_dimensions:
    - diagnosis
    - explanation
    - practice

evaluator:
  default_script: default
  backend: local
  evaluation_method: exact_match
  success_score: 0.85
  case_lifecycle_timeout_sec: 3600

evaluation_result_analyzer:
  diagnosis_agent_max_retries: 2
  diagnosis_agent_max_concurrency: 5
  diagnosis_agent_max_iterations: 20
  max_issues: 8
  evidence_limit_per_issue: 3
  output_filename: issues.yaml

team_skill_optimizer:
  model_config_ref: mock-model
  max_candidates: 1
  freeze: false
  language: cn
  auto_approve: true

member_optimizer:
  model_config_ref: mock-model
  action_group_configs:
    - prompt
    - skill
  freeze: false
  max_roles_per_run: 2
  min_attribution_confidence: 0.1
  execution_concurrency: 1
  role_execution_concurrency: 1
  action_execution_concurrency_per_role: 1

optimization_experience_learner:
  model_config_ref: mock-model
  output_filename: experience_ref.yaml
  enabled: true

scheduling:
  evaluation_strategy: hybrid
  coordination_strategy: team_first_single_pass
  promotion_policy: epoch_full_evaluation
"#;

/// 文件系统操作(注入;测试用内存后端,生产用真实路径)。
pub trait ConfigFs: Send + Sync {
    /// 路径是否为文件。
    fn is_file(&self, path: &Path) -> bool;

    /// 读取文件文本。
    fn read_text(&self, path: &Path) -> Result<String, String>;

    /// 写入文件文本(自动建父目录)。
    fn write_text(&self, path: &Path, content: &str) -> Result<(), String>;
}

/// 引导默认配置到 `path`(对齐 `_bootstrap_default_config`)。
pub fn bootstrap_default_config(fs: &dyn ConfigFs, path: &Path) -> bool {
    fs.write_text(path, DEFAULT_CONFIG_TEMPLATE).is_ok()
}

/// 解析 YAML 文本为 JSON Value(serde_yaml 兼容)。
pub fn parse_yaml(text: &str) -> Result<Value, String> {
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    serde_json::to_value(&value).map_err(|e| e.to_string())
}

/// 加载并校验自动编排配置(对齐 `load_auto_coordinating_harness_config`)。
///
/// `config_path` 展开 `~` 并绝对化;缺失 → 引导默认模板;仍缺失 →
/// FileNotFoundError;非 mapping → ValueError;`team_spec_config_ref`
/// 相对路径 → 相对配置目录绝对化。
pub fn load_auto_coordinating_harness_config(
    fs: &dyn ConfigFs,
    config_path: &str,
) -> Result<AutoCoordinatingHarnessConfig, String> {
    let path = expand_and_resolve(config_path);
    if !fs.is_file(&path) {
        bootstrap_default_config(fs, &path);
    }
    if !fs.is_file(&path) {
        return Err(format!(
            "auto-coordinating harness config not found: {}",
            path.display()
        ));
    }
    let text = fs.read_text(&path)?;
    let data = parse_yaml(&text)?;
    let data = match data {
        Value::Null => Value::Object(Default::default()),
        other => other,
    };
    if !data.is_object() {
        return Err(format!(
            "auto-coordinating harness config must be a mapping: {}",
            path.display()
        ));
    }
    let mut config = AutoCoordinatingHarnessConfig::from_dict(&data)?;
    // team_spec_config_ref 相对路径 → 相对配置目录绝对化。
    let ref_value = config.evaluator.team_spec_config_ref.clone();
    if !ref_value.is_empty() {
        let team_spec_path = PathBuf::from(&ref_value);
        if team_spec_path.is_absolute() {
            config.evaluator.team_spec_config_ref = team_spec_path.to_string_lossy().to_string();
        } else {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let resolved = parent.join(&ref_value);
            config.evaluator.team_spec_config_ref = resolved.to_string_lossy().to_string();
        }
    }
    Ok(config)
}

/// 展开 `~` 并绝对化(对齐 `Path.expanduser().resolve()` 的非严格语义)。
pub fn expand_and_resolve(config_path: &str) -> PathBuf {
    let expanded = expanduser(config_path);
    if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(expanded)
    }
}

/// 展开 `~` 前缀(对齐 `expanduser`;`~` / `~/x` → HOME/x)。
pub fn expanduser(path: &str) -> PathBuf {
    if (path == "~" || path.starts_with("~/"))
        && let Ok(home) = std::env::var("HOME")
    {
        if path == "~" {
            return PathBuf::from(home);
        }
        return PathBuf::from(home).join(&path[2..]);
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// 内存文件系统(测试后端)。
    struct MemFs {
        files: Mutex<BTreeMap<String, String>>,
    }

    impl ConfigFs for MemFs {
        fn is_file(&self, path: &Path) -> bool {
            self.files
                .lock()
                .expect("fs lock")
                .contains_key(&path.to_string_lossy().to_string())
        }

        fn read_text(&self, path: &Path) -> Result<String, String> {
            self.files
                .lock()
                .expect("fs lock")
                .get(&path.to_string_lossy().to_string())
                .cloned()
                .ok_or_else(|| format!("no file: {}", path.display()))
        }

        fn write_text(&self, path: &Path, content: &str) -> Result<(), String> {
            self.files
                .lock()
                .expect("fs lock")
                .insert(path.to_string_lossy().to_string(), content.to_string());
            Ok(())
        }
    }

    #[test]
    fn default_template_parses_as_mapping() {
        let value = parse_yaml(DEFAULT_CONFIG_TEMPLATE).expect("parse");
        assert!(value.is_object());
        assert_eq!(value["max_epochs"], 1);
        assert_eq!(value["data_loader"]["batch_size"], 2);
        assert_eq!(value["scheduling"]["evaluation_strategy"], "hybrid");
    }

    #[test]
    fn bootstrap_writes_template_and_loads() {
        let fs = MemFs {
            files: Mutex::new(BTreeMap::new()),
        };
        let path = "/tmp/rsicfg/missing.yaml";
        // 缺失 → 引导默认模板 → 加载成功。
        let config = match load_auto_coordinating_harness_config(&fs, path) {
            Ok(c) => c,
            Err(e) => panic!("load failed: {e}"),
        };
        assert_eq!(config.max_epochs, 1);
        assert!(!config.freeze_team_skill);
        assert_eq!(config.data_loader.batch_size, 2);
        // 模板已写入。
        assert!(fs.is_file(Path::new(path)));
    }

    #[test]
    fn missing_after_bootstrap_fails_explicitly() {
        // 写入失败的 FS:is_file 恒 false → 引导后仍缺失 → FileNotFoundError。
        struct BrokenFs;
        impl ConfigFs for BrokenFs {
            fn is_file(&self, _p: &Path) -> bool {
                false
            }
            fn read_text(&self, _p: &Path) -> Result<String, String> {
                Err("nope".to_string())
            }
            fn write_text(&self, _p: &Path, _c: &str) -> Result<(), String> {
                Err("write denied".to_string())
            }
        }
        let err = match load_auto_coordinating_harness_config(&BrokenFs, "/cfg/none.yaml") {
            Ok(_) => panic!("expected missing"),
            Err(e) => e,
        };
        assert!(err.contains("auto-coordinating harness config not found"));
    }

    #[test]
    fn non_mapping_config_rejected() {
        let fs = MemFs {
            files: Mutex::new(BTreeMap::new()),
        };
        fs.files
            .lock()
            .expect("fs lock")
            .insert("/cfg/list.yaml".to_string(), "- 1\n- 2\n".to_string());
        let err = match load_auto_coordinating_harness_config(&fs, "/cfg/list.yaml") {
            Ok(_) => panic!("expected list rejection"),
            Err(e) => e,
        };
        assert!(err.contains("must be a mapping"));
    }

    #[test]
    fn team_spec_ref_absolutized() {
        let fs = MemFs {
            files: Mutex::new(BTreeMap::new()),
        };
        // 配置带相对 team_spec_config_ref。
        let yaml = r#"
evaluator:
  team_spec_config_ref: team-spec.yaml
"#;
        fs.files
            .lock()
            .expect("fs lock")
            .insert("/cfg/dir/config.yaml".to_string(), yaml.to_string());
        let config = match load_auto_coordinating_harness_config(&fs, "/cfg/dir/config.yaml") {
            Ok(c) => c,
            Err(e) => panic!("load failed: {e}"),
        };
        assert!(
            config
                .evaluator
                .team_spec_config_ref
                .ends_with("team-spec.yaml"),
            "got: {}",
            config.evaluator.team_spec_config_ref
        );
        assert!(config.evaluator.team_spec_config_ref.contains("cfg/dir"));
    }

    #[test]
    fn expanduser_handles_home() {
        // 无法可靠设置 HOME;至少验证非 ~ 路径直通。
        assert_eq!(expanduser("/a/b"), PathBuf::from("/a/b"));
        assert_eq!(expanduser("relative"), PathBuf::from("relative"));
    }
}
