# 持久化目录(persistence-catalog.md)

> 等价 DSH 的 persistence-catalog:登记全部持久化格式与版本策略。
> 本文档由 session log 落地触发创建(docs/README.md 规划表)。

## 1. 会话事件日志(已实现)

| 项 | 值 |
| --- | --- |
| 格式 | JSONL,每行一个 SessionEvent(seq/timestamp_ms/kind/payload) |
| 文件 | 会话文件路径由 SessionLogPlugin::new(path) 指定(ah-app 传入) |
| 写入 | append-only + flush;启动时逐行恢复 |
| 版本 | 事件 payload 约定见 contracts/session.rs;新增字段必须向后兼容(serde default) |
| 投影 | derive_messages:日志 → 模型可见 ChatMessage 序列(日志即真相) |

payload 约定(投影依赖,修改须更新本文档与 contracts/session.rs):

| kind | payload |
| --- | --- |
| user | {"content": string} |
| assistant(普通) | {"content": string} |
| assistant(工具调用) | {"tool_calls": [{id, name, arguments}]} |
| tool_result | {"tool_call_id": string, "output": string} |
| agent_step | {"iteration": int, "tool_calls": int, "done": bool} |
| system | {"content": string}(预留) |

## 2. Profile 配置(已实现)

profiles/*.toml:TOML,name + bundles[].plugins;无版本化需求(配置非持久数据)。

## 3. 规划中的持久化(能力落地后登记)

| 能力 | 格式草案 | 触发 |
| --- | --- | --- |
| workflow 检查点 | JSON(现有 rp301 迁移) | workflow 引擎落地 |
| Redis 检查点 | 见 ah-plugins-redis(规划) | 真实 Redis 落地 |
| RSI 产物 | YAML(参考 Python 侧) | RSI 落地 |

## 4. 版本纪律

1. 持久化格式一经发布即为 API,变更需迁移逻辑与版本字段;
2. 新事件类型必须先在 event-catalog 登记,再写日志;
3. 投影函数与 payload 约定同步演进,禁止只改一端。
