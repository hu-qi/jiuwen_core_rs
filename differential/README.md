# Python/Rust Differential(P0-01)

同一份**语言中立 fixture**分别驱动 Python agent-core(openjiuwen)与 Rust
agent-harness,比较规范化输出。这是唯一能把"Rust 已实现"升级为"与 Python 行为对等"的
证据来源(见 docs/testing.md §Python/Rust differential 与 docs/ROADMAP.md P0-01)。

## 布局

```text
fixtures/                      语言中立 fixture(仓库根,与 golden 共用)
  stop_condition.json          harness/schema stop_condition 求值器
  messager_inprocess.json      agent_teams/messager inprocess 总线
references/                    Rust baseline(仅由 Rust 生成,已提交)
  stop_condition.json
  messager_inprocess.json
differential/
  run_python.py                Python reference runner(驱动真实 openjiuwen)
  compare.py                   对比 Python outcome 与 Rust reference
  run.sh                       全流程:rust 检查 → python runner → compare
  out/python/{seam}.json       Python 独立输出(不提交,运行生成)
```

## 运行

```sh
# 本地(需 Python 3.11+ 且已装 pydantic;agent-core 已 checkout)
AGENT_CORE_ROOT=/path/to/agent-core \
PYTHON=/path/to/python3.11 \
bash differential/run.sh

# 只跑单侧
AGENT_CORE_ROOT=... python3 differential/run_python.py          # Python 侧
cargo test -p ah-app --test differential                          # Rust 侧(check 模式)
AH_REFGEN=1 cargo test -p ah-app --test differential              # 重新生成 Rust references
python3 differential/compare.py                                   # 只看对比
```

## 纪律(与 docs/testing.md 一致)

1. **`references/` 只能由 Rust 生成**(`AH_REFGEN=1`)。`run_python.py` 绝不写
   `references/`,只写 `differential/out/python/`。Python 永远不覆盖 Rust baseline。
2. Rust 侧在 check 模式下断言当前 Rust 行为与已提交 reference 一致(回归保护);
   reference 变更必须解释行为变化,禁止仅为过测试而重生成。
3. `known_divergence` case 记录**真实语义差异**:compare 报告但视为已知、不失败;
   未标记的差异视为回归,CI 失败。
4. 比较时同时记录 agent-harness HEAD 与 agent-core HEAD,保证对比可复现。

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
