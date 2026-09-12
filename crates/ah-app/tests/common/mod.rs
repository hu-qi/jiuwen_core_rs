use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub struct EnvFileGuard {
    previous: Option<OsString>,
}

impl Drop for EnvFileGuard {
    fn drop(&mut self) {
        // SAFETY: each integration test binary owns its process environment.
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var("AH_ENV_FILE", value),
                None => std::env::remove_var("AH_ENV_FILE"),
            }
        }
    }
}

pub fn empty_env_file(root: &Path) -> PathBuf {
    let path = root.join("test.env");
    std::fs::write(&path, "# deterministic dev profile fixture\n").expect("write env file");
    path
}

pub fn scratch_env_file(root: &Path) -> EnvFileGuard {
    let path = empty_env_file(root);
    let previous = std::env::var_os("AH_ENV_FILE");
    // SAFETY: each integration test binary owns its process environment.
    unsafe { std::env::set_var("AH_ENV_FILE", path) };
    EnvFileGuard { previous }
}
