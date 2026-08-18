//! 记忆/任务工具元数据(与 agent-core harness/prompts/tools/*.py 1:1 对齐)。
//!
//! 覆盖 Python 素材:memory.py、coding_memory.py、compression_recall.py、
//! session_tools.py、todo.py、task_tool.py、goal.py。
//! 每个工具遵循同一模式:DESCRIPTION(cn/en)+ get_*_input_params(language)
//! 返回 JSON schema,最终由 metadata() 汇总为 ToolMetadata 列表;
//! idempotent 对齐 Python 默认 false。

use ah_contracts::tools::ToolMetadata;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// 通用辅助
// ---------------------------------------------------------------------------

/// 由 (key, cn, en, base-schema) 表构造 properties;仅为每个属性注入 description。
fn prop_table(table: &[(&str, &str, &str, Value)], lang: &str) -> Value {
    let mut props = serde_json::Map::new();
    for (key, cn, en, base) in table {
        let mut prop = base.clone();
        let desc = if lang == "en" { *en } else { *cn };
        prop["description"] = Value::String(desc.to_string());
        props.insert((*key).to_string(), prop);
    }
    Value::Object(props)
}

/// 组装 object 型 JSON schema。
fn schema(props: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": props,
        "required": required,
    })
}

/// 从共享 properties 中挑选子集构造新的 properties(保持插入顺序)。
fn pick_props(all: &Value, keys: &[&str]) -> Value {
    let mut m = serde_json::Map::new();
    for key in keys {
        m.insert((*key).to_string(), all[*key].clone());
    }
    Value::Object(m)
}

/// 构造一条工具元数据;cn/en schema 由同一 builder 生成,保证结构一致。
fn meta(name: &str, cn: &str, en: &str, params: fn(&str) -> Value) -> ToolMetadata {
    ToolMetadata {
        name: name.to_string(),
        description_cn: cn.to_string(),
        description_en: en.to_string(),
        params_cn: params("cn"),
        params_en: params("en"),
        idempotent: false,
    }
}

// ---------------------------------------------------------------------------
// memory.py
// ---------------------------------------------------------------------------

const MEMORY_SEARCH_CN: &str =
    "在长期记忆中检索过往信息（决策、偏好、人物、日期、TODO 等），返回相关片段与引用线索。";
const MEMORY_SEARCH_EN: &str = "Search long-term memory (prior decisions, preferences, people, dates, todos) and return relevant snippets and references.";

const MEMORY_GET_CN: &str =
    "按行号切片读取 memory/ 下的记忆 Markdown 文件内容（from_line + lines）。";
const MEMORY_GET_EN: &str =
    "Read a slice of a memory markdown file under memory/ (from_line + lines).";

const WRITE_MEMORY_CN: &str =
    "写入记忆内容到 memory/ 下的 Markdown 文件；支持覆盖写或追加写（append）。";
const WRITE_MEMORY_EN: &str =
    "Write memory content to a markdown file under memory/; supports overwrite or append.";

const EDIT_MEMORY_CN: &str = "在 memory/ 下的记忆文件中做精确字符串替换（old_text → new_text）。";
const EDIT_MEMORY_EN: &str =
    "Perform an exact string replacement inside a memory file (old_text → new_text).";

const READ_MEMORY_CN: &str = "按 offset/limit 读取 memory/ 下记忆文件的部分内容（用于分页阅读）。";
const READ_MEMORY_EN: &str =
    "Read a portion of a memory file under memory/ using offset/limit (for paging).";

/// memory/ 下的目标文件路径（相对路径）
const MEMORY_PATH_CN: &str = "memory/ 下的目标文件路径（相对路径）";
const MEMORY_PATH_EN: &str = "Target path under memory/ (relative path)";

fn get_memory_search_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "query",
                    "检索关键词或问题",
                    "Search query string",
                    json!({"type": "string"}),
                ),
                (
                    "max_results",
                    "最多返回条数（可选）",
                    "Maximum number of results (optional)",
                    json!({"type": "integer"}),
                ),
                (
                    "min_score",
                    "最小相关度阈值（可选）",
                    "Minimum relevance score threshold (optional)",
                    json!({"type": "number"}),
                ),
                (
                    "session_key",
                    "会话键（可选，用于上下文隔离/过滤）",
                    "Session key (optional, for scoping/filtering)",
                    json!({"type": "string"}),
                ),
            ],
            lang,
        ),
        &["query"],
    )
}

fn get_memory_get_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "path",
                    MEMORY_PATH_CN,
                    MEMORY_PATH_EN,
                    json!({"type": "string"}),
                ),
                (
                    "from_line",
                    "起始行号（可选）",
                    "Starting line number (optional)",
                    json!({"type": "integer"}),
                ),
                (
                    "lines",
                    "读取行数（可选）",
                    "Number of lines to read (optional)",
                    json!({"type": "integer"}),
                ),
            ],
            lang,
        ),
        &["path"],
    )
}

fn get_write_memory_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "path",
                    MEMORY_PATH_CN,
                    MEMORY_PATH_EN,
                    json!({"type": "string"}),
                ),
                (
                    "content",
                    "要写入的内容",
                    "Content to write",
                    json!({"type": "string"}),
                ),
                (
                    "append",
                    "是否追加写入（默认 false）",
                    "Append to file instead of overwrite (default false)",
                    json!({"type": "boolean"}),
                ),
            ],
            lang,
        ),
        &["path", "content"],
    )
}

fn get_edit_memory_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "path",
                    MEMORY_PATH_CN,
                    MEMORY_PATH_EN,
                    json!({"type": "string"}),
                ),
                (
                    "old_text",
                    "要替换的原始文本",
                    "Original text to replace",
                    json!({"type": "string"}),
                ),
                (
                    "new_text",
                    "替换后的新文本",
                    "New replacement text",
                    json!({"type": "string"}),
                ),
            ],
            lang,
        ),
        &["path", "old_text", "new_text"],
    )
}

fn get_read_memory_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "path",
                    MEMORY_PATH_CN,
                    MEMORY_PATH_EN,
                    json!({"type": "string"}),
                ),
                (
                    "offset",
                    "从第几行开始读取（可选）",
                    "Line offset to start reading from (optional)",
                    json!({"type": "integer"}),
                ),
                (
                    "limit",
                    "最多读取多少行（可选）",
                    "Maximum number of lines to read (optional)",
                    json!({"type": "integer"}),
                ),
            ],
            lang,
        ),
        &["path"],
    )
}

// ---------------------------------------------------------------------------
// coding_memory.py
// ---------------------------------------------------------------------------

const CODING_MEMORY_READ_CN: &str =
    "按 offset/limit 读取 coding_memory/ 下记忆文件的部分内容（用于分页阅读）。";
const CODING_MEMORY_READ_EN: &str =
    "Read a portion of a memory file under coding_memory/ using offset/limit (for paging).";

const CODING_MEMORY_WRITE_CN: &str =
    "写入记忆内容到 coding_memory/ 下的 Markdown 文件（要求 frontmatter）。";
const CODING_MEMORY_WRITE_EN: &str =
    "Write memory content to a markdown file under coding_memory/ (frontmatter required).";

const CODING_MEMORY_EDIT_CN: &str =
    "在 coding_memory/ 下的记忆文件中做精确字符串替换（old_text → new_text）。";
const CODING_MEMORY_EDIT_EN: &str =
    "Perform an exact string replacement inside a coding memory file (old_text → new_text).";

/// coding_memory/ 下的目标文件路径（相对路径）
const CODING_MEMORY_PATH_CN: &str = "coding_memory/ 下的目标文件路径（相对路径）";
const CODING_MEMORY_PATH_EN: &str = "Target path under coding_memory/ (relative path)";

fn get_coding_memory_read_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "path",
                    CODING_MEMORY_PATH_CN,
                    CODING_MEMORY_PATH_EN,
                    json!({"type": "string"}),
                ),
                (
                    "offset",
                    "从第几行开始读取（可选）",
                    "Line offset to start reading from (optional)",
                    json!({"type": "integer"}),
                ),
                (
                    "limit",
                    "最多读取多少行（可选）",
                    "Maximum number of lines to read (optional)",
                    json!({"type": "integer"}),
                ),
            ],
            lang,
        ),
        &["path"],
    )
}

fn get_coding_memory_write_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "path",
                    CODING_MEMORY_PATH_CN,
                    CODING_MEMORY_PATH_EN,
                    json!({"type": "string"}),
                ),
                (
                    "content",
                    "要写入的内容（含 frontmatter）",
                    "Content to write (with frontmatter)",
                    json!({"type": "string"}),
                ),
            ],
            lang,
        ),
        &["path", "content"],
    )
}

fn get_coding_memory_edit_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "path",
                    CODING_MEMORY_PATH_CN,
                    CODING_MEMORY_PATH_EN,
                    json!({"type": "string"}),
                ),
                (
                    "old_text",
                    "要替换的原始文本",
                    "Original text to replace",
                    json!({"type": "string"}),
                ),
                (
                    "new_text",
                    "替换后的新文本",
                    "New replacement text",
                    json!({"type": "string"}),
                ),
            ],
            lang,
        ),
        &["path", "old_text", "new_text"],
    )
}

// ---------------------------------------------------------------------------
// compression_recall.py
// ---------------------------------------------------------------------------

const RECALL_COMPRESSED_CONTEXT_CN: &str = "在当前 session 内检索指定压缩记忆。先匹配最相关的问答轮次（可能跨多个轮次），再在返回预算内返回相关原文片段。memory_id 必须来自 [[COMPRESSION_RECALL: id=...]] 标记。";
const RECALL_COMPRESSED_CONTEXT_EN: &str = "Search one compressed memory in the current session. It first selects the most relevant turns (possibly across multiple turns), then returns relevant source chunks within a return budget. memory_id must come from a [[COMPRESSION_RECALL: id=...]] marker.";

fn get_recall_compressed_context_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "memory_id",
                    "压缩摘要中 COMPRESSION_RECALL 标记所给出的记忆 ID。",
                    "Memory ID from the COMPRESSION_RECALL marker in the compressed summary.",
                    json!({"type": "string"}),
                ),
                (
                    "query",
                    "用于匹配原始问答轮次和内容片段的检索文本。",
                    "Text used to match the original turn and its source chunks.",
                    json!({"type": "string"}),
                ),
            ],
            lang,
        ),
        &["memory_id", "query"],
    )
}

// ---------------------------------------------------------------------------
// session_tools.py
// ---------------------------------------------------------------------------

const SESSIONS_LIST_CN: &str =
    "查看当前所有后台异步子任务(包括运行中、已完成、失败、已取消)及其元数据";
const SESSIONS_LIST_EN: &str =
    "List all background async tasks (running, completed, failed, canceled) and its metadata";

const SESSIONS_CANCEL_CN: &str = "取消后台异步子任务。此操作会同步阻塞直到任务取消完成。";
const SESSIONS_CANCEL_EN: &str = "Cancel background async task. This operation blocks synchronously until cancellation completes.";

const SESSIONS_SPAWN_CN: &str = r#"创建异步后台子代理任务，立即返回 pending 状态，任务在后台执行，不阻塞当前对话。

可用代理类型及对应工具：
{available_agents}

重要：使用 sessions_spawn 时，必须指定 subagent_type、task_description 参数来选择代理类型和描述任务。
当 subagent_type 为 "browser_agent" 时，还必须指定 browser_capabilities，并从上方可用的 Playwright 能力中选择额外能力类别。
仅使用核心浏览器能力时传入空列表；其他子代理类型不要提供 browser_capabilities。请勿指定你无权访问的其他代理！！！

## 使用场景:
- 任务复杂、多步骤、可独立执行
- 需要并行处理、专注推理、大量上下文 / Token
- 需要沙箱安全执行（代码、搜索、格式化）
- 只需最终输出，不关心中间过程
- 希望在长时间任务执行期间继续处理用户其他问题

## 不使用场景:
- 任务简单，可快速完成
- 需要查看中间步骤
- 拆分无收益、仅增加延迟
- 用户明确要求等待结果后再继续

## 使用原则:
1. 独立任务尽量并行执行以提升性能
2. 用子代理隔离复杂任务，避免主线程过载
3. 提交后立即告知用户任务已在后台执行
4. 不要因为状态是 pending 就重复调用 sessions_spawn 创建相同任务
5. 仅当用户明确要求重试或变更参数时，才发起新任务

## 重要说明:
- 实际任务在后台异步执行

## 示例:

示例 1：并行独立研究
    用户：研究詹姆斯和科比的成就并对比。
    助手：[并行启动 2 个 sessions_spawn 任务分别研究两位球员]
    助手：[汇总对比结果回复用户]
    说明：研究复杂且各球员相互独立，适合拆分并行执行。

示例 2：单任务高上下文隔离
    用户：分析大型代码库的安全漏洞并生成报告。
    助手：[启动单个 sessions_spawn 任务]
    助手：[继续处理用户其他问题]
    说明：用子代理隔离高消耗任务，避免主线程过载。

示例 3：简单任务 - 不要使用 sessions_spawn
    用户：读取 config.json 并告诉我版本号。
    助手：[直接调用 read_file 工具，不用 sessions_spawn]
    说明：简单任务，直接执行更快更清晰。
"#;

const SESSIONS_SPAWN_EN: &str = r#"Create async background subagent task that returns pending status immediately 
while the task executes in the background without blocking the current conversation.

Available agent types and the tools they have access to:
{available_agents}

Important: When using sessions_spawn,
you must specify the subagent_type and task_description parameters to select the agent type and describe the task.
When subagent_type is "browser_agent", you must also specify browser_capabilities as a list of additional
capability categories selected from the available Playwright capabilities above. Use an empty list for a
core-only browser task. Do not provide browser_capabilities for other subagent types.
Do not specify agents you do not have access to!!!

## When to use:
- Tasks that are complex, multi-step, and can be executed independently
- Scenarios requiring parallel processing, focused reasoning, or large context/token usage
- Tasks that require sandboxed execution (e.g., code execution, search, formatting)
- When only the final output is needed and intermediate steps are not required
- When you want to continue handling user queries while a long-running task executes

## When NOT to use:
- Tasks are simple and can be completed quickly
- Intermediate steps need to be observed
- Task decomposition provides no benefit and only adds latency
- User explicitly wants to wait for results before continuing

## Usage Guidelines:
1. Execute independent tasks in parallel whenever possible to improve performance
2. Use sub-agents to isolate complex tasks and avoid main thread overload
3. After spawning, immediately inform user the task is running in background
4. Use sessions_list to check task status and retrieve results
5. Do NOT repeatedly call sessions_spawn for the same task just because status is pending
6. Only create new task when user explicitly requests retry or changes parameters

## Important Notes:
- Actual task execution happens asynchronously in background
- Use sessions_list with task_id to check completion status and get results

## Examples:

Example 1: Parallel independent research
    User: Research achievements of LeBron James and Kobe Bryant, then compare them.
    Assistant: [Spawns 2 parallel sessions_spawn tasks for each player]
    Assistant: [Waits and uses sessions_list to collect results]
    Assistant: [Summarizes comparison for user]
    Reasoning: Complex research tasks that are independent, suitable for parallel execution.

Example 2: High-context isolation
    User: Analyze security vulnerabilities in this large codebase and generate a report.
    Assistant: [Spawns single sessions_spawn task]
    Assistant: [Continues handling other user queries]
    Assistant: [Later retrieves report via sessions_list]
    Reasoning: Isolate high-consumption task to avoid main thread overload.

Example 3: Simple task - DO NOT use sessions_spawn
    User: Read file config.json and tell me the version.
    Assistant: [Directly calls read_file tool, no sessions_spawn]
    Reasoning: Simple task, direct execution is faster and clearer.
"#;

fn get_sessions_list_input_params(_lang: &str) -> Value {
    schema(json!({}), &[])
}

fn get_sessions_spawn_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "subagent_type",
                    "子 agent 类型（必填，须与上方「可用代理类型」列表中的名称完全一致；不要使用列表中未出现的名称）",
                    "Required subagent type; must exactly match a name from the available subagent types list above. Do not use any name that is not listed there.",
                    json!({"type": "string"}),
                ),
                (
                    "task_description",
                    "任务描述",
                    "Task description",
                    json!({"type": "string"}),
                ),
                (
                    "browser_capabilities",
                    "浏览器子代理所需的额外能力类别列表；仅使用核心能力时传入空列表",
                    "Additional capability categories required by browser_agent; use an empty list for core-only tasks",
                    json!({"type": "array", "items": {"type": "string"}}),
                ),
            ],
            lang,
        ),
        &["subagent_type", "task_description"],
    )
}

fn get_sessions_cancel_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[(
                "task_id",
                "要取消的任务 ID（从 sessions_list 获取）",
                "Task ID to cancel (obtained from sessions_list)",
                json!({"type": "string"}),
            )],
            lang,
        ),
        &["task_id"],
    )
}

// ---------------------------------------------------------------------------
// todo.py
// ---------------------------------------------------------------------------

const TODO_CREATE_CN: &str = r#"
创建当前会话的待办事项列表，用于跟踪进度、组织复杂任务，帮助用户了解整体执行情况。

## 使用方式

入参为 JSON 数组（每个任务必须包含 id 字段）：
    {"tasks": [{"id": "translate_doc", "content": "翻译文档", "activeForm": "正在翻译文档", "description": "将文档翻译为目标语言", "selected_model_id": "fast"}, {"id": "analyze_arch", "content": "分析代码架构", "activeForm": "正在分析代码架构", "description": "梳理代码模块结构与依赖关系", "selected_model_id": "smart"}]}

## 规则

- **id 字段为必填**：必须为每个任务指定简短、语义清晰的唯一字符串 ID（如 "translate_doc"、"analyze_code"），禁止使用随机字符或 UUID。同一会话内 ID 不得重复。此 ID 在后续 todo_modify 中用于精准定位任务，是跨轮次更新状态的唯一依据
- 第一个任务自动设为 in_progress，其余为 pending
- 同一时间只能有一个 in_progress 任务
- 任务描述必须具体、可执行、清晰明确
- 调用本工具会覆盖当前会话的任务列表；若需追加任务，请使用 todo_modify
- 当没有获取到当前可用模型信息时，不要添加 selected_model_id 字段；否则必须添加 selected_model_id 指定执行任务的模型 ID
"#;

const TODO_CREATE_EN: &str = r#"
Create a todo list for the current session to track progress, organize complex tasks, and help the user understand overall execution status.

## Usage

Input is a JSON array (each task must include an id field):
    {"tasks": [{"id": "translate_doc", "content": "Translate document", "activeForm": "Translating document", "description": "Translate the document into the target language", "selected_model_id": "fast"}, {"id": "analyze_arch", "content": "Analyze code architecture", "activeForm": "Analyzing code architecture", "description": "Map out module structure and dependencies", "selected_model_id": "smart"}]}

## Rules

- **id is required**: You must provide a short, semantically meaningful unique string ID for each task (e.g. "translate_doc", "analyze_code"). Do NOT use random characters or UUIDs. IDs must be unique within a session. This ID is used by todo_modify to precisely locate tasks and is the sole key for cross-turn status updates
- First task is automatically set to in_progress, others to pending
- Only one task can be in_progress at a time
- Task descriptions must be specific, actionable, and clear
- Calling this tool replaces the current session's task list; use todo_modify to append tasks
- When the currently available model information is not obtained, the selected_model_id field should not be added; otherwise, selected_model_id must be added to specify the model ID for executing the task.
"#;

const TODO_LIST_CN: &str = r#"
检索并显示当前会话的所有待办事项。

## 何时使用 todo_list（而非 todo_modify）

使用 todo_list 的场景：
- 需要查看当前任务全貌和各任务ID，再决定如何更新
- 不确定当前有哪些任务处于 in_progress 或 pending

使用 todo_modify 的场景（不需要先调用 todo_list）：
- 已知任务 ID，直接更新任务信息
- 任务刚完成，立即标记为 completed
"#;

const TODO_LIST_EN: &str = r#"
Retrieve and display all todo items for the current session

## When to Use todo_list (vs. todo_modify)

Use todo_list when:
- You need an overview of all tasks and their IDs before deciding how to update
- You are unsure which tasks are currently in_progress or pending

Use todo_modify directly (no need to call todo_list first) when:
- You already know the task ID and want to update task information
- A task just finished and you want to mark it completed immediately
"#;

const TODO_MODIFY_CN: &str = r#"
修改当前会话的待办事项，包括：更新（update）、删除（delete）、取消（cancel）、追加（append）、在其后插入（insert_after）、在其前插入（insert_before）

重要说明：
- 若需重新规划整个任务列表，请调用 todo_create
- 支持批量处理同一时点发生的结构变更，避免连续多次调用。状态流转应随执行进度即时更新：单次 update 的状态变更通常只涉及当前任务和下一任务，不要将多个 pending 任务批量改为 completed
- **id 使用你在 todo_create 时自定义的语义 ID**（如 "translate_doc"），不要使用 UUID
- **只传入发生变化的字段**：update 每次调用只需给出 id 加上实际改变的字段（通常只是 status），不要把 content、description、activeForm 等未变化的字段重新完整传一遍

action 支持的操作类型：

update：修改现有任务的状态或标题（id 不可修改，只需传入发生变化的字段，其余字段保持不传即可）：
    {
        "action": "update",
        "todos": [
            {"id": "translate_doc", "status": "completed"},
            {"id": "analyze_code", "status": "in_progress"}
        ]
    }

支持修改 selected_model_id：若任务 selected_model_id 不为空，且执行结果质量不佳（输出不准确、逻辑错误、未达预期），应根据模型描述更新质量更高的模型ID：
    {
        "action": "update",
        "todos": [
            {"id": "translate_doc", "selected_model_id": "smart", "status": "pending"}
        ]
    }

cancel：将指定任务标记为 cancelled（任务将被忽略，不再执行）：
    {
        "action": "cancel",
        "ids": ["translate_doc", "analyze_code"]
    }

delete：从列表中永久删除指定任务：
    {
        "action": "delete",
        "ids": ["translate_doc"]
    }

append：在列表末尾追加新任务（id 由你指定，须简短语义且唯一）：
    {
        "action": "append",
        "todos": [
            {"id": "write_report", "content": "新任务内容", "activeForm": "执行新任务", "description": "任务的详细描述", "status": "pending"}
        ]
    }

insert_after：在指定任务之后插入新任务（目标任务状态须为 in_progress 或 pending）：
    {
        "action": "insert_after",
        "todo_data": {"target_id": "translate_doc", "items": [{"id": "review_translation", "content": "插入的任务", "activeForm": "执行插入的任务", "description": "任务的详细描述", "status": "pending", "selected_model_id": "fast"}]}
    }

insert_before：在指定任务之前插入新任务（目标任务状态须为 pending）：
    {
        "action": "insert_before",
        "todo_data": {"target_id": "analyze_code", "items": [{"id": "setup_env", "content": "插入的任务", "activeForm": "执行插入的任务", "description": "任务的详细描述", "status": "pending"}]}
    }

核心规则：
- update 操作：id 字段不可修改，其他字段支持部分更新
- 同一时间只能有一个任务处于 in_progress
- insert_after：目标任务状态必须为 in_progress 或 pending
- insert_before：目标任务状态必须为 pending
- 如果任务的 selected_model_id 为空时，任何操作都不要更改 selected_model_id 字段
"#;

const TODO_MODIFY_EN: &str = r#"
Modify todo items for the current session, including: update, delete, cancel, append, insert_after, and insert_before.

Important notes:
- To re-plan the entire task list, call todo_create instead
- Batch structural changes that occur at the same time to avoid repeated calls. Status transitions should be updated immediately as execution progresses: a single update should normally change only the current task and the next task; do not change multiple pending tasks to completed in one call
- **Use the semantic id you assigned in todo_create** (e.g. "translate_doc"); never use UUIDs
- **Only send the fields that changed**: for update, each call only needs the id plus the fields that actually changed (usually just status) — do not resend unchanged fields like content, description, or activeForm

Supported action types:

update: Modify status or content of existing tasks (id cannot be changed; only send the fields that changed, omit the rest):
    {
        "action": "update",
        "todos": [
            {"id": "translate_doc", "status": "completed"},
            {"id": "analyze_code", "status": "in_progress"}
        ]
    }

Support modifying selected_model_id: If the task's selected_model_id is not empty and the execution result is of poor quality (inaccurate output, logical errors, or failure to meet expectations), the model ID should be updated according to the model description to a higher-quality model:
    {
        "action": "update",
        "todos": [
            {"id": "translate_doc", "selected_model_id": "smart", "status": "pending"}
        ]
    }

cancel: Mark specified tasks as cancelled (tasks will be ignored and not executed):
    {
        "action": "cancel",
        "ids": ["translate_doc", "analyze_code"]
    }

delete: Permanently remove specified tasks from the list:
    {
        "action": "delete",
        "ids": ["translate_doc"]
    }

append: Add new tasks at the end of the list (id must be a short semantic string you choose, unique within the session):
    {
        "action": "append",
        "todos": [
            {"id": "write_report", "content": "New task content", "activeForm": "Executing new task", "description": "Detailed description of the task", "status": "pending"}
        ]
    }

insert_after: Insert new tasks after the specified task (target must be in_progress or pending):
    {
        "action": "insert_after",
        "todo_data": {"target_id": "translate_doc", "items": [{"id": "review_translation", "content": "Inserted task", "activeForm": "Executing inserted task", "description": "Detailed description of the task", "status": "pending", "selected_model_id": "fast"}]}
    }

insert_before: Insert new tasks before the specified task (target must be pending):
    {
        "action": "insert_before",
        "todo_data": {"target_id": "analyze_code", "items": [{"id": "setup_env", "content": "Inserted task", "activeForm": "Executing inserted task", "description": "Detailed description of the task", "status": "pending"}]}
    }

Core rules:
- update: the id field cannot be modified; all other fields support partial updates
- Only one task can be in_progress at a time
- insert_after: target task status must be in_progress or pending
- insert_before: target task status must be pending
- If the task's selected_model_id is empty, do not modify the selected_model_id field in any operation.
"#;

const TODO_GET_CN: &str = r#"
根据任务 ID 获取单个任务的完整详情。

入参：id（任务唯一标识符）

返回：完整的任务信息，包括 id、content（任务摘要）、activeForm、description（任务详细内容）、status、depends_on、result_summary、meta_data、selected_model_id。
"#;

const TODO_GET_EN: &str = r#"
Get full details of a single task by its ID.

Input: id (unique task identifier)

Returns: complete task info including id, content (task summary), activeForm, description (detailed content), status, depends_on, result_summary, meta_data, selected_model_id.
"#;

const TODO_CREATE_TASKS_CN: &str = "子任务列表，JSON 数组格式。每个元素为任务对象，必填字段：
- id：任务唯一标识符，由你自行指定，必须简短且语义清晰（如 \"translate_doc\"、\"analyze_code\"），禁止使用随机字符或 UUID；同一会话内 ID 不得重复；后续 todo_modify 按此 ID 精准定位任务
- content：任务摘要描述
- activeForm：content 的进行语态（如 content 为「翻译文档」，activeForm 为「正在翻译文档」）
- description：任务详细内容
可选字段：
- selected_model_id：执行任务的模型 ID，见系统提示词「模型选择策略」";

const TODO_CREATE_TASKS_EN: &str = "List of subtasks in JSON array format. Each element is a task object with required fields:
- id: unique task identifier that YOU provide; must be short and semantically meaningful (e.g. 'translate_doc', 'analyze_code'); do NOT use random chars or UUIDs; IDs must be unique within a session; todo_modify uses this ID to locate tasks precisely
- content: task summary description
- activeForm: present-tense form of content (e.g., content 'Translate document' -> activeForm 'Translating document')
- description: detailed task content
Optional field:
- selected_model_id: model ID, see 'Model Selection Strategy' in system prompt";

const TODO_MODIFY_TODOS_CN: &str = "根据 action 字段处理的待办事项数组。支持修改 selected_model_id：若某任务执行结果质量不佳（输出不准确、逻辑错误、未达预期），应将 selected_model_id 更新为更高等级的模型 ID，然后将任务状态重置为 pending 或 in_progress 以触发重新执行。";

const TODO_MODIFY_TODOS_EN: &str = "Array of todo items to process based on the action field. Supports updating selected_model_id: if a task produces poor results (inaccurate output, logical errors, unmet objectives), update selected_model_id to a model ID whose description indicates stronger capability, and reset the task status to pending or in_progress to trigger re-execution.";

/// todo 项子 schema 的 properties(对齐 todo.py 的 _todo_item_properties)。
fn todo_item_props(lang: &str) -> Value {
    prop_table(
        &[
            (
                "id",
                "任务唯一标识符，由你自行指定的简短语义字符串（如 \"translate_doc\"），禁止使用 UUID，同一会话内须唯一",
                "Unique task identifier — a short semantic string you provide (e.g. 'translate_doc'); do NOT use UUIDs; must be unique within a session",
                json!({"type": "string"}),
            ),
            (
                "content",
                "任务摘要描述",
                "Task summary description",
                json!({"type": "string"}),
            ),
            (
                "activeForm",
                "content 的进行语态",
                "Present-tense form of content",
                json!({"type": "string"}),
            ),
            (
                "description",
                "任务详细内容",
                "Detailed task content",
                json!({"type": "string"}),
            ),
            (
                "status",
                "任务状态",
                "Task status",
                json!({"type": "string", "enum": ["pending", "in_progress", "completed", "cancelled"]}),
            ),
            (
                "selected_model_id",
                "执行此任务使用的模型 ID。见系统提示词「模型选择策略」。若任务结果不满意，可通过 todo_modify 更换更强的模型 ID 后重试。",
                "Model ID for this task. See 'Model Selection Strategy' in system prompt. If task result is unsatisfactory, update via todo_modify and retry.",
                json!({"type": "string"}),
            ),
        ],
        lang,
    )
}

fn get_todo_create_input_params(lang: &str) -> Value {
    let item_props = todo_item_props(lang);
    let create_item = schema(
        pick_props(
            &item_props,
            &[
                "id",
                "content",
                "activeForm",
                "description",
                "selected_model_id",
            ],
        ),
        &["id", "content", "activeForm", "description"],
    );
    schema(
        prop_table(
            &[(
                "tasks",
                TODO_CREATE_TASKS_CN,
                TODO_CREATE_TASKS_EN,
                json!({"type": "array", "items": create_item}),
            )],
            lang,
        ),
        &["tasks"],
    )
}

fn get_todo_list_input_params(_lang: &str) -> Value {
    schema(json!({}), &[])
}

fn get_todo_modify_input_params(lang: &str) -> Value {
    let item_props = todo_item_props(lang);
    let todo_item_schema = schema(
        pick_props(
            &item_props,
            &[
                "id",
                "content",
                "activeForm",
                "description",
                "status",
                "selected_model_id",
            ],
        ),
        &["id"],
    );
    let target_id_desc = if lang == "en" {
        "Target task ID"
    } else {
        "目标任务 ID"
    };
    let items_desc = if lang == "en" {
        "Tasks to insert"
    } else {
        "要插入的任务列表"
    };
    schema(
        prop_table(
            &[
                (
                    "action",
                    "要执行的操作类型",
                    "Operation type to perform",
                    json!({"type": "string", "enum": ["update", "delete", "cancel", "append", "insert_after", "insert_before"]}),
                ),
                (
                    "ids",
                    "要操作的任务 ID 列表",
                    "List of task IDs to operate on",
                    json!({"type": "array", "items": {"type": "string"}}),
                ),
                (
                    "todos",
                    TODO_MODIFY_TODOS_CN,
                    TODO_MODIFY_TODOS_EN,
                    json!({"type": "array", "items": todo_item_schema.clone()}),
                ),
                (
                    "todo_data",
                    "用于 insert_after/insert_before 操作的对象",
                    "Object for insert_after/insert_before actions",
                    json!({
                        "type": "object",
                        "properties": {
                            "target_id": {"type": "string", "description": target_id_desc},
                            "items": {"type": "array", "description": items_desc, "items": todo_item_schema},
                        },
                        "required": ["target_id", "items"],
                    }),
                ),
            ],
            lang,
        ),
        &["action"],
    )
}

fn get_todo_get_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[(
                "id",
                "任务唯一标识符",
                "Unique task identifier",
                json!({"type": "string"}),
            )],
            lang,
        ),
        &["id"],
    )
}

// ---------------------------------------------------------------------------
// task_tool.py
// ---------------------------------------------------------------------------

const TASK_TOOL_CN: &str = r#"启动新的子代理，自主处理复杂、多步骤任务。

task_tool 启动专门的子代理来自主处理复杂任务。每种子代理类型都有特定的能力和可用工具。

可用代理类型及其工具：

{available_agents}

使用 task_tool 时，请通过 subagent_type 参数选择要使用的子代理类型。使用 task_tool 时，必须显式指定 subagent_type，且其值必须与上方「可用代理类型」列表中的名称完全一致。
当 subagent_type 为 "browser_agent" 时，还必须指定 browser_capabilities，并从上方浏览器能力目录中选择额外能力类别。仅使用核心浏览器能力时传入空列表；其他子代理类型不要提供 browser_capabilities。

何时不使用 task_tool：

- 如果你想读取某个具体文件路径，直接用 read_file 或 glob，比用 task_tool 更快
- 如果你在查找某个具体的类定义（如 "class Foo"），直接用 grep 或 glob，比用 task_tool 更快
- 如果你在 2-3 个特定文件内搜索代码，直接用 read_file，比用 task_tool 更快
- 其他与上述代理描述无关的任务

使用注意事项：

- task_description 应包含完整的上下文信息——子代理没有本次对话的任何记忆
- 子代理完成后会返回一条消息给你。该结果对用户不可见。如需向用户展示结果，你应发送一条文字消息，简明总结子代理的结果。
- 每次 task_tool 调用都是全新启动——请提供完整的任务描述。
- 子代理的输出通常应当被信任。
- 明确告知子代理你期望它写代码还是仅做调研（搜索、读文件、抓取网页等），因为它不知道用户的意图。
- 如果子代理的描述中提到应主动使用它，你应尽量在用户没有明确要求时就使用它。自行判断。
- 如果用户明确要求"并行"运行子代理，你必须在同一条消息中发出多个 task_tool 调用。例如，如果你需要同时启动 build-validator 子代理和 test-runner 子代理，请在同一条消息中发出两个 tool 调用。

## 如何写好任务描述

像给一位刚走进房间的聪明同事做简报一样描述任务——子代理没看过本次对话，不知道你尝试过什么，也不理解这个任务为什么重要。
- 说明你想达成什么目标以及为什么。
- 描述你已经了解到的信息或已排除的可能性。
- 提供足够的问题背景，让子代理能够自主判断，而不是只能机械执行狭窄的指令。
- 如果需要简短回复，请明确说明（"在 200 字内汇报"）。
- 查找类任务：直接给出精确命令。调研类任务：给出要回答的问题——规定好的步骤在前提不成立时只会成为累赘。

简短的命令式描述会产生浅层、泛泛的结果。

**永远不要委托理解。** 不要写"根据你的发现修复 bug"或"根据调研结果实现它"。这类说法把综合分析推给了子代理，而不是你自己完成。写出的描述应能证明你已经理解了问题：包含文件路径、行号、具体需要修改的内容。
"#;

const TASK_TOOL_EN: &str = r#"Launch a new subagent to handle complex, multi-step tasks autonomously.

The task_tool launches specialized subagents that autonomously handle complex tasks. Each subagent type has specific capabilities and tools available to it.

Available subagent types and the tools they have access to:

{available_agents}

When using the task_tool, specify a subagent_type parameter to select which subagent type to use. The value must exactly match one of the names listed above under "Available subagent types".
When subagent_type is "browser_agent", you must also provide browser_capabilities as a list of additional capability categories selected from the browser capability catalog above. Use an empty list for a core-only browser task. Do not provide browser_capabilities for other subagent types.

When NOT to use the task_tool:

- If you want to read a specific file path, use read_file or glob instead of the task_tool, to find the match more quickly
- If you are searching for a specific class definition like "class Foo", use grep or glob instead, to find the match more quickly
- If you are searching for code within a specific file or set of 2-3 files, use read_file instead of the task_tool, to find the match more quickly
- Other tasks that are not related to the subagent descriptions above

Usage notes:

- Provide a thorough task_description with full context — the subagent starts with no memory of this conversation
- When the subagent is done, it will return a single message back to you. The result returned by the subagent is not visible to the user. To show the user the result, you should send a text message back to the user with a concise summary of the result.
- Each task_tool invocation starts fresh — provide a complete task description.
- The subagent's outputs should generally be trusted.
- Clearly tell the subagent whether you expect it to write code or just to do research (search, file reads, web fetches, etc.), since it is not aware of the user's intent.
- If the subagent description mentions that it should be used proactively, then you should try your best to use it without the user having to ask for it first. Use your judgement.
- If the user specifies that they want you to run subagents "in parallel", you MUST send a single message with multiple task_tool calls. For example, if you need to launch both a build-validator subagent and a test-runner subagent in parallel, send a single message with both tool calls.

## Writing the prompt

Brief the subagent like a smart colleague who just walked into the room — it hasn't seen this conversation, doesn't know what you've tried, doesn't understand why this task matters.
- Explain what you're trying to accomplish and why.
- Describe what you've already learned or ruled out.
- Give enough context about the surrounding problem that the subagent can make judgment calls rather than just following a narrow instruction.
- If you need a short response, say so ("report in under 200 words").
- Lookups: hand over the exact command. Investigations: hand over the question — prescribed steps become dead weight when the premise is wrong.

Terse command-style prompts produce shallow, generic work.

**Never delegate understanding.** Don't write "based on your findings, fix the bug" or "based on the research, implement it." Those phrases push synthesis onto the subagent instead of doing it yourself. Write prompts that prove you understood: include file paths, line numbers, what specifically to change.
"#;

fn get_task_tool_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "subagent_type",
                    "子代理类型（必填，须与上方「可用代理类型」列表中的名称完全一致；不要使用列表中未出现的名称）",
                    "Required subagent type; must exactly match a name from the \"Available subagent types\" list above. Do not use any name that is not listed there.",
                    json!({"type": "string"}),
                ),
                (
                    "task_description",
                    "任务描述（必填，需包含完整上下文）",
                    "Required task description with full context for the subagent",
                    json!({"type": "string"}),
                ),
                (
                    "browser_capabilities",
                    "浏览器子代理所需的额外能力类别列表；仅使用核心能力时传入空列表",
                    "Additional capability categories required by browser_agent; use an empty list for core-only tasks",
                    json!({"type": "array", "items": {"type": "string"}}),
                ),
            ],
            lang,
        ),
        &["subagent_type", "task_description"],
    )
}

// ---------------------------------------------------------------------------
// goal.py
// ---------------------------------------------------------------------------

const SUBMIT_GOAL_REPORT_CN: &str = "仅当本次用户消息中包含 <goal_task> 标签（即处于 Goal 执行轮）时才调用本工具；普通对话轮（无 <goal_task>）绝不要调用本工具。本工具用于提交当前 goal 尝试的结构化结果。这是本次 goal 尝试的最终工具动作；工具调用成功后不要再调用其他工具，已有进展但目标仍可继续推进时使用 continue；只有具备可审计完成证据时才使用 complete；只有缺少用户输入、权限、依赖、外部服务或环境状态且无法继续任何有意义推进时才使用 blocked。";

const SUBMIT_GOAL_REPORT_EN: &str = "Only call this tool when the current user message contains a <goal_task> tag (i.e. this is a Goal execution round); never call it on a normal turn without <goal_task>. Submit a structured result for the current goal attempt. This is the final tool action for the current goal attempt; after the tool call succeeds, do not call any other tools. Use continue when progress has been made but the objective can still be advanced; use complete only with auditable completion evidence; use blocked only when user input, permissions, dependencies, external services, or environment state prevent any meaningful progress.";

const GET_CURRENT_GOAL_CN: &str = "查询当前会话的持续目标（goal）。当你不确定当前 goal 任务是什么、或用户提到「目标 / 继续目标 / 那个任务」时调用。返回 objective、status、attempt_count 与上一次评估（last_assessment）。";

const GET_CURRENT_GOAL_EN: &str = "Query the current session's persistent goal. Call this when you are unsure what the goal is, or when the user refers to \"the goal / continue the goal / that task\". Returns objective, status, attempt_count, and the last assessment.";

fn get_submit_goal_report_input_params(lang: &str) -> Value {
    schema(
        prop_table(
            &[
                (
                    "status",
                    "当前 goal 尝试结果：continue、complete 或 blocked。目标尚未完成但仍可推进时，不要报告 blocked。",
                    "Current goal attempt result: continue, complete, or blocked. Do not report blocked when the objective is incomplete but can still be advanced.",
                    json!({"type": "string", "enum": ["continue", "complete", "blocked"]}),
                ),
                (
                    "evidence",
                    "支持该状态的可审计证据，例如测试输出、命令结果、文件路径、工具结果、完成证明，或确切阻塞原因。",
                    "Auditable evidence supporting this status, e.g. test output, command results, file paths, tool results, proof of completion, or exact blocking reason.",
                    json!({"type": "string"}),
                ),
                (
                    "remaining_work",
                    "目标未完成时说明剩余工作；完成时可以为空。",
                    "Remaining work when the objective is not yet met; may be empty when complete.",
                    json!({"type": "string"}),
                ),
                (
                    "next_instruction",
                    "status=continue 时给出下一次尝试的具体、可执行指令；其他状态可以为空。",
                    "Specific, actionable instruction for the next attempt when status=continue; may be empty for other statuses.",
                    json!({"type": "string"}),
                ),
            ],
            lang,
        ),
        &["status", "evidence"],
    )
}

fn get_get_current_goal_input_params(_lang: &str) -> Value {
    schema(json!({}), &[])
}

// ---------------------------------------------------------------------------
// 汇总
// ---------------------------------------------------------------------------

/// 返回全部记忆/任务工具元数据(与 Python 素材 1:1 对齐)。
pub fn metadata() -> Vec<ToolMetadata> {
    vec![
        // memory.py
        meta(
            "memory_search",
            MEMORY_SEARCH_CN,
            MEMORY_SEARCH_EN,
            get_memory_search_input_params,
        ),
        meta(
            "memory_get",
            MEMORY_GET_CN,
            MEMORY_GET_EN,
            get_memory_get_input_params,
        ),
        meta(
            "write_memory",
            WRITE_MEMORY_CN,
            WRITE_MEMORY_EN,
            get_write_memory_input_params,
        ),
        meta(
            "edit_memory",
            EDIT_MEMORY_CN,
            EDIT_MEMORY_EN,
            get_edit_memory_input_params,
        ),
        meta(
            "read_memory",
            READ_MEMORY_CN,
            READ_MEMORY_EN,
            get_read_memory_input_params,
        ),
        // coding_memory.py
        meta(
            "coding_memory_read",
            CODING_MEMORY_READ_CN,
            CODING_MEMORY_READ_EN,
            get_coding_memory_read_input_params,
        ),
        meta(
            "coding_memory_write",
            CODING_MEMORY_WRITE_CN,
            CODING_MEMORY_WRITE_EN,
            get_coding_memory_write_input_params,
        ),
        meta(
            "coding_memory_edit",
            CODING_MEMORY_EDIT_CN,
            CODING_MEMORY_EDIT_EN,
            get_coding_memory_edit_input_params,
        ),
        // compression_recall.py
        meta(
            "recall_compressed_context",
            RECALL_COMPRESSED_CONTEXT_CN,
            RECALL_COMPRESSED_CONTEXT_EN,
            get_recall_compressed_context_input_params,
        ),
        // session_tools.py
        meta(
            "sessions_list",
            SESSIONS_LIST_CN,
            SESSIONS_LIST_EN,
            get_sessions_list_input_params,
        ),
        meta(
            "sessions_spawn",
            SESSIONS_SPAWN_CN,
            SESSIONS_SPAWN_EN,
            get_sessions_spawn_input_params,
        ),
        meta(
            "sessions_cancel",
            SESSIONS_CANCEL_CN,
            SESSIONS_CANCEL_EN,
            get_sessions_cancel_input_params,
        ),
        // todo.py
        meta(
            "todo_create",
            TODO_CREATE_CN,
            TODO_CREATE_EN,
            get_todo_create_input_params,
        ),
        meta(
            "todo_list",
            TODO_LIST_CN,
            TODO_LIST_EN,
            get_todo_list_input_params,
        ),
        meta(
            "todo_modify",
            TODO_MODIFY_CN,
            TODO_MODIFY_EN,
            get_todo_modify_input_params,
        ),
        meta(
            "todo_get",
            TODO_GET_CN,
            TODO_GET_EN,
            get_todo_get_input_params,
        ),
        // task_tool.py
        meta(
            "task_tool",
            TASK_TOOL_CN,
            TASK_TOOL_EN,
            get_task_tool_input_params,
        ),
        // goal.py
        meta(
            "submit_goal_report",
            SUBMIT_GOAL_REPORT_CN,
            SUBMIT_GOAL_REPORT_EN,
            get_submit_goal_report_input_params,
        ),
        meta(
            "get_current_goal",
            GET_CURRENT_GOAL_CN,
            GET_CURRENT_GOAL_EN,
            get_get_current_goal_input_params,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::tools::validate_provider;

    /// 期望的工具名列表(与 Python 素材各 MetadataProvider 一一对应)。
    const EXPECTED_NAMES: &[&str] = &[
        "memory_search",
        "memory_get",
        "write_memory",
        "edit_memory",
        "read_memory",
        "coding_memory_read",
        "coding_memory_write",
        "coding_memory_edit",
        "recall_compressed_context",
        "sessions_list",
        "sessions_spawn",
        "sessions_cancel",
        "todo_create",
        "todo_list",
        "todo_modify",
        "todo_get",
        "task_tool",
        "submit_goal_report",
        "get_current_goal",
    ];

    fn sorted_names() -> Vec<String> {
        let mut names: Vec<String> = metadata().iter().map(|m| m.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn every_tool_passes_validate_provider() {
        let failures: Vec<String> = metadata()
            .iter()
            .filter_map(|m| match validate_provider(m) {
                Ok(()) => None,
                Err(e) => Some(format!("{}: {}", m.name, e.0)),
            })
            .collect();
        assert!(failures.is_empty(), "validation failures: {failures:#?}");
    }

    #[test]
    fn tool_names_match_expected() {
        let mut expected: Vec<String> = EXPECTED_NAMES.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(sorted_names(), expected);
    }

    #[test]
    fn descriptions_are_nonempty_and_differ_between_languages() {
        for m in metadata() {
            assert!(
                !m.description_cn.trim().is_empty(),
                "{} cn description empty",
                m.name
            );
            assert!(
                !m.description_en.trim().is_empty(),
                "{} en description empty",
                m.name
            );
            assert_ne!(m.description_cn, m.description_en, "{} cn == en", m.name);
        }
    }

    #[test]
    fn at_least_one_schema_has_required_params() {
        let with_required = metadata()
            .iter()
            .filter(|m| {
                m.params_cn
                    .get("required")
                    .and_then(Value::as_array)
                    .is_some_and(|r| !r.is_empty())
            })
            .count();
        assert!(with_required > 0, "no tool schema declares required params");
        // 具体抽查:memory_search 必填 query,submit_goal_report 必填 status/evidence。
        let metas = metadata();
        let ms = metas.iter().find(|m| m.name == "memory_search").unwrap();
        assert_eq!(ms.params_cn["required"], json!(["query"]));
        let goal = metas
            .iter()
            .find(|m| m.name == "submit_goal_report")
            .unwrap();
        assert_eq!(goal.params_cn["required"], json!(["status", "evidence"]));
    }

    #[test]
    fn bilingual_schemas_have_identical_structure() {
        // validate_provider 已保证 properties key 集合一致;这里再抽查嵌套场景。
        let metas = metadata();
        let t = metas.iter().find(|m| m.name == "todo_modify").unwrap();
        let cn_props = t.params_cn["properties"].as_object().unwrap();
        let en_props = t.params_en["properties"].as_object().unwrap();
        let cn_keys: Vec<&String> = cn_props.keys().collect();
        let en_keys: Vec<&String> = en_props.keys().collect();
        assert_eq!(cn_keys, en_keys);
        // todo_data 嵌套对象内部属性名一致。
        let cn_td = cn_props["todo_data"]["properties"].as_object().unwrap();
        let en_td = en_props["todo_data"]["properties"].as_object().unwrap();
        let cn_td_keys: Vec<&String> = cn_td.keys().collect();
        let en_td_keys: Vec<&String> = en_td.keys().collect();
        assert_eq!(cn_td_keys, en_td_keys);
    }

    #[test]
    fn idempotent_is_false_for_all_tools() {
        assert!(metadata().iter().all(|m| !m.idempotent));
    }
}
