//! # ah-plugins-telemetry
//!
//! 真实 telemetry 基础:内存 span 存储 + JSONL 文件导出(持久化)。
//! 提供 `telemetry` seam(record_span / spans / export),并在 apply 中注册
//! 两个事件监听器,把真实运行轨迹记录为 span:
//! - `agent/step`(emit):每次 agent 步进 → `agent/step#{iteration}` span;
//! - `tools/post-execute`(serial):每次工具执行 → `tool/{name}` span,
//!   parent 指向当前 agent step(外层 span name 字符串)。
//!
//! 导出:export 把未导出的 span 逐行追加写入 `dir/telemetry.jsonl` 并 flush;
//! 启动时若文件已存在,读回已有行数作为累计导出基数(不重复导出)。
//! OTLP 导出留待后续。

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::agent::AgentStep;
use ah_contracts::keys::TELEMETRY;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::telemetry::{Span, TelemetryError, TelemetryProvider};
use ah_contracts::tools::ToolExecuted;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::json;

/// 当前 epoch 毫秒。
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 统计 JSONL 文件中的非空行数(启动时读回已导出条数)。
fn count_jsonl_lines(path: &Path) -> usize {
    let Ok(reader) = File::open(path) else {
        return 0;
    };
    BufReader::new(reader)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .count()
}

/// 真实 telemetry provider:内存 span 存储 + JSONL 文件导出。
pub struct JsonlTelemetryProvider {
    file: Mutex<File>,
    path: PathBuf,
    spans: Mutex<Vec<Span>>,
    /// 本次进程内已写入文件的 span 数(未导出 = spans.len() - written)。
    written: AtomicUsize,
    /// 累计导出条数(含启动时文件已存在的行数)。
    total_exported: AtomicUsize,
}

impl JsonlTelemetryProvider {
    /// 打开(或创建)JSONL 导出文件;文件已存在则读回条数作为累计基数。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TelemetryError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| TelemetryError(format!("create dir failed: {e}")))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| TelemetryError(format!("open telemetry file failed: {e}")))?;
        let total_exported = count_jsonl_lines(path);
        Ok(Self {
            file: Mutex::new(file),
            path: path.to_path_buf(),
            spans: Mutex::new(Vec::new()),
            written: AtomicUsize::new(0),
            total_exported: AtomicUsize::new(total_exported),
        })
    }

    /// 导出文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 累计已导出条数(含启动时文件已有行)。
    pub fn total_exported(&self) -> usize {
        self.total_exported.load(Ordering::SeqCst)
    }
}

impl Seam for JsonlTelemetryProvider {}

#[async_trait::async_trait]
impl TelemetryProvider for JsonlTelemetryProvider {
    fn record_span(&self, span: Span) -> Result<(), TelemetryError> {
        self.spans.lock().unwrap().push(span);
        Ok(())
    }

    fn spans(&self) -> Vec<Span> {
        self.spans.lock().unwrap().clone()
    }

    async fn export(&self) -> Result<usize, TelemetryError> {
        // 把未导出的 span 逐行追加写入 JSONL 并 flush(真实落盘)。
        let spans = self.spans.lock().unwrap();
        let written = self.written.load(Ordering::SeqCst);
        let count = spans.len().saturating_sub(written);
        if count == 0 {
            return Ok(0);
        }
        let mut file = self.file.lock().unwrap();
        for span in spans.iter().skip(written) {
            let line = serde_json::to_string(span)
                .map_err(|e| TelemetryError(format!("serialize span failed: {e}")))?;
            writeln!(file, "{line}")
                .map_err(|e| TelemetryError(format!("append span failed: {e}")))?;
        }
        file.flush()
            .map_err(|e| TelemetryError(format!("flush failed: {e}")))?;
        self.written.store(spans.len(), Ordering::SeqCst);
        self.total_exported.fetch_add(count, Ordering::SeqCst);
        Ok(count)
    }

    async fn export_otlp(&self, collector_url: &str) -> Result<usize, TelemetryError> {
        // 取尚未导出的 span,构建 OTLP/JSON 载荷(resourceSpans -> scopeSpans -> spans)。
        let spans = self.spans.lock().unwrap();
        let written = self.written.load(Ordering::SeqCst);
        let count = spans.len().saturating_sub(written);
        if count == 0 {
            return Ok(0);
        }
        let otlp_spans: Vec<serde_json::Value> = spans
            .iter()
            .skip(written)
            .map(|span| {
                let start_ns = span.start_ms * 1_000_000;
                let end_ns = (span.start_ms + span.duration_ms) * 1_000_000;
                let mut attrs: Vec<serde_json::Value> = span
                    .attributes
                    .iter()
                    .map(|(k, v)| {
                        serde_json::json!({
                            "key": k,
                            "value": { "stringValue": v.to_string() },
                        })
                    })
                    .collect();
                if let Some(parent) = &span.parent {
                    attrs.push(serde_json::json!({
                        "key": "parent",
                        "value": { "stringValue": parent },
                    }));
                }
                serde_json::json!({
                    "traceId": "00000000000000000000000000000000",
                    "spanId": format!("{:016x}", span.start_ms),
                    "name": span.name,
                    "kind": 2,
                    "startTimeUnixNano": start_ns.to_string(),
                    "endTimeUnixNano": end_ns.to_string(),
                    "attributes": attrs,
                })
            })
            .collect();
        let payload = serde_json::json!({
            "resourceSpans": [{
                "resource": { "attributes": [
                    { "key": "service.name", "value": { "stringValue": "agent-harness" } }
                ]},
                "scopeSpans": [{
                    "scope": { "name": "agent-harness" },
                    "spans": otlp_spans,
                }],
            }]
        });
        let body = serde_json::to_string(&payload)
            .map_err(|e| TelemetryError(format!("serialize otlp: {e}")))?;
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .build();
        let response = agent
            .post(collector_url)
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(|e| TelemetryError(format!("otlp post failed: {e}")))?;
        let status = response.status();
        if !(200..300).contains(&status) {
            return Err(TelemetryError(format!("otlp collector returned {status}")));
        }
        self.written.store(spans.len(), Ordering::SeqCst);
        self.total_exported.fetch_add(count, Ordering::SeqCst);
        Ok(count)
    }
}

/// telemetry 插件:注册 `telemetry` seam,并监听 agent/step 与 tools/post-execute
/// 生成真实 span(监听器在 apply 中注册,返回可逆 Effect)。
pub struct TelemetryPlugin {
    dir: PathBuf,
}

impl TelemetryPlugin {
    /// 以导出目录创建插件(导出文件为 dir/telemetry.jsonl)。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn export_file(&self) -> PathBuf {
        self.dir.join("telemetry.jsonl")
    }
}

impl Plugin for TelemetryPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-telemetry"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TELEMETRY]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: Arc<dyn TelemetryProvider> = Arc::new(
            JsonlTelemetryProvider::open(self.export_file()).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?,
        );
        let mut effects = vec![ctx.register(TELEMETRY, provider.clone())];

        // 当前 agent step 迭代号:tools/post-execute span 的 parent 引用它。
        let step_iteration = Arc::new(AtomicUsize::new(0));

        // 1) agent/step(emit):每次步进记录一条 span(含 workflow/agent 属性)。
        let step_provider = provider.clone();
        let step_counter = step_iteration.clone();
        effects.push(ctx.on::<AgentStep>(move |step| {
            step_counter.store(step.iteration, Ordering::SeqCst);
            let mut attributes = serde_json::Map::new();
            attributes.insert("event".to_string(), json!("agent/step"));
            attributes.insert("component".to_string(), json!("agent-loop"));
            attributes.insert("iteration".to_string(), json!(step.iteration));
            attributes.insert("tool_calls".to_string(), json!(step.tool_calls));
            attributes.insert("done".to_string(), json!(step.done));
            let span = Span {
                name: format!("agent/step#{}", step.iteration),
                parent: Some("agent/run".to_string()),
                attributes,
                start_ms: now_ms(),
                duration_ms: 0,
            };
            if let Err(error) = step_provider.record_span(span) {
                eprintln!("[telemetry] record agent step span failed: {error}");
            }
        }));

        // 2) tools/post-execute(serial):每次工具执行记录一条 span,
        //    parent 用当前 agent step 的 name 字符串。
        let tool_provider = provider.clone();
        let tool_counter = step_iteration.clone();
        effects.push(ctx.on_serial::<ToolExecuted, _, _>(move |event| {
            let tool_provider = tool_provider.clone();
            let tool_counter = tool_counter.clone();
            async move {
                let mut attributes = serde_json::Map::new();
                attributes.insert("event".to_string(), json!("tools/post-execute"));
                attributes.insert("component".to_string(), json!("tools"));
                attributes.insert("tool".to_string(), json!(event.name));
                attributes.insert("elapsed_ms".to_string(), json!(event.elapsed_ms));
                let step = tool_counter.load(Ordering::SeqCst);
                let span = Span {
                    name: format!("tool/{}", event.name),
                    parent: Some(format!("agent/step#{step}")),
                    attributes,
                    start_ms: now_ms().saturating_sub(event.elapsed_ms),
                    duration_ms: event.elapsed_ms,
                };
                if let Err(error) = tool_provider.record_span(span) {
                    eprintln!("[telemetry] record tool span failed: {error}");
                }
            }
        }));

        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TOOLS;
    use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
    use ah_hub::plugin::DynPlugin;
    use async_trait::async_trait;
    use serde_json::Value;

    /// 独立临时目录(按测试名隔离)。
    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ah-telemetry-{tag}-{}", std::process::id()))
    }

    #[test]
    fn record_span_keeps_spans_in_memory() {
        let dir = temp_dir("record");
        let _ = std::fs::remove_dir_all(&dir);
        let provider = JsonlTelemetryProvider::open(dir.join("telemetry.jsonl")).expect("open");

        provider
            .record_span(Span::new("manual/one", 10, 20))
            .expect("record 1");
        let mut attributes = serde_json::Map::new();
        attributes.insert("note".to_string(), json!("manual"));
        provider
            .record_span(Span {
                name: "manual/two".to_string(),
                parent: Some("manual/one".to_string()),
                attributes,
                start_ms: 30,
                duration_ms: 40,
            })
            .expect("record 2");

        let spans = provider.spans();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].name, "manual/one");
        assert_eq!(spans[0].parent, None);
        assert_eq!(spans[1].parent.as_deref(), Some("manual/one"));
        assert_eq!(spans[1].attributes["note"], "manual");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn export_writes_real_jsonl_file_and_reopens() {
        let dir = temp_dir("export");
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("telemetry.jsonl");
        let provider = JsonlTelemetryProvider::open(&file).expect("open");

        provider
            .record_span(Span::new("a", 1, 2))
            .expect("record a");
        let mut attributes = serde_json::Map::new();
        attributes.insert("k".to_string(), json!("v"));
        provider
            .record_span(Span {
                name: "b".to_string(),
                parent: Some("a".to_string()),
                attributes,
                start_ms: 3,
                duration_ms: 4,
            })
            .expect("record b");

        let count = provider.export().await.expect("export");
        assert_eq!(count, 2);
        assert!(file.exists(), "导出后 JSONL 文件真实存在");
        let content = std::fs::read_to_string(&file).expect("read file");
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 2, "JSONL 每行一个 span");

        // 每行可反序列化为 Span,字段往返一致。
        let parsed: Vec<Span> = lines
            .iter()
            .map(|line| serde_json::from_str(line).expect("parse span line"))
            .collect();
        assert_eq!(parsed[0].name, "a");
        assert_eq!(parsed[1].name, "b");
        assert_eq!(parsed[1].parent.as_deref(), Some("a"));
        assert_eq!(parsed[1].attributes["k"], "v");
        assert_eq!(parsed[1].start_ms, 3);
        assert_eq!(parsed[1].duration_ms, 4);

        // 无新增 span 时再次 export 返回 0,文件不重复追加。
        assert_eq!(provider.export().await.expect("re-export"), 0);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 2);

        // 从同一文件重开:读回累计条数;新 span 追加后文件为 3 行。
        let reopened = JsonlTelemetryProvider::open(&file).expect("reopen");
        assert_eq!(reopened.total_exported(), 2, "启动时读回已导出条数");
        assert_eq!(reopened.spans().len(), 0, "内存 span 从空开始(不重复导出)");
        reopened
            .record_span(Span::new("c", 5, 6))
            .expect("record c");
        assert_eq!(reopened.export().await.expect("export c"), 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn plugin_records_spans_from_agent_step_and_tool_executed_events() {
        let dir = temp_dir("events");
        let _ = std::fs::remove_dir_all(&dir);
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TelemetryPlugin::new(&dir));
        let effects = ctx.mount(&plugin).expect("mount telemetry plugin");

        // 触发一次 AgentStep(emit)与一次 ToolExecuted(serial)。
        ctx.emit(AgentStep {
            iteration: 2,
            tool_calls: 1,
            done: false,
        });
        ctx.serial(ToolExecuted {
            name: "read_file".to_string(),
            arguments: json!({ "path": "x.txt" }),
            output: json!({ "ok": true }),
            elapsed_ms: 5,
        })
        .await;

        let provider = ctx
            .service::<dyn TelemetryProvider>(&TELEMETRY)
            .expect("telemetry seam");
        let spans = provider.spans();

        let step = spans
            .iter()
            .find(|s| s.name == "agent/step#2")
            .expect("agent step span");
        assert_eq!(step.parent.as_deref(), Some("agent/run"));
        assert_eq!(step.attributes["component"], "agent-loop");
        assert_eq!(step.attributes["iteration"], 2);
        assert_eq!(step.attributes["done"], false);

        let tool = spans
            .iter()
            .find(|s| s.name == "tool/read_file")
            .expect("tool span");
        assert_eq!(
            tool.parent.as_deref(),
            Some("agent/step#2"),
            "工具 span 挂在当前 agent step 下"
        );
        assert_eq!(tool.attributes["component"], "tools");
        assert_eq!(tool.attributes["tool"], "read_file");
        assert_eq!(tool.attributes["event"], "tools/post-execute");
        assert_eq!(tool.duration_ms, 5);

        // 监听器注册可逆:drop effects 后监听器回滚,不再产生 span。
        drop(effects);
        ctx.emit(AgentStep {
            iteration: 3,
            tool_calls: 0,
            done: true,
        });
        assert_eq!(provider.spans().len(), 2, "卸载后监听器已回滚");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 回显工具:真实工具管线测试用。
    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &'static str {
            "echo"
        }

        fn description(&self) -> &'static str {
            "echo arguments back"
        }

        async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
            Ok(json!({ "ok": true, "echo": arguments }))
        }
    }

    #[tokio::test]
    async fn real_tool_pipeline_generates_exportable_spans() {
        let dir = temp_dir("pipeline");
        let _ = std::fs::remove_dir_all(&dir);
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            std::sync::Arc::new(ah_plugins_tools::ToolsPlugin),
            std::sync::Arc::new(TelemetryPlugin::new(&dir)),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");

        // 真实工具执行管线:invoke → tools/post-execute(serial)→ telemetry span。
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools seam");
        let _tool = registry.register(std::sync::Arc::new(EchoTool));
        registry
            .invoke("echo", json!({ "msg": "hi" }))
            .await
            .expect("invoke");

        let provider = ctx
            .service::<dyn TelemetryProvider>(&TELEMETRY)
            .expect("telemetry seam");
        let tool_spans: Vec<Span> = provider
            .spans()
            .into_iter()
            .filter(|s| s.name == "tool/echo")
            .collect();
        assert_eq!(tool_spans.len(), 1, "真实工具执行产生一条 tool span");
        assert_eq!(tool_spans[0].attributes["tool"], "echo");
        assert_eq!(tool_spans[0].attributes["event"], "tools/post-execute");

        // 真实导出:文件存在且可重读。
        let count = provider.export().await.expect("export");
        assert!(count >= 1);
        let file = dir.join("telemetry.jsonl");
        assert!(file.exists(), "真实导出文件存在");
        let lines: Vec<Span> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("parse"))
            .collect();
        assert!(lines.iter().any(|s| s.name == "tool/echo"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn export_otlp_posts_valid_json_to_collector() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // 真实本地 OTLP collector 端点(捕获 POST 体)。
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let url = format!("http://{addr}/v1/traces");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).into_owned();
                let header_end = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(text.len());
                // 解析 Content-Length,续读剩余字节(避免分包竞态)。
                let content_length = text
                    .lines()
                    .find_map(|line| {
                        let lower = line.to_lowercase();
                        lower
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while buf.len() < header_end + content_length {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let body = String::from_utf8_lossy(&buf[header_end..]).into_owned();
                let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{{}}", 2);
                let _ = stream.write_all(response.as_bytes());
                std::fs::write("/tmp/otlp_captured.json", body).expect("capture");
            }
        });

        let dir = std::env::temp_dir().join(format!("ah-tel-otlp-{}", std::process::id()));
        let provider = JsonlTelemetryProvider::open(dir.join("telemetry.jsonl")).expect("open");
        provider
            .record_span(Span::new("agent/step#1", 1000, 5))
            .expect("record");
        let mut attrs = serde_json::Map::new();
        attrs.insert("tool".to_string(), serde_json::json!("echo"));
        provider
            .record_span(Span {
                name: "tool/echo".to_string(),
                parent: Some("agent/step#1".to_string()),
                attributes: attrs,
                start_ms: 2000,
                duration_ms: 10,
            })
            .expect("record2");

        let count = provider.export_otlp(&url).await.expect("otlp export");
        assert_eq!(count, 2, "two spans exported");

        handle.join().expect("collector");
        let captured = std::fs::read_to_string("/tmp/otlp_captured.json").expect("read capture");
        let payload: serde_json::Value = serde_json::from_str(&captured).expect("valid otlp json");
        assert!(
            payload.get("resourceSpans").is_some(),
            "OTLP/JSON structure"
        );
        let spans = &payload["resourceSpans"][0]["scopeSpans"][0]["spans"];
        assert_eq!(spans.as_array().map(|a| a.len()).unwrap_or(0), 2);
        assert_eq!(spans[0]["name"], "agent/step#1");
        assert!(
            spans[1]["endTimeUnixNano"].as_str().unwrap().len() >= 10,
            "nanos timestamp"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file("/tmp/otlp_captured.json");
    }
}
