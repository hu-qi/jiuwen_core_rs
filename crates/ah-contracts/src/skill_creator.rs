//! skill_creator seam:dev_tools 技能创建的确定性契约。
//!
//! 对齐 `openjiuwen/dev_tools/skill_creator/skills/skill_omni_creation/scripts/`
//! 的确定性部分:
//! - common:`slugify` / `url_to_slug` / `image_ext` / `strip_json_fence` /
//!   `save_fetched_assets`(文件名 dom_NNN)/ `blocks_with_paths_*` /
//!   `strip_hallucinated_images`(去幻影图片 + 空行折叠);
//! - stage_01:URL 判定 / 黑名单 / blocks 构建(确定性规则);
//! - stage_03:过滤判据(内容/数量/去重);
//! - stage_04:SKILL.md 骨架与分组规则;
//! - stage_05:LLM 生成的确定性后处理(由插件注入 LLM 调用)。
//!
//! 契约零实现:HTTP 抓取 / LLM 生成由插件经 trait 注入;纯函数在本契约。

use crate::seam::Seam;

/// 支持的图片扩展(对齐 SUPPORTED_EXTS)。
pub const SUPPORTED_EXTS: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp"];
/// mime → 扩展名(对齐 MIME_TO_EXT;未知 mime 默认 .png)。
pub fn mime_to_ext(mime: &str) -> &'static str {
    match mime {
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "image/svg+xml" => ".svg",
        "image/bmp" => ".bmp",
        _ => ".png",
    }
}

/// 内容长度上限(对齐 MAX_CONTENT_LENGTH 等,以规格为准)。
pub const MAX_CONTENT_LENGTH: usize = 30_000;
/// 资产数上限(对齐 MAX_ASSETS)。
pub const MAX_ASSETS: usize = 20;

/// slug 化(对齐 `slugify`):lower → 删非单词字符 → 折叠空白/下划线/连字符 → 截 80。
///
/// 注意 Python `\w` 为 Unicode 语义;Rust 侧按 char 判断 is_alphanumeric 或 '_'。
/// 首尾空白先 strip(与 `text.lower().strip()` 一致),折叠后首尾 `_` 再剥除。
pub fn slugify(text: &str) -> String {
    let lowered = text.trim().to_lowercase();
    let mut cleaned = String::new();
    for c in lowered.chars() {
        if c.is_alphanumeric() || c == '_' || c.is_whitespace() || c == '-' {
            cleaned.push(c);
        }
    }
    // 折叠空白/下划线/连字符连续串为单个 _。
    let mut out = String::with_capacity(cleaned.len());
    let mut prev_sep = false;
    for c in cleaned.chars() {
        if c.is_whitespace() || c == '_' || c == '-' {
            if !prev_sep {
                out.push('_');
                prev_sep = true;
            }
        } else {
            out.push(c);
            prev_sep = false;
        }
    }
    let out = out.trim_matches('_').to_string();
    out.chars().take(80).collect()
}

/// URL → slug(对齐 `url_to_slug`):host[:port] + path,去 scheme/query/fragment。
///
/// 宽松解析(畸形 URL 不抛错,取 `//` 之后、`?`/`#` 之前)。
pub fn url_to_slug(url: &str) -> String {
    let without_scheme = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    let before_query = without_scheme.split(['?', '#']).next().unwrap_or("");
    let raw = before_query.trim_matches('/');
    slugify(raw)
}

/// 图片扩展名(对齐 `image_ext`):URL path 后缀,未知回退 mime。
pub fn image_ext(url: &str, mime: &str) -> String {
    let path_part = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    let path = path_part.split(['?', '#']).next().unwrap_or("");
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();
    if SUPPORTED_EXTS.contains(&ext.as_str()) {
        ext
    } else {
        mime_to_ext(mime).to_string()
    }
}

/// 去除 markdown 围栏(对齐 `strip_json_fence`,顺序严格)。
pub fn strip_json_fence(text: &str) -> String {
    let mut t = text.trim().to_string();
    // 去开头 ``` 围栏行(语言标识可选)。
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        t = rest.to_string();
    }
    // 去结尾 ``` 围栏行。
    if let Some(prefix) = t.strip_suffix("```") {
        t = prefix.strip_suffix('\n').unwrap_or(prefix).to_string();
    }
    t.trim().to_string()
}

/// 编码为 data URL(对齐 `encode_b64`):`data:<mime>;base64,<b64>`。
pub fn encode_b64(data: &[u8], mime: &str) -> String {
    format!(
        "data:{mime};base64,{}",
        base64_encoder::Standard::encode(data)
    )
}

/// 内部 base64 编码器(自包含,STANDARD 带 padding)。
mod base64_encoder {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub struct Standard;

    impl Standard {
        pub fn encode(data: &[u8]) -> String {
            let mut out = String::new();
            for chunk in data.chunks(3) {
                let b0 = chunk[0] as u32;
                let b1 = *chunk.get(1).unwrap_or(&0) as u32;
                let b2 = *chunk.get(2).unwrap_or(&0) as u32;
                let n = (b0 << 16) | (b1 << 8) | b2;
                out.push(TABLE[(n >> 18) as usize & 63] as char);
                out.push(TABLE[(n >> 12) as usize & 63] as char);
                if chunk.len() > 1 {
                    out.push(TABLE[(n >> 6) as usize & 63] as char);
                } else {
                    out.push('=');
                }
                if chunk.len() > 2 {
                    out.push(TABLE[n as usize & 63] as char);
                } else {
                    out.push('=');
                }
            }
            out
        }
    }
}

/// 资产清单条目(对齐 save_fetched_assets 的 manifest 值)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AssetEntry {
    pub path: String,
    pub mime: String,
}

/// 保存已抓取资产,返回 manifest(对齐 `save_fetched_assets`)。
///
/// 文件名 `{prefix}_{idx:03d}{ext}`,按插入顺序编号。
pub fn save_fetched_assets_manifest(
    fetched: &[(String, Vec<u8>, String)],
    prefix: &str,
) -> std::collections::BTreeMap<String, AssetEntry> {
    let mut manifest = std::collections::BTreeMap::new();
    for (idx, (url, _data, mime)) in fetched.iter().enumerate() {
        let ext = image_ext(url, mime);
        let rel = format!("{prefix}_{idx:03}{ext}");
        manifest.insert(
            url.clone(),
            AssetEntry {
                path: rel,
                mime: mime.clone(),
            },
        );
    }
    manifest
}

/// 去幻影图片(对齐 `strip_hallucinated_images`)。
///
/// 保留 valid_paths 中的图片引用,删除其他;折叠连续空行至多 1 个。
pub fn strip_hallucinated_images(
    md: &str,
    valid_paths: &std::collections::BTreeSet<String>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in md.lines() {
        if let Some(open) = line.find("![")
            && let Some(close) = line[open..].find("](")
            && let Some(end) = line[open + close + 2..].find(')')
        {
            let after_open = open + close + 2;
            let path = line[after_open..after_open + end].trim();
            if valid_paths.contains(path) {
                lines.push(line.to_string());
            } else {
                lines.push(String::new());
            }
            continue;
        }
        lines.push(line.to_string());
    }
    // 折叠连续空行至多 1 个 + 丢开头空行。
    let mut out: Vec<String> = Vec::new();
    let mut prev_blank = false;
    for line in lines {
        let blank = line.trim().is_empty();
        if blank {
            if prev_blank || out.is_empty() {
                continue;
            }
            out.push(String::new());
            prev_blank = true;
        } else {
            out.push(line);
            prev_blank = false;
        }
    }
    out.join("\n").trim().to_string()
}

/// stage_03 过滤判据(对齐 stage_03_filter 的确定性规则)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilterDecision {
    pub keep: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// 内容过滤(对齐 stage_03 的过滤规则)。
pub fn filter_block(
    block: &serde_json::Value,
    seen: &mut std::collections::BTreeSet<String>,
) -> FilterDecision {
    let kind = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let text = block
        .get("text")
        .or_else(|| block.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match kind {
        "heading" | "paragraph" | "list" | "table" | "code" => {
            if text.trim().is_empty() {
                return FilterDecision {
                    keep: false,
                    reason: Some("empty content".to_string()),
                };
            }
            // 去重:同文本已见 → 丢弃。
            if !seen.insert(text.trim().to_string()) {
                return FilterDecision {
                    keep: false,
                    reason: Some("duplicate content".to_string()),
                };
            }
            if text.len() > MAX_CONTENT_LENGTH {
                return FilterDecision {
                    keep: false,
                    reason: Some("content too long".to_string()),
                };
            }
            FilterDecision {
                keep: true,
                reason: None,
            }
        }
        "image" => FilterDecision {
            keep: true,
            reason: None,
        },
        _ => FilterDecision {
            keep: false,
            reason: Some("unsupported block type".to_string()),
        },
    }
}

/// skill_creator 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCreatorError(pub String);

impl core::fmt::Display for SkillCreatorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SkillCreatorError {}

/// 抓取器 trait(对齐 stage_01 的 HTTP 抓取;由插件注入)。
pub trait SkillFetcher: Send + Sync {
    /// 抓取一个 URL,返回 (bytes, mime)。
    fn fetch(&self, url: &str) -> Result<(Vec<u8>, String), SkillCreatorError>;
}

/// LLM 生成 trait(对齐 stage_05;由插件注入;不可用须显式报错)。
pub trait SkillGenerator: Send + Sync {
    /// 生成 SKILL.md 文本。
    fn generate_skill_md(&self, spec: &SkillGenRequest) -> Result<String, SkillCreatorError>;
}

/// stage_05 生成请求(对齐 stage_04_save.json 的贯通字段)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillGenRequest {
    pub title: String,
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub blocks: Vec<serde_json::Value>,
}

/// skill_creator Seam(Service Definition):五阶段流水线。
pub trait SkillCreator: Seam {
    /// 完整流水线(抓取→下载→过滤→保存→生成),返回 SKILL.md 文本。
    fn create_skill(
        &self,
        url: &str,
        title: &str,
        language: &str,
    ) -> Result<String, SkillCreatorError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_normalizes() {
        assert_eq!(slugify("Hello World!"), "hello_world");
        assert_eq!(slugify("  My  Skill---Test  "), "my_skill_test");
        // Unicode 保留(CJK)。
        assert_eq!(slugify("中文 技能"), "中文_技能");
        // 截断 80。
        let long = "a".repeat(100);
        assert_eq!(slugify(&long).len(), 80);
    }

    #[test]
    fn url_to_slug_strips_scheme() {
        // Python 两步:删非 [\w\s-](点/斜杠/问号)→ 折叠 - 为 _。
        assert_eq!(
            url_to_slug("https://support.microsoft.com/en-us/office/guide"),
            "supportmicrosoftcomen_usofficeguide"
        );
        // 畸形 URL 不抛错。
        assert_eq!(url_to_slug("no-scheme/path?q=1"), "no_schemepath");
    }

    #[test]
    fn image_ext_resolves() {
        assert_eq!(image_ext("https://x/a.PNG", "image/png"), ".png");
        assert_eq!(image_ext("https://x/photo", "image/jpeg"), ".jpg");
        assert_eq!(
            image_ext("https://x/photo", "application/octet-stream"),
            ".png"
        );
        assert_eq!(image_ext("https://x/a.svg?q=1", "image/png"), ".svg");
    }

    #[test]
    fn json_fence_stripped() {
        assert_eq!(strip_json_fence("```json\n{\"a\": 1}\n```"), "{\"a\": 1}");
        assert_eq!(strip_json_fence("```\nplain\n```"), "plain");
        assert_eq!(strip_json_fence("no fence"), "no fence");
    }

    #[test]
    fn base64_encodes_standard() {
        assert_eq!(encode_b64(b"", "image/png"), "data:image/png;base64,");
        assert_eq!(
            encode_b64(b"f", "text/plain"),
            "data:text/plain;base64,Zg=="
        );
        assert_eq!(
            encode_b64(b"fo", "text/plain"),
            "data:text/plain;base64,Zm8="
        );
        assert_eq!(
            encode_b64(b"foo", "text/plain"),
            "data:text/plain;base64,Zm9v"
        );
    }

    #[test]
    fn asset_manifest_numbered() {
        let fetched = vec![
            (
                "https://x/1.png".to_string(),
                vec![1],
                "image/png".to_string(),
            ),
            (
                "https://x/2.jpg".to_string(),
                vec![2],
                "image/jpeg".to_string(),
            ),
        ];
        let manifest = save_fetched_assets_manifest(&fetched, "dom");
        assert_eq!(manifest["https://x/1.png"].path, "dom_000.png");
        assert_eq!(manifest["https://x/2.jpg"].path, "dom_001.jpg");
        assert_eq!(manifest.len(), 2);
    }

    #[test]
    fn hallucinated_images_removed_and_blank_collapsed() {
        let mut valid = std::collections::BTreeSet::new();
        valid.insert("dom_000.png".to_string());
        let md = "![keep](dom_000.png)\n\n\n![fake](fake.png)\n\ntext";
        let out = strip_hallucinated_images(md, &valid);
        assert!(out.contains("![keep](dom_000.png)"));
        assert!(!out.contains("fake.png"));
        // 空行折叠:至多 1 个连续空行。
        assert!(!out.contains("\n\n\n"));
    }

    #[test]
    fn filter_block_rules() {
        let mut seen = std::collections::BTreeSet::new();
        let good = serde_json::json!({"type": "paragraph", "text": "hello"});
        assert!(filter_block(&good, &mut seen).keep);
        // 重复 → 丢弃。
        assert!(!filter_block(&good, &mut seen).keep);
        // 空内容 → 丢弃。
        let empty = serde_json::json!({"type": "paragraph", "text": "  "});
        assert!(!filter_block(&empty, &mut seen).keep);
        // 未知类型 → 丢弃。
        let weird = serde_json::json!({"type": "video", "text": "x"});
        assert!(!filter_block(&weird, &mut seen).keep);
        // 图片保留。
        let img = serde_json::json!({"type": "image", "path": "dom_000.png"});
        assert!(filter_block(&img, &mut seen).keep);
    }
}
