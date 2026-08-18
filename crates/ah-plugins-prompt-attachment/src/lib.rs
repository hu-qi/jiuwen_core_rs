//! # ah-plugins-prompt-attachment
//!
//! 真实 prompt 附件核心(对齐 openjiuwen/harness/prompts/prompt_attachment_manager.py
//! 的纯函数部分):自实现 sha256、canonical JSON 语义哈希、安全 id 净化。
//! 无外部 hash crate;sha256 为标准 FIPS 180-4 实现(约 100 行纯函数,无 IO、无状态)。
//!
//! 注册服务键 PROMPT_ATTACHMENT("prompt-attachment"),实现
//! ah_contracts::prompt_attachment::PromptAttachmentApi。

use std::sync::Arc;

use ah_contracts::keys::PROMPT_ATTACHMENT;
use ah_contracts::prelude::Effect;
use ah_contracts::prompt_attachment::{PromptAttachment, PromptAttachmentApi};
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

/// prompt-attachment 插件:注册 prompt-attachment seam。
pub struct PromptAttachmentPlugin;

impl Plugin for PromptAttachmentPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-prompt-attachment"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![PROMPT_ATTACHMENT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let service: Arc<dyn PromptAttachmentApi> = Arc::new(PromptAttachmentService);
        Ok(vec![ctx.register(PROMPT_ATTACHMENT, service)])
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
}
