use std::path::Path;

#[test]
fn cli_plugin_lives_under_the_example_tree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(root.join("plugins/ah-plugins-cli/Cargo.toml").is_file());
    assert!(!root.join("../../crates/ah-plugins-cli/Cargo.toml").exists());
}
