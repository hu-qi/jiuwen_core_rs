//! Remote policy sandbox over a JSON HTTP service.

use std::sync::Arc;

use ah_contracts::sandbox::{
    CommandDecision, FsDecision, SandboxError, SandboxPolicy, SandboxProvider,
};
use ah_contracts::seam::Seam;
use serde_json::{Value, json};

/// HTTP client for a remote sandbox policy/execution control service.
pub struct RemoteSandboxProvider {
    base_url: String,
    bearer_token: Option<String>,
    agent: ureq::Agent,
}

impl RemoteSandboxProvider {
    pub fn new(
        base_url: impl Into<String>,
        bearer_token: Option<String>,
    ) -> Result<Self, SandboxError> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(SandboxError(
                "remote sandbox URL must use http or https".into(),
            ));
        }
        Ok(Self {
            base_url,
            bearer_token,
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(10))
                .build(),
        })
    }

    pub fn from_env() -> Result<Option<Arc<dyn SandboxProvider>>, SandboxError> {
        let Ok(url) = std::env::var("SANDBOX_REMOTE_URL") else {
            return Ok(None);
        };
        if url.trim().is_empty() {
            return Ok(None);
        }
        let token = std::env::var("SANDBOX_REMOTE_TOKEN")
            .ok()
            .filter(|value| !value.is_empty());
        Ok(Some(Arc::new(Self::new(url, token)?)))
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        let request = self
            .agent
            .request(method, &format!("{}{path}", self.base_url));
        if let Some(token) = &self.bearer_token {
            request.set("Authorization", &format!("Bearer {token}"))
        } else {
            request
        }
    }

    fn post_json(&self, path: &str, body: Value) -> Result<Value, SandboxError> {
        self.request("POST", path)
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|error| SandboxError(format!("remote sandbox POST {path}: {error}")))?
            .into_json()
            .map_err(|error| SandboxError(format!("remote sandbox POST {path} JSON: {error}")))
    }

    fn get_json(&self, path: &str) -> Result<Value, SandboxError> {
        self.request("GET", path)
            .call()
            .map_err(|error| SandboxError(format!("remote sandbox GET {path}: {error}")))?
            .into_json()
            .map_err(|error| SandboxError(format!("remote sandbox GET {path} JSON: {error}")))
    }
}

impl Seam for RemoteSandboxProvider {}

impl SandboxProvider for RemoteSandboxProvider {
    fn check_fs(&self, path: &str) -> Result<FsDecision, SandboxError> {
        serde_json::from_value(self.post_json("/check/fs", json!({"path": path}))?)
            .map_err(|error| SandboxError(format!("remote sandbox fs decision: {error}")))
    }

    fn check_command(&self, command: &str) -> Result<CommandDecision, SandboxError> {
        serde_json::from_value(self.post_json("/check/command", json!({"command": command}))?)
            .map_err(|error| SandboxError(format!("remote sandbox command decision: {error}")))
    }

    fn policy(&self) -> Result<SandboxPolicy, SandboxError> {
        serde_json::from_value(self.get_json("/policy")?)
            .map_err(|error| SandboxError(format!("remote sandbox policy: {error}")))
    }

    fn set_policy(&self, policy: SandboxPolicy) -> Result<(), SandboxError> {
        self.post_json(
            "/policy",
            serde_json::to_value(policy).map_err(|error| {
                SandboxError(format!("serialize remote sandbox policy: {error}"))
            })?,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn remote_provider_roundtrips_policy_and_decisions() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let handle = thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut bytes = [0_u8; 2048];
                let size = stream.read(&mut bytes).expect("request");
                let request = String::from_utf8_lossy(&bytes[..size]);
                let body = if request.starts_with("GET /policy") {
                    r#"{"allowed_path_prefixes":["src/"],"denied_command_patterns":["rm -rf"],"allow_absolute_paths":false}"#
                } else if request.starts_with("POST /check/fs") {
                    r#"{"allow":true,"reason":"remote policy"}"#
                } else {
                    "{}"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).expect("response");
            }
        });
        let provider =
            RemoteSandboxProvider::new(format!("http://{address}"), Some("secret-token".into()))
                .expect("provider");
        assert!(provider.check_fs("src/lib.rs").expect("fs").allow);
        assert_eq!(
            provider.policy().expect("policy").allowed_path_prefixes,
            vec!["src/"]
        );
        provider
            .set_policy(SandboxPolicy::default())
            .expect("set policy");
        handle.join().expect("server");
    }
}
