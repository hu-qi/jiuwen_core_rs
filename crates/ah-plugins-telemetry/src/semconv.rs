//! # semconv — OpenTelemetry 语义约定常量(对齐 tracer_otel.semconv)
//!
//! 与 Python 规格 `openjiuwen/extensions/tracer_otel/semconv.py` 逐条对齐:
//! - GenAI 标准属性(OpenLLMetry / GenAI 语义约定 `gen_ai.*`);
//! - 工作流自定义属性(`openjiuwen.workflow.*`);
//! - Agent(非 LLM)属性(`openjiuwen.agent.*`);
//! - trace 桥接属性与基础 span 属性(`openjiuwen.*`)。
//!
//! 所有属性键集中在此定义,避免各 handler 间打字漂移(与 Python 模块同目的)。
//!
//! 与 Python 规格一致,常量/助手是规格表面(供各 handler 消费):当前 handler 已消费
//! agent 级与基础 span 属性,其余常量(gen_ai.* / workflow.* / trace 桥接)供后续
//! LLM span 与 workflow span handler 使用,故整体允许 dead_code。
#![allow(dead_code)]

use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// GenAI 标准属性(对齐 observability/semconv.py)
// ---------------------------------------------------------------------------

pub const GEN_AI_SYSTEM: &str = "gen_ai.system";
pub const GEN_AI_SYSTEM_VALUE: &str = "openjiuwen";
pub const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
pub const GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";
pub const GEN_AI_PROMPT: &str = "gen_ai.prompt";
pub const GEN_AI_COMPLETION: &str = "gen_ai.completion";

pub const GEN_AI_USAGE_PROMPT_TOKENS: &str = "gen_ai.usage.prompt_tokens";
pub const GEN_AI_USAGE_COMPLETION_TOKENS: &str = "gen_ai.usage.completion_tokens";

pub const GEN_AI_TOOL_NAME: &str = "gen_ai.tool.name";

// ---------------------------------------------------------------------------
// openjiuwen.workflow.* — 工作流级自定义属性
// ---------------------------------------------------------------------------

pub const OJ_WORKFLOW_ID: &str = "openjiuwen.workflow.id";
pub const OJ_WORKFLOW_NAME: &str = "openjiuwen.workflow.name";
pub const OJ_WORKFLOW_VERSION: &str = "openjiuwen.workflow.version";
pub const OJ_WORKFLOW_COMPONENT_ID: &str = "openjiuwen.workflow.component.id";
pub const OJ_WORKFLOW_COMPONENT_TYPE: &str = "openjiuwen.workflow.component.type";
pub const OJ_WORKFLOW_COMPONENT_NAME: &str = "openjiuwen.workflow.component.name";
pub const OJ_WORKFLOW_EXECUTION_ID: &str = "openjiuwen.workflow.execution_id";
pub const OJ_WORKFLOW_LOOP_NODE_ID: &str = "openjiuwen.workflow.loop.node_id";
pub const OJ_WORKFLOW_LOOP_INDEX: &str = "openjiuwen.workflow.loop.index";

// ---------------------------------------------------------------------------
// openjiuwen.agent.* — Agent 级自定义属性(非 LLM 类型)
// ---------------------------------------------------------------------------

pub const OJ_AGENT_INVOKE_TYPE: &str = "openjiuwen.agent.invoke_type";
pub const OJ_AGENT_NAME: &str = "openjiuwen.agent.name";
pub const OJ_AGENT_INPUTS: &str = "openjiuwen.agent.inputs";
pub const OJ_AGENT_OUTPUTS: &str = "openjiuwen.agent.outputs";
pub const OJ_AGENT_ERROR_MESSAGE: &str = "openjiuwen.agent.error_message";

// ---------------------------------------------------------------------------
// Trace ID 桥接 — 把 OTel trace 与 tracer UUID 关联
// ---------------------------------------------------------------------------

pub const OJ_TRACE_ID: &str = "openjiuwen.trace.id";
pub const OJ_SESSION_ID: &str = "openjiuwen.session_id";

// ---------------------------------------------------------------------------
// openjiuwen.* — 基础 span 属性(两个 handler 共用)
// ---------------------------------------------------------------------------

pub const OJ_INVOKE_ID: &str = "openjiuwen.invoke_id";
pub const OJ_PARENT_INVOKE_ID: &str = "openjiuwen.parent_invoke_id";
pub const OJ_START_TIME: &str = "openjiuwen.start_time";
pub const OJ_END_TIME: &str = "openjiuwen.end_time";
pub const OJ_ELAPSED_TIME: &str = "openjiuwen.elapsed_time";
pub const OJ_STATUS: &str = "openjiuwen.status";
pub const OJ_ERROR: &str = "openjiuwen.error";
pub const OJ_CHILD_INVOKE_IDS: &str = "openjiuwen.child_invoke_ids";
pub const OJ_META_DATA: &str = "openjiuwen.meta_data";

// ---------------------------------------------------------------------------
// openjiuwen.* — 工作流专用基础属性
// ---------------------------------------------------------------------------

pub const OJ_PARENT_NODE_ID: &str = "openjiuwen.parent_node_id";
pub const OJ_SOURCE_IDS: &str = "openjiuwen.source_ids";
pub const OJ_INNER_ERROR: &str = "openjiuwen.inner_error";
pub const OJ_STREAM_INPUTS: &str = "openjiuwen.stream_inputs";
pub const OJ_STREAM_OUTPUTS: &str = "openjiuwen.stream_outputs";
pub const OJ_INTERACTIVE_INPUTS: &str = "openjiuwen.interactive_inputs";
pub const OJ_WORKFLOW_INPUTS: &str = "openjiuwen.workflow.inputs";
pub const OJ_WORKFLOW_OUTPUTS: &str = "openjiuwen.workflow.outputs";
pub const OJ_WORKFLOW_ERROR_MESSAGE: &str = "openjiuwen.workflow.error_message";
pub const OJ_WORKFLOW_INVOKE_DATA: &str = "openjiuwen.workflow.invoke_data";

// ---------------------------------------------------------------------------
// 属性构建助手(把规格常量落到 span.attributes)
// ---------------------------------------------------------------------------

/// 插入键值(仅当 value 为 Some 时)。
fn insert_opt(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(v) = value {
        map.insert(key.to_string(), v);
    }
}

/// GenAI 标准属性:`gen_ai.*`(system 恒为 "openjiuwen")。
pub fn gen_ai_attributes(
    model: Option<&str>,
    operation: Option<&str>,
    prompt: Option<&str>,
    completion: Option<&str>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    tool_name: Option<&str>,
) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(GEN_AI_SYSTEM.to_string(), json!(GEN_AI_SYSTEM_VALUE));
    insert_opt(&mut map, GEN_AI_REQUEST_MODEL, model.map(|v| json!(v)));
    insert_opt(&mut map, GEN_AI_OPERATION_NAME, operation.map(|v| json!(v)));
    insert_opt(&mut map, GEN_AI_PROMPT, prompt.map(|v| json!(v)));
    insert_opt(&mut map, GEN_AI_COMPLETION, completion.map(|v| json!(v)));
    insert_opt(
        &mut map,
        GEN_AI_USAGE_PROMPT_TOKENS,
        prompt_tokens.map(|v| json!(v)),
    );
    insert_opt(
        &mut map,
        GEN_AI_USAGE_COMPLETION_TOKENS,
        completion_tokens.map(|v| json!(v)),
    );
    insert_opt(&mut map, GEN_AI_TOOL_NAME, tool_name.map(|v| json!(v)));
    map
}

/// Agent 级属性:`openjiuwen.agent.*`。
pub fn agent_attributes(
    invoke_type: &str,
    name: &str,
    inputs: Option<Value>,
    outputs: Option<Value>,
    error_message: Option<&str>,
) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(OJ_AGENT_INVOKE_TYPE.to_string(), json!(invoke_type));
    map.insert(OJ_AGENT_NAME.to_string(), json!(name));
    insert_opt(&mut map, OJ_AGENT_INPUTS, inputs);
    insert_opt(&mut map, OJ_AGENT_OUTPUTS, outputs);
    insert_opt(
        &mut map,
        OJ_AGENT_ERROR_MESSAGE,
        error_message.map(|v| json!(v)),
    );
    map
}

/// 工作流级属性:`openjiuwen.workflow.*`(全部可选)。
#[allow(clippy::too_many_arguments)]
pub fn workflow_attributes(
    id: Option<&str>,
    name: Option<&str>,
    version: Option<&str>,
    component_id: Option<&str>,
    component_type: Option<&str>,
    component_name: Option<&str>,
    execution_id: Option<&str>,
    loop_node_id: Option<&str>,
    loop_index: Option<u64>,
) -> Map<String, Value> {
    let mut map = Map::new();
    insert_opt(&mut map, OJ_WORKFLOW_ID, id.map(|v| json!(v)));
    insert_opt(&mut map, OJ_WORKFLOW_NAME, name.map(|v| json!(v)));
    insert_opt(&mut map, OJ_WORKFLOW_VERSION, version.map(|v| json!(v)));
    insert_opt(
        &mut map,
        OJ_WORKFLOW_COMPONENT_ID,
        component_id.map(|v| json!(v)),
    );
    insert_opt(
        &mut map,
        OJ_WORKFLOW_COMPONENT_TYPE,
        component_type.map(|v| json!(v)),
    );
    insert_opt(
        &mut map,
        OJ_WORKFLOW_COMPONENT_NAME,
        component_name.map(|v| json!(v)),
    );
    insert_opt(
        &mut map,
        OJ_WORKFLOW_EXECUTION_ID,
        execution_id.map(|v| json!(v)),
    );
    insert_opt(
        &mut map,
        OJ_WORKFLOW_LOOP_NODE_ID,
        loop_node_id.map(|v| json!(v)),
    );
    insert_opt(
        &mut map,
        OJ_WORKFLOW_LOOP_INDEX,
        loop_index.map(|v| json!(v)),
    );
    map
}

/// 基础 span 属性:`openjiuwen.*`(invoke_id / 时间 / 状态 / 错误 / 子调用)。
#[allow(clippy::too_many_arguments)]
pub fn base_span_attributes(
    invoke_id: &str,
    parent_invoke_id: Option<&str>,
    start_ms: u64,
    end_ms: u64,
    elapsed_ms: u64,
    status: &str,
    error: Option<&str>,
    child_invoke_ids: Vec<String>,
    meta: Option<Value>,
) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(OJ_INVOKE_ID.to_string(), json!(invoke_id));
    insert_opt(
        &mut map,
        OJ_PARENT_INVOKE_ID,
        parent_invoke_id.map(|v| json!(v)),
    );
    map.insert(OJ_START_TIME.to_string(), json!(start_ms));
    map.insert(OJ_END_TIME.to_string(), json!(end_ms));
    map.insert(OJ_ELAPSED_TIME.to_string(), json!(elapsed_ms));
    map.insert(OJ_STATUS.to_string(), json!(status));
    insert_opt(&mut map, OJ_ERROR, error.map(|v| json!(v)));
    if !child_invoke_ids.is_empty() {
        map.insert(OJ_CHILD_INVOKE_IDS.to_string(), json!(child_invoke_ids));
    }
    insert_opt(&mut map, OJ_META_DATA, meta);
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_tracer_otel_semconv_spec() {
        assert_eq!(GEN_AI_SYSTEM, "gen_ai.system");
        assert_eq!(GEN_AI_SYSTEM_VALUE, "openjiuwen");
        assert_eq!(GEN_AI_REQUEST_MODEL, "gen_ai.request.model");
        assert_eq!(GEN_AI_OPERATION_NAME, "gen_ai.operation.name");
        assert_eq!(GEN_AI_PROMPT, "gen_ai.prompt");
        assert_eq!(GEN_AI_COMPLETION, "gen_ai.completion");
        assert_eq!(GEN_AI_USAGE_PROMPT_TOKENS, "gen_ai.usage.prompt_tokens");
        assert_eq!(
            GEN_AI_USAGE_COMPLETION_TOKENS,
            "gen_ai.usage.completion_tokens"
        );
        assert_eq!(GEN_AI_TOOL_NAME, "gen_ai.tool.name");

        assert_eq!(OJ_WORKFLOW_ID, "openjiuwen.workflow.id");
        assert_eq!(OJ_WORKFLOW_NAME, "openjiuwen.workflow.name");
        assert_eq!(OJ_WORKFLOW_VERSION, "openjiuwen.workflow.version");
        assert_eq!(OJ_WORKFLOW_COMPONENT_ID, "openjiuwen.workflow.component.id");
        assert_eq!(
            OJ_WORKFLOW_COMPONENT_TYPE,
            "openjiuwen.workflow.component.type"
        );
        assert_eq!(
            OJ_WORKFLOW_COMPONENT_NAME,
            "openjiuwen.workflow.component.name"
        );
        assert_eq!(OJ_WORKFLOW_EXECUTION_ID, "openjiuwen.workflow.execution_id");
        assert_eq!(OJ_WORKFLOW_LOOP_NODE_ID, "openjiuwen.workflow.loop.node_id");
        assert_eq!(OJ_WORKFLOW_LOOP_INDEX, "openjiuwen.workflow.loop.index");

        assert_eq!(OJ_AGENT_INVOKE_TYPE, "openjiuwen.agent.invoke_type");
        assert_eq!(OJ_AGENT_NAME, "openjiuwen.agent.name");
        assert_eq!(OJ_AGENT_INPUTS, "openjiuwen.agent.inputs");
        assert_eq!(OJ_AGENT_OUTPUTS, "openjiuwen.agent.outputs");
        assert_eq!(OJ_AGENT_ERROR_MESSAGE, "openjiuwen.agent.error_message");

        assert_eq!(OJ_TRACE_ID, "openjiuwen.trace.id");
        assert_eq!(OJ_SESSION_ID, "openjiuwen.session_id");

        assert_eq!(OJ_INVOKE_ID, "openjiuwen.invoke_id");
        assert_eq!(OJ_PARENT_INVOKE_ID, "openjiuwen.parent_invoke_id");
        assert_eq!(OJ_START_TIME, "openjiuwen.start_time");
        assert_eq!(OJ_END_TIME, "openjiuwen.end_time");
        assert_eq!(OJ_ELAPSED_TIME, "openjiuwen.elapsed_time");
        assert_eq!(OJ_STATUS, "openjiuwen.status");
        assert_eq!(OJ_ERROR, "openjiuwen.error");
        assert_eq!(OJ_CHILD_INVOKE_IDS, "openjiuwen.child_invoke_ids");
        assert_eq!(OJ_META_DATA, "openjiuwen.meta_data");

        assert_eq!(OJ_PARENT_NODE_ID, "openjiuwen.parent_node_id");
        assert_eq!(OJ_SOURCE_IDS, "openjiuwen.source_ids");
        assert_eq!(OJ_INNER_ERROR, "openjiuwen.inner_error");
        assert_eq!(OJ_STREAM_INPUTS, "openjiuwen.stream_inputs");
        assert_eq!(OJ_STREAM_OUTPUTS, "openjiuwen.stream_outputs");
        assert_eq!(OJ_INTERACTIVE_INPUTS, "openjiuwen.interactive_inputs");
        assert_eq!(OJ_WORKFLOW_INPUTS, "openjiuwen.workflow.inputs");
        assert_eq!(OJ_WORKFLOW_OUTPUTS, "openjiuwen.workflow.outputs");
        assert_eq!(
            OJ_WORKFLOW_ERROR_MESSAGE,
            "openjiuwen.workflow.error_message"
        );
        assert_eq!(OJ_WORKFLOW_INVOKE_DATA, "openjiuwen.workflow.invoke_data");
    }

    #[test]
    fn gen_ai_attributes_builder_matches_spec() {
        let attrs = gen_ai_attributes(
            Some("qwen-max"),
            Some("generate"),
            Some("你好"),
            Some("你好!"),
            Some(12),
            Some(5),
            Some("read_file"),
        );
        assert_eq!(attrs[GEN_AI_SYSTEM], "openjiuwen");
        assert_eq!(attrs[GEN_AI_REQUEST_MODEL], "qwen-max");
        assert_eq!(attrs[GEN_AI_OPERATION_NAME], "generate");
        assert_eq!(attrs[GEN_AI_PROMPT], "你好");
        assert_eq!(attrs[GEN_AI_COMPLETION], "你好!");
        assert_eq!(attrs[GEN_AI_USAGE_PROMPT_TOKENS], 12);
        assert_eq!(attrs[GEN_AI_USAGE_COMPLETION_TOKENS], 5);
        assert_eq!(attrs[GEN_AI_TOOL_NAME], "read_file");
        // 未提供的键不出现。
        let minimal = gen_ai_attributes(None, None, None, None, None, None, None);
        assert_eq!(minimal.len(), 1);
        assert_eq!(minimal[GEN_AI_SYSTEM], "openjiuwen");
    }

    #[test]
    fn agent_attributes_builder_matches_spec() {
        let attrs = agent_attributes(
            "agent/step",
            "agent/step#3",
            Some(json!({"task": "x"})),
            Some(json!({"done": false})),
            Some("boom"),
        );
        assert_eq!(attrs[OJ_AGENT_INVOKE_TYPE], "agent/step");
        assert_eq!(attrs[OJ_AGENT_NAME], "agent/step#3");
        assert_eq!(attrs[OJ_AGENT_INPUTS], json!({"task": "x"}));
        assert_eq!(attrs[OJ_AGENT_OUTPUTS], json!({"done": false}));
        assert_eq!(attrs[OJ_AGENT_ERROR_MESSAGE], "boom");

        let no_opt = agent_attributes("tool", "tool/read_file", None, None, None);
        assert_eq!(no_opt.len(), 2, "仅必填键");
        assert_eq!(no_opt[OJ_AGENT_INVOKE_TYPE], "tool");
        assert_eq!(no_opt[OJ_AGENT_NAME], "tool/read_file");
    }

    #[test]
    fn workflow_attributes_builder_matches_spec() {
        let attrs = workflow_attributes(
            Some("wf-1"),
            Some("build"),
            Some("v2"),
            Some("node-7"),
            Some("llm"),
            Some("writer"),
            Some("exec-9"),
            Some("loop-1"),
            Some(3),
        );
        assert_eq!(attrs[OJ_WORKFLOW_ID], "wf-1");
        assert_eq!(attrs[OJ_WORKFLOW_NAME], "build");
        assert_eq!(attrs[OJ_WORKFLOW_VERSION], "v2");
        assert_eq!(attrs[OJ_WORKFLOW_COMPONENT_ID], "node-7");
        assert_eq!(attrs[OJ_WORKFLOW_COMPONENT_TYPE], "llm");
        assert_eq!(attrs[OJ_WORKFLOW_COMPONENT_NAME], "writer");
        assert_eq!(attrs[OJ_WORKFLOW_EXECUTION_ID], "exec-9");
        assert_eq!(attrs[OJ_WORKFLOW_LOOP_NODE_ID], "loop-1");
        assert_eq!(attrs[OJ_WORKFLOW_LOOP_INDEX], 3);

        let empty = workflow_attributes(None, None, None, None, None, None, None, None, None);
        assert!(empty.is_empty());
    }

    #[test]
    fn base_span_attributes_builder_matches_spec() {
        let attrs = base_span_attributes(
            "inv-1",
            Some("inv-0"),
            1000,
            1500,
            500,
            "ok",
            None,
            vec!["inv-2".to_string()],
            Some(json!({"retry": 1})),
        );
        assert_eq!(attrs[OJ_INVOKE_ID], "inv-1");
        assert_eq!(attrs[OJ_PARENT_INVOKE_ID], "inv-0");
        assert_eq!(attrs[OJ_START_TIME], 1000);
        assert_eq!(attrs[OJ_END_TIME], 1500);
        assert_eq!(attrs[OJ_ELAPSED_TIME], 500);
        assert_eq!(attrs[OJ_STATUS], "ok");
        assert_eq!(attrs[OJ_CHILD_INVOKE_IDS], json!(["inv-2"]));
        assert_eq!(attrs[OJ_META_DATA], json!({"retry": 1}));
        assert!(!attrs.contains_key(OJ_ERROR), "无错误时不含 error 键");

        let with_err =
            base_span_attributes("i", None, 0, 1, 1, "error", Some("timeout"), vec![], None);
        assert_eq!(with_err[OJ_ERROR], "timeout");
        assert!(
            !with_err.contains_key(OJ_CHILD_INVOKE_IDS),
            "空子调用不落键"
        );
    }
}
