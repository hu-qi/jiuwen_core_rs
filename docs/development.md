# 开发流程

> 所有贡献者必读。架构约束见 `architecture.md`,测试口径见 `testing.md`,当前任务见
> `ROADMAP.md`。

## 前置条件

- Rust stable toolchain;
- git 2.26+;
- 可选的外部服务或凭据,仅对应 production E2E 需要;
- production profile 当前还依赖本地 Redis、OpenAI 凭据及部分外部命令,无这些依赖时 `boot()` 必须显式失败;当前实测缺少 OpenAI key 时由 `ah-plugins-openai` 返回明确错误。

### 本地凭据文件

`ah-app::boot` 会在启动前读取凭据文件:优先使用 `AH_ENV_FILE` 指定的路径;
未设置时依次尝试当前工作目录 `.env` 和 profile 所在项目根目录的 `.env`。
自动发现的 `.env` 只补充尚未存在于进程环境的变量,因此 shell/CI 环境变量优先;
显式设置 `AH_ENV_FILE` 时,该文件是权威配置,会覆盖同名进程变量,避免旧的 `OPENAI_*`
变量静默选择错误的 provider 或凭据。支持 `KEY=VALUE`、`export KEY=VALUE` 以及单/双引号值;
解析失败会显式阻止启动。
`.env` 与 `.env.*` 已加入 `.gitignore`,禁止提交真实密钥。

OpenAI 最小配置:

```sh
OPENAI_API_KEY=...
```

可选配置:

```sh
OPENAI_BASE_URL=https://api.openai.com/v1
OPENAI_MODEL=gpt-4o-mini
```

生产 Profile 同时挂载 Redis store、Redis queue 和 checkpointer;三者使用同一个 Redis URL。
默认地址为本机 `redis://127.0.0.1:6379/`;远程、认证或 TLS Redis 请在 env 文件中配置:

```sh
REDIS_URL=redis://:password@redis.example.com:6379/0
# TLS 使用 rediss://...
```

也可使用外部文件:

```sh
AH_ENV_FILE=/secure/path/agent-harness.env cargo run --offline -p ah-app --bin ah-app -- profiles/prod.toml
```

```sh
cargo build --workspace
cargo test --workspace
cargo run --offline -p ah-app --bin ah-app -- profiles/dev.toml
```

`dev.toml` 使用 mock LLM 进行本地结构冒烟。当前完整 `ah-app` demo 在清空云凭据后已成功 boot 并执行 application、agent-loop、workflow、tools、memory、retrieval、telemetry 和 symphony 路径;它不证明 production profile 或 Python parity。

## 工作包生命周期

| 阶段 | 产出 | 验收 |
| --- | --- | --- |
| 1. 契约 | `ah-contracts` 中的 seam trait、事件和纯类型 | 契约零实现;类型和序列化单测 |
| 2. 测试 provider | `ah-plugins-mock` 或测试模块中的确定性替身 | dev/test 可组装;不得进入 prod |
| 3. 生产实现 | `ah-plugins-*` 中的真实协议、持久化或子进程路径 | 聚焦测试和 production E2E |
| 4. Rust 回归 | Golden fixture、Rust-only contract 和 Rust regression reference | Rust 行为稳定 |

所有产品实现、默认测试和 CI 均为 Rust-only。历史 Python 源码只用于人工理解行为规格,
不属于构建、运行时或验收依赖。

## 依赖纪律

- `ah-contracts` 不依赖任何本仓库实现 crate;
- `ah-hub` 只依赖 `ah-contracts`;
- 除组装层 `ah-app` 外,插件生产 `[dependencies]` 不得依赖其他 `ah-plugins-*`;
- 测试需要的具体插件放入 `[dev-dependencies]`;
- consumer 通过 `ctx.service::<dyn Trait>(&KEY)` 获取服务;
- 所有注册返回 `Effect`,后台任务、子进程和 socket 也必须有可逆生命周期。

目前该依赖纪律主要靠评审,自动 CI 检查仍是 `ROADMAP.md` 的 P1-08。

## 本地验证

先运行聚焦检查,再根据改动范围运行 workspace 门禁:

```sh
cargo fmt --all --check
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate>
```

提交前或共享契约变更后运行:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo llvm-cov --workspace --fail-under-lines 80
```

外部协议能力还需相应本地 fixture、服务容器或凭据 E2E。命令未完成或超时时必须如实记录,
不能沿用历史回合的通过数字。

## 当前 CI 门禁

`.github/workflows/ci.yml` 当前实际执行:

1. `cargo fmt --all --check`;
2. `cargo clippy --workspace --all-targets -- -D warnings`;
3. `cargo test -p ah-app --test mock_gate`;
4. `cargo test --workspace`;
5. `cargo llvm-cov --workspace --fail-under-lines 80`。


当前 CI 尚未执行:

- `cargo test --workspace --all-features`;
- production profile static composition;
- production `boot()` smoke 的统一门禁;
- 插件生产依赖隔离扫描;
- 文档路径、数字和状态自动一致性检查;
- Linux/Windows/macOS 跨平台矩阵。

这些项目未落地前不得写成现有门禁。

## Definition of Done

一个能力可以标记为 implementation done,必须满足:

1. 契约层只有 trait、事件和纯类型;
2. 生产插件存在可调用实现,不以 mock、静默 fallback 或未注入替身冒充;
3. 消费方通过 seam 调用,不依赖具体 provider;
4. 成功、失败、取消、超时、恢复和序列化中适用项有测试;
5. production E2E 已通过,或状态明确标记 production unverified;
6. fmt、clippy、测试和覆盖率门禁通过;
7. capability-map 记录实现位置、测试名和 commit。

Rust regression reference 不能替代 production E2E,但不需要 Python runner 才能作为 Rust
implementation 的回归门禁。无法由当前 Rust 环境验证的历史行为必须单独记录,不能标记为
已对等。

## 提交规范

```text
<type>(<scope>): <subject>

<body: WHAT + WHY + verification>
```

- type 使用 feat/fix/refactor/docs/test/chore/perf/style;
- scope 使用 crate 或能力域;
- 一次提交一个逻辑变更;
- 提交说明必须与实际 diff 一致;
- 状态变更同步更新结构化账本、审计快照和必要技术目录。

## 变更纪律

- 新行为优先挂在 seam 或事件扩展点;
- 修改 hub/agent-loop 核心语义时同步更新架构、事件和生命周期文档;
- 真实路径失败必须显式返回错误;
- 文档中的当前事实必须来自当前 HEAD 实测;
- 历史文档只作记录,不作为当前验收依据。
