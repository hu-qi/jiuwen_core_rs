//! CLI 集成测试:真实运行 ah-cli 二进制,管道输入任务与会话命令。

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn cli_runs_task_and_manages_sessions() {
    let bin = env!("CARGO_BIN_EXE_ah-cli");
    // 隔离工作目录,避免污染仓库的 .agent-harness。
    let cwd = std::env::temp_dir().join(format!("ah-cli-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cwd);
    std::fs::create_dir_all(&cwd).expect("create cwd");
    let env_file = cwd.join("test.env");
    std::fs::write(&env_file, "# deterministic dev profile fixture\n")
        .expect("write env file");
    let profile = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/dev.toml");

    let mut child = Command::new(bin)
        .current_dir(&cwd)
        .arg(profile)
        .env("AH_ENV_FILE", &env_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn ah-cli");

    let mut stdin = child.stdin.take().expect("stdin");
    let input = "/list\nwrite a note\n/new task-b\n/list\n/fork task-b task-c\n/list\n/quit\n";
    stdin.write_all(input.as_bytes()).expect("write stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // 任务真实执行(agent 循环 + 真实 list_dir 工具)。
    assert!(
        stdout.contains("answer: mock final answer"),
        "got: {stdout}"
    );
    // 会话管理真实持久化。
    assert!(stdout.contains("switched to session task-b"));
    assert!(stdout.contains("forked task-b -> task-c and switched"));
    assert!(stdout.contains("sessions: [\"task-b\", \"task-c\"]"));
    // 会话文件真实落盘。
    assert!(cwd.join(".agent-harness/sessions/task-c.jsonl").exists());

    let _ = std::fs::remove_dir_all(&cwd);
}
