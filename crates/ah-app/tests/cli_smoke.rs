//! CLI e2e 冒烟测试:真实构建的 ah-cli 二进制 + 真实 seam 子命令。
//!
//! 输入/输出走真实 stdin/stdout;子进程在当前目录下写入 .agent-harness
//! (独立临时目录,不污染仓库)。

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn cli_subcommands_run_for_real() {
    let bin = env!("CARGO_BIN_EXE_ah-cli");
    let root = std::env::temp_dir().join(format!("ah-cli-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create temp dir");
    let env_file = root.join("test.env");
    std::fs::write(&env_file, "# deterministic dev profile fixture\n").expect("write env file");
    let profile = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/dev.toml");

    let mut child = Command::new(bin)
        .arg(profile)
        .current_dir(&root)
        .env("AH_ENV_FILE", &env_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ah-cli");

    let commands = [
        "/workspace goal add g1 ship the harness\n",
        "/workspace goal done g1\n",
        "/workspace goals\n",
        "/teams create t1 Alpha\n",
        "/queue publish chat {\"a\":1}\n",
        "/queue consume chat\n",
        "/code run print(7*6)\n",
        "/quit\n",
    ]
    .concat();
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(commands.as_bytes())
        .expect("write commands");

    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "cli must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("goal g1 added"), "workspace goal add");
    assert!(
        stdout.contains("g1 [Done] ship the harness"),
        "goal done + list"
    );
    assert!(stdout.contains("team t1 created"), "teams create (sqlite)");
    assert!(stdout.contains("published seq 0"), "queue publish");
    assert!(stdout.contains("consumed seq 0"), "queue consume");
    assert!(
        stdout.contains("stdout: 42"),
        "code run python3 real execution"
    );

    let _ = std::fs::remove_dir_all(&root);
}
