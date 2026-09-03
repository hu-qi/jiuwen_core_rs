# Python/Rust Differential(辅助行为审计)

Rust-only contract fixture 是主回归门禁;本目录的 Python/Rust differential 只在两端处于同一
抽象层且输入可确定时提供辅助行为审计。它不进入 Rust 构建、运行或生产 Profile,也不能替代
`ah-app/tests/rust_contract.rs` 的 Rust contract 验证。

## 布局

```text
fixtures/                      语言中立 fixture(仓库根)
  session.json                 Rust contract:append/derive/JSONL resume
  tools.json                   Rust contract:真实工具调用与错误分类
  controller.json              Rust contract:状态轨迹与确定性意图
  stop_condition.json          可与 Python 同层比较的 stop_condition
  messager_inprocess.json      可与 Python 同层比较的 inprocess 总线
references/                    Rust baseline(仅由 Rust 生成)
differential/                  可选 Python 辅助审计脚本
  run_python.py                可选 Python reference runner(驱动真实 openjiuwen)
  compare.py                   可选 Python outcome 与 Rust reference 对比
  run.sh                       可选辅助审计流程
  out/python/{seam}.json       Python 独立输出(不提交,运行生成)
```

## 运行

```sh
# Rust-only 主门禁(不需要 Python)
cargo test -p ah-app --test rust_contract

# 可选 Python 辅助审计(需要 Python 3.11+、pydantic 和 agent-core)
AGENT_CORE_ROOT=/path/to/agent-core \
PYTHON=/path/to/python3.11 \
bash differential/run.sh
```

Rust reference 回归仍可单独运行:

```sh
cargo test -p ah-app --test differential
AH_REFGEN=1 cargo test -p ah-app --test differential
```

## 纪律(与 docs/testing.md 一致)

1. Rust contract fixture 是 Rust-only 主门禁,不导入 Python。
2. `references/` 只能由 Rust 生成(`AH_REFGEN=1`);`run_python.py` 绝不写
   `references/`,只写 `differential/out/python/`。
3. Rust reference 是 regression baseline,不是独立 Python 参考结果。
4. `known_divergence` 只记录可同层比较后的真实语义差异;不可同层比较的行为标记为
   `not-comparable`,不能通过削弱 contract 或 fixture 规避。
5. 比较时记录 agent-harness HEAD 与 agent-core HEAD,保证辅助审计可复现。

## 当前已覆盖 seam 与已知差异

| seam | case | 状态 |
| --- | --- | --- |
| stop_condition | max_rounds / token_budget / timeout / completion_promise 状态机 | 4/4 一致 |
| messager_inprocess | 单实例 pub-sub 盖章与退订 / p2p 注册发送退订 | 2/2 一致 |
| messager_inprocess | cross_instance_shared_bus | **known_divergence** |

### 已发现并记录的语义差异

- **总线作用域**:Python `_Bus` 是**进程全局**单例(所有 `InProcessMessager` 共享),
  Rust `InProcessMessager` 持有**每实例** `InProcessBus`。跨实例 fan-out(teammate 发布、
  leader 订阅)Python 可送达、Rust 收不到。Rust 需改为共享/全局总线才能对等。
- **sender_id 盖章条件**:Python 仅当消息对象**有 `sender_id` 字段且为空**时盖章
  (`hasattr` 判定);Rust 在字段**缺失或为空**时都盖章。fixture 当前用带 `sender_id`
  字段的消息规避该差异;字段缺失场景待 Rust 对齐后纳入。
- **多订阅者投递顺序**:Rust `InProcessBus` 用 `HashMap`,多订阅者时投递顺序非确定;
  Python 按订阅插入序。fixture 当前保持单订阅者。

## 新增一个 seam

1. 在 `fixtures/` 加 `{seam}.json`(语言中立:只含输入与期望形状,不含实现);
2. `differential/run_python.py` 增加 `run_{seam}()` 驱动真实 openjiuwen 模块
   (隔离加载见文件头注释),输出与 Rust 侧**同一形状**;
3. `crates/ah-app/tests/differential.rs` 增加 `reference_{seam}` 测试
   (复用 `settle()` 与 `load_fixture()`);
4. `AH_REFGEN=1 cargo test -p ah-app --test differential` 生成 Rust reference;
5. `differential/compare.py` 的 `SEAMS` 加入 `{seam}`;
6. 端到端跑 `bash differential/run.sh`,把两端一致或标记 known_divergence 后提交。

## CI

`.github/workflows/ci.yml` 的 `differential` job:checkout agent-harness + agent-core
(固定 ref,见 job 配置)→ setup-python 3.11 + rust → `bash differential/run.sh`。
agent-core 引用 `aeb88cd8`(与 parity-audit 基线一致);升级基线时同步更新 job 的 `ref`。
