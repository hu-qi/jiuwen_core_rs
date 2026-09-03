# agent-harness 文档集

agent-harness 的目标是以 Rust 独立实现 agent-core(Python)的公开行为,并将能力组织为
Seam 契约、普通插件、类型化事件和 Profile 组合。Python 源码仅作为历史规格参考,不是
构建、测试或生产运行时依赖。

项目不提供 `openjiuwen.*` Python import 路径或 Python 对象模型兼容。产品实现、默认测试
和 CI 均为 Rust-only。

## 当前基线

- agent-harness 当前实现代码 revision:`38ed1a5`;后续审计与文档提交不改变该实现基线。代码审查基线和历史审计快照仍可能引用更早 commit,不能视为当前状态。
- `cargo metadata --no-deps` 当前发现 **112 个 workspace package**;仓库中有 112 个 Cargo manifest。阶段一已将 10 个新增插件加入 workspace members,阶段二已接入 `ah-app::plugin_catalog` 与 dev/prod Profile。
- 阶段二接入插件:`ah-plugins-agentbuilder`、`ah-plugins-a2a`、`ah-plugins-data-loader`、`ah-plugins-dataset-curator`、`ah-plugins-model-allocator`、`ah-plugins-prompt-attachment`、`ah-plugins-interaction-router`、`ah-plugins-inbound-render`、`ah-plugins-external-format`、`ah-plugins-bridge-compose`。
- 当前 Rust-only contract runner 已接入 `session`、`tools`、`controller`、`agent-loop`、`workflow`、`application`,不依赖 Python。默认 CI 只执行 Rust build/test/lint/coverage 和 production composition；Python differential 不属于项目验收门禁。
- Python differential 脚本仅保留在 `differential/` 作为历史审计资产,不在 CI 或 Rust-only 验收中执行。历史结果不作为当前实现结论。

本页只记录当前可复核的结构事实;工作包状态、域汇总和状态百分比由
[audit/ledger.json](../audit/ledger.json) 生成,见 [生成审计摘要](generated/audit-summary.md)。
严格能力验收标准见 [parity-audit.md](parity-audit.md),执行顺序见 [ROADMAP.md](ROADMAP.md)。

## 状态口径

| 维度 | 含义 | 当前结论 |
| 插件架构 | hub、Seam、Effect、事件、Profile 和依赖装配 | 已形成;阶段二 catalog/Profile、targeted mount/invoke/unmount 与 `mount_all` 失败回滚测试已通过,仍需补 production boot 与完整 Rust contract 覆盖 |
| Rust 功能覆盖 | Python 能力是否在 Rust 中有可调用实现 | 广泛覆盖,多数子模块仍为 partial |
| 行为规格一致性 | Rust contract fixture 与历史 agent-core 行为规格一致 | 以 Rust-only fixture、regression reference、生产 smoke 为验收依据;不可由 Rust 直接验证的历史差异单独记录 |
| Python 迁移/切流 | Python agent-core 是否调用或切换到 Rust runtime | 非项目目标,当前没有切流 |

`done` 只表示对应 Rust implementation 和 production 证据满足审计口径；Rust 自生成
reference 只能证明 Rust 行为稳定，不能证明外部 Python 对等。

## 阅读顺序

### 必读

| 文档 | 角色 |
| --- | --- |
| [architecture.md](architecture.md) | 稳定架构边界、依赖方向和硬性规则 |
| [hub-primer.md](hub-primer.md) | Context、Registry、EventBus、Effect、Plugin、Profile 语义 |
| [ROADMAP.md](ROADMAP.md) | 当前 P0-P3 执行顺序、依赖和验收标准 |
| [capability-map.md](capability-map.md) | Python 能力到 Rust seam/plugin 的映射 |
| [parity-audit.md](parity-audit.md) | 指定 commit 的严格行为对等审计快照 |
| [agent-guide.md](agent-guide.md) | 面向编码代理的操作约束和验证命令 |

### 开发与验证

| 文档 | 角色 |
| --- | --- |
| [development.md](development.md) | 实际开发流程、现有 CI、Definition of Done |
| [testing.md](testing.md) | 单元、Golden、Rust regression、Python/Rust differential、E2E |
| [gap-audit.md](gap-audit.md) | 当前未完成能力和工程缺口 |
| [usage.md](usage.md) | 插件作者和集成方使用方式 |

### 技术目录

| 文档 | 角色 |
| --- | --- |
| [event-catalog.md](event-catalog.md) | 事件模式、生产者、消费者和持久化关系 |
| [config-catalog.md](config-catalog.md) | Profile 与宿主解析的配置字段 |
| [persistence-catalog.md](persistence-catalog.md) | 持久化格式、版本与迁移纪律 |
| [module-graph.md](module-graph.md) | crate 分层和依赖规则 |
| [glossary.md](glossary.md) | 统一术语 |

## 历史文档

- [REMAINING_PLAN.md](REMAINING_PLAN.md) 是逐回合历史账目,不再作为当前计划;
- [TIER2_ROADMAP.md](TIER2_ROADMAP.md) 是第二梯队历史验收快照;
- [progress-dashboard.md](progress-dashboard.md) 已由 parity audit 取代。

历史文档中的 crate 数、插件数、测试数和完成声明只代表对应时点。

## 维护纪律

1. 当前任务只写入 `ROADMAP.md`,逐回合历史不再追加到当前路线图正文;
2. 状态变化必须附代码位置、测试名和审计基线 commit;
3. 文档必须区分 implementation、production verification 与 Python parity;
4. 插件/profile/service 数量不得手工长期复制,应以代码实测或生成结果为准;
5. 文档与代码冲突时先以代码为准,随后在同一变更中修正文档;
6. 未在当前 HEAD 实测的测试数、覆盖率、clippy 和 fmt 状态不得写成当前事实。
