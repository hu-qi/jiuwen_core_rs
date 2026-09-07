# ah-code

`ah-code` 是一个可以直接运行的 Claude Code 风格开发工具示例。它使用 `agent-harness` 的真实插件链：ToolRegistry、受限文件系统、受限 Shell、Session JSONL、Agent Loop、CLI Renderer 和 Mock/OpenAI Provider。

## 运行

在 `agent-harness` 目录执行：

```sh
cargo run -p ah-code-cli -- --workspace .
```

无交互冒烟：

```sh
cargo run -p ah-code-cli -- --workspace . --once "inspect this workspace"
```

默认 `mock` 模式不需要模型凭据。Mock 模型会请求真实的 `list_dir` 工具并输出工具结果，证明 Agent Loop 和工具注册链路可以工作。

使用真实 OpenAI-compatible Provider：

```sh
OPENAI_API_KEY=... \
OPENAI_MODEL=gpt-4o-mini \
OPENAI_BASE_URL=https://api.openai.com/v1 \
cargo run -p ah-code-cli -- --model openai --workspace .
```

如果没有有效的 Provider 凭据，`--model openai` 会显式失败，不会回退到 Mock。

## 内置命令

```text
<task>                         运行 Agent Loop
/read <path>                  读取工作区内文件
/write <path> <content>       写入工作区内文件
/edit <path> <old> => <new>   替换工作区文件内容
/run <command> [args...]       在受限工作区执行命令
/tools                        列出已挂载工具
/new <id> | /use <id>         创建或切换 Session
/sessions                     列出 Session
/history                      查看 append-only 会话事件
/clear                        清理终端
/help | /quit                 帮助或退出
```

文件 API 由 `ah-plugins-sysop` 约束在 Workspace 根目录内。`/run` 是显式用户命令，仍然通过 `run_shell` 工具执行，并默认设置 30 秒超时。

## 状态文件

默认写入当前工作区：

```text
.agent-harness-cli/
├── default.jsonl
└── sessions/
```

可以用 `--state-dir` 指定其他位置。Session 日志是 append-only JSONL，便于 `/history` 查看和 Agent Loop 恢复。

## 项目结构

```text
src/lib.rs                               命令解析和参数模型
src/main.rs                              插件装配、REPL、工具命令和 Agent Loop 调用
plugins/ah-plugins-cli/                  示例专属终端渲染与权限确认插件
```

`ah-plugins-cli` 只属于本示例。它不在主项目 `crates/`、`ah-app::plugin_catalog` 或 dev/prod Profile 中注册。

这个示例是开发工具的最小可运行组合，不替代生产 CLI 的鉴权、审批 UI、并发作业管理和完整模型配置系统。
