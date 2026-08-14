# 生成参考(agent-guide.md)

> 面向 AI 代理与代码生成工具:在本仓库生成任何代码前必读。
> 先读 docs/architecture.md 与 docs/capability-map.md;本文件是操作层指南。

## 1. 仓库地图

```text
agent-harness/
  crates/
    ah-contracts/     契约层: Seam trait + Event trait + 纯类型,零实现
    ah-hub/           插件内核: ServiceRegistry + EventBus + Plugin + Profile
    ah-plugins-mock/  Mock 插件集(仅 dev/test profile)
    ah-app/           boot 入口(profile → 插件目录 → mount_all → seam 解析)
  profiles/           dev.toml 等组合配置
  docs/               architecture / capability-map / development / usage / 本文件
  .github/workflows/  CI(fmt / clippy / test)
```

## 2. 硬性规则(违反即返工)

1. **契约零实现**:ah-contracts 里绝不写业务逻辑、Mock、Unsupported、fallback;
2. **插件隔离**:插件 crate 只依赖 ah-hub + ah-contracts,禁止 import 其他插件具体类型;
3. **注册必可逆**:任何注册返回 Effect,禁止无 guard 注册;测试里用具名绑定持有 guard
   (let _x = ...,不要 let _ = ...,后者立即 drop 反注册);
4. **mock 显式化**:mock/fallback 只出现在 ah-plugins-mock 或标注 test 的插件;
5. **生产路径真实**:禁止 todo!/unimplemented!;不支持必须显式返回错误并登记 missing;
6. **waterfall 语义**:监听器不调用 next 即短路;委托必须调用 next;
7. **行为对等才算完成**:本地/mock 测试通过 ≠ done;done 需要真实路径 + 差分契约证据;
8. **不静默降级**:真实路径失败显式报错,禁止切到本地 mock 继续。

## 3. 常见任务配方

### 3.1 新增一个插件(如 ah-plugins-openai)

1. 建 crate,依赖 ah-hub + ah-contracts;
2. 实现契约(如 ModelProvider)与 Plugin trait(provides/inject/apply);
3. apply 内 ctx.register(ServiceKey, Arc<dyn Trait>),返回 Effect;
4. 在 ah-app 插件目录注册名称 → 插件;
5. 加 profile 条目;加单元测试(挂载 + 解析 + 调用);
6. 验证:cargo test -p <crate>,cargo run -p ah-app。

### 3.2 新增一个 seam(如 tools)

1. ah-contracts 定义 trait(继承 Seam)+ 纯类型 + 稳定 ServiceKey;
2. 契约单测(类型层面);
3. mock 实现入 ah-plugins-mock(系统可 boot);
4. 真实实现入 ah-plugins-tools;差分契约验证与 agent-core 行为对等。

### 3.3 新增一个事件(如 session/step)

1. 定义事件类型并实现 Event trait(要求 Clone,稳定 ID);
2. 按语义选分发模式(观察用 emit,顺序副作用用 serial,扇出用 parallel,
   决策链用 waterfall);
3. 注册监听器返回 Effect;文档标注模式与语义。

### 3.4 迁移 agent-core 的一个 Python 模块(如 memory)

1. 在 capability-map.md 找到对应行(memory → memory seam + ah-plugins-memory-*);
2. 先定契约(参考 Python 类的公开行为面,不 import Python);
3. mock 可 boot → 真实实现(优先复用 agent-core_rs 现成资产,见 architecture.md §5);
4. 差分契约覆盖:成功/非法输入/超时/取消/恢复/持久化;
5. 状态从 partial 推到 done,附实现路径与测试证据。

## 4. 验证命令(提交前)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p ah-app        # boot 冒烟
```

只运行与改动相关的检查,不要默认跑全套;CI 拥有穷举覆盖。

## 5. 证据标准

- 声称 done:必须给出生产路径实现位置(文件:行)+ 测试证据(测试名/契约套件);
- 声称 unsupported:必须给出显式返回错误的代码位置;
- 状态变更写入 capability-map.md,禁止只改文档不改代码,或反之。

## 6. 变更纪律速记

- 新行为挂扩展点,不改内核/agent 循环(改了要更新 architecture.md);
- 一次提交一个逻辑变更(Conventional Commits,见 development.md);
- 读代码用 read/grep,不靠文档猜;文档与代码冲突时以代码为准并修正文档。

