# Redis Checkpointer Rust 1:1 移植规格文档

> 规格来源文件(均为行号依据):
> - `agent-core/openjiuwen/extensions/checkpointer/redis/checkpointer.py`(508 行,下文称 `redis/checkpointer.py`)
> - `agent-core/openjiuwen/extensions/checkpointer/redis/storage.py`(587 行,下文称 `redis/storage.py`)
> - `agent-core/openjiuwen/core/session/checkpointer/base.py`(121 行)、`checkpointer.py`(134 行)、`__init__.py`(30 行)
> - `agent-core/openjiuwen/extensions/store/kv/redis_store.py`(531 行)
> - `agent-core/openjiuwen/core/foundation/store/base_kv_store.py`(214 行)
> - 依赖契约:`core/session/state/*`、`core/session/internal/*`、`core/graph/store/*`、`core/session/interaction/interactive_input.py`、`core/session/constants.py`、`core/common/exception/*`、`core/graph/pregel/constants.py`、`core/common/constants/constant.py`

---

## 0. 总览:分层与依赖边界

Python 侧的分层(必须 1:1 保留):

```
RedisCheckpointerProvider.create(conf)          # 工厂:配置解析 → 客户端创建 → 组装
    └─> RedisStore(redis_client)                # 唯一 Redis 访问边界(KV 抽象)
            └─> RedisCheckpointer(store, ttl)   # 钩子编排,只调 Storage/Store,不碰原生客户端
                    ├─ AgentStorage        (agent 单状态)
                    ├─ AgentGroupStorage   (agent-team 单状态)
                    ├─ WorkflowStorage     (workflow 双状态:state + updates)
                    └─ GraphStore          (workflow-graph 图状态)
```

**架构铁律(redis/checkpointer.py:344-346 注释)**:`RedisCheckpointer` 不直接使用 Redis 客户端 API,所有 Redis 操作一律经 `RedisStore`。Rust 移植应把 `RedisStore` 视作唯一的"进程外依赖"边界,`RedisCheckpointer`/四个存储类只依赖该 trait。

---

## 1. 核心基类规格(core/session/checkpointer/)

### 1.1 命名空间常量(base.py:78-86)

| 常量 | 值 | 含义 |
|---|---|---|
| `SESSION_NAMESPACE_AGENT` | `"agent"` | 会话下 agent 状态命名空间 |
| `SESSION_NAMESPACE_AGENT_TEAM` | `"agent-team"` | 会话下 agent 团队状态命名空间(注意是连字符) |
| `SESSION_NAMESPACE_WORKFLOW` | `"workflow"` | 会话下 workflow 自身状态命名空间 |
| `WORKFLOW_NAMESPACE_GRAPH` | `"workflow-graph"` | workflow 下图状态命名空间(与 workflow 自身状态分离) |

### 1.2 key 构造纯函数(base.py:89-121)

- `build_key(*parts: str) -> str`(base.py:89-99):`":".join(parts)`。空 parts 得空串;parts 中任何元素为 None 会抛 TypeError(调用方保证非 None)。
- `build_key_with_namespace(session_id, namespace, entity_id, *suffixes) -> str`(base.py:102-121):`build_key(session_id, namespace, entity_id, *suffixes)`,即 `session_id:namespace:entity_id[:suffix...]`。**参数顺序固定为:session_id → namespace → entity_id → suffixes**。

### 1.3 `Checkpointer` 抽象接口(base.py:14-57)

- 静态方法 `get_thread_id(session) -> str`(base.py:15-17):`":".join([session.session_id(), session.workflow_id()])`。
- 抽象异步方法(全部返回 None,除注明外):
  - `pre_workflow_execute(session, inputs: InteractiveInput)`(base.py:20)
  - `post_workflow_execute(session, result, exception)`(base.py:24)
  - `pre_agent_execute(session, inputs)`(base.py:28)
  - `pre_agent_team_execute(session, inputs)`(base.py:32)
  - `interrupt_agent_execute(session)`(base.py:36)
  - `post_agent_execute(session)`(base.py:40)
  - `post_agent_team_execute(session)`(base.py:44)
  - `session_exists(session_id: str) -> bool`(base.py:48)
  - `release(session_id: str)`(base.py:52)
  - `graph_store() -> Store`(同步方法,base.py:56)

### 1.4 `Storage` 抽象接口(base.py:60-75)

- `save(session)`、`recover(session, inputs: InteractiveInput = None)`、`clear(session_id: str)`、`exists(session) -> bool`,全部抽象异步。
- **签名偏差(移植要点)**:抽象层 `clear(self, session_id: str)` 是单参;而 Redis 实现为 `clear(entity_id, session_id)`(storage.py:203、399)。Rust trait 应以 Redis 实际签名为准:`clear(entity_id: &str, session_id: &str)`。
- 同理 `Checkpointer.release(session_id)` 抽象为单参,Redis 实现加了可选参 `release(session_id, agent_id: Optional[str] = None)`(redis/checkpointer.py:483)。

### 1.5 `CheckpointerProvider` / `CheckpointerFactory` / `CheckpointerConfig`(checkpointer.py:41-134)

- `CheckpointerProvider`(54-57):抽象 `async create(conf: dict) -> Checkpointer`。
- `CheckpointerFactory`(60-125):
  - `_registry: Dict[str, CheckpointerProvider]`;`register(name)` 装饰器把**类实例化后**存入 registry(66-71)。
  - `create(checkpointer_conf: CheckpointerConfig)`(74-79):`provider = _registry.get(type)`;未注册 → `raise Exception()`(**裸异常,无消息**);否则 `await provider.create(conf)`。
  - `set_default_checkpointer` / `set_checkpointer(store_type, instance)` / `get_checkpointer(store_type=None)`(82-125):按 type 优先取实例,`"in_memory"` 回退默认内存 checkpointer,否则取默认。
- `CheckpointerConfig`(41-51):`type: str = "in_memory"`;`conf: dict = {}`;`__repr__`/`__str__` 对 conf 递归脱敏 URL 密码:正则 `^[a-zA-Z][a-zA-Z0-9+.-]*://` 命中字符串时调 `redact_url_password`(24-38)。**Rust 的 Debug/Display 需复刻该脱敏**。
- `InMemoryCheckpointerProvider` 注册 `"in_memory"`(128-134)。

### 1.6 会话/状态 API 契约(供 Rust 定义入参 trait)

- `BaseSession`(session.py:20-49):`config() -> Config`、`state() -> State`、`session_id() -> str`。
- `WorkflowSession.workflow_id()`(internal/workflow.py:79-80)返回 `self._workflow_id`。
- `AgentSession.agent_id()`(internal/agent.py:79-83):`agent_config.id`(若配置存在)否则 `card.id`。
- **`session.group_id()`:整个代码库未定义任何实现**(storage.py:249、redis/checkpointer.py:375、393 直接调用)。它是调用方运行期契约——传入 `pre_agent_team_execute`/`AgentGroupStorage` 的 session 必须提供 `group_id()`,取值为"团队/组 ID"。Rust 侧将其定义为 trait 方法,由宿主会话实现。
- `NodeSession(session, node_id)`(internal/workflow.py:104-174):`node_id()`、`state()`、`workflow_id()`、`session_id()`。
- `Config.get_env(key, default=None)`(config/base.py:136-146):key 在 `_env` 中则返回值,否则返回 default。内置 `_force_del_workflow_state` 默认 `False`(config/base.py:164)。
- 状态 API 关键语义:
  - agent 型 `StateCollection`(state/agent_state.py:9-52):`get_state(copied=False)` 返回原始 dict `{"global_state": ..., "agent_state": ...}`;`set_state(dict)` 分拆写入两个子状态(InMemoryStateLike.set_state 对 falsy 值不覆盖,state/base.py:132-134);`get_global(None)` 返回整个 global_state dict(29-32);属性 `global_state`(44-46)。
  - workflow 型 `CommitState`(state/workflow_state.py:151-181):`get_state(copied=False)` → `{"io_state":..., "global_state": None|..., "comp_state":..., "workflow_state":...}`(workflow_only 时 global 为 None);`get_updates()` → `{"io_state_updates":..., "global_state_updates": None|..., "comp_state_updates":..., "workflow_state_updates":...}`;`set_updates` 逆写;`update_and_commit_workflow_state(data)`(105-107)等价 `workflow_state.update_by_id("workflow", data)` + `commit()`;`commit()`(139-143)提交全部四子状态。
  - `INTERACTIVE_INPUT = "__interactive_input__"`(constant.py:15)。
- `InteractiveInput`(interactive_input.py:16-37):字段 `user_inputs: Dict[str, Any]`(默认 {}),`raw_inputs: Any`(默认 None)。`update(node_id, value)` 在 raw_inputs 已存在或参数为 None 时抛 `INTERACTION_INPUT_INVALID`。`isinstance(inputs, InteractiveInput)` 是类型判断,非鸭子类型。

---

## 2. RedisStore 接口规格(extensions/store/kv/redis_store.py)

继承 `BaseKVStore`(base_kv_store.py:16-161,抽象方法集一致)。**值类型契约**:`str` 值按 UTF-8 编码存储;`bytes` 原样存储;`get` 对 bytes 原样返回、其他类型 `str()` 化(129-155)。客户端默认 `decode_responses=False`,故实际返回 bytes。

| 方法(行号) | 签名 | Redis 命令 / 语义 |
|---|---|---|
| `set`(67-87) | `set(key, value: str\|bytes)` | `SET`。异常 → 记日志并 re-raise |
| `exclusive_set`(89-127) | `exclusive_set(key, value, expiry=None) -> bool` | `SET key value NX` / `SET key value NX EX <expiry>`;返回是否设置成功 |
| `get`(129-158) | `get(key) -> str\|bytes\|None` | `GET`;缺失返回 None;异常 re-raise |
| `exists`(160-177) | `exists(key) -> bool` | `EXISTS` |
| `delete`(179-197) | `delete(key)` | `DEL`;删不存在的 key 返回 0,不算错误 |
| `get_by_prefix`(199-260) | `get_by_prefix(prefix) -> dict[str, str\|bytes]` | `SCAN MATCH "{prefix}*"` 迭代 + 逐个 `GET`;返回"key → value"字典,空则 `{}` |
| `delete_by_prefix`(262-327) | `delete_by_prefix(prefix, batch_size=None)` | `SCAN MATCH "{prefix}*"` 收集;若 batch_size 有效(>0)则每满 batch 删一批,否则**一次性累积全部再删**;`DEL *keys`;非原子 |
| `mget`(329-406) | `mget(keys: List[str]) -> List[str\|bytes\|None]` | `MGET`;失败回退逐个 `GET`(单个失败记 None);空入参返回 `[]` |
| `batch_delete`(408-464) | `batch_delete(keys, batch_size=None) -> int` | `DEL *keys`;batch_size 为 None 或 ≤0 时一次全删,否则按 batch 分块;返回实际删除数;空入参返回 0 |
| `refresh_ttl`(466-505) | `refresh_ttl(keys, ttl_seconds: int)` | 若 `keys` 空或 `ttl_seconds <= 0` 直接返回;否则 pipeline 逐个 `EXPIRE key ttl_seconds`;**异常静默吞掉**(仅 warning 日志) |
| `pipeline`(507-531) | `pipeline()` | 返回底层客户端 pipeline;支持链式 `set/get/exists` + `execute()`(返回结果列表,顺序与入队一致) |

构造:`__init__(redis: Redis | RedisCluster)`(54-65),记录 `self._is_cluster = isinstance(redis, RedisCluster)`(当前各方法未使用该标志,保留字段即可)。

---

## 3. 配置类规格(redis/checkpointer.py)

### 3.1 `RedisTTLConfig`(44-61)

| 字段 | 类型 | 默认值 | 校验 |
|---|---|---|---|
| `default_ttl` | `Optional[float]` | `None` | 无(单位:**分钟**;pydantic v2 lax 模式 int→float 自动转换) |
| `refresh_on_read` | `bool` | `False` | 无 |

无任何 validator。

### 3.2 `RedisConnectionConfig`(64-178)

`model_config = ConfigDict(arbitrary_types_allowed=True)`(97)——允许 `redis_client` 字段装任意类型。

| 字段 | 类型 | 默认值 |
|---|---|---|
| `redis_client` | `Optional[Union[Redis, RedisCluster]]` | `None` |
| `url` | `Optional[str]` | `None` |
| `cluster_mode` | `Optional[bool]` | `None` |
| `connection_args` | `dict[str, Any]` | `default_factory=dict`(每次新建空 dict) |

**校验规则**:
1. `field_validator('url')`(116-132):非 None 时必须以 `redis://`、`rediss://`、`redis+cluster://`、`rediss+cluster://` 之一开头,否则 `ValueError(f"Invalid Redis URL format: {v}. URL must start with redis://, rediss://, redis+cluster://, or rediss+cluster://")`。纯前缀检查,不做 URL 解析。
2. `model_validator(mode='after')`(134-141):`redis_client is None and url is None` → `ValueError("Either 'redis_client' or 'url' must be provided in RedisConnectionConfig")`。

**`is_cluster_mode() -> bool`**(143-160),优先级严格递减:
1. `redis_client is not None` → 返回 `isinstance(redis_client, RedisCluster)`;
2. `cluster_mode is not None` → 返回 `cluster_mode`;
3. `url is not None` → 返回 `url.startswith("redis+cluster://") or url.startswith("rediss+cluster://")`;
4. 兜底 `False`。

**`get_connection_url() -> Optional[str]`**(162-178):
1. `url is None` → 返回 `None`;
2. `startswith("redis+cluster://")` → `replace("redis+cluster://", "redis://")`(替换所有出现,实际只会在前缀);
3. `startswith("rediss+cluster://")` → `replace("rediss+cluster://", "rediss://")`;
4. 否则原样返回 url。

### 3.3 `RedisCheckpointerConfig`(181-226)

| 字段 | 类型 | 默认值 |
|---|---|---|
| `connection` | `RedisConnectionConfig` | 必填(`Field(...)`) |
| `ttl` | `Optional[RedisTTLConfig]` | `None` |

无额外校验。

---

## 4. `RedisCheckpointerProvider.create(conf)` 完整流程(229-337)

注册名:`@CheckpointerFactory.register("redis")`(229)。`async def create(self, conf: dict) -> Checkpointer`。

```
① config = RedisCheckpointerConfig.model_validate(conf)      # 287-294
   失败(pydantic 任何异常)→ raise ValueError(
     f"Invalid Redis checkpointer configuration: {e}. "
     "Configuration must have a 'connection' key with either 'redis_client' or 'url'. "
     "Optional 'ttl' key for TTL configuration.") from e

② connection = config.connection                               # 296

③ if connection.redis_client is not None:                      # 299-302
     redis_store = RedisStore(connection.redis_client)
     ttl_dict = config.ttl.model_dump() if config.ttl else None
     return RedisCheckpointer(redis_store, ttl_dict)            # 直接复用,忽略 url/connection_args

④ connection_url = connection.get_connection_url()             # 305
   if connection_url is None:                                   # 306-309
     raise ValueError("Either 'redis_client' or 'url' must be provided in connection configuration")

⑤ is_cluster = connection.is_cluster_mode()                    # 312

⑥ 创建客户端(315-332):try:
     if is_cluster:  redis = RedisCluster.from_url(connection_url, **connection.connection_args)
     else:           redis = Redis.from_url(connection_url, **connection.connection_args)
   失败 → raise ValueError(
     f"Failed to create Redis client: {e}. URL: {connection_url}, Cluster mode: {is_cluster}") from e

⑦ redis_store = RedisStore(redis)                              # 335-337
   ttl_dict = config.ttl.model_dump() if config.ttl else None
   return RedisCheckpointer(redis_store, ttl_dict)
```

要点:
- 步骤③/⑦ 中 `ttl.model_dump()` 会**同时输出两个键**(`{"default_ttl": <float|None>, "refresh_on_read": <bool>}`)——这是后续 TTL 语义怪癖的来源(见 §9-Q1/Q2)。
- 所有失败均包装为 `ValueError`(pydantic 异常也被包装),错误消息字符串必须逐字复刻。
- 客户端创建是**惰性连接**(redis-py 连接池语义),`from_url` 本身不保证连通性;连接失败发生在首次命令时。

---

## 5. `RedisCheckpointer` 钩子方法语义(340-508)

构造(348-363):`__init__(redis_store, ttl=None)`,创建四个存储:`AgentStorage`、`AgentGroupStorage`、`WorkflowStorage`、`GraphStore`(GraphStore 存于 `self._graph_state`)。

| 钩子(行号) | 签名 | 语义(按序) |
|---|---|---|
| `pre_agent_execute`(365-371) | `(session, inputs)` | ① INFO 日志(agent_id/session_id);② `await agent_storage.recover(session)`;③ 若 `inputs is not None`:`session.state().update({INTERACTIVE_INPUT: [inputs]})`(写入 agent 状态) |
| `pre_agent_team_execute`(373-379) | `(session, inputs)` | ① INFO 日志(group_id/session_id);② `await agent_group_storage.recover(session)`;③ 若 `inputs is not None`:`session.state().update_global({INTERACTIVE_INPUT: [inputs]})`(写入全局状态) |
| `interrupt_agent_execute`(381-384) | `(session)` | INFO 日志;`await agent_storage.save(session)` |
| `post_agent_execute`(386-389) | `(session)` | INFO 日志;`await agent_storage.save(session)` |
| `post_agent_team_execute`(391-396) | `(session)` | INFO 日志;`await agent_group_storage.save(session)` |
| `pre_workflow_execute`(398-440) | `(session, inputs)` | 见下 |
| `post_workflow_execute`(442-459) | `(session, result, exception)` | 见下 |
| `session_exists`(461-481) | `(session_id: str) -> bool` | `prefix = f"{session_id}:"`;`keys = await redis_store.get_by_prefix(prefix)`;返回 `len(keys) > 0`。开头有 `if self._redis_store is None: return False`(防御性,构造必传,实际恒不成立) |
| `release`(483-505) | `(session_id, agent_id=None)` | 见下 |
| `graph_store`(507-508) | `() -> GraphStore` | 返回 `self._graph_state`(同步) |

**`pre_workflow_execute` 详细分支**(398-440):
1. `workflow_id = session.workflow_id()`,INFO 日志。
2. `if isinstance(inputs, InteractiveInput)`:`await workflow_storage.recover(session, inputs)`,返回。
3. 否则(非交互输入):`if not await workflow_storage.exists(session): return`。
4. `if session.config().get_env(FORCE_DEL_WORKFLOW_STATE_KEY, False)`(键 `"_force_del_workflow_state"`,constants.py:22;默认 False):
   - `workflow_id = session.workflow_id()`;若 `workflow_id is None` → WARNING 日志并 return;
   - `session_id = session.session_id()`;
   - `await graph_state.delete(session_id, workflow_id)`(删 `{session_id}:workflow-graph:{workflow_id}` 前缀);
   - `await workflow_storage.clear(workflow_id, session_id)`(删 4 个 workflow 键);
   - INFO 日志。
5. 否则:`raise build_error(StatusCode.CHECKPOINTER_PRE_WORKFLOW_EXECUTION_ERROR, workflow=workflow_id, reason="workflow state exists but non-interactive input and cleanup is disabled")`。
   - 错误码:`(111121, "pre workflow execute error, session_id={session_id}, workflow={workflow}, error='{reason}'")`(codes.py:329-331)。模板缺 `session_id` 占位符 → 渲染为 `<missing:session_id>`(错误消息用 safe-dict 渲染,缺 key 不抛异常)。

**`post_workflow_execute` 详细分支**(442-459):
1. `workflow_id = session.workflow_id(); session_id = session.session_id()`。
2. `if exception is not None`:INFO 日志 → `await workflow_storage.save(session)` → **`raise exception`(原样重抛)**。
3. `if result.get(TASK_STATUS_INTERRUPT) is None`(`"__interrupt__"`,pregel/constants.py:10):INFO 日志 → `await graph_state.delete(session_id, workflow_id)` → `await workflow_storage.clear(workflow_id, session_id)`。
4. 否则(含 `__interrupt__` 键):INFO 日志 → `await workflow_storage.save(session)`。

**`release` 详细分支**(483-505):
1. `if self._redis_store is None` → WARNING 日志并 return(防御性)。
2. `if agent_id is not None`:INFO 日志 → `await agent_storage.clear(agent_id, session_id)`(只清该 agent)。
3. `else`:INFO 日志 → `prefix = f"{session_id}:"` → `await redis_store.delete_by_prefix(prefix, batch_size=500)` → DEBUG 日志。

---

## 6. 存储类规格(redis/storage.py)

### 6.1 模块常量(36-38)

`_DEFAULT_TTL = "default_ttl"`;`_SECONDS_PER_MINUTE = 60`;`_REFRESH_ON_READ = "refresh_on_read"`。

### 6.2 公共基类 `BaseRedisStorage`(41-100)

构造(48-63):
- `self._serde = create_serializer("pickle")`(PickleSerializer);
- `self._ttl_seconds = None`;`self._refresh_on_read = False`;
- `if ttl and _DEFAULT_TTL in ttl: self._ttl_seconds = int(ttl.get(_DEFAULT_TTL) * _SECONDS_PER_MINUTE)` —— **分钟×60 后 int 截断**;
- `if ttl and _REFRESH_ON_READ in ttl: self._refresh_on_read = True` —— **仅判断键存在**(见 §9-Q1)。

序列化辅助:
- `_serialize_state(state) -> Optional[Tuple[str, bytes]]`(65-67):`self._serde.dumps_typed(state)`,Pickle 实现返回 `("pickle", pickle.dumps(state))`(serde.py:33-42);pickle 失败异常在调用点**try 块之外**,直接上抛。
- `_decode_dump_type(dump_type) -> str`(69-73):bytes → `decode("utf-8")`;None → `""`;否则原样。
- `_deserialize_state(dump_type, blob)`(75-85):任一为 None → None;先解码 dump_type;`loads_typed((type_str, blob))` 异常 → ERROR 日志 + 返回 None(PickleSerializer 要求 type 恰为 `"pickle"`,否则返回 None,serde.py:37-42)。
- `_refresh_ttl(keys, entity_name, entity_id)`(87-96):`if not (refresh_on_read and ttl_seconds) or not keys: return`;调 `redis_store.refresh_ttl(keys, ttl_seconds)`;异常 → WARNING 日志(吞掉)。
- `_make_redis_key(*args)`(98-100):`":".join(...)`,静态辅助,实际存储类未使用。

### 6.3 单状态基类 `BaseSingleStateStorage`(103-223)

`_KEY_NUMS = 2`。抽象成员:`_namespace`、`_entity_name`、`_state_blobs_key`、`_state_dump_type_key`(属性,106-124);`_get_entity_id(session)`、`_get_state_to_save(session)`、`_restore_state(session, state)`(127-135)。

**`_build_state_keys(session_id, entity_id) -> (dump_type_key, blob_key)`**(138-145):
```
dump_type_key = build_key_with_namespace(session_id, namespace, entity_id, state_dump_type_key)
blob_key      = build_key_with_namespace(session_id, namespace, entity_id, state_blobs_key)
```

**`save(session)`**(147-168):
1. `state = _get_state_to_save(session)`;`session_id`;`entity_id = _get_entity_id(session)`;
2. `state_blob = _serialize_state(state)`;若 falsy → WARNING 日志 + return(pickle 输出恒非空,实际不触发);
3. pipeline:`SET dump_type_key <dump_type字符串> [EX ttl]` + `SET blob_key <pickle字节> [EX ttl]` 链式入队,`execute()`;
4. 异常 → ERROR 日志 + **re-raise**。

**`recover(session, inputs=None)`**(170-201;`inputs` 形参存在但**未使用**):
1. pipeline:`GET dump_type_key` + `GET blob_key`,`execute()`;
2. `len(results) != 2` → DEBUG 日志 + return(防御);
3. `dump_type, blob = results[0], results[1]`;`state = _deserialize_state(...)`;
4. `state is None` → DEBUG 日志 + return(**此路径不刷新 TTL**);
5. `try: _restore_state(session, state)` → except:ERROR 日志 + re-raise;`finally: _refresh_ttl([dump_type_key, blob_key], entity_name, entity_id)`(即使 set_state 失败也刷新)。

**`clear(entity_id, session_id)`**(203-206):`deleted = redis_store.batch_delete([dump_type_key, blob_key])`;DEBUG 日志(返回值为 DEL 计数)。

**`exists(session)`**(208-223):pipeline `EXISTS` ×2;长度非 2 → False;**两个 key 必须都存在**(`results[0]==1 and results[1]==1`)。

### 6.4 `AgentStorage`(226-239)

| 成员 | 值 |
|---|---|
| `_namespace` | `SESSION_NAMESPACE_AGENT` = `"agent"` |
| `_entity_name` | `"agent"` |
| `_state_blobs_key` | `"agent_state_blobs"` |
| `_state_dump_type_key` | `"agent_state_blobs_dump_type"` |
| `_get_entity_id` | `session.agent_id()` |
| `_get_state_to_save` | `session.state().get_state(copied=False)`(原始 dict,含 `global_state`/`agent_state` 两键) |
| `_restore_state` | `session.state().set_state(state)` |

Key 模式:`{session_id}:agent:{agent_id}:agent_state_blobs_dump_type` 与 `{session_id}:agent:{agent_id}:agent_state_blobs`。

### 6.5 `AgentGroupStorage`(242-255)

| 成员 | 值 |
|---|---|
| `_namespace` | `SESSION_NAMESPACE_AGENT_TEAM` = `"agent-team"` |
| `_entity_name` | `"agent_team"` |
| `_state_blobs_key` | `"agent_group_state_blobs"` |
| `_state_dump_type_key` | `"agent_group_state_blobs_dump_type"` |
| `_get_entity_id` | `session.group_id()`(**运行期契约,见 §1.6**) |
| `_get_state_to_save` | `session.state().get_global(None)`(整个 global_state dict;agent 型会话返回真实 dict;workflow 型会话返回 None → pickle(None)) |
| `_restore_state` | `session.state().global_state.set_state(state)` |

Key 模式:`{session_id}:agent-team:{group_id}:agent_group_state_blobs_dump_type` 与 `{session_id}:agent-team:{group_id}:agent_group_state_blobs`。

### 6.6 `WorkflowStorage`(258-450)

常量(259-265):`_STATE_BLOBS = "workflow_state_blobs"`、`_STATE_BLOBS_DUMP_TYPE = "workflow_state_blobs_dump_type"`、`_UPDATE_BLOBS = "workflow_update_blobs"`、`_UPDATE_BLOBS_DUMP_TYPE = "workflow_update_blobs_dump_type"`、`_KEY_NUMS = 4`。

**`_process_interactive_inputs(session, inputs)`**(267-284):
1. `if inputs.raw_inputs is not None`:`session.state().update_and_commit_workflow_state({INTERACTIVE_INPUT: inputs.raw_inputs})`,返回;
2. `if not (hasattr(inputs, 'user_inputs') and inputs.user_inputs)`:返回;
3. 对每个 `(node_id, value) in inputs.user_inputs.items()`:`node_session = NodeSession(session, node_id)`;`interactive_input = node_session.state().get(INTERACTIVE_INPUT)`;若已是 list → append value 后 update,否则 `update({INTERACTIVE_INPUT: [value]})`;
4. `session.state().commit()`。

**`save(session)`**(286-331):
1. `state = session.state().get_state(copied=False)`;`workflow_id = session.workflow_id()`;`session_id`;
2. pipeline;`has_operations = False`;
3. `state_blob = _serialize_state(state)`;若非空:入队 `SET workflow_state_blobs_dump_type_key` + `SET workflow_state_blobs_key`(各带 ttl),`has_operations = True`;否则 WARNING;
4. `updates = session.state().get_updates()`;`updates_blob = _serialize_state(updates)`;若非空:入队 `SET workflow_update_blobs_dump_type_key` + `SET workflow_update_blobs_key`,`has_operations = True`;
5. `if has_operations: try execute()` except → ERROR 日志 + re-raise。

**`recover(session, inputs=None)`**(333-397):
1. 构造 4 个 key;pipeline `GET` ×4,`execute()`;
2. `len != 4` → WARNING + return;
3. **state 恢复**:`results[0], results[1]`;解码 dump_type;条件 `state_blob and type_str and type_str != "empty"` 才尝试:`try: state = _deserialize_state(...); if not None: session.state().set_state(state)`;except → ERROR 日志(**继续执行,不抛**);`finally: _refresh_ttl([state_dump_type_key, state_blob_key], "workflow", workflow_id)`(仅进入 try 才刷新);
4. `if inputs is not None: _process_interactive_inputs(session, inputs)`(**先恢复 state,再注入交互输入**);
5. **updates 恢复**:`results[2], results[3]`;同样条件(含 `!= "empty"`)尝试 `set_updates`;except → ERROR 日志(继续);finally 刷新 `_refresh_ttl(..., "workflow updates", workflow_id)`。

**`clear(workflow_id, session_id)`**(399-417):`batch_delete([4 个 key])`,DEBUG 日志。

**`exists(session)`**(419-450):pipeline `EXISTS` ×4;长度非 4 → False;**仅要求前两个(state 键)为 1**(updates 可选)。

Key 模式(workflow 键):`{session_id}:workflow:{workflow_id}:workflow_state_blobs_dump_type` / `:workflow_state_blobs` / `:workflow_update_blobs_dump_type` / `:workflow_update_blobs`。

### 6.7 `GraphStore`(453-587)

实现 `core/graph/store/base.py` 的 `Store` 接口(41-52):`get(session_id, ns) -> Optional[GraphState]`、`save(session_id, ns, state)`、`delete(session_id, ns=None)`。

常量(462-464):`_DATA_TYPE = "checkpoint_data_type"`、`_DATA_VALUE = "checkpoint_data_value"`、`_KEY_NUMS = 2`。

构造(466-485):与 BaseRedisStorage 相同的 TTL 解析逻辑(含 §9-Q1 怪癖)。`_serde = create_serializer("pickle")`。

**`get(session_id, ns)`**(487-520):
1. key_type = `build_key_with_namespace(session_id, WORKFLOW_NAMESPACE_GRAPH, ns, "checkpoint_data_type")`;key_value 同理(先 type 后 value);
2. pipeline `GET` ×2;长度非 2 → ERROR 日志 + None;
3. `_type, _value = results`;`if not _type or not _value` → DEBUG 日志 + return None(空/缺失即无;**此路径不刷新 TTL**);
4. bytes → utf-8 解码 type;
5. `try: graph_state = _deserialize_graph_state(type_str, _value)`;None → DEBUG + return None;成功 → 返回;`finally: _refresh_ttl([key_type, key_value], session_id, ns)`。

**`save(session_id, ns, state)`**(522-542):
1. `serialized = _serialize_graph_state(state)`(pickle typed);falsy → WARNING + return;
2. pipeline:`SET key_type <dump_type>` + `SET key_value <pickle字节>`(各带 ttl),`execute()`;
3. 异常 → ERROR + re-raise。

**`delete(session_id, ns=None)`**(544-562):
- `if not ns`:`prefix = build_key(session_id, "workflow-graph")`(`{session_id}:workflow-graph`)→ `delete_by_prefix(prefix, batch_size=500)`(删该 session 下全部图命名空间);
- `else`:`prefix = build_key_with_namespace(session_id, "workflow-graph", ns)`(`{session_id}:workflow-graph:{ns}`)→ `delete_by_prefix(prefix, batch_size=500)`。

`_refresh_ttl`(564-573)、`_serialize_graph_state`(575-577)、`_deserialize_graph_state`(579-587;type 空或 blob None → None;异常 → ERROR + None)同基类语义。

`GraphState` 结构(base.py:31-38):`ns: str`、`step: int`、`channel_values: Dict[str, Any]`、`pending_buffer: List[Message]`、`pending_node: Dict[str, PendingNode]`、`node_version: Dict[str, int]`。

---

## 7. TTL 语义汇总

| 项 | 规则 | 出处 |
|---|---|---|
| 配置单位 | `default_ttl` 为**分钟**(float) | redis/checkpointer.py:54-57 |
| 秒换算 | `_ttl_seconds = int(default_ttl * 60)`,int 截断;未配置(ttl 为 None 或不含 `default_ttl` 键)则 `None` | storage.py:60-61、480-483 |
| SET 携带 | `SET key value ex=<ttl_seconds>`;`ex=None` 时不带过期(redis-py pipeline.set 第三位置参数即 `ex`) | storage.py:161-164、303-305、320-322、535-537 |
| 无钳制 | `default_ttl<=0` 不校验;`EX 0` 会触发 Redis 报错(调用方责任) | — |
| refresh_on_read | 读取成功(或 set_state 失败但数据存在)后用 pipeline `EXPIRE key <ttl_seconds>` 刷新;**仅当 `refresh_on_read && ttl_seconds` 为真且 keys 非空** | storage.py:87-96、199-201、375-377、396-397、518-520、564-573 |
| refresh 失败 | 仅 WARNING 日志,吞掉 | storage.py:95-96、572-573 |
| 触发点 | 单状态:恢复成功路径的 finally;Workflow:state/updates 各自进入 try 的 finally;Graph:两值都存在时的 finally。**数据缺失/反序列化为 None 的路径不刷新** | storage.py:199-201、375-377、396-397、518-520 |
| 上限/下限 | 无下限检查;`refresh_ttl` 内部对 `ttl_seconds <= 0` 直接跳过(redis_store.py:492-493) | — |

---

## 8. 确定性(纯函数) vs 环境依赖(Redis 进程)划分

### 8.1 确定性/纯函数(可单测、无外部依赖,Rust 中可纯函数实现)

| 功能点 | 出处 |
|---|---|
| `build_key` / `build_key_with_namespace` key 拼接 | base.py:89-121 |
| 各存储类 `_build_state_keys`/4-key 构造 | storage.py:138-145、297-302、314-319、338-349、400-411、425-437、489-490、532-534、555-561 |
| TTL 分钟→秒换算 `int(minutes*60)` 与标志解析 | storage.py:60-63、482-485 |
| 配置校验:URL 前缀白名单、必填项检查、错误消息字符串 | redis/checkpointer.py:116-141 |
| `is_cluster_mode` 三分支判定 | redis/checkpointer.py:143-160 |
| `get_connection_url` 的 `+cluster` 前缀规整 | redis/checkpointer.py:162-178 |
| 配置→`ttl_dict` 的 `model_dump()` 双键输出 | redis/checkpointer.py:301、336 |
| dump_type 字节解码(bytes→utf-8、None→"") | storage.py:69-73 |
| typed 序列化格式约定:`"pickle"` + pickle 字节;type 不匹配 → None | serde.py:33-42 |
| `"empty"` dump_type 特判(跳过恢复) | storage.py:367、387 |
| 交互输入处理逻辑(raw_inputs 优先;user_inputs 按 node 追加/包装为 list;commit) | storage.py:267-284 |
| 所有决策分支:exists→return、force-del vs raise、exception→save+raise、interrupt 键有无→save vs clear | redis/checkpointer.py:398-459 |
| 错误/异常构造:`build_error(111121, workflow=..., reason=...)` 及模板渲染(safe-dict,缺 key 渲染 `<missing:key>`) | codes.py:329-331;errors.py:280-293 |
| `{session_id}:`、`{session_id}:workflow-graph` 等前缀字符串构造 | redis/checkpointer.py:479、503;storage.py:555、560 |
| session 状态 API 行为(get_state(copied=False) 原样 dict、set_state 分拆、get_global(None) 全量) | state/agent_state.py:29-46;state/workflow_state.py:151-181 |

### 8.2 环境依赖(需真实 Redis 进程,行为不可由纯函数保证)

| 功能点 | 出处 |
|---|---|
| 客户端创建 `Redis.from_url` / `RedisCluster.from_url`(+ connection_args) | redis/checkpointer.py:315-332 |
| 全部命令:SET / GET / EXISTS / DEL / MGET / SCAN MATCH / EXPIRE | redis_store.py:67-505 |
| pipeline 入队与 execute(单次往返、结果顺序;集群模式跨 slot 行为由客户端决定) | redis_store.py:497-500、507-531 |
| 服务器侧 TTL 生效/过期驱逐 | — |
| SCAN 迭代:非阻塞、非原子、顺序不保证;集群跨节点迭代 | redis_store.py:199-260、262-327 |
| 批量删除计数(实际删除数来自服务器) | redis_store.py:408-464 |
| `refresh_ttl` 异常静默吞掉 | redis_store.py:466-505 |
| `session_exists` 对活数据的扫描+取值 | redis/checkpointer.py:461-481 |
| `release` 前缀批量删除(batch 500) | redis/checkpointer.py:483-505 |
| GraphStore.delete 前缀删除(batch 500) | storage.py:544-562 |
| 网络错误、集群路由、连接池行为 | — |

---

## 9. 已知怪癖与边界(1:1 移植必须复刻或显式决策)

- **Q1(`_refresh_on_read` 键存在即真)**:`if ttl and _REFRESH_ON_READ in ttl: self._refresh_on_read = True`(storage.py:62-63、484-485)只判键存在、不判值真假。由于 Provider 总是传 `model_dump()`(双键齐全),**只要配置了任意 ttl,`refresh_on_read` 实际恒为 True,即便用户传 False**。
- **Q2(`default_ttl=None` 崩溃)**:Provider 传 `{"default_ttl": None, "refresh_on_read": False}` 时,`int(None * 60)` 抛 `TypeError`(storage.py:61)。即 `ttl: {}` 或 `ttl: {"refresh_on_read": true}` 会在存储构造期崩溃,异常穿过 `RedisCheckpointer.__init__` 上抛(不被 create 的 try 捕获)。1:1 移植需决定复刻崩溃还是修正。
- **Q3(`ex=0` 边界)**:`default_ttl=0` → `_ttl_seconds=0` → `SET ... EX 0` 会触发 Redis "invalid expire time" 错误;`refresh_ttl` 侧则对 `<=0` 直接跳过。无任何钳制。
- **Q4(`group_id()` 未定义)**:`session.group_id()` 在代码库无实现(见 §1.6),是调用方契约。
- **Q5(Storage.clear 签名偏差)**:抽象 `clear(session_id)` vs 实际 `clear(entity_id, session_id)`。
- **Q6(反序列化失败即"无状态")**:单状态/Graph 恢复中 `loads_typed` 失败返回 None,视为无状态(不抛);Workflow 恢复中失败仅记日志继续执行。
- **Q7(批量删除空入参)**:`batch_delete([])` 返回 0;`get_by_prefix` 空匹配返回 `{}`;`session_exists` 因此对无键 session 返回 False。
- **Q8(session_exists 读值浪费)**:前缀扫描后逐个 `GET` 取值,实际只用键数——1:1 需保持相同命令序列(行为可观察:会触发大量 GET)。
- **Q9(pickle 字节序)**:dump_type 键存 ASCII `"pickle"`;blob 键存 pickle 字节;`get` 返回 bytes 原样,Rust 侧二进制兼容取决于 pickle 格式(若要求跨语言互通需另定格式,规格本身只约束"typed 二元组"契约)。
- **Q10(异常包装链)**:create 中 pydantic 异常、客户端创建异常均被包装为 `ValueError` 并 `from e`;post_workflow_execute 中 `raise exception` 原样重抛。

---

## 10. 功能点 → 文件:行号速查

| 功能点 | 位置 |
|---|---|
| 命名空间常量 | core/session/checkpointer/base.py:78-86 |
| build_key / build_key_with_namespace | base.py:89-121 |
| Checkpointer 抽象(含 get_thread_id) | base.py:14-57 |
| Storage 抽象 | base.py:60-75 |
| CheckpointerProvider / Factory / Config / 脱敏 | core/session/checkpointer/checkpointer.py:41-134 |
| RedisTTLConfig | redis/checkpointer.py:44-61 |
| RedisConnectionConfig + 校验 + is_cluster_mode + get_connection_url | redis/checkpointer.py:64-178 |
| RedisCheckpointerConfig | redis/checkpointer.py:181-226 |
| Provider.create 全流程 | redis/checkpointer.py:229-337 |
| RedisCheckpointer 构造 + 8 钩子 + graph_store | redis/checkpointer.py:340-508 |
| RedisStore 全部接口 | extensions/store/kv/redis_store.py:16-531 |
| BaseKVStore 抽象 | core/foundation/store/base_kv_store.py:16-161 |
| 存储常量 / BaseRedisStorage / TTL 解析 | redis/storage.py:36-100 |
| BaseSingleStateStorage(save/recover/clear/exists) | redis/storage.py:103-223 |
| AgentStorage | redis/storage.py:226-239 |
| AgentGroupStorage | redis/storage.py:242-255 |
| WorkflowStorage(含交互输入处理) | redis/storage.py:258-450 |
| GraphStore | redis/storage.py:453-587 |
| Serializer / create_serializer / Pickle typed 契约 | core/graph/store/serde.py:11-51 |
| Store / GraphState 接口 | core/graph/store/base.py:24-52 |
| INTERACTIVE_INPUT 常量 | core/common/constants/constant.py:15 |
| TASK_STATUS_INTERRUPT 常量 | core/graph/pregel/constants.py:10 |
| FORCE_DEL_WORKFLOW_STATE_KEY / ENV_KEY | core/session/constants.py:22、31;config/base.py:164 |
| CHECKPOINTER_PRE_WORKFLOW_EXECUTION_ERROR (111121) | core/common/exception/codes.py:329-331 |
| build_error 语义(模板渲染、params) | core/common/exception/errors.py:280-293 |
| InteractiveInput 结构 | core/session/interaction/interactive_input.py:16-37 |
| agent 型 StateCollection(get_state/get_global/global_state) | core/session/state/agent_state.py:9-52 |
| workflow 型 CommitState(get_state/get_updates/set_updates/update_and_commit_workflow_state) | core/session/state/workflow_state.py:105-181 |
| NodeSession(node_id/state/workflow_id/session_id) | core/session/internal/workflow.py:104-174 |
| WorkflowSession.workflow_id / AgentSession.agent_id | internal/workflow.py:79-80;internal/agent.py:79-83 |
| Config.get_env | core/session/config/base.py:136-146 |
