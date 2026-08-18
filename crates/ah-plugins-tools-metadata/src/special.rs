//! 特殊工具元数据(与 harness/prompts/tools/*.py 1:1 对齐)。
//!
//! 覆盖 cron(含 legacy cron_* 工具)、ask_user、agent_mode(switch_mode /
//! enter_plan_mode / exit_plan_mode)、mcp(list_mcp_resources /
//! read_mcp_resource)、skill_tool、list_skill、load_tools、lsp 等特殊工具的
//! 双语描述与参数 JSON Schema。所有工具默认非幂等(idempotent=false),
//! 与 Python `ToolMetadataProvider.is_idempotent` 的默认行为一致。
//!
//! input_params 采用程序化构建(对齐 Python 的 get_*_input_params(language)),
//! 其中 cron/ask_user 的 schema 引用字段描述常量(如 ASK_USER_PARAMS 的等价表)。

use ah_contracts::tools::ToolMetadata;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// 双语辅助
// ---------------------------------------------------------------------------

/// 语言枚举(对齐 Python get_*_input_params(language) 的 "cn"/"en")。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Cn,
    En,
}

impl Lang {
    /// 取当前语言对应的字符串。
    fn pick(self, cn: &'static str, en: &'static str) -> &'static str {
        match self {
            Lang::Cn => cn,
            Lang::En => en,
        }
    }
}

/// 从 (key, cn, en) 描述表中查找字段描述;未命中返回空串(可由 validate 暴露)。
fn field_desc(
    table: &'static [(&'static str, &'static str, &'static str)],
    key: &str,
    lang: Lang,
) -> &'static str {
    for item in table {
        if item.0 == key {
            return lang.pick(item.1, item.2);
        }
    }
    ""
}

/// 组装一条 ToolMetadata(全部默认非幂等)。
fn tool_meta(
    name: &str,
    description_cn: &str,
    description_en: &str,
    params_cn: Value,
    params_en: Value,
) -> ToolMetadata {
    ToolMetadata {
        name: name.to_string(),
        description_cn: description_cn.to_string(),
        description_en: description_en.to_string(),
        params_cn,
        params_en,
        idempotent: false,
    }
}

// ---------------------------------------------------------------------------
// cron 工具(对齐 cron.py)
// ---------------------------------------------------------------------------

const CRON_DESCRIPTION_CN: &str = r#"使用 action 接口：status、list、add、update、remove、run、runs、wake，并兼容结构化 schedule/payload/delivery 字段。处理“2分钟后”“明天上午9点”“下周一”这类时间时，优先根据系统提示中已提供的当前日期与时间直接换算并调用 cron，不要为了简单的时间换算先调用 code 或 bash。创建一次性提醒时，schedule.at 默认直接使用用户当前本地时区偏移来写，例如 +08:00；除非用户明确要求，否则不要改写成 Z 或 UTC。给当前聊天创建提醒时，优先使用 payload.kind=systemEvent 和 sessionTarget=current。向用户确认创建结果时，优先按 schedule.at 里的原始时区/偏移表述，不要自行改写成 UTC。

【投递频道】delivery.channel / targets：用户未明确指定时不填，系统自动使用当前对话渠道；**禁止从历史记录推断。**

【强制：wake_offset_seconds】这是提前唤醒秒数，不是任务执行时间。除非用户明确说“提前 X 秒/分钟唤醒/准备/执行”，否则**禁止**传 wake_offset_seconds 字段；不要根据任务类型、提醒/查询场景、距离触发时间或历史记录推断为 60 或其他正值。用户未明确指定时必须省略该字段，后端会默认使用 0 秒（到点执行，不提前唤醒）。

【重要：cron 表达式格式】只支持7段式(Quartz格式)：秒 分 时 日 月 周 年。字段取值范围：秒(0-59)，分(0-59)，时(0-23)，日(1-31)，月(1-12)，周(1-7或?)，年(1970-2099或*)。日和周字段：不能同时指定具体值，其中一个必须用?表示'不指定'。年份字段：*表示跨年周期，固定年份只在该年执行。真正只执行一次：所有字段均为固定值(无*和?)，如'0 30 17 29 4 ? 2026'表示2026年4月29日17:30:00执行一次。例：每天9点 -> '0 0 9 * * ? *'；每15分钟 -> '0 */15 * * * ? *'；每周一9点 -> '0 0 9 ? * MON *'。注意：一次性任务建议优先使用 schedule.at (ISO8601格式)，cron更适合周期性任务。

【重要：cron 表达式限制】标准 cron 的 */X 语义是'当字段值能被 X 整除时触发'，而非'每隔 X 单位触发'。只有当周期单位能被 X 整除时，间隔才是均匀的。以下是各字段的限制：
- 秒/分(0-59)：*/X 仅支持 X 整除60的值：1/2/3/4/5/6/10/12/15/20/30。
  例如 */40 实际在每小时第0分和第40分触发（间隔40分→20分交替），并非每40分钟。
  用户要求'每隔40分钟'时，必须先告知此限制并让用户确认是否接受不均匀间隔，
  或建议改用整除60的间隔（如20分钟或30分钟）。未经用户确认不得直接创建。
- 小时(0-23)：*/X 仅支持 X 整除24的值：1/2/3/4/6/8/12。
  例如 */5 实际在每天0/5/10/15/20时触发（间隔5h→4h→5h→4h交替），并非每5小时。
  用户要求'每隔5小时'时，必须告知限制并让用户确认，或建议改用整除24的间隔。
- 日(1-31)：*/X 不可靠，因为不同月份天数不同（28/29/30/31）。
  例如 */15 在2月只触发1、15日（共2次），在31天月份触发1、16、31日（共3次）。
  用户要求'每隔X天'时，建议改用'每周X'或指定固定日期（如每月1号、15号）。
- 月(1-12)：*/X 仅支持 X 整除12的值：1/2/3/4/6。
  例如 */5 实际在1/5/10月触发，并非每5个月均匀触发。
- 周(1-7)：*/X 仅支持 X 整除7的值：1/7。1=SUN,7=SAT。
  例如 */2 实际在SUN/TUE/THU触发，并非'每隔2周'。
  用户要求'每隔2周'时，应直接指定具体星期几或建议简化为'每周一'。
处理'每隔X分钟/小时/天'需求时，务必检查 X 是否整除对应周期单位；若不整除，必须告知用户限制，让用户确认后再创建，或建议替代方案。"#;

const CRON_DESCRIPTION_EN: &str = r#"Use the cron action interface. Supports status, list, add, update, remove, run, runs, and wake using structured schedule/payload/delivery fields. For requests like 'in 2 minutes', 'tomorrow at 9am', or 'next Monday', prefer converting the time directly from the current date/time already provided in the system prompt and call cron directly instead of using code or bash for simple time math. When creating one-shot reminders, write schedule.at using the user's current local timezone offset directly, for example +08:00; unless the user explicitly asks for it, do not rewrite it into Z or UTC. For reminders targeting the current chat, prefer payload.kind=systemEvent with sessionTarget=current. When confirming a created reminder to the user, prefer the original timezone/offset from schedule.at instead of rewriting it into UTC.

[Delivery Channel] delivery.channel / targets: leave empty unless user explicitly specifies; system uses current channel. **Never infer from history.**

[MANDATORY: wake_offset_seconds] This is a compatibility wake-ahead offset, not the task execution time. Unless the user explicitly says to wake/prepare/run X seconds/minutes early, DO NOT pass wake_offset_seconds. Never infer 60 or any other positive value from task type, reminder/query scenarios, time until trigger, or history. When the user does not explicitly specify it, omit the field; the backend defaults to 0 seconds (run at the scheduled time, no early wake).

[CRITICAL: Cron Expression Format] Only supports 7-field Quartz format: second minute hour day month dow year. Field ranges: second(0-59), minute(0-59), hour(0-23), day(1-31), month(1-12), dow(1-7 or ?), year(1970-2099 or *). Day and dow fields: cannot both have specific values; one must be '?' (no specific value). Year field: '*' for recurring, fixed year for one-shot within that year. True one-shot: all fields fixed (no '*' or '?'), e.g. '0 30 17 29 4 ? 2026' runs once at 2026-04-29 17:30:00.Examples: daily 9am -> '0 0 9 * * ? *'; every 15min -> '0 */15 * * * ? *'; every Monday 9am -> '0 0 9 ? * MON *'. Note: for one-shot tasks, prefer schedule.at (ISO8601 format); cron is better for recurring tasks.

[CRITICAL: Cron Expression Limits] Standard cron's */X means 'trigger when the field value is divisible by X', NOT 'every X units'. Uniform intervals only work when the cycle unit is divisible by X. Field limits:
- Second/Minute(0-59): */X only works for X dividing 60: 1/2/3/4/5/6/10/12/15/20/30.
  Example: */40 triggers at minute 0 and 40 each hour (alternating 40min-20min gaps), NOT every 40 minutes.
  When user requests 'every 40 minutes', MUST inform user of this limitation first and let user confirm whether to accept uneven intervals, or suggest intervals that divide 60 (e.g. 20 or 30 minutes). Do NOT create without user confirmation.
- Hour(0-23): */X only works for X dividing 24: 1/2/3/4/6/8/12.
  Example: */5 triggers at hours 0/5/10/15/20 (alternating 5h-4h gaps), NOT every 5 hours.
  When user requests 'every 5 hours', MUST inform and let user confirm, or suggest 4 or 6 hours.
- Day(1-31): */X is unreliable due to varying month lengths (28/29/30/31 days).
  Example: */15 triggers on day 1,15 in Feb (2 times), but 1,16,31 in 31-day months (3 times).
  When user requests 'every X days', suggest using 'every week on day X' or fixed dates.
- Month(1-12): */X only works for X dividing 12: 1/2/3/4/6.
  Example: */5 triggers in Jan/May/Oct, NOT uniformly every 5 months.
- Dow(1-7): */X only works for X dividing 7: 1/7. 1=SUN, 7=SAT.
  Example: */2 triggers on SUN/TUE/THU, NOT 'every 2 weeks'.
  When user requests 'every 2 weeks', suggest simplifying to a specific weekday.
When handling 'every X minutes/hours/days' requests, always check if X divides the cycle unit. If not, MUST inform user of the limitation, let user confirm before creating, or suggest alternatives."#;

/// cron 字段描述表(对齐 cron.py FIELD_DESCRIPTIONS)。
const CRON_FIELDS: &[(&str, &str, &str)] = &[
    ("action", "要执行的 cron 操作", "Cron action to execute"),
    (
        "job",
        "用于 add 的任务对象；支持结构化字段和兼容层字段",
        "Job object for add; supports structured fields and compatibility fields",
    ),
    (
        "jobId",
        "用于 update/remove/run/runs 的任务 ID",
        "Job id used by update/remove/run/runs",
    ),
    (
        "patch",
        "用于 update 的补丁对象",
        "Patch object used by update",
    ),
    (
        "includeDisabled",
        "list 时是否包含已禁用任务",
        "Whether list should include disabled jobs",
    ),
    (
        "text",
        "wake 动作要发送的提示文本",
        "Wake text to inject for action=wake",
    ),
    ("mode", "wake 的触发模式", "Wake delivery mode"),
    (
        "contextMessages",
        "保留给上下文提示的兼容字段",
        "Reserved compatibility field for context hints",
    ),
    ("name", "任务名称", "Job name"),
    ("enabled", "任务是否启用", "Whether the job is enabled"),
    (
        "schedule",
        "结构化调度定义，支持 at/every/cron",
        "Structured schedule definition supporting at/every/cron",
    ),
    (
        "schedule.kind",
        "调度类型：at、every 或 cron",
        "Schedule type: at, every, or cron",
    ),
    (
        "schedule.at",
        "一次性执行时间，ISO 8601",
        "One-shot execution time in ISO 8601",
    ),
    (
        "schedule.everyMs",
        "循环间隔，毫秒",
        "Recurring interval in milliseconds",
    ),
    (
        "schedule.anchorMs",
        "every 调度的起始锚点毫秒时间戳",
        "Anchor timestamp in milliseconds for every schedules",
    ),
    (
        "schedule.expr",
        "cron表达式(Quartz格式)。7段式：秒 分 时 日 月 周 年。日和周：不能同时指定具体值，其中一个用?。年份：*跨年周期，固定年份只在该年执行。一次性：所有字段固定值，如'0 0 17 28 3 ? 2026'。例：每天9点'0 0 9 * * ? *'；每15分钟'0 */15 * * * ? *'。详见工具描述。",
        "Cron expression (Quartz format). 7-field: second minute hour day month dow year. Day/dow: cannot both be specific; use '?' for one. Year: '*' for recurring, fixed year limits to that year. One-shot: all fields fixed, e.g. '0 0 17 28 3 ? 2026'. Examples: daily 9am '0 0 9 * * ? *'; every 15min '0 */15 * * * ? *'. See tool description.",
    ),
    (
        "schedule.tz",
        "cron 调度使用的时区",
        "Timezone used by cron schedules",
    ),
    (
        "schedule.staggerMs",
        "cron 调度的可选抖动毫秒数",
        "Optional cron jitter in milliseconds",
    ),
    (
        "payload",
        "结构化任务负载，支持 systemEvent 或 agentTurn",
        "Structured job payload supporting systemEvent or agentTurn",
    ),
    (
        "payload.kind",
        "负载类型：systemEvent 或 agentTurn",
        "Payload type: systemEvent or agentTurn",
    ),
    (
        "payload.text",
        "systemEvent提醒文本。不要包含时间/频率信息（如'每隔40分钟'、'每天9点'）",
        "Reminder text for systemEvent. Do NOT include time/frequency info",
    ),
    (
        "payload.message",
        "agentTurn 发送给代理的消息",
        "Message sent to the agent for agentTurn payloads",
    ),
    (
        "payload.model",
        "agentTurn 可选模型覆盖",
        "Optional model override for agentTurn",
    ),
    (
        "payload.thinking",
        "agentTurn 的思考预算或模式",
        "Thinking mode or budget for agentTurn",
    ),
    (
        "payload.timeoutSeconds",
        "agentTurn 超时时间（秒）",
        "Timeout in seconds for agentTurn",
    ),
    (
        "payload.allowUnsafeExternalContent",
        "是否允许不安全的外部内容",
        "Whether unsafe external content is allowed",
    ),
    (
        "payload.lightContext",
        "是否使用轻量上下文执行",
        "Whether to run with lighter context",
    ),
    (
        "payload.deliver",
        "agentTurn 自带的投递策略字段",
        "Embedded delivery strategy field for agentTurn",
    ),
    (
        "payload.channel",
        "agentTurn 的默认投递频道",
        "Default delivery channel for agentTurn",
    ),
    (
        "payload.to",
        "agentTurn 的目标收件人",
        "Target recipient for agentTurn",
    ),
    (
        "payload.bestEffortDeliver",
        "是否最佳努力投递",
        "Whether delivery should be best effort",
    ),
    (
        "payload.fallbacks",
        "agentTurn 的回退投递列表",
        "Fallback delivery list for agentTurn",
    ),
    (
        "delivery",
        "提醒结果的投递方式",
        "How reminder output should be delivered",
    ),
    (
        "delivery.mode",
        "投递模式：none、announce 或 webhook",
        "Delivery mode: none, announce, or webhook",
    ),
    (
        "delivery.channel",
        "announce 模式投递频道。用户未明确指定时不填，系统自动使用当前对话渠道；**禁止从历史记录推断。**",
        "Delivery channel for announce mode. Leave empty unless user explicitly specifies; system uses current channel. **Never infer from history.**",
    ),
    (
        "delivery.to",
        "目标收件人或会话标识",
        "Target recipient or session identifier",
    ),
    (
        "delivery.accountId",
        "投递账号标识",
        "Account identifier for delivery",
    ),
    (
        "delivery.bestEffort",
        "announce/webhook 是否最佳努力投递",
        "Whether announce/webhook delivery is best effort",
    ),
    (
        "delivery.failureDestination",
        "失败时的兜底投递目标",
        "Fallback destination when delivery fails",
    ),
    (
        "sessionTarget",
        "会话目标：main、isolated、current 或 session:<id>",
        "Session target: main, isolated, current, or session:<id>",
    ),
    (
        "wakeMode",
        "唤醒模式：now 或 next-heartbeat",
        "Wake mode: now or next-heartbeat",
    ),
    (
        "deleteAfterRun",
        "执行后是否自动删除该任务",
        "Whether the job should be deleted after it runs",
    ),
    (
        "cron_expr",
        "兼容层cron表达式(Quartz格式)。7段式：秒 分 时 日 月 周 年。日和周：不能同时指定具体值，其中一个用?。年份：*跨年周期，固定年份只在该年执行。一次性：所有字段固定值，如'0 0 17 28 3 ? 2026'。例：每天9点'0 0 9 * * ? *'；每15分钟'0 */15 * * * ? *'。详见工具描述。",
        "Compatibility cron expression (Quartz format). 7-field: second minute hour day month dow year. Day/dow: cannot both be specific; use '?' for one. Year: '*' for recurring, fixed year limits to that year. One-shot: all fields fixed, e.g. '0 0 17 28 3 ? 2026'. Examples: daily 9am '0 0 9 * * ? *'; every 15min '0 */15 * * * ? *'. See tool description.",
    ),
    ("timezone", "兼容层时区字段", "Compatibility timezone field"),
    (
        "wake_offset_seconds",
        "提前唤醒秒数，不是任务执行时间。除非用户明确说“提前 X 秒/分钟唤醒/准备/执行”，否则禁止传该字段；不要根据任务类型、提醒/查询场景、距离触发时间或历史记录推断为 60 或其他正值。用户未明确指定时必须省略，后端默认 0 秒（到点执行，不提前唤醒）。",
        "Compatibility wake-ahead offset in seconds; not the task execution time. Unless the user explicitly says to wake/prepare/run X seconds/minutes early, do not pass this field. Do not infer 60 or any other positive value from task type, reminder/query scenarios, time until trigger, or history. Omit it when unspecified; the backend defaults to 0 seconds (run at the scheduled time, no early wake).",
    ),
    (
        "description",
        "具体任务内容，到点执行时发给助手。不要包含时间/频率信息（如'每隔40分钟'、'每天9点'）",
        "Task content sent to assistant at scheduled time. Do NOT include time/frequency info",
    ),
    (
        "targets",
        "目标频道。用户未明确指定时不填，系统自动使用当前对话渠道；**禁止从历史记录推断。**",
        "Target channel. Leave empty unless user explicitly specifies; system uses current channel. **Never infer from history.**",
    ),
];

fn cron_field(key: &str, lang: Lang) -> &'static str {
    field_desc(CRON_FIELDS, key, lang)
}

/// job 对象 schema(对齐 get_cron_job_input_params)。
fn cron_job_params(lang: Lang) -> Value {
    json!({
        "type": "object",
        "required": [],
        "additionalProperties": true,
        "properties": {
            "name": { "type": "string", "description": cron_field("name", lang) },
            "enabled": { "type": "boolean", "description": cron_field("enabled", lang) },
            "schedule": {
                "type": "object",
                "description": cron_field("schedule", lang),
                "required": [],
                "additionalProperties": true,
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["at", "every", "cron"],
                        "description": cron_field("schedule.kind", lang),
                    },
                    "at": { "type": "string", "description": cron_field("schedule.at", lang) },
                    "everyMs": { "type": "integer", "description": cron_field("schedule.everyMs", lang) },
                    "anchorMs": { "type": "integer", "description": cron_field("schedule.anchorMs", lang) },
                    "expr": { "type": "string", "description": cron_field("schedule.expr", lang) },
                    "tz": { "type": "string", "description": cron_field("schedule.tz", lang) },
                    "staggerMs": { "type": "integer", "description": cron_field("schedule.staggerMs", lang) },
                },
            },
            "payload": {
                "type": "object",
                "description": cron_field("payload", lang),
                "required": [],
                "additionalProperties": true,
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["systemEvent", "agentTurn"],
                        "description": cron_field("payload.kind", lang),
                    },
                    "text": { "type": "string", "description": cron_field("payload.text", lang) },
                    "message": { "type": "string", "description": cron_field("payload.message", lang) },
                    "model": { "type": "string", "description": cron_field("payload.model", lang) },
                    "thinking": { "type": "string", "description": cron_field("payload.thinking", lang) },
                    "timeoutSeconds": { "type": "integer", "description": cron_field("payload.timeoutSeconds", lang) },
                    "allowUnsafeExternalContent": {
                        "type": "boolean",
                        "description": cron_field("payload.allowUnsafeExternalContent", lang),
                    },
                    "lightContext": { "type": "boolean", "description": cron_field("payload.lightContext", lang) },
                    "deliver": { "type": "string", "description": cron_field("payload.deliver", lang) },
                    "channel": { "type": "string", "description": cron_field("payload.channel", lang) },
                    "to": { "type": "string", "description": cron_field("payload.to", lang) },
                    "bestEffortDeliver": {
                        "type": "boolean",
                        "description": cron_field("payload.bestEffortDeliver", lang),
                    },
                    "fallbacks": {
                        "type": "array",
                        "description": cron_field("payload.fallbacks", lang),
                        "items": { "type": "string", "description": cron_field("payload.fallbacks", lang) },
                    },
                },
            },
            "delivery": {
                "type": "object",
                "description": cron_field("delivery", lang),
                "required": [],
                "additionalProperties": true,
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["none", "announce", "webhook"],
                        "description": cron_field("delivery.mode", lang),
                    },
                    "channel": { "type": "string", "description": cron_field("delivery.channel", lang) },
                    "to": { "type": "string", "description": cron_field("delivery.to", lang) },
                    "accountId": { "type": "string", "description": cron_field("delivery.accountId", lang) },
                    "bestEffort": { "description": cron_field("delivery.bestEffort", lang) },
                    "failureDestination": { "description": cron_field("delivery.failureDestination", lang) },
                },
            },
            "sessionTarget": { "type": "string", "description": cron_field("sessionTarget", lang) },
            "wakeMode": {
                "type": "string",
                "enum": ["now", "next-heartbeat"],
                "description": cron_field("wakeMode", lang),
            },
            "deleteAfterRun": { "type": "boolean", "description": cron_field("deleteAfterRun", lang) },
            "cron_expr": { "type": "string", "description": cron_field("cron_expr", lang) },
            "timezone": { "type": "string", "description": cron_field("timezone", lang) },
            "wake_offset_seconds": {
                "type": "integer",
                "description": cron_field("wake_offset_seconds", lang),
                "default": 0,
            },
            "description": { "type": "string", "description": cron_field("description", lang) },
            "targets": { "type": "string", "description": cron_field("targets", lang) },
        },
    })
}

/// cron 顶层 schema(对齐 get_cron_input_params;job/patch 内嵌 job schema)。
fn cron_params(lang: Lang) -> Value {
    let mut job = cron_job_params(lang);
    job["description"] = json!(cron_field("job", lang));
    let mut patch = cron_job_params(lang);
    patch["description"] = json!(cron_field("patch", lang));
    json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["status", "list", "add", "update", "remove", "run", "runs", "wake"],
                "description": cron_field("action", lang),
            },
            "job": job,
            "jobId": { "type": "string", "description": cron_field("jobId", lang) },
            "patch": patch,
            "includeDisabled": { "type": "boolean", "description": cron_field("includeDisabled", lang) },
            "text": { "type": "string", "description": cron_field("text", lang) },
            "mode": {
                "type": "string",
                "enum": ["now", "next-heartbeat"],
                "description": cron_field("mode", lang),
            },
            "contextMessages": { "type": "integer", "description": cron_field("contextMessages", lang) },
        },
        "required": ["action"],
        "additionalProperties": true,
    })
}

// ---------------------------------------------------------------------------
// legacy cron 工具(对齐 cron.py 的 Legacy cron tools 段落)
// ---------------------------------------------------------------------------

const CRON_CREATE_JOB_DESCRIPTION_CN: &str = r#"创建新的 cron 定时任务，使用扁平字段（name, cron_expr, timezone, targets, description, wake_offset_seconds）。

【targets】用户未明确指定投递渠道时不填，系统自动使用当前对话渠道；**禁止从历史记录推断。**

【强制：wake_offset_seconds】除非用户明确说“提前 X 秒/分钟唤醒/准备/执行”，否则**禁止**传 wake_offset_seconds。
用户未明确指定时必须省略该字段，后端会默认使用 0 秒（到点执行，不提前唤醒）。
不要因为是查询/准备类任务而传 60 或其他正值；禁止从任务类型、距离触发时间或历史记录推断。

【重要：cron 表达式格式】只支持7段式(Quartz格式)：秒 分 时 日 月 周 年。
日和周字段：不能同时指定具体值，其中一个必须用?表示'不指定'。
年份字段：*表示跨年周期执行，固定年份只在该年执行。
真正只执行一次：所有字段均为固定值（无*和?），如'0 0 17 28 3 ? 2026'。
例：每天9点 -> '0 0 9 * * ? *'；每15分钟 -> '0 */15 * * * ? *'；每周一9点 -> '0 0 9 ? * MON *'。

【重要：cron 表达式限制】标准 cron 的 */X 语义是'当字段值能被 X 整除时触发'，而非'每隔 X 单位触发'。
只有当周期单位能被 X 整除时，间隔才是均匀的。以下是各字段的限制：
- 秒/分(0-59)：*/X 仅支持 X 整除60的值：1/2/3/4/5/6/10/12/15/20/30。
  例如 */40 实际在每小时第0分和第40分触发（间隔40分→20分交替），并非每40分钟。
  用户要求'每隔40分钟'时，必须先告知此限制并让用户确认是否接受不均匀间隔，
  或建议改用整除60的间隔（如20分钟或30分钟）。未经用户确认不得直接创建。
- 小时(0-23)：*/X 仅支持 X 整除24的值：1/2/3/4/6/8/12。
  例如 */5 实际在每天0/5/10/15/20时触发（间隔5h→4h→5h→4h交替），并非每5小时。
- 日(1-31)：*/X 不可靠，因为不同月份天数不同（28/29/30/31）。
- 月(1-12)：*/X 仅支持 X 整除12的值：1/2/3/4/6。
- 周(1-7)：*/X 仅支持 X 整除7的值：1/7。1=SUN,7=SAT。

处理'每隔X分钟/小时/天'需求时，务必检查 X 是否整除对应周期单位；
若不整除，必须告知用户限制，让用户确认后再创建，或建议替代方案。"#;

const CRON_CREATE_JOB_DESCRIPTION_EN: &str = r#"Create a new cron job using flat fields (name, cron_expr, timezone, targets, description, wake_offset_seconds).

[targets] Leave empty unless user explicitly specifies a channel; system uses current channel. **Never infer from history.**

[MANDATORY: wake_offset_seconds] Unless the user explicitly says to wake/prepare/run X seconds/minutes early,
DO NOT pass wake_offset_seconds. If the user does not specify it, omit the field; the backend defaults to 0 seconds
(run at the scheduled time, no early wake).
Do not pass 60 for query/preparation tasks, or any other inferred positive value from task type, time until trigger, or history.

[CRITICAL: Cron Expression Format] Only supports 7-field Quartz format: second minute hour day month dow year.
Day and dow fields: cannot both have specific values; one must be '?' (no specific value).
Year field: '*' for recurring, fixed year for one-shot within that year.
True one-shot: all fields fixed (no '*' or '?'), e.g. '0 0 17 28 3 ? 2026'.
Examples: daily 9am -> '0 0 9 * * ? *'; every 15min -> '0 */15 * * * ? *'; every Monday 9am -> '0 0 9 ? * MON *'.

[CRITICAL: Cron Expression Limits] Standard cron's */X means 'trigger when the field value is divisible by X',
NOT 'every X units'. Uniform intervals only work when the cycle unit is divisible by X. Field limits:
- Second/Minute(0-59): */X only works for X dividing 60: 1/2/3/4/5/6/10/12/15/20/30.
  Example: */40 triggers at minute 0 and 40 each hour (alternating 40min-20min gaps), NOT every 40 minutes.
  When user requests 'every 40 minutes', MUST inform user of this limitation first
  and let user confirm whether to accept uneven intervals, or suggest intervals that divide 60.
  Do NOT create without user confirmation.
- Hour(0-23): */X only works for X dividing 24: 1/2/3/4/6/8/12.
  Example: */5 triggers at hours 0/5/10/15/20 (alternating 5h-4h gaps), NOT every 5 hours.
- Day(1-31): */X is unreliable due to varying month lengths (28/29/30/31 days).
- Month(1-12): */X only works for X dividing 12: 1/2/3/4/6.
- Dow(1-7): */X only works for X dividing 7: 1/7. 1=SUN, 7=SAT.

When handling 'every X minutes/hours/days' requests, always check if X divides the cycle unit.
If not, MUST inform user and let user confirm before creating."#;

/// legacy cron 字段描述表(对齐 cron.py LEGACY_FIELD_DESCRIPTIONS)。
const LEGACY_CRON_FIELDS: &[(&str, &str, &str)] = &[
    (
        "job_id_to_look_up",
        "要查询的任务 ID",
        "The job ID to look up",
    ),
    ("job_id_to_update", "要更新的任务 ID", "Job ID to update"),
    ("job_id_to_delete", "要删除的任务 ID", "Job ID to delete"),
    ("job_id_to_toggle", "要启用/禁用的任务 ID", "Job ID"),
    ("job_id_to_preview", "要预览的任务 ID", "Job ID"),
    ("patch", "要更新的字段", "Fields to update"),
    ("enabled", "是否启用该任务", "Whether to enable the job"),
    (
        "count",
        "预览的执行次数（1-50，默认 5）",
        "Number of runs to preview (1-50, default 5)",
    ),
    ("name", "任务名称", "Job name"),
    (
        "cron_expr",
        "Cron表达式(Quartz格式)。7段式：秒 分 时 日 月 周 年。日和周：不能同时指定具体值，其中一个用?。年份：*跨年周期，固定年份只在该年执行。一次性：所有字段固定值，如'0 0 17 28 3 ? 2026'。例：每天9点'0 0 9 * * ? *'；每15分钟'0 */15 * * * ? *'。详见工具描述。",
        "Cron expression (Quartz format). 7-field: second minute hour day month dow year. Day/dow: cannot both be specific; use '?' for one. Year: '*' for recurring, fixed year limits to that year. One-shot: all fields fixed, e.g. '0 0 17 28 3 ? 2026'. Examples: daily 9am '0 0 9 * * ? *'; every 15min '0 */15 * * * ? *'. See tool description.",
    ),
    (
        "timezone",
        "时区，如 Asia/Shanghai",
        "Timezone, e.g. Asia/Shanghai",
    ),
    (
        "targets",
        "目标频道。用户未明确指定时不填，系统自动使用当前对话渠道；**禁止从历史记录推断。**",
        "Target channel. Leave empty unless user explicitly specifies; system uses current channel. **Never infer from history.**",
    ),
    ("legacy_enabled", "是否启用", "Whether to enable the job"),
    (
        "legacy_description",
        "具体任务内容，到点执行时发给助手。不要包含时间/频率信息（如'每隔40分钟'、'每天9点'）",
        "Task content sent to assistant at scheduled time. Do NOT include time/frequency info",
    ),
    (
        "wake_offset_seconds",
        "提前唤醒秒数，不是任务执行时间。除非用户明确说“提前 X 秒/分钟唤醒/准备/执行”，否则禁止传该字段；用户未明确指定时必须省略，后端默认 0 秒（到点执行，不提前唤醒）。不要根据任务类型、提醒/查询场景、距离触发时间或历史记录推断为 60 或其他正值。",
        "Wake-ahead offset in seconds; not the task execution time. Unless the user explicitly says to wake/prepare/run X seconds/minutes early, do not pass this field. Omit it when unspecified; the backend defaults to 0 seconds (run at the scheduled time, no early wake). Do not infer 60 or any other positive value from task type, reminder/query scenarios, time until trigger, or history.",
    ),
];

fn legacy_cron_field(key: &str, lang: Lang) -> &'static str {
    field_desc(LEGACY_CRON_FIELDS, key, lang)
}

fn cron_list_jobs_params(_lang: Lang) -> Value {
    json!({ "type": "object", "properties": {}, "required": [] })
}

fn cron_get_job_params(lang: Lang) -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": { "type": "string", "description": legacy_cron_field("job_id_to_look_up", lang) },
        },
        "required": ["job_id"],
    })
}

fn cron_create_job_params(lang: Lang) -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": legacy_cron_field("name", lang) },
            "cron_expr": { "type": "string", "description": legacy_cron_field("cron_expr", lang) },
            "timezone": {
                "type": "string",
                "description": legacy_cron_field("timezone", lang),
                "default": "Asia/Shanghai",
            },
            "targets": { "type": "string", "description": legacy_cron_field("targets", lang) },
            "enabled": {
                "type": "boolean",
                "description": legacy_cron_field("legacy_enabled", lang),
                "default": true,
            },
            "description": { "type": "string", "description": legacy_cron_field("legacy_description", lang) },
            "wake_offset_seconds": {
                "type": "integer",
                "description": legacy_cron_field("wake_offset_seconds", lang),
                "default": 0,
            },
        },
        "required": ["name", "cron_expr", "timezone", "description"],
    })
}

fn cron_update_job_params(lang: Lang) -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": { "type": "string", "description": legacy_cron_field("job_id_to_update", lang) },
            "patch": {
                "type": "object",
                "description": legacy_cron_field("patch", lang),
                "additionalProperties": true,
            },
        },
        "required": ["job_id", "patch"],
    })
}

fn cron_delete_job_params(lang: Lang) -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": { "type": "string", "description": legacy_cron_field("job_id_to_delete", lang) },
        },
        "required": ["job_id"],
    })
}

fn cron_toggle_job_params(lang: Lang) -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": { "type": "string", "description": legacy_cron_field("job_id_to_toggle", lang) },
            "enabled": { "type": "boolean", "description": legacy_cron_field("enabled", lang) },
        },
        "required": ["job_id", "enabled"],
    })
}

fn cron_preview_job_params(lang: Lang) -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": { "type": "string", "description": legacy_cron_field("job_id_to_preview", lang) },
            "count": {
                "type": "integer",
                "description": legacy_cron_field("count", lang),
                "default": 5,
            },
        },
        "required": ["job_id"],
    })
}

// ---------------------------------------------------------------------------
// ask_user 工具(对齐 ask_user.py)
// ---------------------------------------------------------------------------

const ASK_USER_DESCRIPTION_CN: &str = r#"向用户提问以收集信息、澄清歧义或做出决策。支持1-4个问题，每个问题2-4个选项。

何时主动使用：需求模糊、多种方案可选、涉及用户偏好时，应主动询问而非假设。

【禁止】选项中添加'其他'、'自定义'等兜底选项，系统已自动提供。【推荐】将推荐选项放第一位，label末尾加'（推荐）'。preview字段仅用于单选问题的视觉比较场景。"#;

const ASK_USER_DESCRIPTION_EN: &str = r#"Ask user questions to gather info, clarify ambiguity, or make decisions. Supports 1-4 questions, each with 2-4 options.

When to use proactively: Ask when requirements are vague, multiple approaches exist, or user preferences matter. Don't assume.

FORBIDDEN: Adding 'Other', 'Custom' etc. as options — system provides this automatically. RECOMMENDED: Place recommended option first, append '(Recommended)' to its label. Preview field is only for single-select questions with visual comparison needs."#;

/// ask_user 字段描述表(对齐 ask_user.py ASK_USER_PARAMS)。
const ASK_USER_FIELDS: &[(&str, &str, &str)] = &[
    (
        "questions",
        "向用户提出的问题列表（1-4个）",
        "Questions to ask the user (1-4 questions)",
    ),
    (
        "header",
        "问题的简短标题或标签",
        "A short label or tag for the question (max 12 chars)",
    ),
    ("question", "完整的问题文本", "The complete question to ask"),
    (
        "options",
        "可选答案列表（2-4个）",
        "Available choices for this question (2-4 options)",
    ),
    (
        "options_label",
        "选项显示文本（1-5个词）",
        "The display text for this option (1-5 words).",
    ),
    (
        "options_description",
        "选项详细说明",
        "Explanation of what this option means or what will happen if chosen.",
    ),
    (
        "options_preview",
        "可选的预览内容，用于UI模型、代码片段或视觉比较。仅在单选问题中支持。",
        "Optional preview content rendered when this option is focused. Use for mockups, code snippets, or visual comparisons. Only supported for single-select questions.",
    ),
    (
        "multi_select",
        "是否允许多选",
        "Set to true to allow the user to select multiple options instead of just one.",
    ),
];

fn ask_user_field(key: &str, lang: Lang) -> &'static str {
    field_desc(ASK_USER_FIELDS, key, lang)
}

/// ask_user 顶层 schema(对齐 get_ask_user_input_params)。
fn ask_user_params(lang: Lang) -> Value {
    json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "description": ask_user_field("questions", lang),
                "items": {
                    "type": "object",
                    "properties": {
                        "header": { "type": "string", "description": ask_user_field("header", lang) },
                        "question": { "type": "string", "description": ask_user_field("question", lang) },
                        "options": {
                            "type": "array",
                            "description": ask_user_field("options", lang),
                            "items": {
                                "type": "object",
                                "properties": {
                                    "label": { "type": "string", "description": ask_user_field("options_label", lang) },
                                    "description": { "type": "string", "description": ask_user_field("options_description", lang) },
                                    "preview": { "type": "string", "description": ask_user_field("options_preview", lang) },
                                },
                                "required": ["label", "description"],
                            },
                        },
                        "multi_select": {
                            "type": "boolean",
                            "default": false,
                            "description": ask_user_field("multi_select", lang),
                        },
                    },
                    "required": ["header", "question", "options"],
                },
                "minItems": 1,
                "maxItems": 4,
            },
        },
        "required": ["questions"],
    })
}

// ---------------------------------------------------------------------------
// agent_mode 工具(对齐 agent_mode.py)
// ---------------------------------------------------------------------------

const SWITCH_MODE_DESCRIPTION_CN: &str = r#"在 normal 与 plan 模式间切换当前会话模式。

何时使用：
- 用户明确要求只做规划、不做实现时，（e.g.切到 plan 模式）。
- 你判断当前模式不适合该任务。
- 任务的复杂度或需求发生显著变化。

模式说明：
- plan：规划优先。除 plan 文件外仅允许只读操作。
- normal：完整的开发权限，可修改文件并执行命令。

注意：
- 在意图不明确时先用 ask_user 澄清，再切换模式。"#;

const SWITCH_MODE_DESCRIPTION_EN: &str = r#"Switch the current session between normal and plan modes.

When to use:
- Switch to plan when the user explicitly wants planning only and no implementation.
- You determine the current mode is inappropriate for the task
- A task's complexity or requirements have changed significantly.

Mode characteristics:
- plan: Structured planning before execution, read-only with plan file writing only.
- normal: Full development actions are allowed (editing files, running commands, etc.).

Note:
- If intent is ambiguous, call ask_user first before switching mode."#;

fn switch_mode_params(lang: Lang) -> Value {
    let description = match lang {
        Lang::Cn => "目标模式：normal 或 plan",
        Lang::En => "Target mode: normal or plan",
    };
    json!({
        "type": "object",
        "properties": {
            "mode": { "type": "string", "enum": ["normal", "plan"], "description": description },
        },
        "required": ["mode"],
    })
}

const ENTER_PLAN_MODE_DESCRIPTION_CN: &str = "初始化 plan 文件并返回文件路径。在 plan 模式下，如需生成正式计划可调用此工具；     只读操作（如 gh/git 只读命令、read_file、grep 等）可直接执行，无需先调用本工具。     该工具会创建一个新的 plan 文件（幂等：若已存在则直接返回路径）。";

const ENTER_PLAN_MODE_DESCRIPTION_EN: &str = "Initialize the plan file and return its path. In plan mode, call this tool      when you need to produce a formal plan; read-only actions (e.g. read-only      gh/git commands, read_file, grep) can be run directly without calling this      first. Creates a new plan file (idempotent: returns the existing path if      already created).";

const EXIT_PLAN_MODE_DESCRIPTION_CN: &str = "读取 plan 文件全文并直接返回给用户，结束规划阶段，请求用户审批是否要切换到 normal 模式执行。     当你对最终 plan 文件满意时，必须调用此工具结束规划阶段。tool_result 中包含完整计划内容。";

const EXIT_PLAN_MODE_DESCRIPTION_EN: &str = "Read the full plan file and return the plan directly, ending the planning phase.      Request user approval before switching to normal mode for execution.      Call this when you are satisfied with the final plan.      The tool result contains the complete plan content.";

fn no_params() -> Value {
    json!({ "type": "object", "properties": {}, "required": [] })
}

// ---------------------------------------------------------------------------
// MCP 工具(对齐 mcp.py)
// ---------------------------------------------------------------------------

const LIST_MCP_RESOURCES_DESCRIPTION_CN: &str = "列出指定 MCP 服务器上可用的资源列表。";
const LIST_MCP_RESOURCES_DESCRIPTION_EN: &str =
    "List available resources exposed by the specified MCP server.";

fn list_mcp_resources_params(lang: Lang) -> Value {
    let description = match lang {
        Lang::Cn => "MCP 服务器的 server_id",
        Lang::En => "The server_id of the MCP server",
    };
    json!({
        "type": "object",
        "properties": {
            "server_id": { "type": "string", "description": description },
        },
        "required": ["server_id"],
    })
}

const READ_MCP_RESOURCE_DESCRIPTION_CN: &str = "读取指定 MCP 服务器上某个资源的内容。";
const READ_MCP_RESOURCE_DESCRIPTION_EN: &str =
    "Read the content of a specific resource from the specified MCP server.";

fn read_mcp_resource_params(lang: Lang) -> Value {
    let server_id = match lang {
        Lang::Cn => "MCP 服务器的 server_id",
        Lang::En => "The server_id of the MCP server",
    };
    let uri = match lang {
        Lang::Cn => "要读取的资源 URI",
        Lang::En => "The URI of the resource to read",
    };
    json!({
        "type": "object",
        "properties": {
            "server_id": { "type": "string", "description": server_id },
            "uri": { "type": "string", "description": uri },
        },
        "required": ["server_id", "uri"],
    })
}

// ---------------------------------------------------------------------------
// skill_tool / list_skill / load_tools 工具(对齐 skill_tool.py / list_skill.py / load_tools.py)
// ---------------------------------------------------------------------------

const SKILL_TOOL_DESCRIPTION_CN: &str = "使用此工具查看特定技能的内容。成功时默认附带技能根目录的 ASCII 目录树（directory_tree）     以及嵌套子技能相对路径列表（discovered_skill_names，含 SKILL.md 的子目录）；     加载子技能请再次调用本工具，并设置 relative_file_path（如 designer/SKILL.md）。";

const SKILL_TOOL_DESCRIPTION_EN: &str = "Use this tool to view the skill contents of a certain skill.      On success it always includes an ASCII directory_tree of the skill root      and discovered_skill_names (relative paths of nested dirs that contain SKILL.md).      To load a nested skill, call again with relative_file_path      (e.g. designer/SKILL.md).";

fn skill_tool_params(lang: Lang) -> Value {
    let skill_name = match lang {
        Lang::Cn => "技能的名称",
        Lang::En => "Name of the skill",
    };
    let relative_file_path = match lang {
        Lang::Cn => {
            "可选。查看技能目录中指定路径（relative_file_path）下的特定文件。留空则查看主 SKILL.md 文件。"
        }
        Lang::En => {
            "Optional. Views a specific file within the skill directory at the relative_file_path. Leave blank to view the main SKILL.md file."
        }
    };
    json!({
        "type": "object",
        "properties": {
            "skill_name": { "type": "string", "description": skill_name },
            "relative_file_path": { "type": "string", "description": relative_file_path },
        },
        "required": ["skill_name"],
    })
}

const LIST_SKILL_DESCRIPTION_CN: &str = "列出可用技能或为当前任务选择相关技能。";
const LIST_SKILL_DESCRIPTION_EN: &str =
    "List available skills or select relevant skills for the current task.";

fn list_skill_params(lang: Lang) -> Value {
    let query = match lang {
        Lang::Cn => "可选。当前用户任务。为空时返回所有可用技能。",
        Lang::En => "Optional. Current user task. If empty, return all available skills.",
    };
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": query },
        },
        "required": [],
    })
}

const LOAD_TOOLS_DESCRIPTION_CN: &str = "将选定的真实工具加载到当前 session 可见工具集合中。";
const LOAD_TOOLS_DESCRIPTION_EN: &str =
    "Load selected real tools into the current session-visible tool set.";

fn load_tools_params(lang: Lang) -> Value {
    let tool_names = match lang {
        Lang::Cn => "要在当前 session 中可见的工具名称列表",
        Lang::En => "Names of tools to make visible for the current session",
    };
    let replace = match lang {
        Lang::Cn => "如果为 true，替换当前可见工具集，否则合并",
        Lang::En => "If true, replace the current visible tool set instead of merging",
    };
    json!({
        "type": "object",
        "properties": {
            "tool_names": {
                "type": "array",
                "items": { "type": "string" },
                "description": tool_names,
            },
            "replace": { "type": "boolean", "description": replace },
        },
        "required": ["tool_names"],
    })
}

// ---------------------------------------------------------------------------
// lsp 工具(对齐 lsp_tool.py)
// ---------------------------------------------------------------------------

const LSP_DESCRIPTION_CN: &str = r#"通过 Language Server Protocol (LSP) 服务器获取代码智能功能（如定义跳转、引用查找、诊断等）。

支持的操作：
- goToDefinition: 查找符号的定义位置
- findReferences: 查找符号的所有引用
- documentSymbol: 获取文档中的所有符号（函数、类、变量等）
- workspaceSymbol: 在整个工作区搜索符号
- goToImplementation: 查找接口或抽象方法的具体实现
- prepareCallHierarchy: 获取光标位置的调用层次结构条目
- incomingCalls: 查找所有调用当前函数的函数/方法
- outgoingCalls: 查找当前函数调用的所有函数/方法

注意：hover（悬停信息）操作暂不支持。

导航操作均需要 file_path、line 和 character 参数。
workspaceSymbol 不需要 line 和 character，而是使用 query 参数。

导航操作的结果会自动过滤掉位于 gitignored 目录（如 node_modules、__pycache__ 等）中的条目。

大文件（超过 10MB）不会被发送到 LSP 服务器。

注意：必须为文件类型配置对应的 LSP 服务器。如果没有可用的服务器，将返回错误。"#;

const LSP_DESCRIPTION_EN: &str = r#"Interact with Language Server Protocol (LSP) servers to get code intelligence features.

Supported operations:
- goToDefinition: Find where a symbol is defined
- findReferences: Find all references to a symbol
- documentSymbol: Get all symbols (functions, classes, variables) in a document
- workspaceSymbol: Search for symbols across the entire workspace
- goToImplementation: Find implementations of an interface or abstract method
- prepareCallHierarchy: Get call hierarchy item at a position (functions/methods)
- incomingCalls: Find all functions/methods that call the function at a position
- outgoingCalls: Find all functions/methods called by the function at a position

Note: Hover (hover information) is not currently supported.

Navigation operations require file_path, line, and character.
workspaceSymbol uses query instead of line/character.

Results from gitignored files (node_modules, __pycache__, etc.) are automatically filtered out for navigation operations.

Large files (exceeding 10MB) are not sent to the LSP server.

Note: LSP servers must be configured for the file type. If no server is available, an error will be returned."#;

fn lsp_params(lang: Lang) -> Value {
    let operation = match lang {
        Lang::Cn => {
            "LSP 操作类型，可选值：goToDefinition、findReferences、documentSymbol、workspaceSymbol、goToImplementation、prepareCallHierarchy、incomingCalls、outgoingCalls"
        }
        Lang::En => {
            "LSP operation type. Options: goToDefinition, findReferences, documentSymbol, workspaceSymbol, goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls"
        }
    };
    let file_path = match lang {
        Lang::Cn => "文件路径（绝对路径或相对于工作区根目录的路径）",
        Lang::En => "The absolute or relative path to the file",
    };
    let line = match lang {
        Lang::Cn => "行号（1-indexed，编辑器中显示的行号）",
        Lang::En => "The line number (1-based, as shown in editors)",
    };
    let character = match lang {
        Lang::Cn => "列号（1-indexed，默认为 1）",
        Lang::En => "The character offset (1-based, as shown in editors; defaults to 1)",
    };
    let query = match lang {
        Lang::Cn => "搜索查询字符串；为空时返回所有可用符号（仅 workspaceSymbol 使用）",
        Lang::En => {
            "Search query string; when empty, returns all available symbols (used by workspaceSymbol only)"
        }
    };
    let include_declaration = match lang {
        Lang::Cn => "为 true 时，结果中包含符号的定义位置（默认 true）",
        Lang::En => {
            "When true, the declaration location itself is included in the results (default: true)"
        }
    };
    json!({
        "type": "object",
        "properties": {
            "operation": {
                "type": "string",
                "enum": [
                    "goToDefinition",
                    "findReferences",
                    "documentSymbol",
                    "workspaceSymbol",
                    "goToImplementation",
                    "prepareCallHierarchy",
                    "incomingCalls",
                    "outgoingCalls",
                ],
                "description": operation,
            },
            "file_path": { "type": "string", "description": file_path },
            "line": { "type": "integer", "minimum": 1, "description": line },
            "character": { "type": "integer", "minimum": 1, "description": character },
            "query": { "type": "string", "description": query },
            "include_declaration": { "type": "boolean", "description": include_declaration },
        },
        "required": ["operation", "file_path"],
    })
}

// ---------------------------------------------------------------------------
// 汇总
// ---------------------------------------------------------------------------

/// 返回全部特殊工具元数据。
pub fn metadata() -> Vec<ToolMetadata> {
    vec![
        tool_meta(
            "cron",
            CRON_DESCRIPTION_CN,
            CRON_DESCRIPTION_EN,
            cron_params(Lang::Cn),
            cron_params(Lang::En),
        ),
        tool_meta(
            "cron_list_jobs",
            "列出所有 cron 定时任务。",
            "List all cron jobs.",
            cron_list_jobs_params(Lang::Cn),
            cron_list_jobs_params(Lang::En),
        ),
        tool_meta(
            "cron_get_job",
            "根据任务 ID 获取单个 cron 定时任务的详细信息。",
            "Get a single cron job by its ID.",
            cron_get_job_params(Lang::Cn),
            cron_get_job_params(Lang::En),
        ),
        tool_meta(
            "cron_create_job",
            CRON_CREATE_JOB_DESCRIPTION_CN,
            CRON_CREATE_JOB_DESCRIPTION_EN,
            cron_create_job_params(Lang::Cn),
            cron_create_job_params(Lang::En),
        ),
        tool_meta(
            "cron_update_job",
            "使用扁平字段更新已有的 cron 定时任务。",
            "Update an existing cron job with a flat patch dict.",
            cron_update_job_params(Lang::Cn),
            cron_update_job_params(Lang::En),
        ),
        tool_meta(
            "cron_delete_job",
            "根据任务 ID 删除 cron 定时任务。",
            "Delete a cron job by its ID.",
            cron_delete_job_params(Lang::Cn),
            cron_delete_job_params(Lang::En),
        ),
        tool_meta(
            "cron_toggle_job",
            "启用或禁用指定的 cron 定时任务。",
            "Enable or disable a cron job.",
            cron_toggle_job_params(Lang::Cn),
            cron_toggle_job_params(Lang::En),
        ),
        tool_meta(
            "cron_preview_job",
            "预览 cron 定时任务的下 N 次计划执行时间。",
            "Preview next N scheduled run times for a cron job.",
            cron_preview_job_params(Lang::Cn),
            cron_preview_job_params(Lang::En),
        ),
        tool_meta(
            "ask_user",
            ASK_USER_DESCRIPTION_CN,
            ASK_USER_DESCRIPTION_EN,
            ask_user_params(Lang::Cn),
            ask_user_params(Lang::En),
        ),
        tool_meta(
            "switch_mode",
            SWITCH_MODE_DESCRIPTION_CN,
            SWITCH_MODE_DESCRIPTION_EN,
            switch_mode_params(Lang::Cn),
            switch_mode_params(Lang::En),
        ),
        tool_meta(
            "enter_plan_mode",
            ENTER_PLAN_MODE_DESCRIPTION_CN,
            ENTER_PLAN_MODE_DESCRIPTION_EN,
            no_params(),
            no_params(),
        ),
        tool_meta(
            "exit_plan_mode",
            EXIT_PLAN_MODE_DESCRIPTION_CN,
            EXIT_PLAN_MODE_DESCRIPTION_EN,
            no_params(),
            no_params(),
        ),
        tool_meta(
            "list_mcp_resources",
            LIST_MCP_RESOURCES_DESCRIPTION_CN,
            LIST_MCP_RESOURCES_DESCRIPTION_EN,
            list_mcp_resources_params(Lang::Cn),
            list_mcp_resources_params(Lang::En),
        ),
        tool_meta(
            "read_mcp_resource",
            READ_MCP_RESOURCE_DESCRIPTION_CN,
            READ_MCP_RESOURCE_DESCRIPTION_EN,
            read_mcp_resource_params(Lang::Cn),
            read_mcp_resource_params(Lang::En),
        ),
        tool_meta(
            "skill_tool",
            SKILL_TOOL_DESCRIPTION_CN,
            SKILL_TOOL_DESCRIPTION_EN,
            skill_tool_params(Lang::Cn),
            skill_tool_params(Lang::En),
        ),
        tool_meta(
            "list_skill",
            LIST_SKILL_DESCRIPTION_CN,
            LIST_SKILL_DESCRIPTION_EN,
            list_skill_params(Lang::Cn),
            list_skill_params(Lang::En),
        ),
        tool_meta(
            "load_tools",
            LOAD_TOOLS_DESCRIPTION_CN,
            LOAD_TOOLS_DESCRIPTION_EN,
            load_tools_params(Lang::Cn),
            load_tools_params(Lang::En),
        ),
        tool_meta(
            "lsp",
            LSP_DESCRIPTION_CN,
            LSP_DESCRIPTION_EN,
            lsp_params(Lang::Cn),
            lsp_params(Lang::En),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::tools::validate_provider;
    use serde_json::Value;

    /// 全部特殊工具名(与 Python 素材 1:1)。
    const EXPECTED_NAMES: &[&str] = &[
        "cron",
        "cron_list_jobs",
        "cron_get_job",
        "cron_create_job",
        "cron_update_job",
        "cron_delete_job",
        "cron_toggle_job",
        "cron_preview_job",
        "ask_user",
        "switch_mode",
        "enter_plan_mode",
        "exit_plan_mode",
        "list_mcp_resources",
        "read_mcp_resource",
        "skill_tool",
        "list_skill",
        "load_tools",
        "lsp",
    ];

    #[test]
    fn all_tools_pass_validate_provider() {
        let metas = metadata();
        assert!(!metas.is_empty());
        for m in &metas {
            assert!(
                validate_provider(m).is_ok(),
                "validate_provider failed for tool '{}'",
                m.name
            );
        }
    }

    #[test]
    fn tool_names_match_python_spec() {
        let metas = metadata();
        let mut names: Vec<&str> = metas.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        let mut expected = EXPECTED_NAMES.to_vec();
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    #[test]
    fn descriptions_are_nonempty_and_differ() {
        for m in metadata() {
            assert!(
                !m.description_cn.trim().is_empty(),
                "{} cn description must be non-empty",
                m.name
            );
            assert!(
                !m.description_en.trim().is_empty(),
                "{} en description must be non-empty",
                m.name
            );
            assert_ne!(
                m.description_cn, m.description_en,
                "{} descriptions must differ",
                m.name
            );
        }
    }

    #[test]
    fn idempotent_defaults_false() {
        for m in metadata() {
            assert!(!m.idempotent, "{} must default to non-idempotent", m.name);
        }
    }

    #[test]
    fn ask_user_schema_contains_questions() {
        let metas = metadata();
        let ask = metas
            .iter()
            .find(|m| m.name == "ask_user")
            .expect("ask_user metadata present");
        let props = ask
            .params_cn
            .get("properties")
            .and_then(Value::as_object)
            .expect("params_cn.properties must be an object");
        assert!(
            props.contains_key("questions"),
            "ask_user must declare 'questions' property"
        );
        let required = ask
            .params_cn
            .get("required")
            .and_then(Value::as_array)
            .expect("params_cn.required must be an array");
        assert!(
            required.iter().any(|v| v.as_str() == Some("questions")),
            "ask_user 'questions' must be required"
        );
    }

    #[test]
    fn cron_schema_embeds_job_structure() {
        let metas = metadata();
        let cron = metas
            .iter()
            .find(|m| m.name == "cron")
            .expect("cron metadata present");
        let props = cron
            .params_cn
            .get("properties")
            .and_then(Value::as_object)
            .expect("cron params properties");
        let job = props
            .get("job")
            .and_then(Value::as_object)
            .expect("cron 'job' property must be an object");
        let job_props = job
            .get("properties")
            .and_then(Value::as_object)
            .expect("job.properties must be an object");
        for key in ["schedule", "payload", "delivery", "wake_offset_seconds"] {
            assert!(
                job_props.contains_key(key),
                "cron job schema must contain '{key}'"
            );
        }
        let action = props
            .get("action")
            .and_then(Value::as_object)
            .expect("action property");
        let action_enum = action
            .get("enum")
            .and_then(Value::as_array)
            .expect("action enum");
        assert_eq!(action_enum.len(), 8);
    }
}
