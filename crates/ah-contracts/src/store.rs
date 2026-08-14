//! store seam:通用存储基类(KV / message)。
//!
//! 对应 openjiuwen/core 的 BaseKVStore / BaseMessageStore。
//! 契约零实现;真实后端由插件提供(ah-plugins-store 已提供本地文件后端,
//! 外部 Redis/GaussDB/ES 后端留待后续,文档注明)。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// store 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError(pub String);

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

/// 一条 KV 条目。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KvEntry {
    pub key: String,
    pub value: Value,
    pub updated_ms: u64,
}

/// BaseKVStore(Service Definition):通用键值存储。
pub trait BaseKVStore: Seam {
    /// 取回 key 的值;不存在返回 None。
    fn get(&self, key: &str) -> Result<Option<Value>, StoreError>;

    /// 写入 key(覆盖)。
    fn set(&self, key: &str, value: Value) -> Result<(), StoreError>;

    /// 删除 key;不存在不报错。
    fn delete(&self, key: &str) -> Result<(), StoreError>;

    /// 按前缀扫描全部条目(按 key 排序)。
    fn scan(&self, prefix: &str) -> Result<Vec<KvEntry>, StoreError>;
}

/// 一条已存储消息。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StoredMessage {
    pub channel: String,
    /// 单调递增序号(append-only)。
    pub seq: u64,
    pub payload: Value,
    pub ts_ms: u64,
}

/// BaseMessageStore(Service Definition):按 channel 的 append-only 消息存储。
pub trait BaseMessageStore: Seam {
    /// 追加一条消息,返回带序号的完整记录。
    fn append(&self, channel: &str, payload: Value) -> Result<StoredMessage, StoreError>;

    /// 读取 channel 中 seq 之后的消息(含 after_seq 自身;after_seq=0 读全部)。
    fn read(&self, channel: &str, after_seq: u64) -> Result<Vec<StoredMessage>, StoreError>;

    /// 已存在消息的 channel 列表。
    fn channels(&self) -> Result<Vec<String>, StoreError>;
}

/// store 基类组合:一个实现可同时提供 KV 与 message 两种后端。
#[async_trait]
pub trait StoreProvider: BaseKVStore + BaseMessageStore + Seam {}
