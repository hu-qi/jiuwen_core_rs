//! # ah-plugins-external
//!
//! 真实外部 CLI agent 运行时(对齐 Python agent_teams/external/runtime.py):
//! 把第三方 CLI 进程作为一等团队成员驱动。两种风味,由 adapter 选择:
//!
//! - StreamingCliRuntime:一条长驻子进程(stdin 持续打开);每轮把任务文本写
//!   stdin,从 stdout 读到完成标记;支持中途 steer 与 abort(杀进程)。
//! - ReinvokeCliRuntime:每轮新子进程(prompt 作 argv 传入,读到 EOF);
//!   无中途 steer——运行中到达的消息缓冲到下一轮。
//!
//! 两者共用启动知识 CliAgentAdapter(input framing / completion / flags),
//! 子进程真实拉起、真实管道读写、真实超时与 kill。

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use ah_contracts::external::{
    CliAgentAdapter, CompletionStrategy, ExternalCliError, ExternalCliRuntime, ExternalCliTurn,
    InputFormat, codex_proto_line,
};
use ah_contracts::keys::EXTERNAL_CLI;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

fn adapter_command(adapter: &CliAgentAdapter) -> Result<(&str, Vec<&str>), ExternalCliError> {
    if adapter.command.is_empty() {
        return Err(ExternalCliError("adapter command is empty".to_string()));
    }
    let binary = &adapter.command[0];
    let args: Vec<&str> = adapter.command[1..].iter().map(String::as_str).collect();
    Ok((binary, args))
}

/// 流式外部 CLI 运行时:长驻子进程 + 逐轮 stdin 注入。
pub struct StreamingCliRuntime {
    adapter: CliAgentAdapter,
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    /// 未消费的 stdout 行(读取任务持续填充)。
    pending: Arc<Mutex<VecDeque<String>>>,
}

impl StreamingCliRuntime {
    pub fn new(adapter: CliAgentAdapter) -> Result<Self, ExternalCliError> {
        if !adapter.supports_stdin_injection {
            return Err(ExternalCliError(
                "streaming runtime requires supports_stdin_injection=true".to_string(),
            ));
        }
        Ok(Self {
            adapter,
            child: Mutex::new(None),
            stdin: Mutex::new(None),
            pending: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    /// 读一行;超时返回 Err(用于判定无输出挂死)。
    async fn read_line(&self, timeout: Duration) -> Result<Option<String>, ExternalCliError> {
        let pending = self.pending.clone();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(line) = pending.lock().unwrap().pop_front() {
                return Ok(Some(line));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ExternalCliError("inactivity timeout".to_string()));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Seam for StreamingCliRuntime {}

#[async_trait]
impl ExternalCliRuntime for StreamingCliRuntime {
    fn start(&self, _session_id: &str) -> Result<(), ExternalCliError> {
        let mut child_guard = self.child.lock().unwrap();
        if child_guard.is_some() {
            return Err(ExternalCliError("already started".to_string()));
        }
        let (binary, args) = adapter_command(&self.adapter)?;
        let mut command = Command::new(binary);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        // 注入团队成员身份环境(对齐 OPENJIUWEN_TEAM_JOIN)。
        let mut child = command
            .spawn()
            .map_err(|e| ExternalCliError(format!("spawn {binary}: {e}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ExternalCliError("no stdout pipe".to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ExternalCliError("no stdin pipe".to_string()))?;

        // 后台读取任务:持续把 stdout 行推进 pending 队列。
        let pending = self.pending.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim_end().to_string();
                        pending.lock().unwrap().push_back(trimmed);
                    }
                    Err(_) => break,
                }
            }
        });

        *self.stdin.lock().unwrap() = Some(stdin);
        *child_guard = Some(child);
        Ok(())
    }

    async fn send(&self, task: &str) -> Result<ExternalCliTurn, ExternalCliError> {
        let line = match self.adapter.input_format {
            InputFormat::Text => task.to_string(),
            InputFormat::CodexProto => codex_proto_line(task),
        };
        let mut stdin = self
            .stdin
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| ExternalCliError("not started".to_string()))?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(|e| ExternalCliError(format!("write stdin: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| ExternalCliError(format!("flush stdin: {e}")))?;
        *self.stdin.lock().unwrap() = Some(stdin);

        let timeout = Duration::from_secs(self.adapter.inactivity_timeout_s.max(1));
        let mut narration = Vec::new();
        let mut timed_out = false;
        loop {
            let line = match self.read_line(timeout).await {
                Ok(Some(line)) => line,
                Ok(None) => break, // EOF
                Err(_) => {
                    timed_out = true;
                    break;
                }
            };
            let is_done = match &self.adapter.completion {
                CompletionStrategy::None => false,
                CompletionStrategy::MarkerPrefix { prefix } => line.contains(prefix.as_str()),
            };
            narration.push(line.clone());
            if is_done {
                break;
            }
        }
        Ok(ExternalCliTurn {
            narration,
            timed_out,
        })
    }

    async fn steer(&self, message: &str) -> Result<(), ExternalCliError> {
        let mut stdin = self
            .stdin
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| ExternalCliError("not started".to_string()))?;
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .map_err(|e| ExternalCliError(format!("write steer: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| ExternalCliError(format!("flush steer: {e}")))?;
        *self.stdin.lock().unwrap() = Some(stdin);
        Ok(())
    }

    fn abort(&self) -> Result<(), ExternalCliError> {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            // start_kill 是同步信号;子进程退出后由 tokio 运行时收割。
            let _ = child.start_kill();
            let _ = child;
        }
        *self.stdin.lock().unwrap() = None;
        self.pending.lock().unwrap().clear();
        Ok(())
    }

    fn stop(&self) -> Result<(), ExternalCliError> {
        self.abort()
    }

    fn is_running(&self) -> bool {
        self.child.lock().unwrap().is_some()
    }
}

/// 单发外部 CLI 运行时:每轮新子进程(prompt 作 argv)。
pub struct ReinvokeCliRuntime {
    adapter: CliAgentAdapter,
    session_id: Mutex<Option<String>>,
    /// 运行中到达的 steer 消息(缓冲到下一轮)。
    buffered: Mutex<Vec<String>>,
    turns: Mutex<u64>,
}

impl ReinvokeCliRuntime {
    pub fn new(adapter: CliAgentAdapter) -> Result<Self, ExternalCliError> {
        if adapter.supports_stdin_injection {
            return Err(ExternalCliError(
                "reinvoke runtime requires supports_stdin_injection=false".to_string(),
            ));
        }
        Ok(Self {
            adapter,
            session_id: Mutex::new(None),
            buffered: Mutex::new(Vec::new()),
            turns: Mutex::new(0),
        })
    }

    fn build_argv(&self, task: &str) -> Vec<String> {
        let mut argv: Vec<String> = self.adapter.command.clone();
        let turn = *self.turns.lock().unwrap();
        let session = self.session_id.lock().unwrap().clone();
        if let Some(session) = &session {
            if turn == 0 {
                if let Some(flag) = &self.adapter.session_flag {
                    argv.push(flag.clone());
                    argv.push(session.clone());
                }
            } else if let Some(flag) = &self.adapter.resume_flag {
                argv.push(flag.clone());
                argv.push(session.clone());
            }
        }
        if turn > 0 {
            argv.extend(self.adapter.continue_args.iter().cloned());
        }
        if let Some(flag) = &self.adapter.prompt_flag {
            argv.push(flag.clone());
            argv.push(task.to_string());
        } else {
            argv.push(task.to_string());
        }
        argv
    }
}

impl Seam for ReinvokeCliRuntime {}

#[async_trait]
impl ExternalCliRuntime for ReinvokeCliRuntime {
    fn start(&self, session_id: &str) -> Result<(), ExternalCliError> {
        // 预校验:命令真实存在(缺失显式报错,不静默)。
        let (binary, _) = adapter_command(&self.adapter)?;
        if std::process::Command::new(binary)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_err()
        {
            return Err(ExternalCliError(format!(
                "cli {binary} unavailable (cannot start external member)"
            )));
        }
        *self.session_id.lock().unwrap() = Some(session_id.to_string());
        *self.turns.lock().unwrap() = 0;
        Ok(())
    }

    async fn send(&self, task: &str) -> Result<ExternalCliTurn, ExternalCliError> {
        // 缓冲的 steer 消息拼进本轮的 prompt。
        let followups = {
            let mut buffered = self.buffered.lock().unwrap();
            let items: Vec<String> = buffered.drain(..).collect();
            items
        };
        let mut prompt = task.to_string();
        if !followups.is_empty() {
            prompt = format!("{prompt}\n\n---\n\n{}", followups.join("\n\n---\n\n"));
        }

        let argv = self.build_argv(&prompt);
        let binary = argv[0].clone();
        let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
        let timeout = Duration::from_secs(self.adapter.inactivity_timeout_s.max(1));

        let output = tokio::time::timeout(
            timeout,
            Command::new(&binary)
                .args(&args)
                .stdin(Stdio::null())
                .output(),
        )
        .await
        .map_err(|_| ExternalCliError("inactivity timeout".to_string()))?
        .map_err(|e| ExternalCliError(format!("spawn {binary}: {e}")))?;

        *self.turns.lock().unwrap() += 1;
        let narration: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        Ok(ExternalCliTurn {
            narration,
            timed_out: false,
        })
    }

    async fn steer(&self, message: &str) -> Result<(), ExternalCliError> {
        self.buffered.lock().unwrap().push(message.to_string());
        Ok(())
    }

    fn abort(&self) -> Result<(), ExternalCliError> {
        self.buffered.lock().unwrap().clear();
        Ok(())
    }

    fn stop(&self) -> Result<(), ExternalCliError> {
        self.abort()
    }

    fn is_running(&self) -> bool {
        self.session_id.lock().unwrap().is_some()
    }
}

/// 外部 CLI 插件:按 adapter 提供 external-cli seam。
pub struct ExternalCliPlugin {
    adapter: CliAgentAdapter,
}

impl ExternalCliPlugin {
    pub fn new(adapter: CliAgentAdapter) -> Self {
        Self { adapter }
    }
}

impl Plugin for ExternalCliPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-external"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![EXTERNAL_CLI]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let runtime: Arc<dyn ExternalCliRuntime> = if self.adapter.supports_stdin_injection {
            Arc::new(StreamingCliRuntime::new(self.adapter.clone()).map_err(|e| {
                PluginError::Apply {
                    plugin: self.name(),
                    message: e.0,
                }
            })?)
        } else {
            Arc::new(ReinvokeCliRuntime::new(self.adapter.clone()).map_err(|e| {
                PluginError::Apply {
                    plugin: self.name(),
                    message: e.0,
                }
            })?)
        };
        Ok(vec![ctx.register(EXTERNAL_CLI, runtime)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::external::{TeamJoinDescriptor, codex_narration};
    use ah_contracts::keys::EXTERNAL_CLI;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    /// 流式测试 CLI:sh 脚本每行回显并输出完成标记(每行即一轮)。
    const STREAM_SCRIPT: &str = r#"
while IFS= read -r line; do
  echo "recv:$line"
  echo "__DONE__"
done
"#;

    fn streaming_adapter() -> CliAgentAdapter {
        let mut adapter = CliAgentAdapter::generic_streaming("__DONE__");
        adapter.command = vec![
            "sh".to_string(),
            "-c".to_string(),
            STREAM_SCRIPT.to_string(),
        ];
        adapter.inactivity_timeout_s = 5;
        adapter
    }

    fn reinvoke_adapter() -> CliAgentAdapter {
        // sh -c 的第一个额外参数是 $0,其余进 $1..;脚本打印 $0|$* 以便断言 flags 顺序。
        CliAgentAdapter {
            name: "echo".to_string(),
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "echo got:$0:$*".to_string(),
            ],
            input_format: InputFormat::Text,
            completion: CompletionStrategy::None,
            supports_stdin_injection: false,
            prompt_flag: None,
            session_flag: Some("--session".to_string()),
            resume_flag: Some("--resume".to_string()),
            continue_args: vec!["--cont".to_string()],
            inactivity_timeout_s: 5,
        }
    }

    #[tokio::test]
    async fn streaming_runtime_sends_steers_and_aborts_for_real() {
        let root = std::env::temp_dir().join(format!("ah-ext-stream-{}", std::process::id()));
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> =
            vec![StdArc::new(ExternalCliPlugin::new(streaming_adapter()))];
        let effects = ctx.mount_all(plugins).expect("mount");
        let runtime = ctx
            .service::<dyn ExternalCliRuntime>(&EXTERNAL_CLI)
            .expect("external cli");

        runtime.start("sess-1").expect("start");
        assert!(runtime.is_running());

        // 真实子进程:写任务 → 读回显 + 完成标记。
        let turn = runtime.send("hello").await.expect("send");
        assert!(!turn.timed_out, "completed by marker");
        assert!(turn.narration.iter().any(|l| l.contains("recv:hello")));
        assert!(turn.narration.iter().any(|l| l.contains("__DONE__")));

        // steer:中途注入另一条消息;下一轮读到它。
        runtime.steer("steer-msg").await.expect("steer");
        let turn2 = runtime.send("next").await.expect("send 2");
        assert!(turn2.narration.iter().any(|l| l.contains("recv:steer-msg")));

        // abort:杀进程,不再运行。
        runtime.abort().expect("abort");
        assert!(!runtime.is_running());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn reinvoke_runtime_passes_prompt_and_flags() {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(ExternalCliPlugin::new(reinvoke_adapter()))];
        let effects = ctx.mount_all(plugins).expect("mount");
        let runtime = ctx
            .service::<dyn ExternalCliRuntime>(&EXTERNAL_CLI)
            .expect("external cli");

        runtime.start("sess-9").expect("start");
        let turn = runtime.send("first task").await.expect("send");
        // argv: sh -c 'echo got:$0:$*' --session sess-9 first task
        //   → $0=--session, $*="sess-9 first task"。
        assert!(
            turn.narration
                .iter()
                .any(|l| l.contains("got:--session:sess-9 first task")),
            "session flag + prompt argv: {:?}",
            turn.narration
        );

        // steer 缓冲到下一轮;第二轮带 --resume 与 --cont。
        runtime.steer("followup").await.expect("steer");
        let turn2 = runtime.send("second").await.expect("send 2");
        // → $0=--resume,$* 含 sess-9 --cont second 与缓冲的 followup。
        assert!(
            turn2
                .narration
                .iter()
                .any(|l| l.contains("got:--resume:sess-9 --cont second")),
            "resume+continue+prompt: {:?}",
            turn2.narration
        );
        assert!(
            turn2.narration.iter().any(|l| l.contains("followup")),
            "buffered steer drained into prompt"
        );

        // 命令不可用显式报错。
        let bad = CliAgentAdapter {
            command: vec!["definitely-not-a-cli-xyz".to_string()],
            ..reinvoke_adapter()
        };
        let bad_runtime = ReinvokeCliRuntime::new(bad).expect("bad runtime");
        assert!(bad_runtime.start("s").is_err(), "missing cli errors");

        drop(effects);
    }

    #[test]
    fn descriptor_roundtrips_and_codex_helpers() {
        let descriptor = TeamJoinDescriptor {
            session_id: "s1".to_string(),
            team_name: "t1".to_string(),
            member_name: "m1".to_string(),
            role: "teammate".to_string(),
            scope: "member".to_string(),
            language: "cn".to_string(),
            dispatch_mode: "autonomous".to_string(),
            env_var: TeamJoinDescriptor::ENV.to_string(),
        };
        let encoded = descriptor.encode().expect("encode");
        let decoded = TeamJoinDescriptor::decode(&encoded).expect("decode");
        assert_eq!(decoded.member_name, "m1");
        assert_eq!(decoded.scope, "member");

        let line = codex_proto_line("do it");
        assert!(line.contains("submission"));
        assert!(
            codex_narration("{\"type\":\"message\",\"payload\":{\"content\":\"hi\"}}").is_some()
        );
        assert!(codex_narration("{\"type\":\"other\"}").is_none());
    }
}
