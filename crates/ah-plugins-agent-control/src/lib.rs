use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ah_contracts::agent::{
    AgentCallbackContext, AgentCallbackManager, AgentControl, AgentControlError, InterruptRuntime,
};
use ah_contracts::effect::Effect;
use ah_contracts::keys::{AGENT_CALLBACKS, INTERRUPT};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

#[derive(Default)]
pub struct LocalInterruptRuntime {
    states: Mutex<HashMap<String, AgentControl>>,
}

impl Seam for LocalInterruptRuntime {}

#[async_trait]
impl InterruptRuntime for LocalInterruptRuntime {
    async fn request(
        &self,
        session_id: &str,
        control: AgentControl,
    ) -> Result<(), AgentControlError> {
        if session_id.trim().is_empty() {
            return Err(AgentControlError(
                "session id must not be empty".to_string(),
            ));
        }
        self.states
            .lock()
            .unwrap()
            .insert(session_id.to_string(), control);
        Ok(())
    }

    fn state(&self, session_id: &str) -> AgentControl {
        self.states
            .lock()
            .unwrap()
            .get(session_id)
            .copied()
            .unwrap_or(AgentControl::Continue)
    }

    fn clear(&self, session_id: &str) {
        self.states.lock().unwrap().remove(session_id);
    }
}

#[derive(Default)]
pub struct LocalAgentCallbackManager {
    callbacks: Mutex<Vec<AgentCallbackContext>>,
}

impl LocalAgentCallbackManager {
    pub fn callbacks(&self) -> Vec<AgentCallbackContext> {
        self.callbacks.lock().unwrap().clone()
    }
}

impl Seam for LocalAgentCallbackManager {}

#[async_trait]
impl AgentCallbackManager for LocalAgentCallbackManager {
    async fn notify(&self, callback: AgentCallbackContext) -> Result<(), AgentControlError> {
        self.callbacks.lock().unwrap().push(callback);
        Ok(())
    }
}

pub struct AgentControlPlugin;

impl Plugin for AgentControlPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-agent-control"
    }
    fn provides(&self) -> Vec<ServiceKey> {
        vec![INTERRUPT, AGENT_CALLBACKS]
    }
    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let interrupt: Arc<dyn InterruptRuntime> = Arc::new(LocalInterruptRuntime::default());
        let callbacks: Arc<dyn AgentCallbackManager> =
            Arc::new(LocalAgentCallbackManager::default());
        Ok(vec![
            ctx.register(INTERRUPT, interrupt),
            ctx.register(AGENT_CALLBACKS, callbacks),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::agent::InterruptRuntime;

    #[tokio::test]
    async fn callback_manager_preserves_lifecycle_order() {
        let manager = LocalAgentCallbackManager::default();
        manager
            .notify(AgentCallbackContext {
                session_id: "s1".into(),
                state: ah_contracts::agent::AgentRunState::Running,
                iteration: 0,
                payload: serde_json::json!({"phase": "start"}),
            })
            .await
            .unwrap();
        manager
            .notify(AgentCallbackContext {
                session_id: "s1".into(),
                state: ah_contracts::agent::AgentRunState::Completed,
                iteration: 1,
                payload: serde_json::json!({"phase": "done"}),
            })
            .await
            .unwrap();
        let callbacks = manager.callbacks();
        assert_eq!(callbacks.len(), 2);
        assert_eq!(
            callbacks[0].state,
            ah_contracts::agent::AgentRunState::Running
        );
        assert_eq!(
            callbacks[1].state,
            ah_contracts::agent::AgentRunState::Completed
        );
        assert_eq!(callbacks[1].payload["phase"], "done");
    }

    #[tokio::test]
    async fn control_is_requestable_and_clearable() {
        let runtime = LocalInterruptRuntime::default();
        runtime
            .request("s1", AgentControl::Interrupt)
            .await
            .unwrap();
        assert_eq!(runtime.state("s1"), AgentControl::Interrupt);
        runtime.clear("s1");
        assert_eq!(runtime.state("s1"), AgentControl::Continue);
        assert!(runtime.request("", AgentControl::Cancel).await.is_err());
    }
}
