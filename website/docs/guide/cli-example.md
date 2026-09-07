# CLI 开发工具示例

`ah-code` 是 `agent-harness` 内的可运行开发工具示例，交互方式接近 Claude Code：用户可以输入任务让 Agent Loop 工作，也可以显式读取、写入、编辑文件或运行受限命令。

## 运行

在 `agent-harness` 目录执行：

```sh
cargo run -p ah-code-cli -- --workspace .
```

不进入交互模式，运行一次离线任务：

```sh
cargo run -p ah-code-cli -- --workspace . --once "inspect this workspace"
```

默认使用 Mock Provider。Mock 模型会请求真实 `list_dir` 工具，因此不需要网络和模型凭据即可验证 Plugin → Context → ToolRegistry → Agent Loop → CLI Renderer 链路。

真实 Provider：

```sh
OPENAI_API_KEY=... OPENAI_MODEL=gpt-4o-mini \
cargo run -p ah-code-cli -- --model openai --workspace .
```

`--model openai` 缺少凭据时显式失败，不会自动回退到 Mock。

## 命令

| 命令 | 作用 |
| --- | --- |
| `<task>` | 运行 Agent Loop |
| `/read <path>` | 读取工作区文件 |
| `/write <path> <content>` | 写入工作区文件 |
| `/edit <path> <old> => <new>` | 替换文件内容 |
| `/run <command> [args...]` | 执行受限 Shell 命令 |
| `/tools` | 列出已挂载工具 |
| `/new <id>`、`/use <id>` | 创建或切换 Session |
| `/sessions` | 列出 Session |
| `/history` | 查看 JSONL 会话事件 |
| `/help`、`/quit` | 查看帮助或退出 |

文件访问受 `ah-plugins-sysop` 的 Workspace 根目录约束；Shell 命令通过 `run_shell` 工具执行，默认超时 30 秒。

## 运行链

```text
Mock/OpenAI Provider + Tools + Sysop + Session Log
  → AgentControl + ModelBackup + CLI Renderer
  → AgentLoop
  → REPL / explicit tool commands
```

项目源码位于 `example/ah-code-cli/`；示例专属终端插件位于 `plugins/ah-plugins-cli/`。该插件不属于主项目 `crates/`、`ah-app::plugin_catalog` 或 dev/prod Profile。完整命令解析、插件装配和状态行为见项目 README。
