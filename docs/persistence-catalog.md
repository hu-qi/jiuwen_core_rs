# 持久化目录(persistence-catalog.md)

> 登记持久化格式、版本和迁移策略。当前基线:`agent-harness@cc561c0`。
> 本文只描述已经在代码中存在的格式;production verification 和 Python parity 另见审计文档。

## 1. 会话事件日志(已实现)

| 项 | 值 |
| --- | --- |
| 格式 | JSONL,新写入每行 `{"version":1,"event":SessionEvent}`;读取兼容旧版裸 `SessionEvent` |
| 文件 | 默认会话 = SessionLogPlugin::new(session_path) 指定的单个 .jsonl;多会话 = session_dir/{id}.jsonl;保存的快照 = session_dir/checkpoints/{name}.jsonl(ah-app 传入) |
| 写入 | append-only;Unix 进程间通过 OS advisory `flock` 串行化;每次追加在锁内从磁盘重新计算 seq,写入后 flush + `sync_data`;启动/读取时校验 seq 连续,只丢弃崩溃造成的未完整末行 |
| 版本 | version=1;未知版本显式拒绝;legacy 裸事件仅兼容读取;事件 payload 约定见 contracts/session.rs |
| 投影 | derive_messages:日志 → 模型可见 ChatMessage 序列(日志即真相);未知工具结果投影带有显式 status=unknown 人工核对标记 |

写入协议额外保证:工具调用 claim 在 Unix 上通过独占锁内的磁盘重读、过期检查和 claim 事件追加完成;
跨进程竞争同一 `call_id` 时最多一个 owner,已完成调用和未过期 claim 均拒绝重复取得。
流式响应的每个模型 delta、工具调用 delta 和工具结果均在发送给下游前记录;
模型/工具超时或取消会中止在途 future 并保留已收到的部分事件,重启后可据日志继续判断恢复路径。

payload 约定(投影依赖,修改须更新本文档与 contracts/session.rs;kind 枚举 = SessionEventKind):

| kind | payload |
| --- | --- |
| tool_result | {"tool_call_id": string, "output": string, "status": completed/error/unknown}(status 默认 completed) |
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
| controller task snapshot | versioned JSON envelope | 默认 workspace/controller/tasks.json,可由 Profile 配置 | 自动保存/恢复;未知版本和 malformed envelope 拒绝;兼容 legacy 裸数组;lock file 冲突显式失败 |
| ability manager | JSON | AbilityPlugin 配置的状态路径 | ability 注册/启停状态持久化;损坏状态显式失败 |
| workflow checkpoint | JSON | 由 workflow checkpoint store/config 决定 | 支持部分执行状态保存和续跑;完整流式 checkpoint parity 仍为 partial |
| 轨迹 codec(ah-plugins-evolving::trajectory_codec) | 无持久化(纯编解码) | 无文件 | Trajectory ↔ OTel Span 树(trajectory_to_spans / spans_to_trajectory 可逆);聚合统计;纯函数,内存往返 |

## 4. 未完成的持久化工作

| 能力 | 当前缺口 |
| --- | --- |
| 统一 envelope/version | session event、workflow checkpoint、部分插件 JSONL 尚未统一版本 envelope |
| 迁移工具 | 多数格式只有兼容读取或显式拒绝,缺少批量迁移 CLI |
| 跨进程 checkpoint | 可基于 Redis/store seam 扩展,尚无统一实现和租约语义 |
| workflow streaming restore | 基础 checkpoint 已有,STREAM/TRANSFORM/COLLECT 中间状态恢复仍不完整 |
| OTLP | HTTP OTLP/JSON 编码与发送已存在;完整 OTel SDK exporter、重试/批处理/collector E2E 仍为 partial |
| 并发与原子写 | 除 session log 外,多数文件后端仍需统一临时文件+fsync+rename 和跨进程锁策略 |

## 5. 版本纪律

1. 持久化格式一经发布即为 API,变更需迁移逻辑与版本字段;
2. 新事件类型必须先在 event-catalog 登记,再写日志;
3. 投影函数与 payload 约定同步演进,禁止只改一端;
4. 后端替换(文件 ↔ Redis ↔ PostgreSQL)必须保持同一 seam 语义与值格式(如 KvEntry JSON),可互换不换契约。
