# LSP 子系统 Rust 1:1 移植规格文档

> 来源代码:`agent-core/openjiuwen/harness/lsp/`(Python, asyncio)。
> 本规格覆盖该目录全部 Python 源码,目标是让 Rust 实现与 Python 行为**逐点等价**。
> 全文为规格描述,不包含实现代码。所有行号均指向来源 Python 文件(`agent-core/openjiuwen/harness/lsp/` 为根)。
> 涉及工具包 `openjiuwen/harness/tools/lsp_tool/_schemas.LspOperation` 与 `build_lsp_tool` 属于 **lsp 包之外**的依赖,不在本规格范围内(仅 1.5.6 节注明接口)。

---

## 0. 总览

### 0.1 文件地图

| 文件 | 行数 | 职责 |
|---|---|---|
| `core/types.py` | 89 | 核心数据类型:状态枚举、SpawnHandle、ScopedLspServerConfig、LspServerStatus |
| `core/client.py` | 421 | LSPClient:单 reader 协程模式的 stdio JSON-RPC 通信层 |
| `core/diagnostic_registry.py` | 283 | 诊断注册表:待发队列、去重、上限、排序 |
| `core/instance.py` | 261 | LSPServerInstance:单 server 生命周期 + 崩溃恢复 |
| `core/manager.py` | 504 | LSPServerManager:全局单例多 server 协调器、懒加载 |
| `core/utils/file_uri.py` | 57 | file:// URI ↔ 文件系统路径转换(确定性) |
| `core/utils/git_ignore.py` | 95 | LSP 导航结果的 gitignore 过滤(依赖 `git` 子进程) |
| `core/utils/constants.py` | 30 | 子系统全部常量 |
| `types.py` | 47 | 子系统初始化/状态 API 类型 |
| `servers/registry.py` | 169 | 内建 server 注册表 + 配置构建器 |
| `servers/types.py` | 32 | ServerDefinition(注册表条目) |
| `servers/servers/go.py` | 42 | gopls 定义 |
| `servers/servers/java.py` | 41 | jdtls 定义 |
| `servers/servers/python.py` | 148 | pyright 定义(含 Windows .CMD 解析) |
| `servers/servers/rust.py` | 86 | rust-analyzer 定义(含 Cargo workspace 根查找) |
| `servers/servers/typescript.py` | 50 | typescript-language-server 定义 |
| `__init__.py` | 106 | 包级公共 API 与导出(`__version__ = "0.1.10"`,第 7 行) |
| `core/__init__.py` | 16 | core 子包导出 |
| `servers/__init__.py` | 8 | 触发内建 server 注册(import 副作用) |
| `servers/servers/__init__.py` | 5 | server 模块再导出 |
| `core/utils/__init__.py` | 1 | 空包 |

### 0.2 并发/事件循环模型(移植要点)

- Python 全部异步代码运行在**单 asyncio 事件循环线程**上;`LspDiagnosticRegistry` 明确声明无需加锁(第 107-125 行注释)。
- 子进程 spawn、管道读写、定时器均绑定该循环。
- Rust 移植建议:单 tokio runtime + `Mutex` 保护跨任务共享状态(诊断注册表、manager 的实例表),或继续利用单线程执行器避免锁。**语义上所有回调都在同一执行上下文内串行发生**。
- `manager._lock` 为 `asyncio.Lock`,**惰性创建**(第 39、41-46 行),避免绑定到创建时的 event loop;`shutdown` 后置 None(第 121 行)以便新 loop 重新创建。Rust 侧对应:锁必须在每次初始化时新建,不能跨 runtime 复用。

### 0.3 包级公共 API(`__init__.py`)

| 符号 | 行号 | 签名/语义 |
|---|---|---|
| `__version__` | 7 | `"0.1.10"` |
| `get_pending_lsp_diagnostics(max_per_file=10, max_total=30) -> list[LspDiagnosticFile]` | 23-41 | 转发 `LspDiagnosticRegistry.get_instance().get_and_clear(max_per_file, max_total)` |
| `async initialize_lsp(options=None) -> InitializeResult` | 44-53 | 转发 `LSPServerManager.initialize(options)`;幂等、懒加载 |
| `async shutdown_lsp() -> None` | 56-62 | 转发 `LSPServerManager.shutdown()` |
| `get_lsp_tool() -> dict` | 65-74 | 转发 `tools/lsp_tool/_tool.build_lsp_tool()`(超出本包范围) |
| `get_lsp_status() -> LspStatus` | 77-83 | 未初始化时返回 `LspStatus(initialized=False, servers=[])` |
| 导出集合 | 86-105 | `LspOperation`(外部类型)、`MAX_LSP_FILE_SIZE_BYTES`、诊断类型与常量、`filter_git_ignored_locations` 等 |

`core/__init__.py` 导出:`LspServerState, LspServerStatus, LSPServerManager, ScopedLspServerConfig, ServerInstanceKey, SpawnHandle`(第 1-16 行)。

---

## 1. 公共类型:签名、字段、默认值、语义、错误行为

### 1.1 `core/types.py`

#### 1.1.1 `LspServerState`(Enum,第 10-35 行)

```text
LspServerState {
    STOPPED  = "stopped"    // 第 31 行
    STARTING = "starting"   // 第 32 行
    RUNNING  = "running"    // 第 33 行
    STOPPING = "stopping"   // 第 34 行
    ERROR    = "error"      // 第 35 行
}
```

- 字符串值同时是序列化值(`state.value` 用于日志与错误消息,见 instance.py 第 179 行)。Rust 移植建议用 `#[derive(Serialize)]` 的字符串枚举保持值不变。
- 完整状态机规格见第 2 章。

#### 1.1.2 `SpawnHandle`(dataclass,第 38-51 行)

| 字段 | 类型 | 默认值 | 语义 |
|---|---|---|---|
| `command` | `str` | 必填 | 可执行文件路径 |
| `args` | `list[str]` | `[]`(第 44 行) | 命令行参数 |
| `env` | `dict[str, str]` | `{}`(第 46 行) | **附加**环境变量(与父进程环境合并,见 3.4.1) |
| `initialization_options` | `dict \| None` | `None`(第 48 行) | 随 initialize 请求发送的 `initializationOptions` |
| `startup_timeout` | `int` | `45_000`(第 50 行) | 启动超时毫秒;manager 用它等待整个启动任务(含握手) |

#### 1.1.3 `ScopedLspServerConfig`(dataclass,第 54-69 行)

| 字段 | 类型 | 默认值 | 语义 |
|---|---|---|---|
| `server_id` | `str` | 必填 | 唯一 server 标识(同时是"人类可读名称") |
| `command` | `str` | 必填 | 可执行文件路径;二进制缺失时构建为 `""`(见 7.3) |
| `workspace_folder` | `str` | 必填 | 该 server 的工作区根目录(spawn 的 `cwd`,rootUri 来源) |
| `args` | `list[str]` | `[]`(第 64 行) | 命令行参数 |
| `env` | `dict[str, str]` | `{}`(第 65 行) | 附加环境变量 |
| `initialization_options` | `dict \| None` | `None`(第 66 行) | initialize 请求的 `initializationOptions`;同时作为 `workspace/didChangeConfiguration` 的 settings |
| `startup_timeout` | `int` | `45_000`(第 67 行) | 启动超时毫秒 |
| `extension_to_language` | `dict[str, str]` | `{}`(第 68 行) | **文件扩展名(含点,如 ".py")→ languageId** 映射;注册表构建扩展索引的唯一来源 |

注意:该 dataclass 是**可变**的;`build_configs_async` 对既有条目直接改字段(registry.py 第 139-144 行)。

#### 1.1.4 `LspServerStatus`(dataclass,第 72-89 行)

| 字段 | 类型 | 默认值 | 语义 |
|---|---|---|---|
| `server_id` | `str` | 必填 | server 唯一标识 |
| `name` | `str` | 必填 | 人类可读名称;manager 中恒等于 `server_id`(manager.py 第 496 行) |
| `running` | `bool` | 必填 | 兼容字段:是否 RUNNING(`inst.running`) |
| `state` | `LspServerState` | `STOPPED`(第 82 行) | 当前生命周期状态 |
| `root` | `str \| None` | `None`(第 84 行) | 工作区根目录 |
| `crash_count` | `int` | `0`(第 86 行) | 崩溃次数(用于恢复决策) |
| `last_error` | `str \| None` | `None`(第 88 行) | ERROR 状态下的最后错误消息(由 `str(instance.last_error)` 得来) |

### 1.2 `types.py`(API 类型)

#### 1.2.1 `InitializeOptions`(第 12-17 行)

| 字段 | 类型 | 默认值 | 语义 |
|---|---|---|---|
| `cwd` | `str \| None` | `None`(第 16 行) | 工作区根;`None` 时取 `os.getcwd()`(manager.py 第 74 行) |
| `custom_servers` | `dict[str, CustomServerConfig] \| None` | `None`(第 17 行) | 用户覆盖/扩展内建 server 的配置,key 为 server_id |

#### 1.2.2 `CustomServerConfig`(第 20-30 行)

| 字段 | 类型 | 默认值 | 语义 |
|---|---|---|---|
| `command` | `str \| None` | `None` | 覆盖既有条目的 command;`None` 表示不改 |
| `args` | `list[str] \| None` | `None` | 覆盖既有条目的 args;`None` 表示不改 |
| `env` | `dict[str, str] \| None` | `None` | **仅新增条目时生效**(既有条目不覆盖 env,见 7.3) |
| `extensions` | `list[str] \| None` | `None` | 新增条目时构建 extension→languageId 映射 |
| `language_id` | `str \| None` | `None` | 新增条目时作为 languageId;缺省回退为 server_id |
| `initialization_options` | `dict \| None` | `None` | 覆盖既有条目 / 作为新增条目值 |
| `disabled` | `bool` | `False`(第 30 行) | `True` 时从配置列表中**删除**该 server_id 全部条目 |

#### 1.2.3 `InitializeResult`(第 33-39 行)

| 字段 | 类型 | 默认值 | 语义 |
|---|---|---|---|
| `success` | `bool` | 必填 | 恒为 `True`(当前代码所有返回路径均 success=True) |
| `servers_loaded` | `int` | 必填 | 已加载配置数:首次初始化 = `len(configs)`;已初始化时 = 当前实例数(`len(get_status())`)或 `0`(竞态下锁内重查) |
| `duration_ms` | `float` | `0.0`(第 39 行) | 当前实现**恒为 0.0**(manager.py 第 112 行硬编码) |

#### 1.2.4 `LspStatus`(第 42-47 行)

| 字段 | 类型 | 默认值 | 语义 |
|---|---|---|---|
| `initialized` | `bool` | 必填 | 单例是否存在 |
| `servers` | `list[LspServerStatus]` | `[]`(第 46 行) | 各已启动实例状态快照 |

### 1.3 `servers/types.py` — `ServerDefinition`(第 11-32 行)

| 字段 | 类型 | 默认值 | 语义 |
|---|---|---|---|
| `id` | `str` | 必填 | 唯一标识(注册表 key) |
| `extensions` | `list[str]` | 必填 | 支持的扩展名(**含点**) |
| `language_id` | `str` | 必填 | LSP languageId |
| `priority` | `int` | `100`(第 29 行) | 优先级,**数值越小优先级越高**;⚠️ 当前代码中**未被任何选择/排序逻辑读取**(见 7.2 注) |
| `global_server` | `bool` | `False`(第 30 行) | 是否为全局 server;⚠️ 当前代码中**未被读取**(保留字段) |
| `find_root` | `Callable[[str], str \| None]` | `...`(占位,第 31 行) | 给定文件路径返回工作区根;可为同步或异步(`asyncio.iscoroutine` 判别) |
| `spawn` | `Callable[[str], SpawnHandle \| None]` | `...`(占位,第 32 行) | 给定根目录返回 SpawnHandle;返回 `None` 表示二进制缺失;可为同步或异步 |

### 1.4 `core/client.py` — `LSPError`(第 17-22 行)

```text
LSPError { code: int, message: str }
```

- 字段:`code`、`message`。
- `str()` 输出 `"LSP Error {code}: {message}"`(第 22 行)。
- 抛出位置:
  - 响应带 `error` 字段时(第 282-285、322-325 行):`LSPError(err.code 或 -1, err.message 或 "Unknown error")`。
  - 请求超时(第 378-381 行):`LSPError(-1, f"Request timeout after {DEFAULT_REQUEST_TIMEOUT_MS}ms: {method}")`。

### 1.5 其他公共类型

#### 1.5.1 `ServerInstanceKey`(frozen dataclass,manager.py 第 20-30 行)

| 字段 | 类型 | 语义 |
|---|---|---|
| `server_id` | `str` | server 标识 |
| `root` | `str` | 项目根目录 |

- `@dataclass(frozen=True)` → **不可变、按值相等/哈希**;是 `_instances` 与 `_spawning` 的 key,支持同一语言在 monorepo 多根下各自实例化(第 24-26 行注释)。

#### 1.5.2 `LspDiagnosticItem` / `LspDiagnosticFile`(见第 3 章)

#### 1.5.3 常量(见第 8.3 节)

#### 1.5.4 包级便捷函数 `uri_to_file_path`(git_ignore.py 第 14-16 行)

- `uri_to_file_path(uri) -> str` = `file_uri_to_path(uri)` 的别名。

#### 1.5.5 `nearest_root` 返回的闭包、`go_root`、`rust_root`(见第 7 章)

#### 1.5.6 外部依赖(不在本规格内)

- `LspOperation`(lsp/__init__.py 第 10 行):来自 `tools/lsp_tool/_schemas`。
- `build_lsp_tool()`(lsp/__init__.py 第 72 行):来自 `tools/lsp_tool/_tool`。
- `tool_logger`(client.py 第 12 行、instance.py 第 9 行):来自 `openjiuwen.core.common.logging`;manager.py 与 git_ignore.py 用标准 `logging`。
- Rust 移植可将其映射为 `tracing`/`log`,但**日志文本(含占位符与级别)应保持等价**以便排障。

---

## 2. LspServerState 状态机

### 2.1 状态

| 状态 | 值 | 语义 |
|---|---|---|
| `STOPPED` | "stopped" | 初始态;实例未运行 |
| `STARTING` | "starting" | 已发起 spawn,握手未完成 |
| `RUNNING` | "running" | 握手成功,可收发请求 |
| `STOPPING` | "stopping" | 停止流程进行中(shutdown/exit 已发或已取消 reader) |
| `ERROR` | "error" | 启动失败或运行中崩溃 |

### 2.2 迁移边(全部)

```text
STOPPED ──start()──▶ STARTING ──握手成功──▶ RUNNING
   ^                      │                      │
   │                      └──spawn/握手异常──▶ ERROR ──┐
   │                              [崩溃回调]          │
   │                                           start() │ (crash_count < MAX_CRASH_RECOVERY_ATTEMPTS)
   │                                                  │
stop() ◀──────────────────────────────────────────────┘
```

| # | 迁移 | 触发点 | 守卫条件 | 副作用 | 位置 |
|---|---|---|---|---|---|
| 1 | `STOPPED → STARTING` | `LSPServerInstance.start()` | 状态不在 {STARTING, RUNNING} | `_client=None`;随后 spawn + `client.initialize()` | instance.py 第 96-114 行 |
| 2 | `STARTING → RUNNING` | `client.initialize()` 成功返回 | 无 | `_crash_count=0`;`_last_error=None` | instance.py 第 133-136 行 |
| 3 | `STARTING → ERROR` | `start()` 内任何异常 | 无 | `_last_error=异常`;若已有 client 则 `await client.stop()` 并置 None;异常**继续向上抛出** | instance.py 第 143-149 行 |
| 4 | `RUNNING → ERROR` | `_handle_crash(code)`(由 client 的 `_on_crash` 经 `on_exit` 回调触发) | 状态 != STOPPING | `_crash_count += 1`;`_last_error = RuntimeError("Server '{id}' exited with code {code}")`;调用 `on_error` 回调 | instance.py 第 236-261 行 |
| 5 | `ERROR → STARTING`(崩溃恢复) | `start()` | `_crash_count < MAX_CRASH_RECOVERY_ATTEMPTS` | 记日志;继续正常启动流程 | instance.py 第 99-110 行 |
| 6 | `ERROR → STOPPED` | `stop()` | 状态 != STOPPED | 见 3.3 | instance.py 第 151-168 行 |
| 7 | `RUNNING/STARTING/ERROR → STOPPING → STOPPED` | `stop()` | 状态 != STOPPED | 置 STOPPING;`client.stop()`;置 STOPPED | instance.py 第 161-167 行 |
| 8 | `STOPPED → STOPPED`(no-op) | `stop()` | 状态 == STOPPED | 直接返回 | instance.py 第 158-159 行 |
| 9 | `STARTING → STARTING` / `RUNNING → RUNNING`(no-op) | `start()` | 状态在 {STARTING, RUNNING} | 直接返回(幂等) | instance.py 第 96-97 行 |

### 2.3 关键常量(崩溃恢复相关)

| 常量 | 值 | 位置 | 语义 |
|---|---|---|---|
| `MAX_CRASH_RECOVERY_ATTEMPTS` | `3` | constants.py 第 25 行 | ERROR 状态下允许的自动恢复启动次数上限;**达到上限后 `start()` 抛 `RuntimeError`**(instance.py 第 100-104 行) |
| `MAX_RETRIES_FOR_CONTENT_MODIFIED` | `3` | constants.py 第 8 行 | ContentModified(-32801) 重试次数上限(共 4 次尝试) |
| `RETRY_BASE_DELAY_MS` | `500` | constants.py 第 11 行 | 指数退避基数:第 n 次重试前睡 `500 * 2^n` ms |

### 2.4 错误行为(状态机相关)

- **`start()` 在 ERROR 且 `crash_count >= MAX_CRASH_RECOVERY_ATTEMPTS` 时**抛 `RuntimeError`:
  `"Server '{server_id}' exceeded max crash recovery attempts ({MAX}); last error: {last_error}"`(instance.py 第 100-104 行)。
- **`start()` 成功会重置 `_crash_count` 为 0**(第 135 行):只要成功启动一次,恢复额度刷新。
- **`_handle_crash` 在 STOPPING 状态下直接返回**(第 244-245 行):停止流程中到达的崩溃回调被忽略。
- **`_on_crash` 在 client 内只执行一次**(`_crash_reported` 守卫,client.py 第 412-414 行):子进程多次退出只产生一次崩溃上报。

### 2.5 ⚠️ 重要行为细节(1:1 移植必须保留)

1. **干净退出不触发 RUNNING→ERROR**:`_read_loop` 读到 EOF(`readline()` 返回空)时**正常 return**,不调用 `_on_crash`(client.py 第 222-223 行)。`_on_crash` 只在读循环**抛异常**时触发(第 252-256 行)。因此:
   - 子进程在两帧之间正常退出(exit code 0)→ 实例保持 RUNNING,`is_alive` 变 False;
   - 这种"僵尸"由 manager 的 `is_healthy()` 检查发现并重启(manager.py 第 216-224 行)。
   - 若进程在**帧中间**退出导致 `readexactly` 抛异常 → 触发 `_on_crash(None)` → RUNNING→ERROR。
2. **`_is_stopping` 字段(instance 层无此字段;client 层有)只写不读**:client.py 第 46、144 行写入,无任何读取点。Rust 移植为 1:1 可保留该字段(或按死代码省略,但需在评审中显式说明)。
3. **`on_error` 回调(instance 构造参数)**:manager 传入 `lambda e: self._log_server_error(active_config.server_id, e)`(manager.py 第 270-272 行),只做日志,不改变状态。

---

## 3. LSPServerInstance(`core/instance.py`,261 行)

### 3.1 构造与字段

```text
LSPServerInstance.__init__(config: ScopedLspServerConfig, on_error: Callable[[Exception], None] | None = None)
```

| 私有字段 | 初始值 | 语义 |
|---|---|---|
| `_config` | 构造参数 | 实例配置 |
| `_on_error` | 构造参数(可 None) | 崩溃回调(仅日志) |
| `_state` | `STOPPED`(第 54 行) | 状态机 |
| `_crash_count` | `0`(第 55 行) | 崩溃次数 |
| `_last_error` | `None`(第 56 行) | 最后错误(`Exception`) |
| `_client` | `None`(第 57 行) | 底层 LSPClient |

### 3.2 属性(只读)

| 属性 | 实现 | 语义 |
|---|---|---|
| `name` | `config.server_id`(第 59-61 行) | 名称 = server_id |
| `config` | `_config`(第 63-65 行) | 配置引用 |
| `state` | `_state`(第 67-70 行) | 当前状态 |
| `running` | `_state == RUNNING`(第 72-75 行) | 是否运行 |
| `crash_count` | `_crash_count`(第 77-80 行) | 崩溃次数 |
| `last_error` | `_last_error`(第 82-85 行) | 最后错误 |

### 3.3 `async start()`(第 87-149 行)

1. 状态在 {STARTING, RUNNING} → 直接返回(幂等)。
2. 状态为 ERROR:
   - `crash_count >= MAX_CRASH_RECOVERY_ATTEMPTS` → 抛 `RuntimeError`(见 2.4);
   - 否则记"restarting"日志。
3. 置 `_state = STARTING`;`_client = None`。
4. spawn:
   - `create_subprocess_exec(command, *args, ...)`(第 117-125 行);
   - `env`:**`config.env` 非空时**为 `{**os.environ, **config.env}`(子进程 env 合并父进程环境);**为空时传 `None`**(继承父进程环境,第 120 行);
   - `cwd = config.workspace_folder`;
   - stdin/stdout/stderr 均接管道。
5. 创建 `LSPClient(config, process, on_exit=lambda code: self._handle_crash(code))`(第 127-131 行)。
6. `await client.initialize()`(完整握手,见第 4 章);成功后:
   - `_state = RUNNING`;`_crash_count = 0`;`_last_error = None`;记"started successfully (root=...)"日志。
7. 异常路径:置 ERROR、记录错误;若已有 client → `await client.stop()` 并置 None;**重新抛出异常**(第 143-149 行)。

> ⚠️ 注意:`start()` 内部**没有超时**;超时由 manager `_start_server` 的 `wait_for(task, startup_timeout/1000)` 施加(manager.py 第 288 行)。

### 3.4 `async stop()`(第 151-168 行)

1. 状态 == STOPPED → 直接返回。
2. 置 `_state = STOPPING`。
3. 若 `_client` 非空:`await client.stop()`(graceful shutdown/exit + terminate + 清理,见 4.7);置 `_client = None`。
4. 置 `_state = STOPPED`;记"stopped"日志。

### 3.5 `async send_request(method, params) -> Any`(第 170-196 行)

- 前置:`not running or not _client` → 抛 `RuntimeError(f"Server '{server_id}' not running (state={state.value})")`。
- ContentModified 自动重试(共 `MAX_RETRIES_FOR_CONTENT_MODIFIED + 1 = 4` 次尝试):
  - 尝试 0..3;`LSPError` 且 `code == LSP_ERROR_CONTENT_MODIFIED(-32801)` 且 `attempt < 3` → 睡 `RETRY_BASE_DELAY_MS * 2^attempt` ms(`500/1000/2000` ms)后重试;
  - 其他异常或重试耗尽 → `raise error from last_error`(Python 链式;last_error 记录最后一次 ContentModified)。
- 语义:请求级超时由 client 内部 `DEFAULT_REQUEST_TIMEOUT_MS` 处理(见 4.8);此处**不**额外加超时。

### 3.6 `async send_notification(method, params)`(第 198-202 行)

- `not running or not _client` → **静默返回**(不报错)。
- 否则转发 `client.send_notification`。

### 3.7 `add_notification_handler(method, handler)`(第 204-221 行)

- `_client` 非空时转发到 client;为空时**静默忽略**。
- 文档注明:建议在 `start()` 之后注册,或依赖 manager 的 `_ensure_diagnostic_handler`(在 open/change 前注册)。

### 3.8 `async is_healthy() -> bool`(第 223-234 行)

- `True` 当且仅当:`_state == RUNNING` 且 `_client` 非空且 `client.is_alive`(`process.returncode is None`)。
- 否则 `False`(注意:状态 RUNNING 但进程已退出时返回 False,用于 manager 僵尸检测)。

### 3.9 `_handle_crash(code: int | None)`(第 236-261 行)

见 2.2 迁移 #4。额外要点:
- `code` 为 None 时(读循环异常路径)错误消息为 `"Server '{id}' exited with code None"`。
- 记 warning 日志 `"crashed (crash #{n}, state={state.value})"`。
- 最后调用 `on_error(self._last_error)`(仅日志,不影响状态)。

---

## 4. LSPClient(`core/client.py`,421 行)— stdio JSON-RPC 层

### 4.1 构造与字段

```text
LSPClient.__init__(config: Any, process: asyncio.subprocess.Process, on_exit: Callable[[int | None], None])
```

| 私有字段 | 初始值 | 语义 |
|---|---|---|
| `_config` | 参数 | 即 ScopedLspServerConfig |
| `_process` | 参数 | 子进程句柄 |
| `_on_exit_callback` | 参数 | 崩溃上报回调(instance 的 `_handle_crash`) |
| `_capabilities` | `None`(第 42 行) | initialize 响应中的 capabilities |
| `_is_initialized` | `False`(第 43 行) | 握手完成标志 |
| `_reader_task` | `None`(第 44 行) | `lsp-reader` 读循环任务 |
| `_pending` | `{}`(第 45 行) | `msg_id(str) → (Future, method)` 在途请求表 |
| `_is_stopping` | `False`(第 46 行) | ⚠️ 只写不读(见 2.5.2) |
| `_crash_reported` | `False`(第 50 行) | 崩溃上报一次性守卫 |
| `_stderr_task` | `None`(第 55 行) | stderr 消费任务 |
| `_notification_handlers` | `{}`(第 57 行) | `method → [handler(params)]` 列表 |

属性:`capabilities`(第 59-61 行)、`is_initialized`(第 63-65 行)、`is_alive`(第 67-70 行,`returncode is None`)。

### 4.2 帧协议(stdio,单 reader 协程模式)

**编码(写出)**:`Content-Length: {len(body)}\r\n\r\n` + `body`(utf-8)。应用于:
- 请求(第 392-398 行):`{"jsonrpc":"2.0","id":msg_id,"method":m,"params":p}`;写后 `await stdin.drain()`。
- 通知(第 403-408 行):`{"jsonrpc":"2.0","method":m,"params":p}`;无 `id`。
- 响应(第 362-364 行):`{"jsonrpc":"2.0","id":id,"result":...}`。

**解码(`_read_loop`,第 202-264 行)**:
1. 逐行读头:`readline()`;空行(`b''`)→ 正常 return(EOF,**不触发崩溃上报**);
2. 行按 `ascii` 解码(`errors="replace"`);`"\r\n"` 空行结束头;含 `:` 的行按 `partition(": ")` 拆 key/value 并 strip;
3. 取 `Content-Length` 头:`int()` 失败 → `continue`(跳过该消息);`<= 0` → `continue`;
4. `await reader.readexactly(content_length)` 读 body;`utf-8 errors="replace"` 解码后 `lstrip("\r\n")`;body 空白 → `continue`;
5. `json.loads` 失败 → `continue`(静默丢弃坏帧);
6. `_dispatch(message)`。
7. 异常处理:非 CancelledError → 记 error 日志并 `_on_crash(None)`;CancelledError 重新抛出;`finally` 中取消 stderr 任务并将 `_stderr_task=None`(第 257-264 行)。

**stderr 消费(`_consume_stderr_forever`,第 330-338 行)**:启动读循环时创建(第 213-215 行),循环 `read(4096)` 直到 EOF,异常仅记 debug 日志;目的:防 Windows/Python 3.13+ 下 stderr 缓冲区满阻塞 stdout 读取。

### 4.3 `async initialize()`(第 72-117 行)— 完整握手

1. 创建 `_read_loop` 任务(命名 `lsp-reader`)。
2. 构造 initialize 请求参数(第 78-100 行):
   - `processId`: 客户端 PID(`os.getpid()`);
   - `rootUri`: `path_to_file_uri(config.workspace_folder)`;
   - `workspaceFolders`: `[{uri: path_to_file_uri(workspace_folder), name: "workspace"}]`;
   - `capabilities`:
     - `window.workDoneProgress = true`;
     - `workspace.applyEdit = true`;`workspace.workspaceEdit.documentChanges = true`;`workspace.workspaceFolders = true`;`workspace.configuration = true`;
     - `textDocument.synchronization.didOpen = true`;`didChange.willSave/willSaveWaitUntil/save = true`;
     - `textDocument.publishDiagnostics.versionSupport = true`;
   - 若 `config.initialization_options` 非空 → 追加 `initializationOptions`。
3. `await _rpc_request("initialize", params)`;`_capabilities = result.get("capabilities", {})`(第 104-105 行)。
4. `await _rpc_notification("initialized", {})`(第 107 行)。
5. `await _rpc_notification("workspace/didChangeConfiguration", {"settings": config.initialization_options or {}})`(第 109-112 行)——注释:主动发配置避免 pyright 等配置请求。
6. `await asyncio.sleep(0.1)`(第 114 行)——等待 reader 处理完配置请求。
7. `_is_initialized = True`;返回完整 initialize 响应 dict(第 116-117 行)。

> ⚠️ 若握手期间服务器请求 `workspace/configuration`,`_handle_server_request` 返回空数组 `result: []`(第 342-344 行)。

### 4.4 `send_request(method, params) -> Any`(第 130-134 行)

- `not _is_initialized` → 抛 `RuntimeError("Client not initialized")`。
- 否则 `await _rpc_request(method, params)`。

### 4.5 `send_notification(method, params)`(第 136-140 行)

- `not _is_initialized` → **静默返回**。
- 否则 `await _rpc_notification(method, params)`。

### 4.6 `add_notification_handler(method, handler)`(第 119-128 行)

- `_notification_handlers.setdefault(method, []).append(handler)`;同方法多 handler 按注册顺序调用;可在 initialize 前注册。

### 4.7 `async stop()`(第 142-200 行)— graceful 停止

1. `_is_stopping = True`。
2. `await wait_for(_rpc_request("shutdown", {}), 2.0)`;**任何异常仅记 debug 日志并忽略**(第 146-149 行)。
3. `await _rpc_notification("exit", {})`;异常同样忽略(第 150-153 行)。
4. 取消 reader 任务并 await(吞 CancelledError)。
5. 取消 stderr 任务并 await(若未 done;吞 CancelledError)——**显式取消,不依赖 _read_loop 的 finally**。
6. 依次关闭 stdin/stdout/stderr(异常仅 debug 日志)。
7. `process.terminate()`;`await wait_for(process.wait(), 5.0)`;超时 → `process.kill()` 并 wait(第 187-192 行)。
8. 将所有在途 future 置 `ConnectionError("LSP client stopped")`(若未 done);清空 `_pending`(第 194-197 行)。
9. `_is_initialized = False`;`_capabilities = None`(第 199-200 行)。

### 4.8 `_rpc_request(method, params)`(第 370-399 行)— 超时与取消

- `msg_id = str(uuid.uuid4())`(v4,字符串形式作为 JSON-RPC id)。
- 在运行 loop 上创建 future。
- 超时守卫:`loop.call_later(DEFAULT_REQUEST_TIMEOUT_MS/1000 = 15s, on_timeout)`;`on_timeout` 在 future 未 done 时置 `LSPError(-1, "Request timeout after 15000ms: {method}")`。
- future 的 done 回调取消定时器(`cleanup`,第 385-388 行)。
- 注册 `_pending[msg_id] = (future, method)`。
- 发送帧(见 4.2)→ `return await future`。

> 移植注意:超时是通过**延迟置异常**实现的;发送后 `await` 挂起直到响应/错误/超时。Rust 对应 `tokio::time::timeout` + 在途表,或 channel + 定时器,二者语义等价:超时后未来到达的响应因 `_pending` 中已无该 id 而被忽略(见 4.9 分支 3 的"找不到条目→return")。

### 4.9 `_dispatch(message)`(第 266-328 行)— 三分支分发

**分支 1:`id` 与 `method` 都存在(服务器发起请求)**:
- 若 `str(id)` 在 `_pending` 中(理论上不应发生,防御性):按响应处理(见下)。
- 否则:**服务器发起的请求**,`asyncio.create_task(_handle_server_request(method, id, params))` 异步处理,不阻塞读循环(第 291 行)。

**分支 2:`id` 为 None 且 `method` 存在(服务器推送通知)**:
- 查 `_notification_handlers[method]`;有 handler → 逐个调用 `handler(params)`,单个 handler 异常仅 debug 日志(第 300-306 行);
- 无 handler 且 method 以 `"window/"` 或 `"telemetry/"` 开头 → debug 日志;其余静默。

**分支 3:`id` 存在、无 `method`(对我们请求的响应)**:
- `str(id)` 查 `_pending`;无 → **直接 return**(迟到响应/未知 id 被丢弃);
- future 已 done → return;
- 含 `error` → `future.set_exception(LSPError(...))`;否则 `future.set_result(result)`;
- **先解析 future,再从 `_pending` 移除**(第 279-280、320-321 行注释:保证 done 回调检查 `_pending` 时仍可见)。

### 4.10 `_handle_server_request(method, id, params)`(第 340-357 行)

| method | 响应 result | 位置 |
|---|---|---|
| `workspace/configuration` | `[]` | 第 342-344 行 |
| `workspace/workspaceFolders` | `[]` | 第 345-347 行 |
| `client/registerCapability` | `None` | 第 348-350 行 |
| `client/unregisterCapability` | `None` | 第 351-353 行 |
| 其他 | `None`(debug 日志) | 第 354-357 行 |

### 4.11 `_on_crash(code: int | None)`(第 410-421 行)

1. `_crash_reported` 已置 → 返回(一次性)。
2. 置 `_crash_reported = True`;`_is_initialized = False`。
3. 所有在途 future 置 `ConnectionError(f"LSP server crashed with code {code}")`;清空 `_pending`。
4. 调用 `_on_exit_callback(code)`(→ instance `_handle_crash` → RUNNING→ERROR)。

### 4.12 同步(文本同步)相关(manager 层,见 6.7-6.8)

client 本身不解析 didOpen/didChange;由 manager 构造并发送。同步模式为 **LSP full sync**(didChange 的 contentChanges 为单条无 range 的全量文本替换,manager.py 第 431-433、475 行注释)。

---

## 5. LSPServerManager(`core/manager.py`,504 行)

### 5.1 单例与类级状态

| 类成员 | 初始值 | 语义 |
|---|---|---|
| `_instance` | `None`(第 38 行) | 全局单例 |
| `_lock` | `None`(第 39 行) | 初始化互斥锁;**惰性创建**(第 41-46 行 `_get_lock`),避免绑定创建时 event loop;`shutdown` 时置 None(第 121 行) |

### 5.2 `async initialize(options=None) -> InitializeResult`(类方法,第 48-113 行)

1. `opts = options or InitializeOptions()`。
2. 若 `_instance` 已存在 → 返回 `InitializeResult(success=True, servers_loaded=len(_instance.get_status()))`(幂等快路径)。
3. 加锁;锁内再查 `_instance` → 返回 `InitializeResult(success=True, servers_loaded=0)`(双检)。
4. `cwd` 解析(第 74-78 行):`opts.cwd or os.getcwd()`;`Path(cwd).resolve()` 失败时回退 `os.getcwd()`。
5. `configs = await build_configs_async(opts, cwd)`。
6. `configs` 为空 → `_instance = None`;返回 `(success=True, servers_loaded=0)`。
7. 构建扩展索引(第 86-93 行):`extension_map: dict[str(小写扩展名), list[server_id]]`;遍历每个 config 的 `extension_to_language.keys()`,`ext.lower()` 归一化;**按 configs 顺序追加**,同一 server_id 不重复。
8. 构建 `server_configs_dict: dict[server_id, list[ScopedLspServerConfig]]`(第 96-100 行),按 server_id 分组。
9. 填充 manager 字段:`_configs`、`_instances={}`、`_spawning={}`、`_extension_map`、`_workspace_root=cwd`(第 101-105 行);`cls._instance = manager`。
10. 返回 `InitializeResult(success=True, servers_loaded=len(configs), duration_ms=0.0)`。

### 5.3 `async shutdown()`(类方法,第 115-121 行)

- 有实例 → `await _instance.stop_all()`;`_instance = None`;`_lock = None`(允许新 loop 重建锁)。

### 5.4 `async stop_all()`(第 123-141 行)

1. 取消全部 `_spawning` 任务;`gather(return_exceptions=True)` + `wait_for(..., 5.0)`(TimeoutError 忽略)。
2. 清空 `_spawning`。
3. 对全部实例 `inst.stop()` 并发 `gather(return_exceptions=True)`;清空 `_instances`。

### 5.5 实例方法状态与属性

`__init__`(第 159-171 行):

| 字段 | 类型 | 语义 |
|---|---|---|
| `_workspace_root` | `str` = `""` | 初始化时的工作区根 |
| `_configs` | `dict[str, list[ScopedLspServerConfig]]` | server_id → 配置列表 |
| `_instances` | `dict[ServerInstanceKey, LSPServerInstance]` | 运行实例缓存 |
| `_spawning` | `dict[ServerInstanceKey, Task]` | 启动中任务 |
| `_extension_map` | `dict[str, list[str]]` | 小写扩展名 → server_id 列表 |
| `_diag_handler_instances` | `set[int]` | 已注册 publishDiagnostics handler 的实例对象 id(`id(server)`) |
| `_doc_versions` | `dict[str(uri), int]` | 文档版本号 |

`get_workspace_root()`(第 173-175 行)、`get_instance()`(类方法,第 143-146 行)、`get_status()`(类方法,第 148-157 行:`LspStatus(initialized=_instance is not None, servers=...)`)。

### 5.6 `async get_or_start_server(file_path) -> LSPServerInstance | None`(第 177-238 行)

算法(顺序执行):
1. `ext = Path(file_path).suffix.lower()`;`server_ids = _extension_map.get(ext, [])`(第 192-193 行)。
2. 对每个 `server_id`:
   a. `server_def = BUILTIN_SERVERS.get(server_id)`;不存在 → `continue`(第 196-198 行)。
   b. `root_result = server_def.find_root(file_path)`;`asyncio.iscoroutine` 则 await(第 200-204 行);`root is None` → `continue`。
   c. 对 `_configs.get(server_id, [])` 中每个 config:
      - `key = ServerInstanceKey(server_id, root)`。
      - 若 `key in _instances`:
        - `instance.running` 为 True:若 `is_healthy()` → **直接返回**;否则视为**僵尸**,记日志、`_instances.pop(key)` 后**落入重启**(第 216-224 行)。
        - 状态为 ERROR:`_instances.pop(key)`(交给 `_start_server` 做崩溃恢复决策;第 225-227 行)。
        - `key in _spawning`:await 该任务;若 `_instances[key].running` → 返回(第 228-231 行)。
      - `instance = await _start_server(key, config, root)`;若 `instance` 且 `instance.running` → 返回(第 234-236 行)。
3. 返回 `None`(无可用 server)。

### 5.7 `async _start_server(key, config, root=None)`(第 240-303 行)

1. `key in _instances` 且 running → 返回既有实例。
2. `key in _spawning` → await 任务,返回 `_instances.get(key)`(去重并发启动)。
3. **动态根**:若 `root` 非空且 `root != config.workspace_folder` → 以 `root` 为 `workspace_folder` **复制**出新 config(`active_config`),其余字段原样(第 255-265 行);否则 `active_config = config`。
4. 构造 `LSPServerInstance(active_config, on_error=lambda e: _log_server_error(active_config.server_id, e))`。
5. 包装任务 `start_and_cache`:`await instance.start()`;异常 → `_log_server_error`;`finally: _spawning.pop(key)`(第 274-281 行)。
6. `_instances[key] = instance`;`task = create_task(start_and_cache())`;`_spawning[key] = task`。
7. `await wait_for(task, active_config.startup_timeout / 1000)`(超时秒 = ms/1000):
   - `TimeoutError` → `task.cancel()`;记 `RuntimeError(f"Server start timeout after {timeout}ms")`;从 `_spawning`、`_instances` 同时移除(防僵尸缓存);返回 `None`(第 289-297 行)。
   - 其他异常 → 记日志;`_instances.pop(key)`;返回 `None`(第 298-301 行)。
8. 返回 `instance if instance.running else None`。

### 5.8 其他方法

- `_path_belongs_to_root(file_path, root) -> bool`(静态,第 305-314 行):`Path.resolve()` 后 `relative_to` 判断;**⚠️ 当前包内无任何调用点**(保留/死代码)。
- `_log_server_error(server_id, error)`(静态,第 316-319 行):warning 日志 `"[LSP] Server '{id}' failed: {error}"`。
- `_ensure_diagnostic_handler(server)`(第 321-358 行):
  - 守卫 `id(server) in _diag_handler_instances` → 幂等返回;
  - `registry = LspDiagnosticRegistry.get_instance()`;`server_name = server.config.server_id`;
  - handler(`_publish_diagnostics_handler`):`params` 非 dict → return;`uri = params.get("uri","")`;`raw = params.get("diagnostics",[])`;`not uri or not isinstance(raw, list)` → return;否则 `registry.register(server_name, uri, raw)`;
  - `server.add_notification_handler("textDocument/publishDiagnostics", handler)`;加入守卫集合。
- `get_pending_diagnostics(max_per_file=10, max_total=30)`(类方法,第 360-380 行):转发注册表 `get_and_clear`。
- `is_file_open(uri) -> bool`(第 382-384 行):`uri in _doc_versions`。
- `async send_request(file_path, method, params)`(第 479-489 行):`server = get_or_start_server(file_path)`;为 None → 抛 `RuntimeError(f"No LSP server for file: {file_path}")`;否则 `server.send_request(...)`。
- `get_status() -> list[LspServerStatus]`(第 491-504 行):对 `_instances.values()` 生成快照(`name=server_id`、`root=config.workspace_folder`、`last_error=str(...)`)。

### 5.9 文档同步(didOpen / didChange / 版本)

`async open_file(file_path, language_id)`(第 386-421 行):
1. `server = get_or_start_server(file_path)`;为 None → 返回。
2. `_ensure_diagnostic_handler(server)`(先于通知注册,捕获 didOpen 触发的全文件诊断)。
3. `text = Path(file_path).read_text(encoding="utf-8")`;`OSError` → `text = ""`(仅 debug 日志)。
4. `uri = path_to_file_uri(file_path)`;`_doc_versions[uri] = 0`。
5. 通知 `textDocument/didOpen`:
   ```json
   {"textDocument": {"uri": uri, "languageId": language_id, "version": 0, "text": text}}
   ```

`async change_file(file_path, language_id, content=None)`(第 423-477 行):
1. `server = get_or_start_server(file_path)`;为 None → 返回。
2. `_ensure_diagnostic_handler(server)`。
3. `content is None` → 从磁盘读(OSError → `""`)。
4. `uri = path_to_file_uri(file_path)`;`version = _doc_versions.get(uri, 0) + 1`;写入 `_doc_versions[uri]`(第 465-466 行)。**未 open 过时首个版本为 1**。
5. 通知 `textDocument/didChange`(full sync,无 range):
   ```json
   {"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": content}]}
   ```

> 注意:manager 层**没有 didClose** 方法;`_doc_versions` 只增不减(除非整个 manager 重建)。

---

## 6. LspDiagnosticRegistry(`core/diagnostic_registry.py`,283 行)

### 6.1 常量

| 常量 | 值 | 位置 | 语义 |
|---|---|---|---|
| `MAX_DIAG_PER_FILE` | `10` | 第 28 行 | 每文件默认上限 |
| `MAX_DIAG_TOTAL` | `30` | 第 29 行 | 全局默认上限 |

### 6.2 数据结构

`LspDiagnosticItem`(第 36-44 行):

| 字段 | 类型 | 语义 |
|---|---|---|
| `message` | `str` | 诊断消息(必填;空消息条目被丢弃) |
| `severity` | `int` | **1=Error, 2=Warning, 3=Info, 4=Hint** |
| `range` | `dict` | LSP Range 原始 dict(不校验结构) |
| `source` | `str \| None` | 来源;空/假值归一为 None |
| `code` | `str \| int \| None` | 错误码,原样保留 |

`LspDiagnosticFile`(第 47-54 行):

| 字段 | 类型 | 语义 |
|---|---|---|
| `uri` | `str` | 文件 URI |
| `diagnostics` | `list[LspDiagnosticItem]` | 该文件本次交付的诊断 |
| `server_name` | `str` | 产生诊断的 server_id(批次中第一个注册者) |
| `local_path` | `str` | `file_uri_to_path(uri)` 解析的本地路径 |

### 6.3 内部结构(`__init__`,第 149-153 行)

- `_pending: dict[batch_id(str), tuple(server_name, uri, list[LspDiagnosticItem])]` —— 待交付批次(按 UUID 键)。
- `_delivered: dict[uri, set[dedup_key]]` —— 已交付过的去重键(跨轮抑制)。

### 6.4 单例

- `get_instance()`(类方法,第 133-138 行):进程全局单例,惰性创建。
- `reset()`(类方法,第 140-143 行):销毁单例(测试隔离用)。
- `pending_count` 属性(第 159-162 行):`len(_pending)`。

### 6.5 `register(server_name, uri, raw_diagnostics) -> str`(第 164-186 行)

1. `items = _parse_raw(raw_diagnostics)`;无有效条目 → 返回 `""`。
2. `batch_id = str(uuid.uuid4())`;存入 `_pending[batch_id] = (server_name, uri, items)`。
3. 返回 batch_id。

**`_parse_raw`(第 61-87 行)**:遍历原始 LSP `diagnostics` 数组——
- 非 dict 条目 → 丢弃;
- `message` 空/缺失 → 丢弃;
- `severity` 缺失 → 默认 `3`(Info);
- `range` 缺失 → `{}`;
- `source` 假值 → `None`;
- `code` 原样。

**`_diag_key(item) -> str`(第 90-100 行)** —— 稳定去重键:
```
f"{message}|{severity}|{line}:{char}|{code}"
```
其中 `line = range.start.line or 0`、`char = range.start.character or 0`(`range` 非 dict 或 `start` 缺失时取 0)。

### 6.6 `get_and_clear(max_per_file=10, max_total=30) -> list[LspDiagnosticFile]`(第 188-278 行)

**输入**:两个上限参数(默认 10/30)。
**空队列**:`_pending` 为空 → 返回 `[]`。

算法(6 步,顺序严格):
1. **按 uri 合并批次 + 轮内去重**(第 211-223 行):`by_uri: dict[uri, (server_name, dict[key→item])]`;遍历 `_pending.values()`(注意:server_name 取该 uri 的**首个**批次);每个 item 计算 `_diag_key`,已存在则跳过(保序插入,保留首次出现);随后 `_pending.clear()`。
2. **跨轮去重**(第 225-239 行):对每个 uri,取 `_delivered[uri]`;仅保留 key 不在已交付集合中的 item;若某 uri 无新条目则整体排除。**全部无新条目 → 返回 `[]`**。
3. **按 severity 升序排序**(第 241-244 行):`sorted(items, key=severity)` —— Error(1) 最先;**排序稳定**(Python sort 稳定,同 severity 保持插入序)。
4. **每文件上限**(第 246-250 行):`len > max_per_file` → 截断 `items[:max_per_file]`(保留最严重)。
5. **全局上限**(第 252-268 行):按 `by_uri` 的**插入顺序**遍历;累计 `total >= max_total` 即 `break`;每条取 `items[:remaining]`;生成 `LspDiagnosticFile(uri, diagnostics=clipped, server_name, local_path=file_uri_to_path(uri))`;`total += len(clipped)`。
6. **更新交付历史**(第 270-277 行):对每个返回文件,把其全部 `_diag_key` 加入 `_delivered[uri]`。

**返回**:`list[LspDiagnosticFile]`。

> 移植注意:
> - 去重键**不含** range 的 end 与 source,仅 message/severity/start/line/char/code;
> - `_delivered` 是**只增**集合(无过期),意味着同一诊断不会在连续轮次重复出现,直到 `clear_all`;
> - uri 遍历顺序 = `_pending` 插入顺序(合并时首次出现顺序),全局上限截断依赖该顺序;
> - `local_path` 在**交付时**才解析,不缓存。

### 6.7 `clear_all()`(第 280-283 行)

- 清空 `_pending` 与 `_delivered`(完全重置,后续同键诊断可再次交付)。

### 6.8 线程/异步安全模型

- 官方注释(第 107-125 行):所有公共方法只在 asyncio 事件循环线程调用(通知 handler 在 reader 任务中运行;`get_and_clear` 由协程调用),**无需额外锁**。
- Rust 移植:若跨任务共享,需 `Mutex`;若保持单线程执行器语义,可无锁。语义上必须满足:register 与 get_and_clear 之间是**顺序一致的**(无并发交错)。

---

## 7. servers 注册表与各语言配置(`servers/registry.py`、`servers/servers/*`)

### 7.1 注册与导入顺序

- `BUILTIN_SERVERS: dict[str, ServerDefinition] = {}`(registry.py 第 16 行)。
- 注册是**模块导入副作用**:`servers/__init__.py`(第 5-6 行)依次 import `rust, typescript, java, python, go`,每个模块末尾执行 `BUILTIN_SERVERS[id] = server`。
- **插入顺序 = 注册顺序 = rust → typescript → java → python → go**(go.py 第 41 行、java.py 第 40-41 行、python.py 第 147-148 行、rust.py 第 85-86 行、typescript.py 第 49-50 行)。
- 该顺序影响 `build_configs_async` 的 configs 顺序 → 影响 `extension_map` 中 server_id 的列表顺序 → 影响 `get_or_start_server` 的尝试顺序(当前各语言扩展名互不重叠,实际无竞争)。

### 7.2 ⚠️ priority 与 global_server 的实际作用

- `ServerDefinition.priority`(默认 100,各内建均设 10)与 `global_server` 在**当前代码中没有任何读取点**(grep 确认:仅定义与赋值处)。`get_or_start_server` 完全按 `_extension_map` 列表顺序尝试,不按 priority 排序。
- Rust 移植:**保留字段与默认值**(保持数据模型 1:1),但选择逻辑同样不应依赖它们;如需将来启用,应在规格变更评审中单独设计。

### 7.3 `build_configs_async(options, cwd)`(registry.py 第 78-161 行)

对每个 `BUILTIN_SERVERS` 条目:
1. `spawn_result = server_def.spawn(cwd)`;若为协程则 await(第 92-96 行)。
2. **spawn 返回 None(二进制缺失)→ 占位配置**(第 98-114 行):`command=""`、`args=[]`、`env={}`、`workspace_folder=cwd`、`initialization_options=None`、`startup_timeout=45_000`、`extension_to_language={ext: server_def.language_id for ext in server_def.extensions}`。
3. **spawn 返回 SpawnHandle → 完整配置**(第 116-129 行):字段取自 handle;`workspace_folder=cwd`;`extension_to_language` 同上。

然后处理 `options.custom_servers`(第 131-159 行),对每个 `(server_id, custom)`:
- `custom.disabled` → 从 configs 中**删除**该 server_id 全部条目,`continue`;
- 已有同 server_id 条目:仅当 `custom.command` 非空时覆盖 command;`custom.args is not None` 时覆盖 args;`custom.initialization_options is not None` 时覆盖 initialization_options;**env 不覆盖**(第 137-144 行);
- 无既有条目 → 追加新配置:``command=custom.command or ""``、`args=custom.args or []`、`env=custom.env or {}`、`workspace_folder=cwd`、`initialization_options=custom.initialization_options`、`extension_to_language={ext: custom.language_id or server_id for ext in custom.extensions or []}`、`startup_timeout` 取 ScopedLspServerConfig 默认 `45_000`(第 145-159 行)。

`build_configs(options, cwd)`(第 164-169 行):`asyncio.run(build_configs_async(...))` 的同步包装(必须运行在无活动 loop 的线程)。

> ⚠️ 占位配置的 `command=""` 若被实际启动,`create_subprocess_exec("")` 必然失败 → ERROR → manager 记日志并返回 None;open_file/change_file 随后静默返回。这是"二进制缺失"的运行时表现。

### 7.4 `nearest_root(include_patterns, exclude_patterns=None, stop_dir=None)`(registry.py 第 19-75 行)

返回一个 async 闭包 `find_root(file_path) -> str | None`(通过 `asyncio.to_thread` 执行同步搜索避免阻塞事件循环):

1. `start_dir = Path(file_path).parent.resolve()`;解析异常 → `None`。
2. `stop = (Path(stop_dir) if stop_dir else Path.cwd()).resolve()`。
3. **先检查 start_dir 自身**(第 46-48 行):任一 include_pattern 存在 → 返回 `str(start_dir)`(注释:确保文件直接在 CWD 或起始目录含 exclude 模式如 .git 时也能找到根)。
4. `start_dir == stop` → 返回 `None`。
5. 从 `current = start_dir.parent` 向上遍历,`while current.parent != current`:
   a. 任一 include_pattern 存在于 `current` → 返回 `str(current)`;
   b. `current == stop` → 返回 `None`;
   c. 任一 exclude_pattern 存在于 `current` → 返回 `None`;
   d. `current = current.parent`。
6. 循环结束(到达文件系统根)→ 返回 `None`。

**语义要点**:include 检查先于 stop/exclude 检查;起始目录豁免 exclude 与 stop;`stop_dir=None` 时以 `Path.cwd()`(解析后)为界。

### 7.5 各语言 server 定义(全部 priority=10)

#### 7.5.1 Go — `go.py`(42 行)

| 项 | 值 | 位置 |
|---|---|---|
| id | `"gopls"` | 第 34 行 |
| extensions | `[".go"]` | 第 35 行 |
| language_id | `"go"` | 第 36 行 |
| find_root | `go_root` | 第 38 行 |
| spawn | `_spawn_go` | 第 39 行 |

- `go_root(file_path)`(第 12-17 行):先 `nearest_root(["go.work"])`(支持多模块 workspace);命中即返回;否则 `nearest_root(["go.mod","go.sum"])`。
- `_spawn_go(root)`(第 20-30 行):`shutil.which("gopls")` 失败 → None;否则 `SpawnHandle(command="gopls", args=[], initialization_options={"staticcheck": True}, startup_timeout=60_000)`。
  - ⚠️ 60_000 为**字面量**,未引用 `DEFAULT_GOPLS_TIMEOUT_MS`(该常量当前无读取点)。

#### 7.5.2 Java — `java.py`(41 行)

| 项 | 值 | 位置 |
|---|---|---|
| id | `"jdtls"` | 第 25 行 |
| extensions | `[".java"]` | 第 26 行 |
| language_id | `"java"` | 第 27 行 |
| find_root | `nearest_root(include=["pom.xml","build.gradle","build.gradle.kts",".project"], exclude=[".git"])` | 第 29-37 行 |
| spawn | `_spawn_java` | 第 38 行 |

- `_spawn_java(root)`(第 12-21 行):`cmd = shutil.which("jdtls")`;失败 → None;否则 `SpawnHandle(command=cmd, args=[], initialization_options=None)`(startup_timeout 取默认 45_000)。

#### 7.5.3 Python — `python.py`(148 行)

| 项 | 值 | 位置 |
|---|---|---|
| id | `"pyright"` | 第 130 行 |
| extensions | `[".py", ".pyi"]` | 第 131 行 |
| language_id | `"python"` | 第 132 行 |
| find_root | `nearest_root(include=["pyproject.toml","setup.py","setup.cfg","requirements.txt","Pipfile","pyrightconfig.json"], exclude=[".git"])` | 第 134-144 行 |
| spawn | `_spawn_python` | 第 145 行 |

**`_resolve_pyright_command() -> (command, args) | None`**(第 16-91 行),解析 pyright-langserver 到真实可执行:
1. `npm list -g --depth=0 pyright`(subprocess.run,`capture_output, text, encoding="utf-8", errors="replace"`):
   - returncode == 0 时解析 stdout 行:`prefix` = 含 `"pyright@"` 行的**上一行**(strip);`js_path = prefix/node_modules/pyright/langserver.index.js`;存在且 `shutil.which("node")` 非空 → 返回 `(node, [js_path, "--stdio"])`。
   - 任意失败仅 warning 日志,落入 fallback。
2. fallback:`raw = shutil.which("pyright-langserver")`;None → 返回 None;不以 `.CMD` 结尾(大小写不敏感)→ 返回 `(raw, ["--stdio"])`(直接执行)。
3. `.CMD` 解析:读文件(utf-8 errors="replace");取**第一个**同时含 `"node_modules"` 与 `".js"` 的行;按 `'"'` split 取第 2 段(`parts[1]`)为 `js_path`;`cmd_dir = raw 的父目录`;若 `js_path` 以 `"%dp0%\\"` 或 `"%dp0%/"` 开头 → 替换 `%dp0%` 为 cmd_dir;否则若相对 → `join(cmd_dir, js_path)`;`node = shutil.which("node")`;None → None;返回 `(node, [js_path, "--stdio"])`。

**`_spawn_python(root)`**(第 94-126 行):
1. 解析失败 → None。
2. `initialization_options = {}`。
3. venv 探测(第 103-120 行),候选依次:`os.environ.get("VIRTUAL_ENV")`、`root/.venv`、`root/venv`(跳过空值):
   - `python_path = Path(venv, "Scripts" if os.name=="nt" else "bin", "python").as_posix()`;
   - `candidate = python_path.replace("/bin/", "/Scripts/") if os.name=="nt" else python_path`(仅探测路径变换);
   - 首个 `Path(candidate).exists()` 命中 → `initialization_options["pythonPath"] = python_path`(**存储的是原 python_path,非 candidate**);break。
4. 返回 `SpawnHandle(command, args, initialization_options=initialization_options or None)`(startup_timeout 默认 45_000)。

#### 7.5.4 Rust — `rust.py`(86 行)

| 项 | 值 | 位置 |
|---|---|---|
| id | `"rust"` | 第 78 行 |
| extensions | `[".rs"]` | 第 79 行 |
| language_id | `"rust"` | 第 80 行 |
| find_root | `rust_root` | 第 82 行 |
| spawn | `_spawn_rust` | 第 83 行 |

- `_spawn_rust(root)`(第 66-74 行):`shutil.which("rust-analyzer")` 失败 → None;否则 `SpawnHandle(command="rust-analyzer", args=[])`。

**`rust_root(file_path)`**(第 17-63 行,`asyncio.to_thread` 内同步执行):
1. `start = Path(file_path).parent.resolve()`;`root=None`;`current = start`;`stop = Path(file_path).parent`(**未 resolve**)。
2. `while current != stop and current.parent != current:`:若 `current/Cargo.toml` 存在 → `root = str(current)`;break;否则上移。
3. 无 root → 返回 None。
4. 读取 `root/Cargo.toml`(OSError → 直接返回 root):含 `"[workspace]"` → 返回 root。
5. 否则从 `current = root` 向上:`current.parent/Cargo.toml` 存在且含 `"[workspace]"` → 返回 `str(current.parent)`;读失败仅 debug 日志;直到文件系统根。
6. 返回最初找到的 `root`。

> ⚠️ **1:1 移植必须保留的怪癖**:第 2 步循环条件 `current != stop` 在 `start == stop` 时(即文件路径无符号链接,`resolve()` 无变化)**立即为假,循环体一次都不执行 → 返回 None**。因此 symlink-free 路径下 `rust_root` 恒返回 None,rust server 不会被启动。仅在路径含符号链接(如 macOS `/tmp` → `/private/tmp`)时循环才会执行。这是现状行为(疑似 bug),移植时应**照抄**并在评审中标注,或经需求方确认后修正(修正即非 1:1)。

#### 7.5.5 TypeScript/JavaScript — `typescript.py`(50 行)

| 项 | 值 | 位置 |
|---|---|---|
| id | `"typescript"` | 第 34 行 |
| extensions | `[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"]` | 第 35 行 |
| language_id | `"typescript"` | 第 36 行 |
| find_root | `nearest_root(include=["package.json","package-lock.json","pnpm-lock.yaml","yarn.lock"], exclude=["deno.json","deno.jsonc"])` | 第 38-46 行 |
| spawn | `_spawn_ts` | 第 47 行 |

- `_spawn_ts(root)`(第 13-30 行):`shutil.which("typescript-language-server")` 失败 → None;`args = ["--stdio"]`;若 `root` 下**无** tsconfig.json 且无 jsconfig.json → 追加 `"--ignore-node-modules"`;返回 `SpawnHandle(command, args, initialization_options=None)`。

### 7.6 extension→languageId 映射总结

| 扩展名(小写归一) | languageId | server_id |
|---|---|---|
| `.go` | `go` | gopls |
| `.java` | `java` | jdtls |
| `.py`, `.pyi` | `python` | pyright |
| `.rs` | `rust` | rust |
| `.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs` | `typescript` | typescript |

- 映射构造:config 的 `extension_to_language`(值恒 = `server_def.language_id`,或 custom 的 `language_id or server_id`)。
- manager 扩展索引:`ext.lower()` 归一,因此打开文件时 `Path(file_path).suffix.lower()`(manager.py 第 192 行)与之匹配(大小写不敏感)。
- 注意 `.mjs/.cjs` 等同时被 languageId `"typescript"` 覆盖(非 `"javascript"`),didOpen 时传入的 languageId 由调用方决定(manager.open_file 的参数),与 server 定义无关。

---

## 8. 确定性工具函数

### 8.1 `file_uri.py`(57 行)

#### 8.1.1 `path_to_file_uri(file_path) -> str`(第 10-22 行)

- **Windows(`sys.platform == "win32"`,第 17-20 行)**:`"file:///" + Path(file_path).resolve().as_posix()`(再 `replace("\\","/")` 冗余防御)。**不做百分号编码**(含空格/特殊字符的路径原样保留)。
- **非 Windows(第 21-22 行)**:`"file://" + urllib.parse.quote(Path(file_path).resolve().as_posix(), safe="/:")` —— `/` 与 `:` 不编码,其余按 URL 编码(空格→`%20` 等)。
- 两边都先 `resolve()`(绝对化 + 解析符号链接)。

示例(非 Windows):`/home/user/a b.py` → `file:///home/user/a%20b.py`;Windows:`C:\My Docs\x.py` → `file:///C:/My Docs/x.py`(不编码)。

#### 8.1.2 `file_uri_to_path(uri) -> str`(第 25-57 行)

1. 不以 `"file://"` 开头 → **原样返回 uri**(第 35-36 行)。
2. 去前缀:`path = uri[7:]`。
3. `urllib.parse.unquote(path)`;`ValueError`(畸形百分号编码)→ 保留原串(第 42-45 行)。
4. **Windows**(第 47-55 行):
   - 若 `len >= 3 and path[0]=="/" and path[2]==":"` → 去掉开头的 `/`(即 `/C:/...` → `C:/...`);
   - `path = path.replace("/", "\\")`;
   - 若 `len >= 2 and path[1]==":"` → 盘符大写(`path[0].upper()` + 其余)。
5. 返回。

示例:Windows:`file:///d%3A/foo` → unquote → `/d:/foo` → `D:\foo`;`file:///C:/foo` → `C:\foo`;Linux:`file:///home/user/a%20b.py` → `/home/user/a b.py`;非 file URI(如 `untitled:...`)→ 原样返回。

> 移植注意:`unquote` 在 `%` 后无合法十六进制时 Python 保留字面量(不抛错路径以 ValueError 形式出现)。Rust 侧需复刻 `urllib.parse.unquote` 的宽容语义(无效序列保留原样)。

### 8.2 `git_ignore.py`(95 行)

#### 8.2.1 `filter_git_ignored_locations(locations, cwd) -> list`(第 19-95 行)— 依赖 `git` 子进程

输入:list[LSP Location 或 SymbolInformation dict];输出:过滤后的 list。

1. 空输入 → 原样返回。
2. **提取 uri**(第 39-52 行),每项按优先级取:`loc["uri"]` → `loc["targetUri"]` → `loc["location"]["uri"]`(仅当 `location` 是 dict);去重建立 `uri_to_path`。
3. `unique_paths = set(uri_to_path.values())`;空 → 原样返回。
4. **批量查询**:`batch_size = 50`,`timeout = 5.0`(秒/批);`git check-ignore --stdin`(cwd=cwd),stdin 为 `"\n".join(batch)`(utf-8);`communicate` 受 `wait_for` 5s 限制。
   - `returncode == 0` 且 stdout 非空 → 每行 strip 后加入 `ignored_paths`(第 77-81 行);
   - `OSError` / `TimeoutError` → debug 日志,**该批忽略**(第 82-83 行)。
5. `ignored_paths` 空 → 原样返回。
6. **过滤**(第 88-95 行):保留条件 = `loc["uri"]` 与 `loc["targetUri"]` 均缺失(**注意:不再检查 location.uri**)或解析路径不在 ignored_paths。

> ⚠️ 怪癖:提取阶段看 `location.uri`,过滤阶段只看顶层 `uri`/`targetUri`;仅有 `location.uri` 的条目其路径虽被收集,但过滤时视为"无 uri"→ 恒保留。

### 8.3 `constants.py`(30 行)— 全部常量表

| 常量 | 值 | 行号 | 语义 | 读取点 |
|---|---|---|---|---|
| `LSP_ERROR_CONTENT_MODIFIED` | `-32801` | 4 | RPC 错误码:内容已修改(可重试) | instance.py 第 189 行 |
| `MAX_RETRIES_FOR_CONTENT_MODIFIED` | `3` | 8 | ContentModified 最大重试次数 | instance.py 第 183 行 |
| `RETRY_BASE_DELAY_MS` | `500` | 11 | 退避基数(ms) | instance.py 第 192 行 |
| `DEFAULT_STARTUP_TIMEOUT_MS` | `45_000` | 15 | 默认启动超时(ms) | ⚠️ 无读取点(默认值以字面量 45_000 散落于 types.py/registry.py) |
| `MAX_LSP_FILE_SIZE_BYTES` | `10 * 1024 * 1024` = 10485760 | 18 | LSP 操作最大文件字节数 | ⚠️ 仅包级导出(`__init__.py` 第 12、99 行),包内无读取点 |
| `DEFAULT_GOPLS_TIMEOUT_MS` | `60_000` | 21 | gopls 专用启动超时(ms) | ⚠️ 无读取点(go.py 第 29 行硬编码 60_000) |
| `MAX_CRASH_RECOVERY_ATTEMPTS` | `3` | 25 | 崩溃恢复上限 | instance.py 第 100、103、109 行 |
| `DEFAULT_REQUEST_TIMEOUT_MS` | `15_000` | 29 | 单请求超时(ms) | client.py 第 14、380、383 行 |

---

## 9. 确定性 vs 环境依赖划分

### 9.1 纯确定性(不触子进程/网络;给定输入必然同输出)

| 模块/函数 | 说明 |
|---|---|
| `core/types.py` 全部类型 | 数据结构与枚举 |
| `types.py` 全部类型 | 同上 |
| `servers/types.py` `ServerDefinition` | 同上 |
| `LspServerState` 状态机决策(instance 的状态字段、start/stop 的守卫分支) | 逻辑本身纯;但 `start()` 内部调用 spawn/握手,属环境依赖(见下) |
| `LspDiagnosticRegistry` 全部(`register`/`get_and_clear`/`clear_all`/`_parse_raw`/`_diag_key`) | 纯内存;`local_path` 解析依赖 file_uri 纯函数 |
| `file_uri.path_to_file_uri` / `file_uri_to_path` | 纯(平台分支固定后);可对两平台分别做黄金用例测试 |
| `constants.py` | 常量表 |
| `nearest_root` 算法逻辑(给定目录快照) | 逻辑纯,但读取文件系统 → 归入"FS 依赖"更严谨 |
| `build_configs_async` 的 custom_servers 合并规则(给定 spawn 结果) | 合并/覆盖/删除规则纯;spawn 本身环境依赖 |
| manager 的 `extension_map` 构建、`ServerInstanceKey` 相等性、`get_status` 快照 | 纯 |
| `_diag_key`、去重/排序/截断算法 | 纯(输入构造好即可单测) |

### 9.2 环境依赖(子进程/文件系统/平台)

| 模块/函数 | 依赖 | 说明 |
|---|---|---|
| `LSPClient` 全部(stdin/stdout 读写、帧解析、超时、分发) | 活着的 server 子进程 | 帧编解码与分发逻辑可脱离进程单测(喂字节流);收发必须有进程 |
| `LSPServerInstance.start`(spawn、`client.initialize` 握手) | 子进程 + 二进制存在性 | 二进制缺失(placeholder `command=""`)→ ERROR |
| `LSPServerManager` 的 spawn/重启/僵尸检测编排 | 子进程生命周期 | 决策逻辑(分支)可 mock 后单测 |
| `nearest_root` / `rust_root` / `go_root` | 文件系统遍历 | 可对固定目录夹具测试 |
| `_spawn_*` 与 `shutil.which` 二进制探测 | PATH 环境 | 决定 SpawnHandle 或 None(占位配置) |
| `_resolve_pyright_command` | `npm`/`node`/pyright 安装、.CMD 文件 | Windows 特有 |
| `filter_git_ignored_locations` | `git` 子进程、仓库状态 | 批处理/超时/忽略失败语义 |
| `manager.open_file`/`change_file` 的磁盘读文件 | 文件系统 | OSError → 空串 |
| `path_to_file_uri`/`file_uri_to_path` 的平台分支 | `sys.platform` / `os.name` | 同一输入在 Win/Unix 输出不同(平台分支内仍是纯函数) |
| 环境变量(`VIRTUAL_ENV` 等) | 进程环境 | pyright venv 探测、spawn env 合并 |

### 9.3 测试策略建议(Rust 侧)

- **纯函数黄金测试**:`file_uri`(Win/Unix 两套期望)、`_diag_key`、`get_and_clear` 的 6 步算法(去重/排序/双上限/跨轮抑制/clear_all 重置)、`build_configs_async` 合并规则、`nearest_root` 目录夹具。
- **协议测试**:用假 server(echo 帧)或字节流夹具测 `LSPClient` 帧解析、超时置错、`_dispatch` 三分支、stop 流程(含 shutdown 超时忽略)。
- **集成测试**:真实二进制存在时走全链路;缺失时验证占位配置 + ERROR + manager 返回 None。

---

## 10. 逐行功能点索引

> 格式:`文件:行号 — 功能`。根 = `agent-core/openjiuwen/harness/lsp/`。

### 10.1 `core/types.py`

| 行 | 功能 |
|---|---|
| 10-35 | `LspServerState` 枚举定义与状态机文档注释 |
| 31-35 | 五个状态值 STOPPED/STARTING/RUNNING/STOPPING/ERROR |
| 38-51 | `SpawnHandle`(command/args/env/initialization_options/startup_timeout=45_000) |
| 54-69 | `ScopedLspServerConfig`(server_id/command/workspace_folder/args/env/initialization_options/startup_timeout/extension_to_language) |
| 72-89 | `LspServerStatus`(server_id/name/running/state=STOPPED/root/crash_count/last_error) |

### 10.2 `core/client.py`

| 行 | 功能 |
|---|---|
| 17-22 | `LSPError(code, message)` 与字符串化 |
| 25 | `_CONTENT_LENGTH = "Content-Length"` |
| 28-57 | `LSPClient.__init__` 与全部字段 |
| 59-70 | 属性 capabilities / is_initialized / is_alive |
| 72-117 | `initialize()` 握手(initialize 请求参数、capabilities、initialized、didChangeConfiguration、sleep 0.1s) |
| 78-100 | initialize 请求参数构造(processId/rootUri/workspaceFolders/capabilities) |
| 101-102 | initializationOptions 条件追加 |
| 104-107 | 解析 capabilities、发送 initialized |
| 109-114 | didChangeConfiguration + 100ms 等待 |
| 116-117 | 置 _is_initialized 并返回响应 |
| 119-128 | `add_notification_handler` |
| 130-134 | `send_request`(未初始化抛 RuntimeError) |
| 136-140 | `send_notification`(未初始化静默返回) |
| 142-200 | `stop()` 完整优雅停止(shutdown 2s 超时、exit、取消 reader/stderr、关管道、terminate→5s→kill、pending 置 ConnectionError) |
| 202-264 | `_read_loop`(帧解码、EOF 正常返回、异常→_on_crash(None)、finally 清理 stderr 任务) |
| 213-215 | stderr 消费任务创建 |
| 219-250 | 头解析(ascii/replace)、Content-Length、readexactly、utf-8、JSON 解析、坏帧静默跳过 |
| 252-256 | 读循环异常 → `_on_crash(None)` |
| 266-328 | `_dispatch` 三分支(服务器请求/通知/响应) |
| 273-292 | 分支 1:id+method;pending 命中按响应处理;否则异步处理服务器请求 |
| 296-309 | 分支 2:通知 handler 分发;window//telemetry/ 日志 |
| 312-328 | 分支 3:响应解析(先解析后移除、LSPError 构造) |
| 330-338 | `_consume_stderr_forever` |
| 340-357 | `_handle_server_request` 五类响应 |
| 359-368 | `_send_response`(写帧 + drain) |
| 370-399 | `_rpc_request`(uuid4 id、15s 超时 call_later、pending 注册、发送、await) |
| 401-408 | `_rpc_notification`(fire-and-forget) |
| 410-421 | `_on_crash`(一次性、pending 置 ConnectionError、on_exit 回调) |

### 10.3 `core/diagnostic_registry.py`

| 行 | 功能 |
|---|---|
| 28-29 | MAX_DIAG_PER_FILE=10 / MAX_DIAG_TOTAL=30 |
| 36-44 | `LspDiagnosticItem` |
| 47-54 | `LspDiagnosticFile`(含 local_path 解析) |
| 61-87 | `_parse_raw`(丢弃非 dict/空 message;severity 默认 3;source 归一) |
| 90-100 | `_diag_key`(message\|severity\|line:char\|code) |
| 127-143 | 单例 get_instance / reset |
| 149-153 | `_pending` / `_delivered` 结构 |
| 159-162 | pending_count |
| 164-186 | `register`(空批返回 "") |
| 188-278 | `get_and_clear` 六步算法 |
| 211-223 | 步骤 1-2:按 uri 合并 + 轮内去重、清空 pending |
| 225-239 | 步骤 3:跨轮去重;全空返回 [] |
| 241-244 | 步骤 4:severity 升序稳定排序 |
| 246-250 | 步骤 5:每文件上限截断 |
| 252-268 | 步骤 6:全局上限 + LspDiagnosticFile 组装(含 local_path) |
| 270-277 | 更新 delivered 历史 |
| 280-283 | `clear_all` |

### 10.4 `core/instance.py`

| 行 | 功能 |
|---|---|
| 21-45 | 类文档(状态机注释) |
| 47-57 | `__init__`(config/on_error/_state/_crash_count/_last_error/_client) |
| 59-85 | 六个只读属性 |
| 87-149 | `start()`(幂等、崩溃恢复守卫、spawn env 合并、握手、异常→ERROR+stop 清理+重抛) |
| 96-97 | STARTING/RUNNING 幂等返回 |
| 99-110 | ERROR 恢复:超限抛 RuntimeError / 记 restarting 日志 |
| 112-125 | 置 STARTING、spawn 参数(env 合并/cwd/三管道) |
| 127-136 | 构造 LSPClient、握手、RUNNING、重置计数 |
| 143-149 | 异常路径:ERROR、client.stop、重抛 |
| 151-168 | `stop()`(STOPPED 短路、STOPPING、client.stop、STOPPED) |
| 170-196 | `send_request`(非 RUNNING 抛错;ContentModified 指数退避重试) |
| 198-202 | `send_notification`(静默) |
| 204-221 | `add_notification_handler`(client 为空静默) |
| 223-234 | `is_healthy`(RUNNING + is_alive) |
| 236-261 | `_handle_crash`(STOPPING 短路、ERROR、计数、on_error 回调) |

### 10.5 `core/manager.py`

| 行 | 功能 |
|---|---|
| 20-30 | `ServerInstanceKey`(frozen, server_id+root) |
| 38-46 | 单例 `_instance` / 惰性 `_lock` |
| 48-113 | `initialize`(双检、cwd 解析、build_configs、extension_map、分组、返回 InitializeResult) |
| 74-78 | cwd 解析(Path.resolve 失败回退 getcwd) |
| 86-93 | 扩展索引构建(小写归一、按序、去重) |
| 96-104 | server_id 分组与 manager 字段填充 |
| 115-121 | `shutdown`(stop_all、_instance=None、_lock=None) |
| 123-141 | `stop_all`(取消 spawn 任务 5s、并发 stop 实例) |
| 143-157 | `get_instance` / `get_status` 类方法 |
| 159-171 | `__init__` 全部字段 |
| 173-175 | `get_workspace_root` |
| 177-238 | `get_or_start_server`(扩展匹配→find_root→缓存判定→僵尸/ERROR/启动中分支→_start_server) |
| 192-193 | 扩展名小写匹配 |
| 200-207 | find_root 协程/同步判别;root None 跳过 |
| 214-231 | 缓存三分支(健康返回/僵尸重启/ERROR 恢复/启动中等待) |
| 234-236 | 启动并校验 running |
| 240-303 | `_start_server`(动态根复制 config、start_and_cache、wait_for 超时清理) |
| 255-265 | 动态 root 的 active_config 复制 |
| 274-281 | start_and_cache 包装(异常记日志、finally 清理 spawning) |
| 288-297 | 启动超时(TimeoutError→cancel+清理+None) |
| 298-301 | 启动异常(清理+None) |
| 305-314 | `_path_belongs_to_root`(⚠️ 无调用点) |
| 316-319 | `_log_server_error` |
| 321-358 | `_ensure_diagnostic_handler`(id() 幂等、publishDiagnostics→registry.register) |
| 360-380 | `get_pending_diagnostics` 类方法 |
| 382-384 | `is_file_open` |
| 386-421 | `open_file`(版本 0、didOpen、读盘失败空串) |
| 423-477 | `change_file`(版本自增、full sync didChange) |
| 465-466 | 版本号计算(未 open 从 1 起) |
| 479-489 | `send_request`(无 server 抛 RuntimeError) |
| 491-504 | `get_status` 快照 |

### 10.6 `core/utils/file_uri.py`

| 行 | 功能 |
|---|---|
| 10-22 | `path_to_file_uri`(Win 不编码 / Unix quote safe="/:") |
| 17-20 | Windows 分支(file:/// + as_posix) |
| 21-22 | Unix 分支(quote) |
| 25-57 | `file_uri_to_path`(非 file:// 原样、unquote、Windows 盘符处理) |
| 35-36 | 非 file:// 前缀原样返回 |
| 42-45 | unquote 容错(ValueError 保留原串) |
| 47-55 | Windows:去前导斜杠、\\ 归一、盘符大写 |

### 10.7 `core/utils/git_ignore.py`

| 行 | 功能 |
|---|---|
| 14-16 | `uri_to_file_path` 别名 |
| 19-95 | `filter_git_ignored_locations`(批量 50、5s 超时、git check-ignore --stdin、忽略失败批) |
| 39-52 | uri 提取(uri/targetUri/location.uri)与去重 |
| 61-83 | 分批执行 git 子进程(returncode==0 收集、异常忽略) |
| 88-95 | 过滤(仅看顶层 uri/targetUri) |

### 10.8 `core/utils/constants.py`

| 行 | 功能 |
|---|---|
| 4 | LSP_ERROR_CONTENT_MODIFIED = -32801 |
| 8 | MAX_RETRIES_FOR_CONTENT_MODIFIED = 3 |
| 11 | RETRY_BASE_DELAY_MS = 500 |
| 15 | DEFAULT_STARTUP_TIMEOUT_MS = 45_000(⚠️ 无读取点) |
| 18 | MAX_LSP_FILE_SIZE_BYTES = 10MB(⚠️ 仅导出) |
| 21 | DEFAULT_GOPLS_TIMEOUT_MS = 60_000(⚠️ 无读取点) |
| 25 | MAX_CRASH_RECOVERY_ATTEMPTS = 3 |
| 29 | DEFAULT_REQUEST_TIMEOUT_MS = 15_000 |

### 10.9 `types.py`

| 行 | 功能 |
|---|---|
| 12-17 | `InitializeOptions`(cwd/custom_servers) |
| 20-30 | `CustomServerConfig`(七个字段 + disabled) |
| 33-39 | `InitializeResult`(success/servers_loaded/duration_ms=0.0) |
| 42-47 | `LspStatus`(initialized/servers) |

### 10.10 `servers/registry.py`

| 行 | 功能 |
|---|---|
| 16 | `BUILTIN_SERVERS` 全局注册表 |
| 19-75 | `nearest_root`(start 目录豁免、include 先于 stop/exclude、to_thread) |
| 33-73 | 同步根查找闭包(起始目录检查、向上遍历、exclude 拦截) |
| 78-161 | `build_configs_async`(占位配置、custom 覆盖/新增/禁用) |
| 91-96 | spawn 协程/同步判别 |
| 98-114 | 二进制缺失 → 占位配置(command="") |
| 116-129 | 正常配置(含 extension_to_language) |
| 131-159 | custom_servers 三规则(disabled 删除 / 覆盖 command+args+init_opts / 新增) |
| 164-169 | `build_configs` 同步包装(asyncio.run) |

### 10.11 `servers/types.py`

| 行 | 功能 |
|---|---|
| 11-32 | `ServerDefinition`(priority=100 / global_server=False / find_root / spawn 占位) |

### 10.12 `servers/servers/go.py`

| 行 | 功能 |
|---|---|
| 12-17 | `go_root`(go.work → go.mod/go.sum) |
| 20-30 | `_spawn_go`(gopls、staticcheck=True、60_000 超时) |
| 33-40 | go_server 定义(priority=10) |
| 41 | 注册 |

### 10.13 `servers/servers/java.py`

| 行 | 功能 |
|---|---|
| 12-21 | `_spawn_java`(jdtls、args=[], init=None) |
| 24-39 | java_server 定义(find_root 四模式 + exclude .git) |
| 40-41 | 注册 |

### 10.14 `servers/servers/python.py`

| 行 | 功能 |
|---|---|
| 16-91 | `_resolve_pyright_command`(npm 全局前缀 → node+js;.CMD 解析;直接执行 fallback) |
| 29-55 | npm list 解析(pyright@ 上一行为 prefix) |
| 58-64 | which fallback 与非 .CMD 直接返回 |
| 66-91 | .CMD 内容解析(%dp0% 展开、node 探测) |
| 94-126 | `_spawn_python`(venv 探测顺序 VIRTUAL_ENV→.venv→venv;pythonPath) |
| 103-120 | venv 候选与存在性检查 |
| 129-146 | python_server 定义(六模式 + exclude .git) |
| 147-148 | 注册 |

### 10.15 `servers/servers/rust.py`

| 行 | 功能 |
|---|---|
| 17-63 | `rust_root`(Cargo.toml 最近查找 + [workspace] 上溯;⚠️ symlink 怪癖) |
| 25-61 | 同步查找实现 |
| 66-74 | `_spawn_rust`(rust-analyzer) |
| 77-84 | rust_server 定义 |
| 85-86 | 注册 |

### 10.16 `servers/servers/typescript.py`

| 行 | 功能 |
|---|---|
| 13-30 | `_spawn_ts`(--stdio;无 tsconfig/jsconfig 加 --ignore-node-modules) |
| 33-48 | typescript_server 定义(六扩展名;exclude deno.json/jsonc) |
| 49-50 | 注册 |

### 10.17 `__init__.py`(包级)

| 行 | 功能 |
|---|---|
| 7 | `__version__ = "0.1.10"` |
| 9-20 | 公共符号导入 |
| 23-41 | `get_pending_lsp_diagnostics` |
| 44-53 | `initialize_lsp` |
| 56-62 | `shutdown_lsp` |
| 65-74 | `get_lsp_tool`(外部依赖) |
| 77-83 | `get_lsp_status` |
| 86-105 | `__all__` |

---

## 11. 移植核对清单(关键等价点)

1. **URI 编码/解码**:Windows 不编码 vs Unix `safe="/:"`;`file_uri_to_path` 先 unquote 再处理盘符;非 `file://` 前缀原样返回。
2. **帧解析容错**:非法 Content-Length、坏 JSON、空 body 一律**静默跳过**;头行 ascii `errors="replace"`。
3. **EOF 语义**:读循环 EOF 正常返回、**不**触发崩溃上报;RUNNING→ERROR 仅由读循环异常或显式回调触发;僵尸由 manager `is_healthy` 兜底。
4. **一次性守卫**:client `_crash_reported`;instance `_handle_crash` 的 STOPPING 短路;manager `id(server)` 诊断 handler 幂等。
5. **去重键**:`message|severity|start.line:start.character|code`(无 end/source);`_delivered` 只增。
6. **排序稳定性**:severity 升序稳定排序;同 severity 保持插入序。
7. **全局上限截断顺序**:按 uri 首次出现顺序;达到 max_total 即 break(后续 uri 被完全丢弃)。
8. **版本号**:open→0,change→+1,未 open 首改→1;didChange 全量替换无 range。
9. **env 合并**:config.env 非空 → `{os.environ, ...config.env}`;为空 → 继承(传 None)。
10. **超时单位**:请求 15s(client);shutdown 2s;进程退出等待 5s 后 kill;启动等待 `startup_timeout/1000` 秒(manager 层);git 批 5s。
11. **错误消息文本**:所有 RuntimeError/LSPError/ConnectionError 消息文本需逐字等价(含 `state={state.value}` 的字符串值)。
12. **注册顺序**:rust→typescript→java→python→go 决定 configs/扩展索引顺序(当前无扩展冲突)。
13. **保留但未使用的字段**:`priority`、`global_server`、`_is_stopping`、`_path_belongs_to_root`、`DEFAULT_STARTUP_TIMEOUT_MS`/`DEFAULT_GOPLS_TIMEOUT_MS`/`MAX_LSP_FILE_SIZE_BYTES` 常量——按 1:1 保留或显式评审后省略。
14. **rust_root symlink 怪癖**(10.15/7.5.4)与 git_ignore 过滤不对称(8.2)为现状行为,照抄并在评审中标注。
