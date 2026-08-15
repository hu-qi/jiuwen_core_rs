# 持久化目录(persistence-catalog.md)

> 等价 DSH 的 persistence-catalog:登记全部持久化格式与版本策略。
> 本文档由 session log 落地触发创建(docs/README.md 规划表),现按 crates/*/src 实际落盘代码
> 全面核对补充(2025-08:Redis store/queue、PostgreSQL、graph-memory、RSI checkpoint、evolving 经验等)。

## 1. 会话事件日志(已实现)

| 项 | 值 |
| --- | --- |
| 格式 | JSONL,每行一个 SessionEvent(seq/timestamp_ms/kind/payload) |
| 文件 | 默认会话 = SessionLogPlugin::new(session_path) 指定的单个 .jsonl;多会话 = session_dir/{id}.jsonl;保存的快照 = session_dir/checkpoints/{name}.jsonl(ah-app 传入) |
| 写入 | append-only + flush;启动时逐行恢复 |
| 版本 | 事件 payload 约定见 contracts/session.rs;新增字段必须向后兼容(serde default) |
| 投影 | derive_messages:日志 → 模型可见 ChatMessage 序列(日志即真相) |

payload 约定(投影依赖,修改须更新本文档与 contracts/session.rs;kind 枚举 = SessionEventKind):

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

## 3. 已实现的其他持久化(按能力)

| 能力 | 格式/后端 | 文件与命名 | 说明 |
| --- | --- | --- | --- |
| store 文件后端(ah-plugins-store) | JSON / JSONL | KV:dir/kv/{key}.json(KvEntry);message:dir/messages/{channel}.jsonl | set 落盘、get/delete 真实读写、scan 按前缀枚举;message append-only,read 按 seq 增量 |
| queue 文件后端(ah-plugins-queue) | JSONL + 游标文件 | dir/{channel}.jsonl(QueueMessage 逐行)+ dir/{channel}.cursor(已消费最大 seq) | 日志为真相 + 游标;启动恢复 |
| Redis store(ah-plugins-store-redis) | Redis String | key 原样;值 = KvEntry JSON(SET/GET/DEL/KEYS) | 与文件后端同一 seam 可互换;连接失败显式报错 |
| Redis queue(ah-plugins-queue-redis) | Redis LIST / 计数器 / String | 每 channel 三键,前缀 ah:q:{channel}:log(RPUSH)、:seq(INCR)、:cursor(STRING) | 与文件后端同构:日志为真相 + 游标;前缀隔离多租户 |
| PostgreSQL store(ah-plugins-store-pg) | SQL 两表 | kv(key TEXT PK, value TEXT, updated_ms BIGINT);messages(channel TEXT, seq BIGINT, payload TEXT, ts_ms BIGINT, PK(channel,seq)) | 连接时幂等 CREATE TABLE IF NOT EXISTS;UPSERT 写 kv |
| memory(ah-plugins-memory) | JSON | dir/{key}.json(MemoryRecord) | remember/recall/forget 真实落盘 |
| retrieval(ah-plugins-retrieval) | JSON | dir/{doc_id}.json(DocFile) | 知识库文档持久化,启动枚举恢复 |
| prompt(ah-plugins-prompt) | JSON | dir/{name}.json(PromptTemplate) | 同名注册版本递增,最新版本生效,文件落盘 |
| graph-memory(ah-plugins-graph-memory) | JSONL 三集合 | entities.jsonl / relations.jsonl / episodes.jsonl | 启动逐行恢复;每次变更整文件重写(实体按规范化名合并去重) |
| evolving 经验(ah-plugins-evolving) | JSONL | dir/experiences.jsonl(Experience: id/task/verdict/score/issues/saved_ms) | append-only;load 按保存顺序;search 关键词打分 |
| telemetry(ah-plugins-telemetry) | JSONL | dir/telemetry.jsonl(Span 逐行) | export 把未导出 span 追加写入并 flush;启动统计已导出数 |
| RSI checkpoint(ah-plugins-rsi) | JSONL | dir/rsi_checkpoints.jsonl(RsiCheckpoint: 含 best_prompt/best_score/cases 等) | append-only;load 取末行(最近一轮);数据集在内存生成(不落盘) |
| RSI single-harness(ah-plugins-rsi-single-harness) | JSONL | dir/single_harness.jsonl(SingleHarnessCheckpoint: best_prompt/best_score/completed_epochs) | append-only;续跑时加载 checkpoint 跳过已完成 epoch |
| 轨迹 codec(ah-plugins-evolving::trajectory_codec) | 无持久化(纯编解码) | 无文件 | Trajectory ↔ OTel Span 树(trajectory_to_spans / spans_to_trajectory 可逆);聚合统计;纯函数,内存往返 |

## 4. 规划中的持久化

| 能力 | 格式草案 | 触发 |
| --- | --- | --- |
| workflow 检查点 | JSON(现有 rp301 迁移) | workflow 引擎已实现但执行状态纯内存,检查点/续跑未落地 |
| Redis 检查点(通用) | 见 §3 Redis store/queue 已落地 | 若需跨进程检查点,可基于现有 Redis 后端扩展 |
| OTLP 导出 | telemetry 当前为 JSONL,OTLP 留待后续 | 真实 collector 接入时 |

## 5. 版本纪律

1. 持久化格式一经发布即为 API,变更需迁移逻辑与版本字段;
2. 新事件类型必须先在 event-catalog 登记,再写日志;
3. 投影函数与 payload 约定同步演进,禁止只改一端;
4. 后端替换(文件 ↔ Redis ↔ PostgreSQL)必须保持同一 seam 语义与值格式(如 KvEntry JSON),可互换不换契约。
