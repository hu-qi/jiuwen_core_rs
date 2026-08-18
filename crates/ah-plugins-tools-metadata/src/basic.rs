//! 基础工具元数据(对齐 harness/prompts/tools/{bash,code,filesystem,web_tools,search_tools}.py)。
use ah_contracts::tools::ToolMetadata;
use serde_json::{Value, json};

fn pick(cn: &'static str, en: &'static str, language: &str) -> &'static str {
    if language == "en" { en } else { cn }
}

fn tool(
    name: &str,
    description_cn: &str,
    description_en: &str,
    params_cn: Value,
    params_en: Value,
    idempotent: bool,
) -> ToolMetadata {
    ToolMetadata {
        name: name.to_string(),
        description_cn: description_cn.to_string(),
        description_en: description_en.to_string(),
        params_cn,
        params_en,
        idempotent,
    }
}

// ---------------------------------------------------------------------------
// bash
// ---------------------------------------------------------------------------
const BASH_DESCRIPTION_CN: &str = r#"执行 Shell 命令并返回输出。命令在持久的工作目录中运行,命令之间的环境变量会保留,但 shell 状态(变量、函数、别名)不会保留。"#;
const BASH_DESCRIPTION_EN: &str = r#"Execute a bash command and return its output. Commands run in a persistent working directory; environment variables persist between commands, but shell state (variables, functions, aliases) does not."#;

fn get_bash_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "command": { "type": "string", "description": d("要执行的 bash 命令。", "The bash command to execute.") },
            "description": { "type": "string", "description": d("命令作用的简要描述(主动语态)。", "A short description of what the command does (active voice).") },
            "timeout_ms": { "type": "integer", "description": d("命令执行超时(毫秒)。", "Execution timeout in milliseconds.") }
        },
        "required": ["command", "description"]
    })
}

// ---------------------------------------------------------------------------
// code
// ---------------------------------------------------------------------------
const CODE_DESCRIPTION_CN: &str = r#"执行代码(Python 或 JavaScript)。"#;
const CODE_DESCRIPTION_EN: &str = r#"Execute code (Python or JavaScript)."#;

fn get_code_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "code": { "type": "string", "description": d("要执行的代码。", "The code to execute.") },
            "language": { "type": "string", "description": d("代码语言(python 或 javascript)。", "Code language (python or javascript).") }
        },
        "required": ["code", "language"]
    })
}

// ---------------------------------------------------------------------------
// read_file / write_file / edit_file
// ---------------------------------------------------------------------------
const READ_FILE_DESCRIPTION_CN: &str =
    r#"增强版文件读取工具。支持文本、图片、PDF 与 Jupyter Notebook。"#;
const READ_FILE_DESCRIPTION_EN: &str =
    r#"Enhanced file reader for text, images, PDFs, and Jupyter notebooks."#;

fn get_read_file_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": d("要读取的文件路径。", "Path of the file to read.") },
            "offset": { "type": "integer", "description": d("起始行号(0 基)。", "Starting line (0-based).") },
            "limit": { "type": "integer", "description": d("最大读取行数。", "Maximum number of lines to read.") }
        },
        "required": ["path"]
    })
}

const WRITE_FILE_DESCRIPTION_CN: &str = r#"写入文件内容。如果文件已存在,将完全覆盖。"#;
const WRITE_FILE_DESCRIPTION_EN: &str = r#"Write file contents. Overwrites existing files."#;

fn get_write_file_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": d("要写入的文件路径。", "Path of the file to write.") },
            "content": { "type": "string", "description": d("要写入的完整内容。", "Full content to write.") }
        },
        "required": ["path", "content"]
    })
}

const EDIT_FILE_DESCRIPTION_CN: &str =
    r#"增强版文件编辑工具,对已有文件执行精确的字符串替换操作,仅传输差量。"#;
const EDIT_FILE_DESCRIPTION_EN: &str = r#"Enhanced file edit tool. Performs exact string replacement on existing files, transmitting only the diff."#;

fn get_edit_file_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": d("要编辑的文件路径。", "Path of the file to edit.") },
            "old_string": { "type": "string", "description": d("要替换的原文(必须唯一匹配)。", "Original text to replace (must match uniquely).") },
            "new_string": { "type": "string", "description": d("替换后的新文本。", "Replacement text.") }
        },
        "required": ["path", "old_string", "new_string"]
    })
}

// ---------------------------------------------------------------------------
// glob / list_files / grep / powershell
// ---------------------------------------------------------------------------
const GLOB_DESCRIPTION_CN: &str = r#"使用 glob 模式查找文件。"#;
const GLOB_DESCRIPTION_EN: &str = r#"Find files using glob patterns."#;

fn get_glob_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string", "description": d("glob 模式(如 **/*.rs)。", "Glob pattern (e.g. **/*.rs).") },
            "path": { "type": "string", "description": d("搜索起始目录(默认当前)。", "Base directory (default current).") }
        },
        "required": ["pattern"]
    })
}

const LIST_DIR_DESCRIPTION_CN: &str = r#"列出目录内容。"#;
const LIST_DIR_DESCRIPTION_EN: &str = r#"List directory contents."#;

fn get_list_dir_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": d("目录路径。", "Directory path.") }
        },
        "required": ["path"]
    })
}

const GREP_DESCRIPTION_CN: &str = r#"在文件中搜索内容。支持正则表达式。"#;
const GREP_DESCRIPTION_EN: &str = r#"Search file contents with regex support."#;

fn get_grep_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string", "description": d("搜索模式(正则)。", "Search pattern (regex).") },
            "path": { "type": "string", "description": d("要搜索的文件或目录。", "File or directory to search.") },
            "include": { "type": "string", "description": d("文件 glob 过滤。", "File glob filter.") }
        },
        "required": ["pattern", "path"]
    })
}

const POWERSHELL_DESCRIPTION_CN: &str =
    r#"执行给定的 PowerShell 命令并返回输出。工作目录在命令之间保持不变;shell 状态不保留。"#;
const POWERSHELL_DESCRIPTION_EN: &str = r#"Execute a PowerShell command and return its output. Working directory persists; shell state does not."#;

fn get_powershell_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "command": { "type": "string", "description": d("要执行的 PowerShell 命令。", "The PowerShell command to execute.") },
            "description": { "type": "string", "description": d("命令作用的简要描述。", "A short description of what the command does.") }
        },
        "required": ["command", "description"]
    })
}

// ---------------------------------------------------------------------------
// free_search / paid_search / fetch_webpage / search_tools
// ---------------------------------------------------------------------------
const FREE_SEARCH_DESCRIPTION_CN: &str = r#"免费搜索,返回结果 URL 和摘要。如果前几条结果相关但不足以直接回答,应抓取前 1-3 条中的至少 2 条。"#;
const FREE_SEARCH_DESCRIPTION_EN: &str = r#"Free search returning result URLs with snippets. If top results are relevant but insufficient, fetch at least 2 of the top 1-3 results."#;

fn get_free_search_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": d("搜索查询。", "Search query.") }
        },
        "required": ["query"]
    })
}

const PAID_SEARCH_DESCRIPTION_CN: &str = r#"付费搜索,支持 provider=auto|bocha|perplexity|serper|jina。配置 API 时这是首选联网搜索工具。"#;
const PAID_SEARCH_DESCRIPTION_EN: &str = r#"Paid search via Bocha/Perplexity/SERPER/JINA. Preferred web search tool when an API is configured."#;

fn get_paid_search_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": d("搜索查询。", "Search query.") },
            "provider": { "type": "string", "description": d("搜索 provider(auto|bocha|perplexity|serper|jina)。", "Search provider (auto|bocha|perplexity|serper|jina).") }
        },
        "required": ["query"]
    })
}

const FETCH_WEBPAGE_DESCRIPTION_CN: &str =
    r#"抓取网页文本,返回状态码、标题和正文。通常配合搜索工具使用:先搜索,再抓取结果页。"#;
const FETCH_WEBPAGE_DESCRIPTION_EN: &str = r#"Fetch webpage text returning status, title, and body. Usually used after search: search first, then fetch result pages."#;

fn get_fetch_webpage_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "url": { "type": "string", "description": d("要抓取的网页 URL。", "Webpage URL to fetch.") },
            "max_chars": { "type": "integer", "description": d("返回内容最大字符数;设为 0 表示不截断。", "Maximum content characters; 0 disables clipping."), "default": 20000 },
            "timeout_seconds": { "type": "integer", "description": d("请求超时时间(秒)。", "Request timeout in seconds."), "default": 45 }
        },
        "required": ["url"]
    })
}

const SEARCH_TOOLS_DESCRIPTION_CN: &str =
    r#"根据能力、名称、描述或参数提示搜索候选工具。仅用于发现,不会直接调用工具。"#;
const SEARCH_TOOLS_DESCRIPTION_EN: &str = r#"Search candidate tools by capability, name, description, or parameter hints. Discovery only; tools are not directly callable."#;

fn get_search_tools_input_params(language: &str) -> Value {
    let d = |cn: &'static str, en: &'static str| pick(cn, en, language);
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": d("搜索候选工具的查询文本", "Search query for finding relevant candidate tools") },
            "limit": { "type": "integer", "description": d("返回候选工具的最大数量", "Maximum number of candidate tools to return") },
            "detail_level": { "type": "integer", "description": d("1=name+描述, 2=+参数摘要, 3=+完整参数", "1=name+description, 2=+parameter summary, 3=+full parameters") }
        },
        "required": ["query"]
    })
}

/// 全部基础工具元数据。
#[allow(clippy::vec_init_then_push)]
pub fn metadata() -> Vec<ToolMetadata> {
    let mut all = Vec::new();
    all.push(tool(
        "bash",
        BASH_DESCRIPTION_CN,
        BASH_DESCRIPTION_EN,
        get_bash_input_params("cn"),
        get_bash_input_params("en"),
        false,
    ));
    all.push(tool(
        "code",
        CODE_DESCRIPTION_CN,
        CODE_DESCRIPTION_EN,
        get_code_input_params("cn"),
        get_code_input_params("en"),
        false,
    ));
    all.push(tool(
        "read_file",
        READ_FILE_DESCRIPTION_CN,
        READ_FILE_DESCRIPTION_EN,
        get_read_file_input_params("cn"),
        get_read_file_input_params("en"),
        false,
    ));
    all.push(tool(
        "write_file",
        WRITE_FILE_DESCRIPTION_CN,
        WRITE_FILE_DESCRIPTION_EN,
        get_write_file_input_params("cn"),
        get_write_file_input_params("en"),
        false,
    ));
    all.push(tool(
        "edit_file",
        EDIT_FILE_DESCRIPTION_CN,
        EDIT_FILE_DESCRIPTION_EN,
        get_edit_file_input_params("cn"),
        get_edit_file_input_params("en"),
        false,
    ));
    all.push(tool(
        "glob",
        GLOB_DESCRIPTION_CN,
        GLOB_DESCRIPTION_EN,
        get_glob_input_params("cn"),
        get_glob_input_params("en"),
        false,
    ));
    all.push(tool(
        "list_files",
        LIST_DIR_DESCRIPTION_CN,
        LIST_DIR_DESCRIPTION_EN,
        get_list_dir_input_params("cn"),
        get_list_dir_input_params("en"),
        false,
    ));
    all.push(tool(
        "grep",
        GREP_DESCRIPTION_CN,
        GREP_DESCRIPTION_EN,
        get_grep_input_params("cn"),
        get_grep_input_params("en"),
        false,
    ));
    all.push(tool(
        "powershell",
        POWERSHELL_DESCRIPTION_CN,
        POWERSHELL_DESCRIPTION_EN,
        get_powershell_input_params("cn"),
        get_powershell_input_params("en"),
        false,
    ));
    all.push(tool(
        "free_search",
        FREE_SEARCH_DESCRIPTION_CN,
        FREE_SEARCH_DESCRIPTION_EN,
        get_free_search_input_params("cn"),
        get_free_search_input_params("en"),
        false,
    ));
    all.push(tool(
        "paid_search",
        PAID_SEARCH_DESCRIPTION_CN,
        PAID_SEARCH_DESCRIPTION_EN,
        get_paid_search_input_params("cn"),
        get_paid_search_input_params("en"),
        false,
    ));
    all.push(tool(
        "fetch_webpage",
        FETCH_WEBPAGE_DESCRIPTION_CN,
        FETCH_WEBPAGE_DESCRIPTION_EN,
        get_fetch_webpage_input_params("cn"),
        get_fetch_webpage_input_params("en"),
        false,
    ));
    all.push(tool(
        "search_tools",
        SEARCH_TOOLS_DESCRIPTION_CN,
        SEARCH_TOOLS_DESCRIPTION_EN,
        get_search_tools_input_params("cn"),
        get_search_tools_input_params("en"),
        false,
    ));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::tools::validate_provider;

    fn all() -> Vec<ToolMetadata> {
        metadata()
    }

    fn find(name: &str) -> ToolMetadata {
        all()
            .into_iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("tool {name} not found"))
    }

    #[test]
    fn metadata_not_empty_and_valid() {
        let metas = all();
        assert!(!metas.is_empty());
        for meta in &metas {
            validate_provider(meta).unwrap_or_else(|e| panic!("{}: {e}", meta.name));
        }
    }

    #[test]
    fn tool_names_match_python_providers() {
        let all = all();
        let mut actual: Vec<&str> = all.iter().map(|m| m.name.as_str()).collect();
        actual.sort_unstable();
        let mut expected = [
            "bash",
            "code",
            "edit_file",
            "fetch_webpage",
            "free_search",
            "glob",
            "grep",
            "list_files",
            "paid_search",
            "powershell",
            "read_file",
            "search_tools",
            "write_file",
        ];
        expected.sort_unstable();
        assert_eq!(actual, expected);
    }

    #[test]
    fn descriptions_differ_between_languages() {
        for meta in all() {
            assert!(
                !meta.description_cn.trim().is_empty(),
                "{} cn empty",
                meta.name
            );
            assert!(
                !meta.description_en.trim().is_empty(),
                "{} en empty",
                meta.name
            );
            assert_ne!(
                meta.description_cn, meta.description_en,
                "{} identical",
                meta.name
            );
        }
    }

    #[test]
    fn idempotent_flags_default_false() {
        for meta in all() {
            assert!(!meta.idempotent, "{} idempotent", meta.name);
        }
    }

    #[test]
    fn schemas_bilingual_matching_properties() {
        for meta in all() {
            let cn = meta.params_cn.as_object().unwrap();
            let en = meta.params_en.as_object().unwrap();
            assert_eq!(cn.get("type"), Some(&serde_json::json!("object")));
            assert_eq!(en.get("type"), Some(&serde_json::json!("object")));
            let ck: std::collections::BTreeSet<&String> = cn
                .get("properties")
                .and_then(|p| p.as_object())
                .unwrap()
                .keys()
                .collect();
            let ek: std::collections::BTreeSet<&String> = en
                .get("properties")
                .and_then(|p| p.as_object())
                .unwrap()
                .keys()
                .collect();
            assert_eq!(ck, ek, "{} keys differ", meta.name);
        }
    }

    #[test]
    fn bash_params_has_command() {
        let bash = find("bash");
        let props = bash
            .params_cn
            .as_object()
            .unwrap()
            .get("properties")
            .unwrap()
            .as_object()
            .unwrap();
        assert!(props.contains_key("command"));
    }

    #[test]
    fn search_tools_requires_query() {
        let st = find("search_tools");
        let req = st
            .params_cn
            .as_object()
            .unwrap()
            .get("required")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(req[0], serde_json::json!("query"));
    }

    #[test]
    fn free_and_paid_search_distinct() {
        let f = find("free_search");
        let p = find("paid_search");
        assert_ne!(f.description_cn, p.description_cn);
    }
}
