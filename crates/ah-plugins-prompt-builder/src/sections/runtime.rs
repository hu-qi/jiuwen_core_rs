//! 运行时 section 构建器(heartbeat/memory/coding_memory/prompt_attachments/offload/reload/compression_recall)。
//!
//! 素材与 Python 源码逐字对齐(build_*_section 的常量部分);
//! 动态 section(workspace 目录扫描 / context 配置文件读取)留待后续。

use ah_contracts::prompt_builder::{PromptSection, section_name};
use std::collections::BTreeMap;

// --- heartbeat ---
pub const HEARTBEAT_SYSTEM_PROMPT_CN: &str = r#"
## 心跳检测

判定规则：
1. 若 `<heartbeat_user_task>` 与 `</heartbeat_user_task>` 之间仅有空白（含空行）或完全为空：视为**无心跳用户任务**。你必须且仅能输出一行，内容**精确**为 `HEARTBEAT_OK`（不要解释、不要前后缀、不要 Markdown、不要工具调用说明）。
2. 若 `<heartbeat_user_task>` 与 `</heartbeat_user_task>` 之间存在**任意非空白字符**：该段即用户下发的心跳任务正文。你必须**完整阅读并执行**其中的指令并给出**直接回答**；**禁止**在回复中出现 `HEARTBEAT_OK` 四字（含单独一行、前缀、后缀、用标点或破折号拼接，例如 `HEARTBEAT_OK — …` 一律视为违规）。

系统**仅在**满足上一条规则 1（标签内无任务）时，才把单独一行的 `HEARTBEAT_OK` 视为心跳确认；有任务时**不得**为了「确认心跳」而输出或附带 `HEARTBEAT_OK`。
**禁止**用「心跳任务已完成」「任务已完成 ✓」「已处理」「无新内容」「安静待着」等**状态话术**代替任务正文所要求的**具体可核验输出**（若正文要求输出某文本，回复中必须出现该文本本身，而不是完成声明）。

重要约束：
- 每一轮心跳调用都是独立调度，只要 `<heartbeat_user_task>` 标签内有非空白任务正文，你就必须**当场**按正文完成指令所要求的动作或输出。**禁止**以「上一轮刚执行过」等理由省略执行或把「记录」当成完成——**记录不等于执行**。
- 若需修改 HEARTBEAT.md 文件，禁止给原本没有 <!-- --> 注释的内容添加注释标记
- 非注释文本仅可在用户明确要求时修改或删除，否则必须保持原样
- 心跳执行结果必须直接返回；除非心跳内容明确要求更新 HEARTBEAT.md，不要写入 daily memory 或其他记忆文件
"#;
pub const HEARTBEAT_SYSTEM_PROMPT_EN: &str = r#"
## Heartbeat

Decision rules:
1. If between `<heartbeat_user_task>` and `</heartbeat_user_task>` there is only whitespace (including blank lines) or nothing: treat as **no heartbeat user task**. You MUST output exactly one line whose content is **precisely** `HEARTBEAT_OK` (no explanation, no prefix/suffix, no Markdown, no tool narration).
2. If there is **any** non-whitespace character between `<heartbeat_user_task>` and `</heartbeat_user_task>`: that span is the **user-issued heartbeat task body**. You MUST read and carry out the instructions in full and reply **directly**; the substring `HEARTBEAT_OK` MUST NOT appear anywhere in your reply (not alone, not as a prefix/suffix, not glued with punctuation or em dashes—e.g. `HEARTBEAT_OK — …` is forbidden).

The system treats a single line of exactly `HEARTBEAT_OK` **only** under rule 1 (empty task) as the heartbeat acknowledgment; when there is a task body, you MUST NOT emit or append `HEARTBEAT_OK` “to confirm the heartbeat”.
You MUST NOT replace substantive output with status-only phrases such as “heartbeat task completed”, “task done ✓”, “nothing new”, “stay quiet”, etc. If the body asks for specific text, that text MUST appear in the reply itself, not a declaration that you completed it.

Important Constraints:
- Each heartbeat invocation is scheduled independently; whenever the `<heartbeat_user_task>` tags contain a non-empty task body, you MUST **on the spot** complete whatever actions or outputs the body requires. You MUST NOT skip execution or treat “logging/recording” as completion with excuses such as “already executed last round”—**recording is not execution**.
- When modifying HEARTBEAT.md, DO NOT add <!-- --> comment markers to content that originally had no such markers
- Non-commented text may only be modified or deleted when explicitly requested by the user; otherwise preserve it as-is
- Return heartbeat execution results directly; unless heartbeat content explicitly asks you to update HEARTBEAT.md, do not write them to daily memory or other memory files
"#;

pub fn build_heartbeat_section() -> PromptSection {
    let mut content = BTreeMap::new();
    content.insert("cn".to_string(), HEARTBEAT_SYSTEM_PROMPT_CN.to_string());
    content.insert("en".to_string(), HEARTBEAT_SYSTEM_PROMPT_EN.to_string());
    PromptSection::new(section_name::HEARTBEAT, content, 100)
}

// --- memory ---
pub const MEMORY_PROMPT_CN: &str = r#"# 记忆使用策略（主动模式）

每轮对话默认不包含历史记忆正文。跨会话信息依赖工作区记忆文件。涉及今天、昨天、之前、上次、继续、历史、偏好、用户画像、长期背景、项目进展等上下文时，先调用 `memory_search` 或 `read_memory` 获取事实，再回答或行动。

## 存储层级

- `IDENTITY.md`：Agent 自身身份、名字、角色定位、用户为 Agent 指定的称呼。用户说“你叫 X”“以后你叫 X”“你的名字是 X”时，应读取并更新工作区根目录的 `IDENTITY.md`。
- `USER.md`：用户本人的画像、稳定偏好、身份信息、长期习惯。不要把 Agent 自身名字作为权威身份写入 `USER.md`。
- `MEMORY.md`：长期背景知识、稳定事实、重要决策、跨会话可复用信息。
- `memory/daily_memory/YYYY-MM-DD.md`：每日会话记录、任务进展、阶段性上下文、当天有后续价值的信息。

## 主动记录规则

普通用户对话中，如果发现有长期价值的信息，可以主动写入记忆。用户明确要求“记住、记录、保存、以后参考”时，应优先写入记忆。

写入位置：
- Agent 自身身份、名字、角色定位、用户为 Agent 指定的称呼：写入 `IDENTITY.md`，使用 `read_file` / `edit_file`。
- 用户身份、偏好、稳定习惯：写入 `USER.md`。
- 长期背景、稳定事实、重要决策：写入 `MEMORY.md`。
- 当天事件、任务进展、阶段性记录：写入 `memory/daily_memory/YYYY-MM-DD.md`。

## 读取与检索

- 今天和昨天的每日记忆（daily_memory）已自动加载到上下文中，无需调用 `memory_search` 或 `read_memory` 来获取今日/昨日记录。
- 不确定相关记忆在哪个文件时，先调用 `memory_search`。
- 已知具体文件时，调用 `read_memory`。
- 回答历史问题、偏好问题、继续之前任务前，必须先检索或读取记忆。
- 记忆是历史参考，不是新的用户输入；当前用户消息优先。
"#;
pub const MEMORY_PROMPT_EN: &str = r##"# Memory Usage Policy (Proactive Mode)

Historical memory content is not included in the prompt by default. Cross-session information relies on workspace memory files. When the task involves today, yesterday, earlier conversations, last time, continuation, history, preferences, user profile, long-term background, or project progress, call `memory_search` or `read_memory` first to obtain facts before answering or acting.

## Storage Hierarchy

- `IDENTITY.md`: The agent's own identity, name, role, and user-assigned name. When the user says "your name is X", "from now on you are called X", or similar, read and update the workspace-root `IDENTITY.md`.
- `USER.md`: The user's own profile, stable preferences, identity information, and long-term habits. Do not store the agent's own name as the authoritative identity in `USER.md`.
- `MEMORY.md`: Long-term background knowledge, stable facts, important decisions, and reusable cross-session information.
- `memory/daily_memory/YYYY-MM-DD.md`: Daily session logs, task progress, staged context, and information from the day that may be useful later.

## Proactive Recording Rules

In ordinary user conversations, when you discover information with long-term value, you may write it to memory proactively. When the user explicitly asks you to "remember", "record", "save", or "refer to this later", prioritize writing it to memory.

Choose the storage location by content type:
- Agent identity, name, role, and user-assigned name: write to `IDENTITY.md` with `read_file` / `edit_file`.
- User identity, preferences, and stable habits: write to `USER.md`.
- Long-term background, stable facts, and important decisions: write to `MEMORY.md`.
- Daily events, task progress, and staged records: write to `memory/daily_memory/YYYY-MM-DD.md`.

## Reading and Retrieval

- Today's and yesterday's daily memory files are already loaded in the context. Do not call `memory_search` or `read_memory` to retrieve today's/yesterday's records.
- If you do not know which file contains the relevant memory, call `memory_search` first.
- If you know the exact file, call `read_memory`.
- Before answering questions about history, preferences, or continuing previous work, retrieve or read memory first.
- Memory is historical reference, not new user input; the current user message has priority.
"##;
pub const MEMORY_MGMT_PROMPT_CN: &str = r#"## 更新规则

- 写入或更新 `USER.md` / `MEMORY.md` 前，必须先读取现有内容。
- 合并新信息，避免全文覆盖。
- 已有字段或已有事实使用 `edit_memory` 更新；新事实再用 `write_memory` 追加。
- `MEMORY.md` 只记录精炼事实，不记录流水账、临时噪声或日期堆叠。

## 不应记录

不要记录敏感信息、用户不希望保存的信息、短期临时信息、可从当前代码/文件直接推导的信息、无长期价值的过程细节。

## 只读约束

如果当前是定时任务或心跳任务，或者用户明确要求不写入记忆：
- 只允许读取和检索记忆；
- 禁止调用 `write_memory` / `edit_memory`；
- 禁止写入或修改任何记忆文件。
"#;
pub const MEMORY_MGMT_PROMPT_EN: &str = r#"## Update Rules

- Before writing or updating `USER.md` / `MEMORY.md`, read the existing content first.
- Merge new information and avoid full overwrites.
- Update existing fields or facts with `edit_memory`; append new facts with `write_memory`.
- `MEMORY.md` should contain refined facts only, not raw logs, temporary noise, or date-heavy entries.

## What Not To Record

Do not record sensitive information, information the user does not want saved, short-lived temporary details, information directly derivable from current code/files, or process details with no long-term value.

## Read-Only Constraint

If the current run is a scheduled task or heartbeat task, or the user explicitly asks not to write memory:
- Only read and retrieve memories.
- Do not call `write_memory` or `edit_memory`.
- Do not write or modify any memory file.
"#;
pub const MEMORY_DATE_PROMPT_CN: &str = r#"## 每日记忆路径

操作每日会话记录时，使用 `memory/daily_memory/YYYY-MM-DD.md` 路径格式。只有在实际调用记忆工具时，才根据当前任务上下文确定具体日期。
"#;
pub const MEMORY_DATE_PROMPT_EN: &str = r#"## Daily Memory Path

When operating a daily session log, use the `memory/daily_memory/YYYY-MM-DD.md` path format. Resolve the actual date from the current task context only when you call the memory tool.
"#;
pub const MEMORY_INACTIVE_PROMPT_CN: &str = r##"# 记忆使用策略（被动模式）

每轮对话默认不包含历史记忆正文。只有在用户明确需要历史上下文，或明确要求保存信息时，才使用记忆工具。

## 存储层级

- `IDENTITY.md`：Agent 自身身份、名字、角色定位、用户为 Agent 指定的称呼。
- `USER.md`：用户本人的画像、稳定偏好、身份信息、长期习惯。不要把 Agent 自身名字作为权威身份写入 `USER.md`。
- `MEMORY.md`：长期背景知识、稳定事实、重要决策。
- `memory/daily_memory/YYYY-MM-DD.md`：每日会话记录、任务进展、阶段性上下文。

## 被动使用规则

- 今天和昨天的每日记忆（daily_memory）已自动加载到上下文中，无需调用 `memory_search` 或 `read_memory` 来获取今日/昨日记录。
- 只有当用户明确说"记住、记录、保存、以后参考"等含义时，才写入或修改记忆。
- 只有当用户询问“之前、上次、继续、历史、回忆、偏好”等内容，或回答确实依赖历史信息时，才调用 `memory_search` / `read_memory`。
- 普通闲聊、一次性任务、当前上下文足够回答的问题，不要调用记忆工具。
- 当前用户消息优先于历史记忆。

## 写入规则

- Agent 自身身份、名字、角色定位、用户为 Agent 指定的称呼：写入 `IDENTITY.md`，使用 `read_file` / `edit_file`。
- 用户身份、偏好、稳定习惯：写入 `USER.md`。
- 长期背景、稳定事实、重要决策：写入 `MEMORY.md`。
- 当天事件、任务进展、阶段性记录：写入 `memory/daily_memory/YYYY-MM-DD.md`。
- 更新前先读取现有内容，避免重复、冲突或覆盖。
- 已有字段或已有事实使用 `edit_memory` 更新；新事实再用 `write_memory` 追加。

## 不应记录

不要记录敏感信息、用户不希望保存的信息、短期临时信息、可从当前代码/文件直接推导的信息、无长期价值的过程细节。

"##;
pub const MEMORY_INACTIVE_PROMPT_EN: &str = r##"# Memory Usage Policy (Passive Mode)

Historical memory content is not included in the prompt by default. Use memory tools only when the user explicitly needs historical context or explicitly asks you to save information.

## Storage Hierarchy

- `IDENTITY.md`: The agent's own identity, name, role, and user-assigned name.
- `USER.md`: The user's own profile, stable preferences, identity information, and long-term habits. Do not store the agent's own name as the authoritative identity in `USER.md`.
- `MEMORY.md`: Long-term background knowledge, stable facts, and important decisions.
- `memory/daily_memory/YYYY-MM-DD.md`: Daily session logs, task progress, and staged context.

## Passive Usage Rules

- Today's and yesterday's daily memory files are already loaded in the context. Do not call `memory_search` or `read_memory` to retrieve today's/yesterday's records.
- Write or modify memory only when the user explicitly says "remember", "record", "save", "refer to this later", or similar.
- Call `memory_search` / `read_memory` only when the user asks about previous context, last time, continuation, history, recall, preferences, or when the answer genuinely depends on historical information.
- Do not call memory tools for casual conversation, one-off tasks, or questions that can be answered from the current context.
- The current user message has priority over historical memory.

## Write Rules

- Agent identity, name, role, and user-assigned name: write to `IDENTITY.md` with `read_file` / `edit_file`.
- User identity, preferences, and stable habits: write to `USER.md`.
- Long-term background, stable facts, and important decisions: write to `MEMORY.md`.
- Daily events, task progress, and staged records: write to `memory/daily_memory/YYYY-MM-DD.md`.
- Read existing content before updating to avoid duplication, conflicts, or overwrites.
- Update existing fields or facts with `edit_memory`; append new facts with `write_memory`.

## What Not To Record

Do not record sensitive information, information the user does not want saved, short-lived temporary details, information directly derivable from current code/files, or process details with no long-term value.

"##;

pub fn build_memory_section() -> PromptSection {
    let mut content = BTreeMap::new();
    content.insert("cn".to_string(), MEMORY_PROMPT_CN.to_string());
    content.insert("en".to_string(), MEMORY_PROMPT_EN.to_string());
    PromptSection::new(section_name::MEMORY, content, 100)
}

// --- coding_memory ---
pub const CODING_MEMORY_PROMPT_CN: &str = r#"# Coding Memory 使用策略

Coding memory 位于 `{memory_dir}`。Coding memory 内容不会自动注入系统提示词。涉及用户长期偏好、过往反馈、项目背景、外部系统引用、之前的工程决策或需要继续过去工作时，先调用 coding memory 工具获取事实，再回答或行动。

## 记忆类型

| 类型 | 记录内容 | 何时保存 |
|------|----------|----------|
| user | 用户角色、目标、技术背景、长期偏好 | 了解到用户身份、目标或稳定偏好时 |
| feedback | 用户对工作方式的纠正或认可 | 用户说“不要这样做”、指出问题或确认某种做法有效时 |
| project | 不能从代码直接推导的项目背景、截止日期、决策原因 | 了解到项目为什么这样做、谁负责、时间约束或历史决策时 |
| reference | 外部系统位置或使用线索 | 了解到 Jira、Grafana、Slack、文档、服务入口等外部资源时 |

`feedback` 和 `project` 类型的内容建议包含：规则/事实、原因、如何应用。

## 获取记忆

- 不要假设 coding memory 正文已经在上下文中。
- 当记忆可能相关，或用户提到之前的工作、上次反馈、历史决策、项目背景时，先读取相关记忆。
- 已知具体记忆文件时，调用 `coding_memory_read`。
- 当前用户指令优先于历史记忆；如果历史记忆较旧或与当前指令冲突，先按当前指令执行，并在需要时说明记忆可能已过期。
- 用户要求忽略记忆时，按没有相关记忆处理。

## 保存和更新记忆

- 新建记忆时，使用 `coding_memory_write` 写入独立 `.md` 文件，并包含 frontmatter：

      ---
      name: 记忆名称
      description: 一行具体描述
      type: user | feedback | project | reference
      ---

      记忆内容

- 修改已有记忆时，先用 `coding_memory_read` 读取原文，再用 `coding_memory_edit` 精确替换指定文本。
- 写入前先判断是否已有可更新的记忆，避免重复记录。
- 写入后系统会维护索引，不需要手动维护 `MEMORY.md`。

## 不应保存

- 可从代码、项目文件、README、文档或配置直接推导的信息。
- Git 历史、最近改动、调试过程或临时任务细节。
- 已经在项目文档中明确记录的信息。
- 敏感信息、用户不希望保存的信息、无长期价值的过程细节。

## 写入冲突处理

如果 `coding_memory_write` 返回 `conflict_detected: true` 或 `conflicting_files`：
- 使用 `coding_memory_read` 读取冲突文件。
- 使用 `coding_memory_edit` 更新已有记忆，或删除/替换过时内容。
- 不要在未检查冲突内容的情况下继续追加重复记忆。

## 只读约束

如果当前是定时任务或心跳任务，或者用户明确要求不写入记忆：
- 只允许读取 coding memory；
- 禁止调用 `coding_memory_write` / `coding_memory_edit`；
- 禁止写入或修改任何 coding memory 文件。
"#;
pub const CODING_MEMORY_PROMPT_EN: &str = r#"# Coding Memory Usage Policy

Coding memory is located at `{memory_dir}`. Coding memory content is not automatically injected into the system prompt. When the task involves long-term user preferences, previous feedback, project background, external system references, prior engineering decisions, or continuing past work, call coding memory tools first to obtain facts before answering or acting.

## Memory Types

| Type | What to record | When to save |
|------|----------------|--------------|
| user | User role, goals, technical background, and long-term preferences | When you learn the user's identity, goals, or stable preferences |
| feedback | User corrections or confirmations about your work style | When the user says not to do something, points out an issue, or confirms an approach worked |
| project | Project background, deadlines, and decision reasons not derivable from code | When you learn why the project works a certain way, who owns what, time constraints, or historical decisions |
| reference | External system locations or usage hints | When you learn about Jira, Grafana, Slack, docs, service entry points, or other external resources |

`feedback` and `project` memories should usually include: rule/fact, why, and how to apply it.

## Reading Memories

- Do not assume coding memory content is already in context.
- When memory may be relevant, or the user mentions prior work, previous feedback, historical decisions, or project background, read the relevant memory first.
- If you know the exact memory file, call `coding_memory_read`.
- Current user instructions have priority over historical memories. If a memory is old or conflicts with the current instruction, follow the current instruction and mention that the memory may be outdated when useful.
- If the user asks you to ignore memories, proceed as if no relevant memory exists.

## Saving and Updating Memories

- To create a memory, use `coding_memory_write` to write a standalone `.md` file with frontmatter:

      ---
      name: memory name
      description: one-line, specific description
      type: user | feedback | project | reference
      ---

      memory content

- To edit an existing memory, first use `coding_memory_read` to read the exact content, then use `coding_memory_edit` to replace specific text.
- Before writing, decide whether an existing memory should be updated instead, to avoid duplicates.
- The system maintains the index after writes; do not manually maintain `MEMORY.md`.

## What Not To Save

- Information directly derivable from code, project files, README, docs, or configuration.
- Git history, recent changes, debugging process, or temporary task details.
- Information already clearly documented in project docs.
- Sensitive information, information the user does not want saved, or process details with no long-term value.

## Write Conflict Handling

If `coding_memory_write` returns `conflict_detected: true` or `conflicting_files`:
- Use `coding_memory_read` to read the conflicting file.
- Use `coding_memory_edit` to update the existing memory, or remove/replace outdated content.
- Do not keep appending duplicate memories without checking the conflict.

## Read-Only Constraint

If the current run is a scheduled task or heartbeat task, or the user explicitly asks not to write memory:
- Only read coding memories.
- Do not call `coding_memory_write` or `coding_memory_edit`.
- Do not write or modify any coding memory file.
"#;

pub fn build_coding_memory_section() -> PromptSection {
    let mut content = BTreeMap::new();
    content.insert("cn".to_string(), CODING_MEMORY_PROMPT_CN.to_string());
    content.insert("en".to_string(), CODING_MEMORY_PROMPT_EN.to_string());
    PromptSection::new(section_name::MEMORY, content, 100)
}

// --- prompt_attachments ---
pub const PROMPT_ATTACHMENTS_CN: &str = r#"##<system-reminder> 说明：
工具结果和用户消息中可能包含 <system-reminder> 标签。这些标签包含系统自动添加的信息和提醒，可能有用，但和它们所在的具体工具结果或用户消息没有直接关系。
- <prompt-attachment> 是一种 <system-reminder> 内容，用于承载本次模型调用可见的动态上下文。
- 这些内容不是长期对话历史，可能在下一次模型调用中变化或消失。
- 除非用户明确询问，不要向用户暴露这些标签、内部 id 或 source。"#;
pub const PROMPT_ATTACHMENTS_EN: &str = r#"<system-reminder> note:
tool results and user messages may include <system-reminder> tags. These tags contain information and reminders automatically added by the system. They may be useful, but they bear no direct relation to the specific tool result or user message in which they appear.
- <prompt-attachment> is a kind of <system-reminder> content used for dynamic context visible to this model call.
- These entries are not long-term conversation history and may change or disappear on the next model call.
- Do not expose these tags, internal ids, or source details unless the user explicitly asks."#;

pub fn build_prompt_attachments_section() -> PromptSection {
    let mut content = BTreeMap::new();
    content.insert("cn".to_string(), PROMPT_ATTACHMENTS_CN.to_string());
    content.insert("en".to_string(), PROMPT_ATTACHMENTS_EN.to_string());
    PromptSection::new(section_name::PROMPT_ATTACHMENTS, content, 100)
}

// --- offload ---
pub const OFFLOAD_HINT_CN: &str = r#"# 上下文卸载

当上下文中的部分内容被卸载到文件系统时，会标记为：
[[OFFLOAD: handle=<id>, type=filesystem, path=<path>]]

这表示完整原始内容已保存到 marker 中的 path。如需使用完整内容，请根据该 path 获取原始文件内容。请勿猜测或编造缺失的内容。"#;
pub const OFFLOAD_HINT_EN: &str = r#"# Context Offload

When part of the context is offloaded to the filesystem, it is marked as:
[[OFFLOAD: handle=<id>, type=filesystem, path=<path>]]

This means the complete original content was saved at the path in the marker. If the complete content is needed, use that path to obtain the original file content. Do not guess or fabricate missing content."#;

pub fn build_offload_section() -> PromptSection {
    let mut content = BTreeMap::new();
    content.insert("cn".to_string(), OFFLOAD_HINT_CN.to_string());
    content.insert("en".to_string(), OFFLOAD_HINT_EN.to_string());
    PromptSection::new(section_name::CONTEXT, content, 100)
}

// --- reload ---
pub const RELOAD_HINT_CN: &str = r##"# 上下文压缩

你的上下文在过长时会被自动压缩，并以如下 marker 标记：

[[OFFLOAD: handle=<id>, type=<type>]]
[[OFFLOAD: handle=<id>, type=<type>, path=<path>]]

当你看到这类 marker，并且回答问题需要被隐藏的原始内容时，优先调用 read_file 工具读取 marker 中 path 指向的 offload 文件，从文件中恢复被压缩前的原始消息内容。

调用规则：
- 必须使用 marker 中 path= 后面的精确文件路径作为 read_file 的 file_path。
- handle 是 offload 内容的标识，不是文件路径；不要把 handle 当作 file_path。
- type 表示存储类型；只有 marker 中包含 path 字段时，才能通过 read_file 精确恢复原始内容。
- 如果 marker 没有 path 字段，不要猜测路径，也不要从 handle 或其他字段自行拼接路径；应说明无法通过 read_file 精确恢复原始内容。
- 如果 read_file 提示文件超过大小限制，请使用 offset 和 limit 分段读取同一个 path。
- 如果只需要确认或定位特定内容，优先使用搜索工具读取相关片段，不要盲目读取整个 offload 文件。

示例：看到 [[OFFLOAD: handle=abc123, type=filesystem, path=C:\\x\\MessageSummaryOffloader_abc123.json]] 时，应调用 read_file(file_path="C:\\x\\MessageSummaryOffloader_abc123.json")。

请勿猜测或编造缺失的内容。

存储类型："filesystem" 表示内容已持久化到 path 指向的文件；"in_memory" 表示会话缓存内容，如果 marker 没有 path，则无法通过 read_file 恢复。"##;
pub const RELOAD_HINT_EN: &str = r##"# Context Compression

Your context may be automatically compressed when it becomes too long and marked with one of these markers:

[[OFFLOAD: handle=<id>, type=<type>]]
[[OFFLOAD: handle=<id>, type=<type>, path=<path>]]

When you see one of these markers and the hidden original content would help, prefer calling read_file on the exact path in the marker to restore the original message content from the offload file.

Call rules:
- Use the exact value after path= as read_file.file_path.
- handle identifies the offloaded content, but it is not a file path; do not pass handle as file_path.
- type identifies the storage backend; only markers with a path field can be precisely restored with read_file.
- If the marker has no path field, do not guess, infer, or construct a path from handle or other fields; explain that read_file cannot precisely restore the original content.
- If read_file reports that the file exceeds the size limit, use offset and limit to read the same path in chunks.
- If you only need to confirm or locate specific content, prefer search tools for relevant portions instead of blindly reading the whole offload file.

Example: for [[OFFLOAD: handle=abc123, type=filesystem, path=C:\\x\\MessageSummaryOffloader_abc123.json]], call read_file(file_path="C:\\x\\MessageSummaryOffloader_abc123.json").

Do not guess or fabricate missing content.

Storage type "filesystem" means the content is persisted at path; "in_memory" means session-cache content and cannot be restored with read_file when no path is present."##;

pub fn build_reload_section() -> PromptSection {
    let mut content = BTreeMap::new();
    content.insert("cn".to_string(), RELOAD_HINT_CN.to_string());
    content.insert("en".to_string(), RELOAD_HINT_EN.to_string());
    PromptSection::new(section_name::CONTEXT, content, 100)
}

// --- compression_recall ---
pub const _HINTS_CN: &str = r#"# 压缩上下文召回

上下文中出现 `[[COMPRESSION_RECALL: id=<memory_id>]]` 标记，表示该处的历史消息已被压缩，压缩前的原始内容已归档。

当压缩后的摘要不足以回答当前问题、需要找回被压缩掉的内容时，调用 `recall_compressed_context` 工具：传入标记中的 `memory_id`，并用需要查找的信息作为 `query`。工具会从该标记对应的原始上下文中检索，返回与 query 相关的内容。

如果没有匹配到内容，可以更换关键词重试——使用同义词、换另一种语言、或使用原文中的标识符（如函数名、文件路径、错误码）。工具同时返回归档路径 `archive_path`，你可以根据需要自行使用。"#;
pub const _HINTS_EN: &str = r#"# Compressed Context Recall

A `[[COMPRESSION_RECALL: id=<memory_id>]]` marker in the context means the messages at that point have been compressed, and their original content has been archived.

When the compressed summary is insufficient and you need to retrieve what was compressed away, call the `recall_compressed_context` tool: pass the `memory_id` from the marker and use the information you are looking for as the `query`. The tool searches the original context corresponding to that marker and returns content relevant to the query.

If nothing matches, retry with different keywords — use synonyms, another language, or identifiers from the original text (such as function names, file paths, or error codes). The tool also returns the archive path as `archive_path`, which you may use as you see fit."#;

pub fn build_compression_recall_section() -> PromptSection {
    let mut content = BTreeMap::new();
    content.insert("cn".to_string(), _HINTS_CN.to_string());
    content.insert("en".to_string(), _HINTS_EN.to_string());
    PromptSection::new(section_name::CONTEXT, content, 100)
}
