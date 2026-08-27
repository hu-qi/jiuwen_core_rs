# agent-harness 文档集

agent-harness 的目标是以 Rust 独立实现 agent-core(Python)的公开行为,并将能力组织为
Seam 契约、普通插件、类型化事件和 Profile 组合。Python 源码是规格参考,不是生产运行时依赖。

项目不提供 `openjiuwen.*` Python import 路径或 Python 对象模型兼容。"Rust 中已有实现"、
"生产组合可运行"和"与 Python 行为对等"是三个不同结论,不得混用。

## 当前基线

- agent-harness 审查基线:`cc561c0`;
- agent-core Python 参考基线:`aeb88cd8`;
- 当前 workspace 约 111 个 crate,其中约 108 个 `ah-plugins-*` crate;
- `profiles/dev.toml` 当前声明 113 个插件(展开去重后),包含 `ah-plugins-mock`;
- `profiles/prod.toml` 当前声明 112 个插件,不包含 `ah-plugins-mock`;
- `ah-hub` 聚焦测试 18 项通过,`ah-plugins-application` 聚焦测试 14 项通过;
- 本次审查未在执行时限内完成全 workspace、production boot 和覆盖率实测,不得引用历史回合数字作为当前结果。

以上是代码结构与本次实测快照,不是 Python 行为对等证明。严格状态见
[parity-audit.md](parity-audit.md),当前任务顺序见 [ROADMAP.md](ROADMAP.md)。

## 状态口径

| 维度 | 含义 | 当前结论 |
| --- | --- | --- |
| 插件架构 | hub、Seam、Effect、事件、Profile 和依赖装配 | 已形成,仍需补 production composition/boot 与失败原子性测试 |
| Rust 功能覆盖 | Python 能力是否在 Rust 中有可调用实现 | 广泛覆盖,多数子模块仍为 partial |
| 严格行为对等 | 同一 fixture 驱动 Python 与 Rust 后行为一致 | 尚未建立真实 Python/Rust 差分门禁 |
| Python 迁移/切流 | Python agent-core 是否调用或切换到 Rust runtime | 非项目目标,当前没有切流 |

`done` 只表示对应审计口径下满足完成定义。Golden fixture 或 Rust 自生成 reference 不能单独证明
Python 对等。

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
