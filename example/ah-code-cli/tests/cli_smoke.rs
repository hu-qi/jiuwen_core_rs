use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn temp_workspace(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ah-code-cli-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create workspace");
    root
}

#[test]
fn once_mode_runs_mock_agent_through_real_tool_chain() {
    let root = temp_workspace("agent");
    fs::write(root.join("README.md"), "example workspace\n").expect("seed workspace");

    let output = Command::new(env!("CARGO_BIN_EXE_ah-code"))
        .args([
            "--workspace",
            root.to_str().unwrap(),
            "--once",
            "inspect this workspace",
        ])
        .output()
        .expect("run ah-code");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ah-code"));
    assert!(stdout.contains("mock final answer"));
    assert!(stdout.contains("LS(.)"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_write_command_stays_inside_workspace() {
    let root = temp_workspace("write");

    let output = Command::new(env!("CARGO_BIN_EXE_ah-code"))
        .args([
            "--workspace",
            root.to_str().unwrap(),
            "--once",
            "/write notes.txt hello from cli",
        ])
        .output()
        .expect("run ah-code");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "hello from cli"
    );
    let _ = fs::remove_dir_all(root);
}
