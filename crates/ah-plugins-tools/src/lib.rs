//! # ah-plugins-tools
//!
//! 提供 tools seam 的注册表实现(真实基础设施)。
//! 注册表本身是通用存储;工具的能力由各插件注册到它上面
//! (如 ah-plugins-sysop 注册真实的 read_file/run_shell 等)。
//!
//! 工具执行经过真实管线:tools/pre-execute(waterfall,可拒绝/改写)→ 执行
//! → tools/post-execute(serial,顺序通知)。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ah_contracts::keys::TOOLS;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{
    Tool, ToolDecision, ToolError, ToolExecuted, ToolInvocation, ToolInvocationContext,
    ToolRegistry,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::Value;

/// 内存工具注册表:实现 ToolRegistry seam,并承载工具执行管线。
pub struct LocalToolRegistry {
    tools: Arc<Mutex<HashMap<String, Arc<dyn Tool>>>>,
    ctx: Context,
}

impl LocalToolRegistry {
    /// 创建注册表(需要 Context 以发布管线事件)。
    pub fn new(ctx: Context) -> Self {
        Self {
            tools: Arc::new(Mutex::new(HashMap::new())),
            ctx,
        }
    }
}
impl LocalToolRegistry {
    async fn invoke_impl(
        &self,
        name: &str,
        call_id: Option<&str>,
        context: ToolInvocationContext,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        let decision = self
            .ctx
            .waterfall(
                ToolInvocation {
                    name: name.to_string(),
                    arguments: arguments.clone(),
                    context: context.clone(),
                },
                ToolDecision::allow(arguments.clone()),
            )
            .await;
        if !decision.allow {
            let reason = decision
                .reason
                .unwrap_or_else(|| "rejected by rail".to_string());
            return Err(ToolError(format!("rejected by rail: {reason}")));
        }

        let tool = self
            .get(name)
            .ok_or_else(|| ToolError(format!("tool not found: {name}")))?;
        let started = Instant::now();
        let output = if let Some(call_id) = call_id {
            tool.invoke_with_id_and_context(&context, call_id, decision.arguments)
                .await?
        } else {
            tool.invoke_with_context(&context, decision.arguments)
                .await?
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        self.ctx
            .serial(ToolExecuted {
                name: name.to_string(),
                arguments,
                output: output.clone(),
                elapsed_ms,
                context,
            })
            .await;
        Ok(output)
    }
}

impl Seam for LocalToolRegistry {}

#[async_trait]
impl ToolRegistry for LocalToolRegistry {
    fn register(&self, tool: Arc<dyn Tool>) -> Effect {
        let name = tool.name().to_string();
        self.tools.lock().unwrap().insert(name.clone(), tool);
        let tools = self.tools.clone();
        Effect::new(move || {
            tools.lock().unwrap().remove(&name);
        })
    }

    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.lock().unwrap().get(name).cloned()
    }

    fn names(&self) -> Vec<String> {
        self.tools.lock().unwrap().keys().cloned().collect()
    }

    async fn invoke(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        self.invoke_impl(name, None, ToolInvocationContext::default(), arguments)
            .await
    }

    async fn invoke_with_context(
        &self,
        name: &str,
        context: ToolInvocationContext,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        self.invoke_impl(name, None, context, arguments).await
    }

    async fn invoke_with_id(
        &self,
        name: &str,
        call_id: &str,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        self.invoke_impl(
            name,
            Some(call_id),
            ToolInvocationContext::default(),
            arguments,
        )
        .await
    }

    async fn invoke_with_id_and_context(
        &self,
        name: &str,
        call_id: &str,
        context: ToolInvocationContext,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        self.invoke_impl(name, Some(call_id), context, arguments)
            .await
    }
}

/// 提供 tools seam 的插件。
pub struct ToolsPlugin;

impl Plugin for ToolsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-tools"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let registry: Arc<dyn ToolRegistry> = Arc::new(LocalToolRegistry::new(ctx.clone()));
        Ok(vec![ctx.register(TOOLS, registry)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 回显工具:返回 {ok: true, echo: arguments}。
    struct NoopTool;

    #[async_trait]
    impl Tool for NoopTool {
        fn name(&self) -> &'static str {
            "noop"
        }

        fn description(&self) -> &'static str {
            "do nothing"
        }

        async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
            Ok(json!({ "ok": true, "echo": arguments }))
        }
    }

    fn mount_with_tool() -> (Context, Vec<Effect>, String) {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ToolsPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools seam");
        let effect = registry.register(Arc::new(NoopTool));
        let mut all = effects;
        all.push(effect);
        (ctx, all, "noop".to_string())
    }

    #[tokio::test]
    async fn invoke_publishes_post_execute_event() {
        let (ctx, effects, name) = mount_with_tool();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let _listener = ctx.on_serial::<ToolExecuted, _, _>(move |event| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push(event.name.clone());
            }
        });

        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let output = registry
            .invoke(&name, json!({ "x": 1 }))
            .await
            .expect("invoke");
        assert_eq!(output["echo"]["x"], 1);
        assert_eq!(*seen.lock().unwrap(), vec!["noop".to_string()]);

        drop(effects);
    }
    #[tokio::test]
    async fn invoke_with_id_passes_stable_id_to_tool() {
        struct IdAwareTool {
            seen: Arc<Mutex<Vec<String>>>,
        }

        #[async_trait]
        impl Tool for IdAwareTool {
            fn name(&self) -> &'static str {
                "id_aware"
            }

            fn description(&self) -> &'static str {
                "records the idempotency key"
            }

            fn idempotent(&self) -> bool {
                true
            }

            async fn invoke(&self, _arguments: Value) -> Result<Value, ToolError> {
                Ok(json!({"fallback": true}))
            }

            async fn invoke_with_id(
                &self,
                call_id: &str,
                _arguments: Value,
            ) -> Result<Value, ToolError> {
                self.seen.lock().unwrap().push(call_id.to_string());
                Ok(json!({"deduplicated": true}))
            }
        }

        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ToolsPlugin);
        let effects = ctx.mount(&plugin).expect("mount tools");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let tool_effect = registry.register(Arc::new(IdAwareTool { seen: seen.clone() }));
        let result = registry
            .invoke_with_id("id_aware", "call-42", json!({}))
            .await
            .expect("invoke with id");
        assert_eq!(result["deduplicated"], true);
        assert_eq!(*seen.lock().unwrap(), vec!["call-42"]);
        drop(tool_effect);
        drop(effects);
    }

    #[tokio::test]
    async fn invocation_context_reaches_tool_and_pipeline_events() {
        struct ContextAwareTool {
            seen: Arc<Mutex<Vec<ToolInvocationContext>>>,
        }

        #[async_trait]
        impl Tool for ContextAwareTool {
            fn name(&self) -> &'static str {
                "context_aware"
            }
            fn description(&self) -> &'static str {
                "records invocation context"
            }
            async fn invoke(&self, _arguments: Value) -> Result<Value, ToolError> {
                panic!("context-aware path must be used")
            }
            async fn invoke_with_context(
                &self,
                context: &ToolInvocationContext,
                _arguments: Value,
            ) -> Result<Value, ToolError> {
                self.seen.lock().unwrap().push(context.clone());
                Ok(json!({"ok": true}))
            }
        }

        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ToolsPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let pre = Arc::new(Mutex::new(Vec::new()));
        let post = Arc::new(Mutex::new(Vec::new()));
        let pre_seen = pre.clone();
        let _pre_listener =
            ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(move |event, decision, next| {
                pre_seen.lock().unwrap().push(event.context);
                async move { next.next(decision).await }
            });
        let post_seen = post.clone();
        let _post_listener = ctx.on_serial::<ToolExecuted, _, _>(move |event| {
            post_seen.lock().unwrap().push(event.context);
            async {}
        });
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let _tool = registry.register(Arc::new(ContextAwareTool { seen: seen.clone() }));
        let first = ToolInvocationContext::new("session-a", Some("user-a".into()));
        let second = ToolInvocationContext::new("session-b", Some("user-b".into()));
        for invocation in [&first, &second] {
            registry
                .invoke_with_context("context_aware", invocation.clone(), json!({}))
                .await
                .expect("invoke with context");
        }

        let expected = vec![first, second];
        assert_eq!(*seen.lock().unwrap(), expected);
        assert_eq!(*pre.lock().unwrap(), expected);
        assert_eq!(*post.lock().unwrap(), expected);
        drop(effects);
    }

    #[tokio::test]
    async fn pre_execute_rewrite_changes_arguments() {
        let (ctx, effects, name) = mount_with_tool();
        // pre-execute 改写:给参数加一个字段。
        let _listener = ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(
            |_event, decision, next| async move {
                let mut args = decision.arguments.clone();
                args["rewritten"] = json!(true);
                next.next(ToolDecision::allow(args)).await
            },
        );

        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let output = registry
            .invoke(&name, json!({ "x": 1 }))
            .await
            .expect("invoke");
        assert_eq!(output["echo"]["rewritten"], true);

        drop(effects);
    }

    #[tokio::test]
    async fn pre_execute_deny_short_circuits_execution() {
        let (ctx, effects, name) = mount_with_tool();
        // 拒绝所有调用:不委托 next 即短路。
        let _listener = ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(
            |_event, decision, _next| async move { ToolDecision::deny(decision.arguments, "nope") },
        );
        // post-execute 不应触发。
        let post = Arc::new(AtomicUsize::new(0));
        let p = post.clone();
        let _p_listener = ctx.on_serial::<ToolExecuted, _, _>(move |_e| {
            let p = p.clone();
            async move {
                p.fetch_add(1, Ordering::SeqCst);
            }
        });

        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let error = registry.invoke(&name, json!({})).await.expect_err("denied");
        assert!(error.0.contains("nope"));
        assert_eq!(post.load(Ordering::SeqCst), 0);

        drop(effects);
    }
}
