//! # ah-plugins-prompt-attachment
//!
//! 真实 prompt 附件核心(对齐 openjiuwen/harness/prompts/prompt_attachment_manager.py
//! 的纯函数部分):自实现 sha256、canonical JSON 语义哈希、安全 id 净化。
//! 无外部 hash crate;sha256 为标准 FIPS 180-4 实现(约 100 行纯函数,无 IO、无状态)。
//!
//! 注册服务键 PROMPT_ATTACHMENT("prompt-attachment"),实现
//! ah_contracts::prompt_attachment::PromptAttachmentApi。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ah_contracts::keys::{PROMPT_ATTACHMENT, PROMPT_ATTACHMENT_STORE};
use ah_contracts::prelude::Effect;
use ah_contracts::prompt_attachment::{
    AttachmentError, AttachmentFilter, PromptAttachment, PromptAttachmentApi, PromptAttachmentKind,
    PromptAttachmentStore, PromptAttachmentUpdate, stable_sort_key,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// SHA-256 轮常量 K[0..63](FIPS 180-4 §4.2.2)。
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 初始哈希值 H[0..7](FIPS 180-4 §5.3.3)。
const SHA256_H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// 标准 SHA-256(自实现,FIPS 180-4):任意字节序列 -> 64 位小写 hex。
///
/// 步骤:1) padding(0x80 + 零填充至 ≡56 mod 64 + 64 位大端比特长度);
/// 2) 逐 64 字节块展开消息调度表并做 64 轮压缩;3) 大端输出 8 个 32 位字。
pub fn sha256_hex(data: &[u8]) -> String {
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut h = SHA256_H0;
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(big_s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = big_s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
}
/// 递归键排序:返回键有序的 Value 副本(嵌套对象同样排序)。
fn sort_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut sorted = serde_json::Map::new();
            for (k, v) in entries {
                sorted.insert(k, sort_value(v));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_value).collect()),
        other => other,
    }
}

/// canonical JSON:递归键排序 + 紧凑分隔符(无空格)+ 非 ASCII 原样输出。
///
/// 对齐 Python json.dumps(value, ensure_ascii=False, sort_keys=True,
/// separators=(",", ":")):serde_json 紧凑序列化且默认不转义非 ASCII。
fn canonical_json(value: Value) -> String {
    serde_json::to_string(&sort_value(value)).expect("JSON value always serializes")
}

/// 真实 prompt 附件核心实现(纯函数,无状态)。
pub struct PromptAttachmentService;

impl Seam for PromptAttachmentService {}

impl PromptAttachmentApi for PromptAttachmentService {
    /// sha256((content or "").encode("utf-8"));None 视为空串。
    fn content_sha256(&self, content: Option<&str>) -> String {
        sha256_hex(content.unwrap_or("").as_bytes())
    }

    /// sha256(rendered.encode("utf-8"))。
    fn hash_rendered(&self, rendered: &str) -> String {
        sha256_hex(rendered.as_bytes())
    }

    /// 语义哈希:序列化为 JSON → 排除 content_sha256/created_at/updated_at →
    /// canonical JSON → sha256(对齐 hash_prompt_attachment)。
    fn hash_attachment(&self, attachment: &PromptAttachment) -> String {
        let mut value =
            serde_json::to_value(attachment).expect("PromptAttachment is always serializable");
        if let Value::Object(ref mut map) = value {
            map.remove("content_sha256");
            map.remove("created_at");
            map.remove("updated_at");
        }
        sha256_hex(canonical_json(value).as_bytes())
    }

    /// 安全 id 片段(对齐 _safe_id_part):
    /// raw = value.trim();空 → fallback;
    /// 非 [A-Za-z0-9_.-] 连续字符 → 单个 '_';strip 两端 '._-';
    /// 非空 → 截断 80;全非法 → sha256(raw) 前 12 位。
    fn safe_id_part(&self, value: Option<&str>, fallback: &str) -> String {
        let raw = value.unwrap_or("").trim();
        if raw.is_empty() {
            return fallback.to_string();
        }
        let mut safe = String::with_capacity(raw.len());
        let mut pending_underscore = false;
        for ch in raw.chars() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
                if pending_underscore {
                    safe.push('_');
                    pending_underscore = false;
                }
                safe.push(ch);
            } else {
                pending_underscore = true;
            }
        }
        let trimmed = safe.trim_matches(|c| matches!(c, '.' | '_' | '-'));
        if trimmed.is_empty() {
            return sha256_hex(raw.as_bytes())[..12].to_string();
        }
        trimmed.chars().take(80).collect()
    }
}

/// 内存 prompt 附件管理(对齐 Python `PromptAttachmentManager` 确定性 CRUD)。
pub struct InMemoryPromptAttachmentStore {
    items: Mutex<HashMap<String, HashMap<String, PromptAttachment>>>,
    api: PromptAttachmentService,
}

impl InMemoryPromptAttachmentStore {
    pub fn new() -> Self {
        Self {
            items: Mutex::new(HashMap::new()),
            api: PromptAttachmentService,
        }
    }

    /// 对齐 `_make_section_id`:session.{safe_id(session)}.{section_value}。
    fn make_section_id(&self, session_id: &str, section: &str) -> String {
        let safe_session = self.api.safe_id_part(Some(session_id), "session");
        let safe_section = self.api.safe_id_part(Some(section), "section");
        format!("session.{safe_session}.{safe_section}")
    }

    /// 对齐 `_section_value`:safe_id_part(str(value), fallback="section")。
    fn section_value(&self, section: &str) -> String {
        self.api.safe_id_part(Some(section), "section")
    }

    /// 对齐 `_normalize_for_write`:时间戳/内容哈希/metadata.section 注入。
    fn normalize_for_write(
        &self,
        mut attachment: PromptAttachment,
        is_new: bool,
    ) -> Result<PromptAttachment, AttachmentError> {
        let now = utc_iso_now();
        if attachment.session_id.is_empty() {
            return Err(AttachmentError(
                "prompt attachment requires session_id".to_string(),
            ));
        }
        if attachment.section.is_empty() {
            return Err(AttachmentError(
                "prompt attachment requires section".to_string(),
            ));
        }
        if is_new || attachment.created_at.is_none() {
            attachment.created_at = Some(now.clone());
        }
        attachment.updated_at = Some(now);
        attachment.content_sha256 = Some(self.api.content_sha256(attachment.content.as_deref()));
        let mut metadata = attachment.metadata.clone();
        metadata.insert(
            "section".to_string(),
            Value::String(attachment.section.clone()),
        );
        if let Some(source) = &attachment.source {
            metadata
                .entry("source".to_string())
                .or_insert_with(|| Value::String(source.clone()));
        }
        attachment.metadata = metadata;
        Ok(attachment)
    }

    /// 对齐 `_find_location_by_id_unlocked` / `_find_by_id`(带 session 约束)。
    fn find_by_id(
        &self,
        prompt_attachment_id: &str,
        session_id: Option<&str>,
    ) -> Option<(String, String)> {
        let items = self.items.lock().unwrap();
        for (sid, bucket) in items.iter() {
            if let Some(expected) = session_id
                && sid != expected
            {
                continue;
            }
            for (section, item) in bucket.iter() {
                if item.id == prompt_attachment_id {
                    return Some((sid.clone(), section.clone()));
                }
            }
        }
        None
    }
}

impl Default for InMemoryPromptAttachmentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for InMemoryPromptAttachmentStore {}

impl PromptAttachmentStore for InMemoryPromptAttachmentStore {
    fn add_section(
        &self,
        session_id: &str,
        section: &str,
        content: &str,
        kind: PromptAttachmentKind,
        source: &str,
        priority: i32,
        metadata: Option<&serde_json::Map<String, serde_json::Value>>,
        content_kind: &str,
        expires_at: Option<&str>,
    ) -> Result<PromptAttachment, AttachmentError> {
        let section_id = self.section_value(section);
        let mut merged = metadata.cloned().unwrap_or_default();
        merged.insert("section".to_string(), Value::String(section_id.clone()));
        merged.insert("source".to_string(), Value::String(source.to_string()));
        let item = PromptAttachment {
            id: self.make_section_id(session_id, &section_id),
            section: section_id.clone(),
            kind,
            content: Some(content.to_string()),
            priority,
            source: Some(source.to_string()),
            session_id: session_id.to_string(),
            created_at: None,
            updated_at: None,
            expires_at: expires_at.map(str::to_string),
            metadata: merged,
            content_kind: content_kind.to_string(),
            content_path: None,
            content_sha256: None,
        };
        let mut items = self.items.lock().unwrap();
        let existing = items.get(session_id).and_then(|b| b.get(&section_id));
        let is_new = existing.is_none();
        let normalized = self.normalize_for_write(item, is_new)?;
        items
            .entry(session_id.to_string())
            .or_default()
            .insert(section_id, normalized.clone());
        Ok(normalized)
    }

    fn clear_section(&self, session_id: &str, section: &str) -> usize {
        let section_id = self.section_value(section);
        let mut items = self.items.lock().unwrap();
        let Some(bucket) = items.get_mut(session_id) else {
            return 0;
        };
        if bucket.remove(&section_id).is_none() {
            return 0;
        }
        if bucket.is_empty() {
            items.remove(session_id);
        }
        1
    }

    fn get_by_id(
        &self,
        prompt_attachment_id: &str,
        session_id: Option<&str>,
    ) -> Option<PromptAttachment> {
        self.find_by_id(prompt_attachment_id, session_id)
            .map(|(sid, section)| self.items.lock().unwrap()[&sid][&section].clone())
    }

    fn update_by_id(
        &self,
        prompt_attachment_id: &str,
        update: &PromptAttachmentUpdate,
    ) -> Result<PromptAttachment, AttachmentError> {
        let mut items = self.items.lock().unwrap();
        let mut location = None;
        for (sid, bucket) in items.iter() {
            for (section, item) in bucket.iter() {
                if item.id == prompt_attachment_id {
                    location = Some((sid.clone(), section.clone()));
                }
            }
        }
        let Some((sid, section)) = location else {
            return Err(AttachmentError(format!(
                "prompt attachment not found: {prompt_attachment_id}"
            )));
        };
        let current = items[&sid][&section].clone();
        let mut data = serde_json::to_value(&current).expect("attachment serializes");
        let obj = data.as_object_mut().expect("object");
        if let Some(kind) = update.kind {
            obj.insert("kind".to_string(), serde_json::to_value(kind).unwrap());
        }
        if let Some(content) = &update.content {
            obj.insert("content".to_string(), Value::String(content.clone()));
        }
        if let Some(priority) = update.priority {
            obj.insert("priority".to_string(), Value::from(priority));
        }
        if let Some(source) = &update.source {
            obj.insert("source".to_string(), Value::String(source.clone()));
        }
        if let Some(expires_at) = &update.expires_at {
            obj.insert("expires_at".to_string(), Value::String(expires_at.clone()));
        }
        if let Some(metadata) = &update.metadata {
            obj.insert("metadata".to_string(), Value::Object(metadata.clone()));
        }
        if let Some(content_kind) = &update.content_kind {
            obj.insert(
                "content_kind".to_string(),
                Value::String(content_kind.clone()),
            );
        }
        // 不可变字段回写。
        obj.insert("id".to_string(), Value::String(current.id.clone()));
        obj.insert(
            "section".to_string(),
            Value::String(current.section.clone()),
        );
        obj.insert(
            "session_id".to_string(),
            Value::String(current.session_id.clone()),
        );
        obj.insert(
            "created_at".to_string(),
            current
                .created_at
                .as_ref()
                .map(|s| Value::String(s.clone()))
                .unwrap_or(Value::Null),
        );
        let decoded: PromptAttachment = serde_json::from_value(data)
            .map_err(|e| AttachmentError(format!("invalid attachment update: {e}")))?;
        let updated = self.normalize_for_write(decoded, false)?;
        items
            .get_mut(&sid)
            .unwrap()
            .insert(section, updated.clone());
        Ok(updated)
    }

    fn remove_by_id(&self, prompt_attachment_id: &str, session_id: Option<&str>) -> bool {
        let Some((sid, section)) = self.find_by_id(prompt_attachment_id, session_id) else {
            return false;
        };
        let mut items = self.items.lock().unwrap();
        let bucket = items.get_mut(&sid).unwrap();
        bucket.remove(&section);
        if bucket.is_empty() {
            items.remove(&sid);
        }
        true
    }

    fn list_by_filter(&self, filter: &AttachmentFilter) -> Vec<PromptAttachment> {
        let mut items: Vec<PromptAttachment> = self
            .items
            .lock()
            .unwrap()
            .iter()
            .filter(|(sid, _)| {
                filter
                    .session_id
                    .as_ref()
                    .map(|s| s == *sid)
                    .unwrap_or(true)
            })
            .flat_map(|(_, bucket)| bucket.values().cloned())
            .filter(|item| {
                filter
                    .section
                    .as_ref()
                    .map(|s| item.section == *s)
                    .unwrap_or(true)
                    && filter.kind.map(|k| k == item.kind).unwrap_or(true)
                    && filter
                        .source
                        .as_ref()
                        .map(|s| item.source.as_deref() == Some(s.as_str()))
                        .unwrap_or(true)
            })
            .collect();
        items.sort_by_key(stable_sort_key);
        items
    }

    fn remove_by_filter(
        &self,
        filter: &AttachmentFilter,
        allow_all: bool,
    ) -> Result<usize, AttachmentError> {
        let has_filter = filter.session_id.is_some()
            || filter.section.is_some()
            || filter.kind.is_some()
            || filter.source.is_some();
        if !has_filter && !allow_all {
            return Err(AttachmentError(
                "destructive prompt attachment operation requires at least one filter".to_string(),
            ));
        }
        let targets = self.list_by_filter(filter);
        let mut count = 0;
        for item in targets {
            if self.remove_by_id(&item.id, Some(&item.session_id)) {
                count += 1;
            }
        }
        Ok(count)
    }

    fn clear_session(&self, session_id: &str) -> usize {
        let mut items = self.items.lock().unwrap();
        items.remove(session_id).map(|b| b.len()).unwrap_or(0)
    }

    fn clear_all(&self) -> usize {
        let mut items = self.items.lock().unwrap();
        let count: usize = items.values().map(|b| b.len()).sum();
        items.clear();
        count
    }

    fn collect_for_session(&self, session_id: &str) -> Vec<PromptAttachment> {
        let now = utc_iso_now();
        let mut items = self.items.lock().unwrap();
        let mut expired: Vec<String> = Vec::new();
        let mut result: Vec<PromptAttachment> = Vec::new();
        if let Some(bucket) = items.get_mut(session_id) {
            let mut keep: HashMap<String, PromptAttachment> = HashMap::new();
            for (section, item) in bucket.drain() {
                if ah_contracts::prompt_attachment::is_expired(&item, &now) {
                    expired.push(section);
                } else {
                    keep.insert(section, item);
                }
            }
            *bucket = keep;
            result.extend(bucket.values().cloned());
        }
        // 空会话桶清理。
        if let Some(bucket) = items.get(session_id)
            && bucket.is_empty()
        {
            items.remove(session_id);
        }
        let _ = expired;
        result.sort_by_key(stable_sort_key);
        result
    }
    fn update_content_by_id(
        &self,
        prompt_attachment_id: &str,
        content: Option<&str>,
        session_id: Option<&str>,
        content_kind: Option<&str>,
    ) -> Result<PromptAttachment, AttachmentError> {
        let Some((sid, section)) = self.find_by_id(prompt_attachment_id, session_id) else {
            return Err(AttachmentError(format!(
                "prompt attachment not found: {prompt_attachment_id}"
            )));
        };
        let mut items = self.items.lock().unwrap();
        let current = items[&sid][&section].clone();
        let mut updated = current.clone();
        updated.content = content.map(str::to_string);
        if let Some(content_kind) = content_kind {
            updated.content_kind = content_kind.to_string();
        }
        let updated = self.normalize_for_write(updated, false)?;
        items
            .get_mut(&sid)
            .unwrap()
            .insert(section, updated.clone());
        Ok(updated)
    }

    fn update_metadata_by_id(
        &self,
        prompt_attachment_id: &str,
        metadata: &serde_json::Map<String, serde_json::Value>,
        session_id: Option<&str>,
        merge: bool,
    ) -> Result<PromptAttachment, AttachmentError> {
        let Some(current) = self.get_by_id(prompt_attachment_id, session_id) else {
            return Err(AttachmentError(format!(
                "prompt attachment not found: {prompt_attachment_id}"
            )));
        };
        let next = if merge {
            let mut merged = current.metadata.clone();
            merged.extend(metadata.clone());
            merged
        } else {
            metadata.clone()
        };
        self.update_by_id(
            prompt_attachment_id,
            &PromptAttachmentUpdate {
                kind: None,
                content: None,
                priority: None,
                source: None,
                expires_at: None,
                metadata: Some(next),
                content_kind: None,
            },
        )
    }

    fn replace_source(
        &self,
        source: &str,
        attachments: &[PromptAttachment],
        session_id: Option<&str>,
    ) -> Result<Vec<PromptAttachment>, AttachmentError> {
        let filter = AttachmentFilter {
            session_id: session_id.map(str::to_string),
            source: Some(source.to_string()),
            ..Default::default()
        };
        self.remove_by_filter(&filter, false)?;
        attachments
            .iter()
            .map(|item| {
                let target_session = session_id.unwrap_or(&item.session_id);
                self.add_section(
                    target_session,
                    &item.section,
                    item.content.as_deref().unwrap_or_default(),
                    item.kind,
                    source,
                    item.priority,
                    Some(&item.metadata),
                    &item.content_kind,
                    item.expires_at.as_deref(),
                )
            })
            .collect()
    }

    fn clear_source(&self, source: &str, session_id: Option<&str>) -> usize {
        let filter = AttachmentFilter {
            session_id: session_id.map(str::to_string),
            source: Some(source.to_string()),
            ..Default::default()
        };
        self.remove_by_filter(&filter, false).unwrap_or(0)
    }

    fn add_file_reference(
        &self,
        file_path: &str,
        summary: Option<&str>,
        session_id: &str,
        section: Option<&str>,
        source: Option<&str>,
        priority: i32,
        metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Result<PromptAttachment, AttachmentError> {
        let mut merged = metadata.cloned().unwrap_or_default();
        merged.insert("file_path".into(), Value::String(file_path.into()));
        let fallback_section = format!("file_{}", self.api.safe_id_part(Some(file_path), "file"));
        let fallback_content = format!("File reference: {file_path}");
        self.add_section(
            session_id,
            section.unwrap_or(&fallback_section),
            summary.unwrap_or(&fallback_content),
            PromptAttachmentKind::File,
            source.unwrap_or("file_reference"),
            priority,
            Some(&merged),
            "text/markdown",
            None,
        )
    }
}

/// UTC 时间(ISO-8601 近似,与 Python `datetime.now(timezone.utc).isoformat()`
/// 同为 UTC 时间戳)。
fn utc_iso_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    let secs = (now / 1_000_000) as i64;
    let micros = (now % 1_000_000) as u32;
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{micros:06}+00:00")
}

/// days(自 1970-01-01)→ (year, month, day)(Howard Hinnant 算法)。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as i64;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as i64;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// prompt-attachment 插件:注册 prompt-attachment seam + prompt-attachment-store seam。
pub struct PromptAttachmentPlugin;

impl Plugin for PromptAttachmentPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-prompt-attachment"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![PROMPT_ATTACHMENT, PROMPT_ATTACHMENT_STORE]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let service: Arc<dyn PromptAttachmentApi> = Arc::new(PromptAttachmentService);
        let store: Arc<dyn PromptAttachmentStore> = Arc::new(InMemoryPromptAttachmentStore::new());
        Ok(vec![
            ctx.register(PROMPT_ATTACHMENT, service),
            ctx.register(PROMPT_ATTACHMENT_STORE, store),
        ])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::PROMPT_ATTACHMENT;
    use ah_contracts::prompt_attachment::{
        AttachmentError, PromptAttachment, PromptAttachmentApi, PromptAttachmentKind,
        PromptAttachmentUpdate,
    };
    use ah_hub::context::Context;
    use ah_hub::plugin::DynPlugin;
    use serde_json::{Map, Value, json};

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// 构造一个稳定的测试附件(Text / priority 50 / source "test")。
    fn attachment(id: &str, content: Option<&str>) -> PromptAttachment {
        PromptAttachment {
            id: id.to_string(),
            section: "mem".to_string(),
            kind: PromptAttachmentKind::Text,
            content: content.map(str::to_string),
            priority: 50,
            source: Some("test".to_string()),
            session_id: "sid".to_string(),
            created_at: Some("t0".to_string()),
            updated_at: Some("t1".to_string()),
            expires_at: None,
            metadata: Map::new(),
            content_kind: "text/plain".to_string(),
            content_path: None,
            content_sha256: None,
        }
    }

    #[test]
    fn sha256_empty_and_abc_vectors() {
        assert_eq!(sha256_hex(b""), EMPTY_SHA256);
        assert_eq!(sha256_hex(b"abc"), ABC_SHA256);
    }

    #[test]
    fn sha256_known_vectors_multi_block_and_boundaries() {
        // 经典 NIST 向量(多块,55/56/57 字节覆盖 padding 边界;64/65/128 字节整块)。
        assert_eq!(
            sha256_hex(b"The quick brown fox jumps over the lazy dog"),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha256_hex(&[b'x'; 64]),
            "7ce100971f64e7001e8fe5a51973ecdfe1ced42befe7ee8d5fd6219506b5393c"
        );
        assert_eq!(
            sha256_hex(&[b'a'; 55]),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        assert_eq!(
            sha256_hex(&[b'a'; 56]),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
        assert_eq!(
            sha256_hex(&[b'a'; 57]),
            "f13b2d724659eb3bf47f2dd6af1accc87b81f09f59f2b75e5c0bed6589dfe8c6"
        );
        assert_eq!(
            sha256_hex(&[b'a'; 65]),
            "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0"
        );
        assert_eq!(
            sha256_hex(&[b'a'; 128]),
            "6836cf13bac400e9105071cd6af47084dfacad4e5e302c94bfed24e013afb73e"
        );
    }

    #[test]
    fn sha256_million_a_vector() {
        // NIST 百万 'a' 向量,验证多块长输入。
        assert_eq!(
            sha256_hex(&[b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sha256_non_ascii_utf8() {
        // UTF-8 多字节输入。
        assert_eq!(
            sha256_hex("你好".as_bytes()),
            "670d9743542cae3ea7ebe36af56bd53648b0a1126162e78d81a32934a711302e"
        );
    }

    #[test]
    fn content_sha256_none_is_empty_string_hash() {
        let api = PromptAttachmentService;
        // None 与空串都哈希空串。
        assert_eq!(api.content_sha256(None), EMPTY_SHA256);
        assert_eq!(api.content_sha256(Some("")), EMPTY_SHA256);
        assert_eq!(api.content_sha256(Some("abc")), ABC_SHA256);
    }

    #[test]
    fn hash_rendered_stable_and_distinct() {
        let api = PromptAttachmentService;
        assert_eq!(api.hash_rendered("abc"), ABC_SHA256);
        assert_eq!(api.hash_rendered(""), EMPTY_SHA256);
        assert_eq!(
            api.hash_rendered("你好"),
            "670d9743542cae3ea7ebe36af56bd53648b0a1126162e78d81a32934a711302e"
        );
        assert_ne!(api.hash_rendered("abc"), api.hash_rendered("abd"));
    }

    #[test]
    fn hash_attachment_stable_same_attachment() {
        let api = PromptAttachmentService;
        let a = attachment("a1", Some("hello"));
        assert_eq!(api.hash_attachment(&a), api.hash_attachment(&a));
    }

    #[test]
    fn hash_attachment_changes_with_content() {
        let api = PromptAttachmentService;
        let a = attachment("a1", Some("hello"));
        let b = attachment("a1", Some("hello!"));
        assert_ne!(api.hash_attachment(&a), api.hash_attachment(&b));
    }

    #[test]
    fn hash_attachment_ignores_excluded_fields() {
        let api = PromptAttachmentService;
        let mut a = attachment("a1", Some("hi"));
        let mut b = a.clone();
        a.content_sha256 = Some("aaa".to_string());
        a.created_at = Some("2026-01-01T00:00:00+00:00".to_string());
        a.updated_at = Some("2026-01-02T00:00:00+00:00".to_string());
        b.content_sha256 = Some("bbb".to_string());
        b.created_at = Some("1999-01-01T00:00:00+00:00".to_string());
        b.updated_at = Some("2000-01-01T00:00:00+00:00".to_string());
        // 三个排除字段任意变化不影响语义哈希。
        assert_eq!(api.hash_attachment(&a), api.hash_attachment(&b));
        // 非排除字段变化仍会改变哈希。
        let mut c = a.clone();
        c.priority = 99;
        assert_ne!(api.hash_attachment(&a), api.hash_attachment(&c));
    }
    #[test]
    fn hash_attachment_matches_python_canonical_vector() {
        // 与 Python 规格 hash_prompt_attachment 的精确输出对拍
        // (含排除字段、键排序、紧凑分隔符)。
        let api = PromptAttachmentService;
        let mut metadata = Map::new();
        metadata.insert("x".to_string(), json!(1));
        let a = PromptAttachment {
            id: "session.sid.mem".to_string(),
            section: "mem".to_string(),
            kind: PromptAttachmentKind::Text,
            content: Some("hi".to_string()),
            priority: 50,
            source: Some("s1".to_string()),
            session_id: "sid".to_string(),
            created_at: Some("t0".to_string()),
            updated_at: Some("t1".to_string()),
            expires_at: None,
            metadata,
            content_kind: "text/plain".to_string(),
            content_path: None,
            content_sha256: Some("deadbeef".to_string()),
        };
        assert_eq!(
            api.hash_attachment(&a),
            "9acc22902d30589e7aaa5d5a43e80260f74f3d7fd5bd94a6c654b6416f3454f0"
        );
    }

    #[test]
    fn hash_attachment_preserves_non_ascii_and_sorts_nested() {
        // 非 ASCII 原样(ensure_ascii=false)+ 嵌套对象键排序,与 Python 对拍。
        let api = PromptAttachmentService;
        let mut metadata = Map::new();
        metadata.insert("n".to_string(), json!([1, 2, { "b": true, "a": "汉" }]));
        let a = PromptAttachment {
            id: "id2".to_string(),
            section: "sec2".to_string(),
            kind: PromptAttachmentKind::TodoReminder,
            content: Some("你好 世界".to_string()),
            priority: 100,
            source: None,
            session_id: "s2".to_string(),
            created_at: None,
            updated_at: None,
            expires_at: None,
            metadata,
            content_kind: "text/plain".to_string(),
            content_path: Some("/tmp/f".to_string()),
            content_sha256: Some("x".to_string()),
        };
        assert_eq!(
            api.hash_attachment(&a),
            "871496b362710fdc21762c68dd511742eac3879bbe01ebcded581ffc26297120"
        );
    }

    #[test]
    fn hash_attachment_canonical_ignores_metadata_key_order() {
        let api = PromptAttachmentService;
        let mut m1 = Map::new();
        m1.insert("a".to_string(), json!(1));
        m1.insert("z".to_string(), json!(2));
        let mut m2 = Map::new();
        m2.insert("z".to_string(), json!(2));
        m2.insert("a".to_string(), json!(1));
        let mut x = attachment("a1", Some("hi"));
        let mut y = x.clone();
        x.metadata = m1;
        y.metadata = m2;
        assert_eq!(api.hash_attachment(&x), api.hash_attachment(&y));
    }
    #[test]
    fn safe_id_part_keeps_legal_replaces_illegal() {
        let api = PromptAttachmentService;
        assert_eq!(api.safe_id_part(Some("hello-world"), "fb"), "hello-world");
        // 连续非法字符折叠为单个 '_'。
        assert_eq!(api.safe_id_part(Some("a b/c"), "fb"), "a_b_c");
        assert_eq!(api.safe_id_part(Some("a!!b"), "fb"), "a_b");
        // 非 ASCII 一律视为非法。
        assert_eq!(api.safe_id_part(Some("a你b"), "fb"), "a_b");
    }

    #[test]
    fn safe_id_part_strips_edge_dots_underscores_dashes() {
        let api = PromptAttachmentService;
        assert_eq!(api.safe_id_part(Some(".abc."), "fb"), "abc");
        assert_eq!(api.safe_id_part(Some("-x-"), "fb"), "x");
        assert_eq!(api.safe_id_part(Some("_keep_"), "fb"), "keep");
        assert_eq!(api.safe_id_part(Some("a_b.c-d"), "fb"), "a_b.c-d");
    }

    #[test]
    fn safe_id_part_all_illegal_falls_back_to_sha256_prefix() {
        let api = PromptAttachmentService;
        assert_eq!(api.safe_id_part(Some("!!!"), "fb"), "e84c538e7fe2");
        assert_eq!(api.safe_id_part(Some("你好世界"), "fb"), "beca6335b20f");
        assert_eq!(api.safe_id_part(Some("_._"), "fb"), "ef8b8f3f2117");
    }

    #[test]
    fn safe_id_part_truncates_and_empty_fallback() {
        let api = PromptAttachmentService;
        let long = "a".repeat(100);
        assert_eq!(api.safe_id_part(Some(&long), "fb"), "a".repeat(80));
        assert_eq!(api.safe_id_part(None, "fb"), "fb");
        assert_eq!(api.safe_id_part(Some(""), "fb"), "fb");
        assert_eq!(api.safe_id_part(Some("   "), "fb"), "fb");
    }
    #[test]
    fn prompt_attachment_serde_defaults_round_trip() {
        // 最小 JSON:kind=Generic、priority=100、content_kind="text/plain"。
        let minimal = r#"{"id":"a1","section":"mem","session_id":"sid"}"#;
        let att: PromptAttachment = serde_json::from_str(minimal).expect("minimal decodes");
        assert_eq!(att.kind, PromptAttachmentKind::Generic);
        assert_eq!(att.priority, 100);
        assert_eq!(att.content_kind, "text/plain");
        assert!(att.metadata.is_empty());
        assert_eq!(att.content, None);
        assert_eq!(att.source, None);
        assert_eq!(att.created_at, None);
        assert_eq!(att.content_sha256, None);
        // 序列化包含默认字段。
        let v: Value =
            serde_json::from_str(&serde_json::to_string(&att).expect("serializes")).expect("json");
        assert_eq!(v["kind"], "generic");
        assert_eq!(v["priority"], 100);
        assert_eq!(v["content_kind"], "text/plain");
        // 完整往返。
        let full = attachment("a1", Some("hi"));
        let json = serde_json::to_string(&full).expect("serializes");
        let back: PromptAttachment = serde_json::from_str(&json).expect("decodes");
        assert_eq!(full, back);
    }

    #[test]
    fn prompt_attachment_update_serde() {
        let update = PromptAttachmentUpdate {
            kind: Some(PromptAttachmentKind::Memory),
            content: Some("new".to_string()),
            priority: Some(1),
            source: None,
            expires_at: None,
            metadata: None,
            content_kind: Some("text/markdown".to_string()),
        };
        let json = serde_json::to_string(&update).expect("serializes");
        let back: PromptAttachmentUpdate = serde_json::from_str(&json).expect("decodes");
        assert_eq!(update, back);
        assert_eq!(back.kind, Some(PromptAttachmentKind::Memory));
        // 缺失字段 → None。
        let empty: PromptAttachmentUpdate = serde_json::from_str("{}").expect("empty decodes");
        assert_eq!(empty.kind, None);
        assert_eq!(empty.content, None);
        assert_eq!(empty.metadata, None);
    }

    #[test]
    fn prompt_attachment_kind_snake_case_serde() {
        assert_eq!(
            serde_json::to_string(&PromptAttachmentKind::Generic).expect("ser"),
            "\"generic\""
        );
        assert_eq!(
            serde_json::to_string(&PromptAttachmentKind::TodoReminder).expect("ser"),
            "\"todo_reminder\""
        );
        assert_eq!(
            serde_json::to_string(&PromptAttachmentKind::WorkspaceDelta).expect("ser"),
            "\"workspace_delta\""
        );
        let all: Vec<PromptAttachmentKind> = serde_json::from_str(
            r#"["generic","text","runtime","memory","file","tool","skill","diagnostic","todo_reminder","workspace_delta"]"#,
        )
        .expect("kinds decode");
        assert_eq!(all.len(), 10);
        assert_eq!(all[0], PromptAttachmentKind::Generic);
        assert_eq!(all[8], PromptAttachmentKind::TodoReminder);
        assert_eq!(all[9], PromptAttachmentKind::WorkspaceDelta);
        // 非法枚举值显式报错。
        assert!(serde_json::from_str::<PromptAttachmentKind>("\"bogus\"").is_err());
    }

    #[test]
    fn attachment_error_display_and_error() {
        let err = AttachmentError("boom".to_string());
        assert_eq!(err.to_string(), "boom");
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn plugin_registers_prompt_attachment() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(PromptAttachmentPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let api = ctx
            .service::<dyn PromptAttachmentApi>(&PROMPT_ATTACHMENT)
            .expect("prompt-attachment seam");
        assert_eq!(api.hash_rendered("abc"), ABC_SHA256);
        assert_eq!(api.content_sha256(None), EMPTY_SHA256);
        assert_eq!(api.safe_id_part(Some("a b"), "fb"), "a_b");
        // 卸载(Effect drop)后服务回滚。
        drop(effects);
        assert!(!ctx.has_service(&PROMPT_ATTACHMENT));
    }

    #[test]
    fn store_add_get_update_remove() {
        use ah_contracts::keys::PROMPT_ATTACHMENT_STORE;
        use ah_contracts::prompt_attachment::PromptAttachmentStore;
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(PromptAttachmentPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let store = ctx
            .service::<dyn PromptAttachmentStore>(&PROMPT_ATTACHMENT_STORE)
            .expect("store seam");

        let added = store
            .add_section(
                "sid1",
                "mem",
                "hello",
                PromptAttachmentKind::Memory,
                "test",
                50,
                None,
                "text/plain",
                None,
            )
            .expect("add");
        assert_eq!(added.id, "session.sid1.mem");
        assert_eq!(added.section, "mem");
        assert_eq!(added.metadata["section"], "mem");
        assert_eq!(added.metadata["source"], "test");
        assert!(added.content_sha256.is_some());
        assert!(added.created_at.is_some());

        // 替换同 section → id 不变,内容更新。
        let replaced = store
            .add_section(
                "sid1",
                "mem",
                "new",
                PromptAttachmentKind::Memory,
                "test",
                50,
                None,
                "text/plain",
                None,
            )
            .expect("replace");
        assert_eq!(replaced.id, "session.sid1.mem");
        assert_eq!(replaced.content.as_deref(), Some("new"));

        let fetched = store.get_by_id("session.sid1.mem", None).expect("get");
        assert_eq!(fetched.content.as_deref(), Some("new"));

        // 更新内容。
        let update = PromptAttachmentUpdate {
            kind: None,
            content: Some("updated".to_string()),
            priority: Some(10),
            source: None,
            expires_at: None,
            metadata: None,
            content_kind: None,
        };
        let updated = store
            .update_by_id("session.sid1.mem", &update)
            .expect("update");
        assert_eq!(updated.content.as_deref(), Some("updated"));
        assert_eq!(updated.priority, 10);
        // 未找到显式报错。
        assert!(store.update_by_id("nope", &update).is_err());

        // 删除。
        assert!(store.remove_by_id("session.sid1.mem", None));
        assert!(!store.remove_by_id("session.sid1.mem", None));
        assert!(store.get_by_id("session.sid1.mem", None).is_none());

        drop(effects);
    }

    #[test]
    fn store_filter_and_destructive_guard() {
        use ah_contracts::keys::PROMPT_ATTACHMENT_STORE;
        use ah_contracts::prompt_attachment::{AttachmentFilter, PromptAttachmentStore};
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(PromptAttachmentPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let store = ctx
            .service::<dyn PromptAttachmentStore>(&PROMPT_ATTACHMENT_STORE)
            .expect("store");

        store
            .add_section(
                "s1",
                "mem",
                "a",
                PromptAttachmentKind::Memory,
                "src1",
                50,
                None,
                "text/plain",
                None,
            )
            .expect("add");
        store
            .add_section(
                "s1",
                "todo",
                "b",
                PromptAttachmentKind::TodoReminder,
                "src2",
                10,
                None,
                "text/plain",
                None,
            )
            .expect("add");
        store
            .add_section(
                "s2",
                "mem",
                "c",
                PromptAttachmentKind::Memory,
                "src1",
                5,
                None,
                "text/plain",
                None,
            )
            .expect("add");

        // 稳定排序:priority 升序,其次 source,再次 section。
        let all = store.list_by_filter(&AttachmentFilter::default());
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].content.as_deref(), Some("c")); // priority 5
        assert_eq!(all[1].content.as_deref(), Some("b")); // priority 10
        assert_eq!(all[2].content.as_deref(), Some("a")); // priority 50

        // 按 section 过滤。
        let mem = store.list_by_filter(&AttachmentFilter {
            section: Some("mem".to_string()),
            ..Default::default()
        });
        assert_eq!(mem.len(), 2);

        // 按 kind 过滤。
        let todo = store.list_by_filter(&AttachmentFilter {
            kind: Some(PromptAttachmentKind::TodoReminder),
            ..Default::default()
        });
        assert_eq!(todo.len(), 1);
        assert_eq!(todo[0].section, "todo");

        // 按 session 过滤。
        let s1 = store.list_by_filter(&AttachmentFilter {
            session_id: Some("s1".to_string()),
            ..Default::default()
        });
        assert_eq!(s1.len(), 2);

        // 按 source 过滤。
        let src1 = store.list_by_filter(&AttachmentFilter {
            source: Some("src1".to_string()),
            ..Default::default()
        });
        assert_eq!(src1.len(), 2);

        // 无过滤破坏性删除 → 显式报错。
        assert!(
            store
                .remove_by_filter(&AttachmentFilter::default(), false)
                .is_err()
        );

        // 按 source 删除。
        let removed = store
            .remove_by_filter(
                &AttachmentFilter {
                    source: Some("src1".to_string()),
                    ..Default::default()
                },
                false,
            )
            .expect("remove");
        assert_eq!(removed, 2);
        let remaining = store.list_by_filter(&AttachmentFilter::default());
        assert_eq!(remaining.len(), 1);

        // clear_session。
        assert_eq!(store.clear_session("s1"), 1);
        assert_eq!(store.clear_session("s1"), 0);
        assert_eq!(store.list_by_filter(&AttachmentFilter::default()).len(), 0);

        drop(effects);
    }

    #[test]
    fn store_expiry_collect_and_clear_all() {
        use ah_contracts::keys::PROMPT_ATTACHMENT_STORE;
        use ah_contracts::prompt_attachment::PromptAttachmentStore;
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(PromptAttachmentPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let store = ctx
            .service::<dyn PromptAttachmentStore>(&PROMPT_ATTACHMENT_STORE)
            .expect("store");

        // 过期时间在过去 → collect 剔除。
        store
            .add_section(
                "s1",
                "old",
                "x",
                PromptAttachmentKind::Text,
                "src",
                50,
                None,
                "text/plain",
                Some("2000-01-01T00:00:00+00:00"),
            )
            .expect("add");
        store
            .add_section(
                "s1",
                "fresh",
                "y",
                PromptAttachmentKind::Text,
                "src",
                50,
                None,
                "text/plain",
                None,
            )
            .expect("add");
        let collected = store.collect_for_session("s1");
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].section, "fresh");

        assert_eq!(store.clear_all(), 1);
        assert_eq!(store.clear_all(), 0);

        drop(effects);
    }

    #[test]
    fn render_blocks_and_truncation() {
        use ah_contracts::prompt_attachment::{
            DEFAULT_MAX_PROMPT_ATTACHMENT_CHARS, DEFAULT_MAX_RENDERED_CHARS, render,
        };
        let a = attachment("session.sid1.mem", Some("hello <world> & \"quoted\""));
        let rendered = render(
            &[a],
            DEFAULT_MAX_PROMPT_ATTACHMENT_CHARS,
            DEFAULT_MAX_RENDERED_CHARS,
        );
        assert!(rendered.starts_with("<system-reminder>\n"));
        assert!(rendered.contains("The following context is automatically attached"));
        assert!(rendered.contains("<prompt-attachment type=\"text\">"));
        // XML 文本转义(quote=False:引号不转义)。
        assert!(rendered.contains("hello &lt;world&gt; &amp; \"quoted\""));
        assert!(rendered.ends_with("</system-reminder>"));
        // 空列表 → 空串。
        assert_eq!(render(&[], 100, 100), "");

        // 单附件超限截断。
        let big = attachment("id-big", Some(&"x".repeat(200)));
        let truncated = render(&[big], 100, 10_000);
        assert!(truncated.contains("[Prompt attachment truncated: content exceeded"));
    }

    #[test]
    fn render_sorts_and_inject_messages() {
        use ah_contracts::prompt_attachment::{inject_messages, render};
        let mut low = attachment("a", Some("CONTENT-LOW"));
        low.priority = 100;
        let mut high = attachment("b", Some("CONTENT-HIGH"));
        high.priority = 5;
        let rendered = render(&[low, high], 100, 10_000);
        let high_pos = rendered.find("CONTENT-HIGH").expect("high present");
        let low_pos = rendered.find("CONTENT-LOW").expect("low present");
        assert!(high_pos < low_pos, "priority 5 first: {rendered}");

        // inject_messages:追加为独立 user 消息;空渲染 → 原样。
        let msgs = vec!["m1".to_string(), "m2".to_string()];
        let injected = inject_messages(&msgs, "RENDERED");
        assert_eq!(injected.len(), 3);
        assert_eq!(injected[2], "RENDERED");
        let unchanged = inject_messages(&msgs, "");
        assert_eq!(unchanged, msgs);
    }

    #[test]
    fn store_supports_source_replacement_metadata_and_file_lifecycle() {
        use ah_contracts::keys::PROMPT_ATTACHMENT_STORE;
        use ah_contracts::prompt_attachment::PromptAttachmentStore;
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(PromptAttachmentPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let store = ctx
            .service::<dyn PromptAttachmentStore>(&PROMPT_ATTACHMENT_STORE)
            .expect("store");
        let first = store
            .add_section(
                "s1",
                "old",
                "old content",
                PromptAttachmentKind::Text,
                "source-a",
                50,
                None,
                "text/plain",
                None,
            )
            .expect("add");
        let mut metadata = Map::new();
        metadata.insert("request".into(), json!("r1"));
        let updated = store
            .update_metadata_by_id(&first.id, &metadata, Some("s1"), true)
            .expect("metadata update");
        assert_eq!(updated.metadata["request"], "r1");
        let cleared = store
            .update_content_by_id(&first.id, None, Some("s1"), None)
            .expect("clear content");
        assert_eq!(cleared.content, None);
        assert_eq!(cleared.content_sha256.as_deref(), Some(EMPTY_SHA256));

        let replacement = PromptAttachment {
            id: "ignored".into(),
            section: "new".into(),
            kind: PromptAttachmentKind::Memory,
            content: Some("replacement".into()),
            priority: 1,
            source: None,
            session_id: "ignored".into(),
            created_at: None,
            updated_at: None,
            expires_at: None,
            metadata: Map::new(),
            content_kind: "text/plain".into(),
            content_path: None,
            content_sha256: None,
        };
        let replaced = store
            .replace_source("source-a", &[replacement], Some("s1"))
            .expect("replace source");
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].source.as_deref(), Some("source-a"));
        assert!(
            store
                .list_by_filter(&AttachmentFilter {
                    source: Some("source-a".into()),
                    ..Default::default()
                })
                .iter()
                .all(|item| item.section == "new")
        );

        let file = store
            .add_file_reference("docs/readme.md", None, "s1", None, None, 10, None)
            .expect("file reference");
        assert_eq!(file.kind, PromptAttachmentKind::File);
        assert_eq!(file.metadata["file_path"], "docs/readme.md");
        assert_eq!(store.clear_source("file_reference", Some("s1")), 1);
        drop(effects);
    }
}
