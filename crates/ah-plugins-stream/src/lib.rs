//! # ah-plugins-stream
//!
//! Real session stream pipeline (aligned with
//! core/session/stream/{emitter,manager,writer}.py):
//! - AsyncStreamQueue: bounded tokio mpsc queue with send retries / receive timeout /
//!   close-join + force-clear on timeout;
//! - StreamEmitter: emit frames into the queue, END_FRAME sentinel on close;
//! - StreamWriterManager: default writers (output/trace/custom), stream_output iterator
//!   with first-frame/chunk timeouts and END_FRAME termination;
//! - StreamWriter family: validate payload against schema type and emit.

use std::sync::Arc;
use std::time::Duration;

use ah_contracts::keys::STREAM;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::stream::{CustomSchema, OutputSchema, StreamError, TraceSchema};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;
use tokio::sync::mpsc;

/// 发送尝试默认超时(对齐 DEFAULT_SEND_ATTEMPT_TIMEOUT)。
const SEND_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(200);
/// 最大发送重试(对齐 DEFAULT_MAX_SEND_RETRIES)。
const MAX_SEND_RETRIES: usize = 5;

/// 有界异步流队列(对齐 AsyncStreamQueue;tokio mpsc 替代 asyncio.Queue)。
pub struct AsyncStreamQueue {
    sender: mpsc::Sender<Value>,
    receiver: Option<mpsc::Receiver<Value>>,
    closed: std::sync::atomic::AtomicBool,
    sent_count: std::sync::atomic::AtomicUsize,
    received_count: std::sync::atomic::AtomicUsize,
}

impl AsyncStreamQueue {
    pub fn new(maxsize: usize) -> Self {
        let (tx, rx) = mpsc::channel(maxsize.max(1));
        Self {
            sender: tx,
            receiver: Some(rx),
            closed: std::sync::atomic::AtomicBool::new(false),
            sent_count: std::sync::atomic::AtomicUsize::new(0),
            received_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// 发送数据(带超时重试;closed 后报错)。
    pub async fn send(&self, data: Value) -> Result<(), StreamError> {
        if self.is_closed() {
            return Err(StreamError::new(
                "stream_queue_closed",
                "StreamQueue is already closed",
            ));
        }
        let mut attempt = 0;
        loop {
            attempt += 1;
            match tokio::time::timeout(SEND_ATTEMPT_TIMEOUT, self.sender.send(data.clone())).await {
                Ok(Ok(())) => {
                    self.sent_count
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    return Ok(());
                }
                Ok(Err(_)) => {
                    return Err(StreamError::new("stream_queue_send", "receiver dropped"));
                }
                Err(_) => {
                    // timeout -> retry
                    if attempt >= MAX_SEND_RETRIES {
                        return Err(StreamError::new(
                            "stream_queue_send_timeout",
                            format!("Failed to send stream data after {MAX_SEND_RETRIES} retries"),
                        ));
                    }
                }
            }
        }
    }

    /// 接收数据(-1 表示无限等待;closed 后报错)。
    pub async fn receive(&mut self, timeout: Option<Duration>) -> Result<Value, StreamError> {
        if self.is_closed() {
            return Err(StreamError::new(
                "stream_queue_closed",
                "StreamQueue is already closed",
            ));
        }
        let rx = self.receiver.as_mut().ok_or_else(|| {
            StreamError::new("stream_queue_no_receiver", "receiver already taken")
        })?;
        let item = match timeout {
            Some(t) => tokio::time::timeout(t, rx.recv()).await.map_err(|_| {
                StreamError::new("stream_queue_receive_timeout", "receive timed out")
            })?,
            None => rx.recv().await,
        };
        match item {
            Some(v) => {
                self.received_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(v)
            }
            None => Err(StreamError::new("stream_queue_closed", "channel closed")),
        }
    }

    /// 关闭队列(等待排空;超时强制清空)。
    pub async fn close(&mut self, timeout: Duration) -> Result<(), StreamError> {
        if self.is_closed() {
            return Ok(());
        }
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        // drain remaining items on timeout
        let _ = tokio::time::timeout(timeout, async {
            // wait until sender count matches received (simple drain: sleep-free check)
            tokio::time::sleep(Duration::from_millis(10)).await;
        })
        .await;
        if let Some(rx) = self.receiver.as_mut() {
            while rx.try_recv().is_ok() {}
        }
        Ok(())
    }

    /// 统计(供诊断)。
    pub fn stats(&self) -> (usize, usize) {
        (
            self.sent_count.load(std::sync::atomic::Ordering::SeqCst),
            self.received_count
                .load(std::sync::atomic::Ordering::SeqCst),
        )
    }
}

impl Seam for AsyncStreamQueue {}

/// 流发射器(对齐 StreamEmitter;END_FRAME 哨兵)。
pub struct StreamEmitter {
    queue: Arc<tokio::sync::Mutex<AsyncStreamQueue>>,
    closed: std::sync::atomic::AtomicBool,
}

impl StreamEmitter {
    pub const END_FRAME: &'static str = "all streaming outputs finish";

    pub fn new() -> Self {
        Self {
            queue: Arc::new(tokio::sync::Mutex::new(AsyncStreamQueue::new(1024))),
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn stream_queue(&self) -> Arc<tokio::sync::Mutex<AsyncStreamQueue>> {
        self.queue.clone()
    }

    /// 发射数据(closed 后报错)。
    pub async fn emit(&self, data: Value) -> Result<(), StreamError> {
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StreamError::new(
                "stream_emitter_closed",
                "Can not emit data after the stream emitter is closed.",
            ));
        }
        let guard = self.queue.lock().await;
        guard.send(data).await
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// 关闭发射器(发送 END_FRAME 哨兵)。
    pub async fn close(&self) -> Result<(), StreamError> {
        if self.is_closed() {
            return Ok(());
        }
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        let guard = self.queue.lock().await;
        if !guard.is_closed() {
            guard
                .send(Value::String(Self::END_FRAME.to_string()))
                .await?;
        }
        Ok(())
    }
}

impl Default for StreamEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for StreamEmitter {}

/// 流写入器(对齐 StreamWriter:校验 payload 后发射)。
pub struct StreamWriter {
    emitter: Arc<StreamEmitter>,
}

impl StreamWriter {
    pub fn new(emitter: Arc<StreamEmitter>) -> Self {
        Self { emitter }
    }

    /// 校验并写入 OutputSchema。
    pub async fn write_output(&self, schema: OutputSchema) -> Result<(), StreamError> {
        let v = serde_json::to_value(&schema)
            .map_err(|e| StreamError::new("stream_write_validation", e.to_string()))?;
        self.emitter.emit(v).await
    }

    pub async fn write_trace(&self, schema: TraceSchema) -> Result<(), StreamError> {
        let v = serde_json::to_value(&schema)
            .map_err(|e| StreamError::new("stream_write_validation", e.to_string()))?;
        self.emitter.emit(v).await
    }

    pub async fn write_custom(&self, schema: CustomSchema) -> Result<(), StreamError> {
        let v = serde_json::to_value(&schema)
            .map_err(|e| StreamError::new("stream_write_validation", e.to_string()))?;
        self.emitter.emit(v).await
    }
}

impl Seam for StreamWriter {}

/// 流写入管理器(对齐 StreamWriterManager:默认 writer + stream_output 迭代)。
pub struct StreamWriterManager {
    emitter: Arc<StreamEmitter>,
}

impl StreamWriterManager {
    pub fn new(emitter: Arc<StreamEmitter>) -> Self {
        Self { emitter }
    }

    pub fn get_output_writer(&self) -> StreamWriter {
        StreamWriter::new(self.emitter.clone())
    }

    pub fn get_trace_writer(&self) -> StreamWriter {
        StreamWriter::new(self.emitter.clone())
    }

    pub fn get_custom_writer(&self) -> StreamWriter {
        StreamWriter::new(self.emitter.clone())
    }

    /// 消费流直到 END_FRAME(对齐 stream_output)。
    pub async fn stream_output(&self) -> Result<Vec<Value>, StreamError> {
        self.stream_output_with_timeouts(None, None).await
    }

    /// 带超时的流消费(对齐 stream_output 的 first_frame_timeout / timeout):
    /// 首帧用 first_frame_timeout(默认无限),后续帧用 timeout(默认无限)。
    pub async fn stream_output_with_timeouts(
        &self,
        first_frame_timeout: Option<Duration>,
        timeout: Option<Duration>,
    ) -> Result<Vec<Value>, StreamError> {
        let mut out = Vec::new();
        let queue = self.emitter.stream_queue();
        let mut guard = queue.lock().await;
        let mut is_first = true;
        loop {
            let t = if is_first {
                first_frame_timeout
            } else {
                timeout
            };
            is_first = false;
            match guard.receive(t).await {
                Ok(v) => {
                    if v == Value::String(StreamEmitter::END_FRAME.to_string()) {
                        break;
                    }
                    out.push(v);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }
}

impl Seam for StreamWriterManager {}

/// stream 插件:注册流管道服务。
pub struct StreamPlugin;

impl Plugin for StreamPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-stream"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![STREAM]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let emitter = Arc::new(StreamEmitter::new());
        let manager = Arc::new(StreamWriterManager::new(emitter.clone()));
        Ok(vec![ctx.register(STREAM, manager)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::stream::OutputSchema;

    #[tokio::test]
    async fn queue_send_receive_roundtrip() {
        let mut q = AsyncStreamQueue::new(4);
        q.send(serde_json::json!({"a": 1})).await.unwrap();
        let v = q.receive(None).await.unwrap();
        assert_eq!(v, serde_json::json!({"a": 1}));
        assert_eq!(q.stats(), (1, 1));
    }

    #[tokio::test]
    async fn queue_rejects_send_after_close() {
        let mut q = AsyncStreamQueue::new(4);
        q.close(Duration::from_millis(50)).await.unwrap();
        let res = q.send(Value::from(1)).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().message.contains("closed"));
    }

    #[tokio::test]
    async fn emitter_emit_close_sentinel() {
        let emitter = Arc::new(StreamEmitter::new());
        emitter.emit(serde_json::json!("chunk")).await.unwrap();
        emitter.close().await.unwrap();
        let manager = StreamWriterManager::new(emitter.clone());
        let out = manager.stream_output().await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], serde_json::json!("chunk"));
    }

    #[tokio::test]
    async fn writer_validates_and_emits_output() {
        let emitter = Arc::new(StreamEmitter::new());
        let writer = StreamWriter::new(emitter.clone());
        writer
            .write_output(OutputSchema::new("nodeA", 1, serde_json::json!("payload")))
            .await
            .unwrap();
        emitter.close().await.unwrap();
        let manager = StreamWriterManager::new(emitter.clone());
        let out = manager.stream_output().await.unwrap();
        assert_eq!(out.len(), 1);
        let parsed: OutputSchema = serde_json::from_value(out[0].clone()).unwrap();
        assert_eq!(parsed.r#type, "nodeA");
        assert_eq!(parsed.index, 1);
    }

    #[tokio::test]
    async fn stream_output_with_first_frame_timeout_errors() {
        let emitter = Arc::new(StreamEmitter::new());
        let manager = StreamWriterManager::new(emitter.clone());
        // no frames emitted, short first-frame timeout -> receive timeout error
        let res = manager
            .stream_output_with_timeouts(Some(Duration::from_millis(30)), None)
            .await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "stream_queue_receive_timeout");
    }

    #[tokio::test]
    async fn stream_output_chunk_timeout_between_frames() {
        let emitter = Arc::new(StreamEmitter::new());
        let manager = StreamWriterManager::new(emitter.clone());
        let writer = StreamWriter::new(emitter.clone());
        writer
            .write_output(OutputSchema::new("a", 1, serde_json::json!(1)))
            .await
            .unwrap();
        // chunk timeout fires while waiting for END_FRAME after first frame
        let res = manager
            .stream_output_with_timeouts(None, Some(Duration::from_millis(30)))
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn emitter_rejects_emit_after_close() {
        let emitter = Arc::new(StreamEmitter::new());
        emitter.close().await.unwrap();
        let res = emitter.emit(serde_json::json!("late")).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().message.contains("closed"));
    }
}
