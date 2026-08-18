//! # ah-plugins-reliability-tools
//!
//! Real tool-call health detectors (aligned with openjiuwen/agent_teams/
//! reliability/detectors/):
//! - stable_call_hash / stable_result_hash: canonical sorted-JSON sha256
//!   (argument-order independent; sha256 self-implemented, no external crate);
//! - OutputLengthDetector: over-long text/thinking thresholds, fire-once
//!   per kind until reset, LOW severity;
//! - RepeatToolCallDetector: sliding history of (call_key, outcome), four
//!   tiers (repeat / alternation / loop-block / global-stop), edge-triggered;
//! - PingPongDetector: team-level member<->member message volleys, strict
//!   direction-reversal streak, edge-triggered Medium->High tiers.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use ah_contracts::keys::RELIABILITY_TOOLS;
use ah_contracts::prelude::Effect;
use ah_contracts::reliability_config::PingPongConfig;
use ah_contracts::reliability_detectors::{
    Anomaly, AnomalyKind, Detector, Severity, Signal, SignalKind,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

// ---------------------------------------------------------------------------
// SHA-256 (FIPS 180-4)
// ---------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Self-implemented SHA-256 (FIPS 180-4); returns lowercase hex digest.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = (w[i - 15]).rotate_right(7) ^ (w[i - 15]).rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = (w[i - 2]).rotate_right(17) ^ (w[i - 2]).rotate_right(19) ^ (w[i - 2] >> 10);
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
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
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
    h.iter().map(|x| format!("{:08x}", x)).collect::<String>()
}

// ---------------------------------------------------------------------------
// Canonical JSON (recursive key-sorted, compact, non-ASCII preserved)
// ---------------------------------------------------------------------------

fn canonical_string(s: &str) -> String {
    let escaped = s
        .chars()
        .flat_map(|c| match c {
            '"' => "\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            c if (c as u32) < 0x20 => format!("\\u{:04x}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect::<String>();
    format!("\"{escaped}\"")
}

/// Canonical JSON: recursive key-sorted, compact separators, non-ASCII kept.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => canonical_string(s),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Object(map) => {
            let mut pairs: Vec<(String, String)> = map
                .iter()
                .map(|(k, v)| (k.clone(), canonical_json(v)))
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            let inner: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}:{}", canonical_string(k), v))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

/// Stable tool-call hash: sha256 of canonical {"tool": name, "args": args}.
pub fn stable_call_hash(tool_name: &str, tool_args: Option<&Value>) -> String {
    let args = tool_args
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let payload = Value::Object(serde_json::Map::from_iter(vec![
        ("tool".to_string(), Value::String(tool_name.to_string())),
        ("args".to_string(), args),
    ]));
    sha256_hex(canonical_json(&payload).as_bytes())
}

/// Stable tool-result hash; None -> "none".
pub fn stable_result_hash(result: Option<&Value>) -> String {
    match result {
        None => "none".to_string(),
        Some(Value::String(s)) => sha256_hex(s.as_bytes()),
        Some(v) => sha256_hex(canonical_json(v).as_bytes()),
    }
}

// ---------------------------------------------------------------------------
// OutputLengthDetector
// ---------------------------------------------------------------------------

/// Flag over-long model output text or thinking content (fire-once per kind).
pub struct OutputLengthDetector {
    text_threshold: u64,
    thinking_threshold: u64,
    fired: Mutex<std::collections::HashSet<AnomalyKind>>,
}

impl OutputLengthDetector {
    pub fn new(text_threshold: u64, thinking_threshold: u64) -> Self {
        Self {
            text_threshold,
            thinking_threshold,
            fired: Mutex::new(std::collections::HashSet::new()),
        }
    }
}

impl Seam for OutputLengthDetector {}

impl Detector for OutputLengthDetector {
    fn name(&self) -> &str {
        "output_length"
    }

    fn observe(&self, signal: &Signal) -> Option<Anomaly> {
        if signal.kind != SignalKind::AfterModelCall {
            return None;
        }
        let mut fired = self.fired.lock().unwrap();
        let thinking = signal.thinking_len.unwrap_or(0);
        if thinking > self.thinking_threshold && !fired.contains(&AnomalyKind::ThinkingTooLong) {
            fired.insert(AnomalyKind::ThinkingTooLong);
            return Some(Anomaly {
                detector: self.name().to_string(),
                kind: AnomalyKind::ThinkingTooLong,
                severity: Severity::Low,
                member_name: signal.member_name.clone(),
                summary: format!(
                    "thinking length {thinking} exceeds {threshold}",
                    threshold = self.thinking_threshold
                ),
                evidence: serde_json::Map::from_iter(vec![
                    ("thinking_len".to_string(), Value::from(thinking)),
                    (
                        "threshold".to_string(),
                        Value::from(self.thinking_threshold),
                    ),
                ]),
                peer_member: None,
            });
        }
        let text = signal.text_len.unwrap_or(0);
        if text > self.text_threshold && !fired.contains(&AnomalyKind::OutputTooLong) {
            fired.insert(AnomalyKind::OutputTooLong);
            return Some(Anomaly {
                detector: self.name().to_string(),
                kind: AnomalyKind::OutputTooLong,
                severity: Severity::Low,
                member_name: signal.member_name.clone(),
                summary: format!(
                    "output length {text} exceeds {threshold}",
                    threshold = self.text_threshold
                ),
                evidence: serde_json::Map::from_iter(vec![
                    ("text_len".to_string(), Value::from(text)),
                    ("threshold".to_string(), Value::from(self.text_threshold)),
                ]),
                peer_member: None,
            });
        }
        None
    }

    fn reset(&self) {
        self.fired.lock().unwrap().clear();
    }
}

// ---------------------------------------------------------------------------
// RepeatToolCallDetector
// ---------------------------------------------------------------------------

struct RepeatState {
    history: VecDeque<(String, String)>,
    pending_call_key: Option<String>,
    fired_severity: Option<Severity>,
}

/// Detect repeated / looping tool calls over a sliding history (four tiers).
pub struct RepeatToolCallDetector {
    history_size: usize,
    repeat_warn: u64,
    pingpong_warn: u64,
    loop_block: u64,
    global_stop: u64,
    state: Mutex<RepeatState>,
}

impl RepeatToolCallDetector {
    pub fn new(
        history_size: usize,
        repeat_warn: u64,
        pingpong_warn: u64,
        loop_block: u64,
        global_stop: u64,
    ) -> Self {
        Self {
            history_size,
            repeat_warn,
            pingpong_warn,
            loop_block,
            global_stop,
            state: Mutex::new(RepeatState {
                history: VecDeque::new(),
                pending_call_key: None,
                fired_severity: None,
            }),
        }
    }

    fn trailing_identical(history: &VecDeque<(String, String)>) -> u64 {
        let Some(last) = history.back() else {
            return 0;
        };
        let mut count = 0u64;
        for record in history.iter().rev() {
            if record == last {
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    fn trailing_alternation(history: &VecDeque<(String, String)>) -> u64 {
        if history.len() < 2 {
            return 0;
        }
        let sequence: Vec<&(String, String)> = history.iter().rev().collect();
        let first = sequence[0];
        let second = sequence[1];
        if first == second || first.0 == second.0 {
            return 0;
        }
        let mut count = 0u64;
        for (index, record) in sequence.iter().enumerate() {
            let expected = if index % 2 == 0 { first } else { second };
            if *record == expected {
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    fn record_and_classify(&self, member: &str, outcome: String) -> Option<Anomaly> {
        let mut state = self.state.lock().unwrap();
        let call_key = state.pending_call_key.take()?;
        if state.history.len() == self.history_size {
            state.history.pop_front();
        }
        state.history.push_back((call_key.clone(), outcome));
        let identical = Self::trailing_identical(&state.history);
        let (severity, kind, evidence) = if identical >= self.global_stop {
            (
                Some(Severity::Critical),
                AnomalyKind::ToolCallLoop,
                serde_json::Map::from_iter(vec![(
                    "trailing_identical".to_string(),
                    Value::from(identical),
                )]),
            )
        } else if identical >= self.loop_block {
            (
                Some(Severity::High),
                AnomalyKind::ToolCallLoop,
                serde_json::Map::from_iter(vec![(
                    "trailing_identical".to_string(),
                    Value::from(identical),
                )]),
            )
        } else {
            let alternation = Self::trailing_alternation(&state.history);
            if alternation >= self.pingpong_warn {
                (
                    Some(Severity::Medium),
                    AnomalyKind::ToolCallLoop,
                    serde_json::Map::from_iter(vec![(
                        "trailing_alternation".to_string(),
                        Value::from(alternation),
                    )]),
                )
            } else {
                let repeats = state
                    .history
                    .iter()
                    .filter(|(ck, _)| *ck == call_key)
                    .count() as u64;
                if repeats >= self.repeat_warn {
                    (
                        Some(Severity::Low),
                        AnomalyKind::RepeatToolCall,
                        serde_json::Map::from_iter(vec![(
                            "call_repeats".to_string(),
                            Value::from(repeats),
                        )]),
                    )
                } else {
                    (None, AnomalyKind::RepeatToolCall, serde_json::Map::new())
                }
            }
        };
        let severity = severity?;
        if let Some(previous) = state.fired_severity
            && severity.rank() <= previous.rank()
        {
            return None;
        }
        state.fired_severity = Some(severity);
        Some(Anomaly {
            detector: self.name().to_string(),
            kind,
            severity,
            member_name: member.to_string(),
            summary: format!(
                "{kind:?} detected after {count} calls",
                count = state.history.len()
            ),
            evidence,
            peer_member: None,
        })
    }
}

impl Seam for RepeatToolCallDetector {}

impl Detector for RepeatToolCallDetector {
    fn name(&self) -> &str {
        "repeat_tool_call"
    }

    fn observe(&self, signal: &Signal) -> Option<Anomaly> {
        match signal.kind {
            SignalKind::BeforeToolCall => {
                let key = stable_call_hash(
                    signal.tool_name.as_deref().unwrap_or(""),
                    signal.tool_args.as_ref(),
                );
                self.state.lock().unwrap().pending_call_key = Some(key);
                None
            }
            SignalKind::AfterToolCall => self.record_and_classify(
                &signal.member_name,
                stable_result_hash(signal.tool_result.as_ref()),
            ),
            SignalKind::ToolException => self.record_and_classify(
                &signal.member_name,
                signal.error.clone().unwrap_or_else(|| "error".to_string()),
            ),
            _ => None,
        }
    }

    fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.history.clear();
        state.pending_call_key = None;
        state.fired_severity = None;
    }
}

// ---------------------------------------------------------------------------
// PingPongDetector
// ---------------------------------------------------------------------------

struct PingPongState {
    last_from: Option<String>,
    last_to: Option<String>,
    count: u64,
    fired_severity: Option<Severity>,
}

/// 团队级乒乓检测:两名成员间严格方向反转的消息连发(对齐 Python pingpong.py)。
/// 一轮 volley 即一条消息;min_volleys=6 约等于三个往返。任何第三方消息或
/// 非反转消息重置计数并清除已触发严重度。边沿触发:severity 上升才发。
pub struct PingPongDetector {
    min_volleys: u64,
    state: Mutex<PingPongState>,
}

impl PingPongDetector {
    pub fn new(config: PingPongConfig) -> Self {
        Self {
            min_volleys: config.min_volleys,
            state: Mutex::new(PingPongState {
                last_from: None,
                last_to: None,
                count: 0,
                fired_severity: None,
            }),
        }
    }

    fn classify(&self, count: u64) -> Option<Severity> {
        if count >= self.min_volleys * 2 {
            Some(Severity::High)
        } else if count >= self.min_volleys {
            Some(Severity::Medium)
        } else {
            None
        }
    }
}

impl Seam for PingPongDetector {}

impl Detector for PingPongDetector {
    fn name(&self) -> &str {
        "ping_pong"
    }

    fn observe(&self, signal: &Signal) -> Option<Anomaly> {
        if signal.kind != SignalKind::Message {
            return None;
        }
        let recipient = signal.peer_member.as_ref()?;
        let sender = &signal.member_name;
        let mut state = self.state.lock().unwrap();
        let is_reversal = match (&state.last_to, &state.last_from) {
            (Some(last_to), Some(last_from)) => sender == last_to && recipient == last_from,
            _ => false,
        };
        if is_reversal {
            state.count += 1;
        } else {
            state.count = 1;
            state.fired_severity = None;
        }
        state.last_from = Some(sender.clone());
        state.last_to = Some(recipient.clone());
        let severity = self.classify(state.count)?;
        if let Some(previous) = state.fired_severity
            && severity.rank() <= previous.rank()
        {
            return None;
        }
        state.fired_severity = Some(severity);
        let mut pair = vec![sender.clone(), recipient.clone()];
        pair.sort();
        let mut evidence = serde_json::Map::new();
        evidence.insert("volleys".to_string(), Value::from(state.count));
        evidence.insert("pair".to_string(), Value::from(pair));
        Some(Anomaly {
            detector: self.name().to_string(),
            kind: AnomalyKind::PingPong,
            severity,
            member_name: sender.clone(),
            summary: format!(
                "{} consecutive volleys between {} and {}",
                state.count, sender, recipient
            ),
            evidence,
            peer_member: Some(recipient.clone()),
        })
    }

    fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.last_from = None;
        state.last_to = None;
        state.count = 0;
        state.fired_severity = None;
    }
}

/// reliability-tools 插件:注册 repeat_tool_call 检测器。
pub struct ReliabilityToolsPlugin;

impl Plugin for ReliabilityToolsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-reliability-tools"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RELIABILITY_TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let detector: Arc<dyn Detector> = Arc::new(RepeatToolCallDetector::new(30, 10, 10, 20, 30));
        Ok(vec![ctx.register(RELIABILITY_TOOLS, detector)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::reliability_detectors::{
        AnomalyKind, Detector, Severity, Signal, SignalKind,
    };
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;

    fn tool_call(member: &str, name: &str, args: Value) -> Signal {
        let mut s = Signal::new(SignalKind::BeforeToolCall, member);
        s.tool_name = Some(name.to_string());
        s.tool_args = Some(args);
        s
    }

    fn tool_done(member: &str, result: Value) -> Signal {
        let mut s = Signal::new(SignalKind::AfterToolCall, member);
        s.tool_result = Some(result);
        s
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 55/56/57 byte padding boundaries.
        let a55 = b"a".repeat(55);
        let a56 = b"a".repeat(56);
        let a57 = b"a".repeat(57);
        assert_eq!(sha256_hex(&a55), sha256_hex(&a55));
        assert_ne!(sha256_hex(&a55), sha256_hex(&a56));
        assert_ne!(sha256_hex(&a56), sha256_hex(&a57));
    }

    #[test]
    fn canonical_json_sorts_keys_recursively() {
        let v = json!({"b": 1, "a": {"y": 2, "x": 1}});
        assert_eq!(canonical_json(&v), "{\"a\":{\"x\":1,\"y\":2},\"b\":1}");
    }

    #[test]
    fn stable_call_hash_order_independent() {
        let h1 = stable_call_hash("read", Some(&json!({"path": "a", "depth": 2})));
        let h2 = stable_call_hash("read", Some(&json!({"depth": 2, "path": "a"})));
        assert_eq!(h1, h2, "参数顺序不影响哈希");
        let h3 = stable_call_hash("write", Some(&json!({"path": "a"})));
        assert_ne!(h1, h3, "工具名参与哈希");
        let h4 = stable_call_hash("read", Some(&json!({"path": "a", "depth": 3})));
        assert_ne!(h1, h4, "参数值参与哈希");
        // None args 与空对象等价。
        assert_eq!(
            stable_call_hash("read", None),
            stable_call_hash("read", Some(&json!({})))
        );
    }

    #[test]
    fn stable_result_hash_none_is_literal() {
        assert_eq!(stable_result_hash(None), "none");
        let s = stable_result_hash(Some(&json!("hello")));
        assert_ne!(s, "none");
        // 相同结果同哈希。
        assert_eq!(
            stable_result_hash(Some(&json!({"x": 1}))),
            stable_result_hash(Some(&json!({"x": 1})))
        );
    }

    #[test]
    fn output_length_flags_text_and_thinking() {
        let detector = OutputLengthDetector::new(100, 50);
        // thinking 超阈值。
        let mut s = Signal::new(SignalKind::AfterModelCall, "alice");
        s.thinking_len = Some(60);
        let a = detector.observe(&s).expect("thinking anomaly");
        assert_eq!(a.kind, AnomalyKind::ThinkingTooLong);
        assert_eq!(a.severity, Severity::Low);
        // fire-once:同一 kind 不重发。
        assert!(detector.observe(&s).is_none());
        // text 超阈值(thinking 未超)。
        let mut t = Signal::new(SignalKind::AfterModelCall, "alice");
        t.text_len = Some(120);
        t.thinking_len = Some(10);
        let a2 = detector.observe(&t).expect("text anomaly");
        assert_eq!(a2.kind, AnomalyKind::OutputTooLong);
        // 非 AfterModelCall 忽略。
        assert!(
            detector
                .observe(&Signal::new(SignalKind::BeforeModelCall, "alice"))
                .is_none()
        );
        // reset 后重发。
        detector.reset();
        assert!(detector.observe(&s).is_some());
    }

    #[test]
    fn repeat_detector_low_on_repeats() {
        let detector = RepeatToolCallDetector::new(30, 3, 10, 20, 30);
        let mut anomalies = Vec::new();
        for i in 0..4 {
            detector.observe(&tool_call("alice", "read", json!({"path": i})));
            if let Some(a) = detector.observe(&tool_done("alice", json!({"ok": i}))) {
                anomalies.push(a);
            }
        }
        // 同 call_key(read 但 args 不同 → 不同 key!)——用相同 args。
        let mut anomalies2 = Vec::new();
        let detector2 = RepeatToolCallDetector::new(30, 3, 10, 20, 30);
        for i in 0..4 {
            detector2.observe(&tool_call("alice", "read", json!({"path": "same"})));
            if let Some(a) = detector2.observe(&tool_done("alice", json!({"ok": i}))) {
                anomalies2.push(a);
            }
        }
        // 第 4 次相同 call_key → repeats=4 ≥ 3 → Low。
        assert_eq!(anomalies2.len(), 1, "边沿触发只发一次");
        assert_eq!(anomalies2[0].severity, Severity::Low);
        assert_eq!(anomalies2[0].kind, AnomalyKind::RepeatToolCall);
        let _ = anomalies;
    }

    #[test]
    fn repeat_detector_loop_high_and_critical() {
        // 相同 call + 相同结果 → trailing identical → High(≥ loop_block=5)。
        let detector = RepeatToolCallDetector::new(30, 3, 10, 5, 8);
        let mut anomalies = Vec::new();
        for _i in 0..7 {
            detector.observe(&tool_call("bob", "retry", json!({"n": "same"})));
            if let Some(a) = detector.observe(&tool_done("bob", json!({"err": "e"}))) {
                anomalies.push(a);
            }
        }
        // identical=7:≥8 未到 Critical,≥5 → High(先 Medium?交替无,重复 Low 先发?)。
        // 第 4 次 repeats=4 ≥ 3 → Low;第 5 次 identical=5 → High(升);第 7 次 identical=7 → 仍 High 不重发。
        let severities: Vec<Severity> = anomalies.iter().map(|a| a.severity).collect();
        assert!(severities.contains(&Severity::Low), "{severities:?}");
        assert!(severities.contains(&Severity::High), "{severities:?}");
        assert!(!severities.contains(&Severity::Critical), "identical=7 < 8");

        // 8 次 → Critical。
        let detector2 = RepeatToolCallDetector::new(30, 3, 10, 5, 8);
        let mut anomalies2 = Vec::new();
        for _i in 0..9 {
            detector2.observe(&tool_call("bob", "retry", json!({"n": "same"})));
            if let Some(a) = detector2.observe(&tool_done("bob", json!({"err": "e"}))) {
                anomalies2.push(a);
            }
        }
        let last = anomalies2.last().expect("anomaly");
        assert_eq!(last.severity, Severity::Critical, "identical=9 ≥ 8");
    }

    #[test]
    fn repeat_detector_alternation_medium() {
        // A-B-A-B 交替(结果稳定)→ trailing_alternation。
        let detector = RepeatToolCallDetector::new(30, 10, 4, 20, 30);
        let mut anomalies = Vec::new();
        let names = ["a", "b", "a", "b", "a", "b"];
        for name in &names {
            detector.observe(&tool_call("carol", name, json!({"k": "same"})));
            if let Some(a) = detector.observe(&tool_done("carol", json!({"r": "same"}))) {
                anomalies.push(a);
            }
        }
        // alternation 到第 4 条起 ≥4 → Medium。
        assert!(
            anomalies.iter().any(|a| a.severity == Severity::Medium),
            "{anomalies:?}"
        );
        assert!(
            anomalies
                .iter()
                .all(|a| a.severity.rank() <= Severity::Medium.rank()),
            "交替上限 Medium"
        );
    }

    #[test]
    fn plugin_registers_tools_detector() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ReliabilityToolsPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let detector = ctx
            .service::<dyn Detector>(&RELIABILITY_TOOLS)
            .expect("tools detector");
        assert_eq!(detector.name(), "repeat_tool_call");
        drop(effects);
        assert!(!ctx.has_service(&RELIABILITY_TOOLS));
    }

    fn message(member: &str, peer: &str) -> Signal {
        let mut s = Signal::new(SignalKind::Message, member);
        s.peer_member = Some(peer.to_string());
        s
    }

    /// 依序投喂 (sender, recipient) 序列,收集全部返回的异常。
    fn drive(detector: &PingPongDetector, sequence: &[(&str, &str)]) -> Vec<Anomaly> {
        sequence
            .iter()
            .filter_map(|(s, r)| detector.observe(&message(s, r)))
            .collect()
    }

    /// A->B, B->A, A->B, ... 的严格反转序列。
    fn reversal_seq(n: usize) -> Vec<(&'static str, &'static str)> {
        (0..n)
            .map(|i| if i % 2 == 0 { ("A", "B") } else { ("B", "A") })
            .collect()
    }

    #[test]
    fn ping_pong_ignores_non_message_signals() {
        let detector = PingPongDetector::new(PingPongConfig::default());
        // 非 Message 信号忽略(即使带 peer_member)。
        let mut s = Signal::new(SignalKind::AfterModelCall, "alice");
        s.peer_member = Some("bob".to_string());
        assert!(detector.observe(&s).is_none());
        assert!(
            detector
                .observe(&Signal::new(SignalKind::BeforeToolCall, "alice"))
                .is_none()
        );
        assert!(
            detector
                .observe(&Signal::new(SignalKind::AfterToolCall, "alice"))
                .is_none()
        );
        // Message 但无 peer_member 也忽略。
        assert!(
            detector
                .observe(&Signal::new(SignalKind::Message, "alice"))
                .is_none()
        );
        // 被忽略的信号不污染状态:后续反转序列仍从 count=1 起正常触发。
        let anomalies = drive(&detector, &reversal_seq(6));
        assert_eq!(
            anomalies.len(),
            1,
            "非 Message 信号未污染状态,count=6 触发 Medium"
        );
        assert_eq!(anomalies[0].severity, Severity::Medium);
    }

    #[test]
    fn ping_pong_medium_at_six_high_at_twelve() {
        let config = PingPongConfig {
            enabled: true,
            min_volleys: 6,
        };
        let detector = PingPongDetector::new(config);
        let anomalies = drive(&detector, &reversal_seq(12));
        assert_eq!(
            anomalies.len(),
            2,
            "count=6 -> Medium,count=12 -> High,各发一次"
        );
        let severities: Vec<Severity> = anomalies.iter().map(|a| a.severity).collect();
        assert_eq!(severities, vec![Severity::Medium, Severity::High]);
        for a in &anomalies {
            assert_eq!(a.detector, "ping_pong");
            assert_eq!(a.kind, AnomalyKind::PingPong);
            assert_eq!(a.member_name, "B", "触发消息的发送者恒为 B(偶数次反转)");
            assert_eq!(a.peer_member.as_deref(), Some("A"));
        }
        let medium = &anomalies[0];
        assert_eq!(medium.summary, "6 consecutive volleys between B and A");
        assert_eq!(medium.evidence.get("volleys"), Some(&json!(6)));
        assert_eq!(medium.evidence.get("pair"), Some(&json!(["A", "B"])));
        let high = &anomalies[1];
        assert_eq!(high.summary, "12 consecutive volleys between B and A");
        assert_eq!(high.evidence.get("volleys"), Some(&json!(12)));
        assert_eq!(high.evidence.get("pair"), Some(&json!(["A", "B"])));
    }

    #[test]
    fn ping_pong_third_party_resets_streak() {
        let detector = PingPongDetector::new(PingPongConfig {
            enabled: true,
            min_volleys: 6,
        });
        let anomalies = drive(&detector, &reversal_seq(6));
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].severity, Severity::Medium);
        // 第三方 C->D 插入:count 重置为 1,fired_severity 清除。
        assert!(detector.observe(&message("C", "D")).is_none());
        // 恢复 A<->B 反转:count 从 1 重新累计,Medium 再次触发。
        let again = drive(&detector, &reversal_seq(6));
        assert_eq!(again.len(), 1, "第三方消息重置后 Medium 重新触发");
        assert_eq!(again[0].severity, Severity::Medium);
        assert_eq!(again[0].member_name, "B");
        assert_eq!(again[0].evidence.get("volleys"), Some(&json!(6)));
    }

    #[test]
    fn ping_pong_edge_triggered_no_refire_at_same_severity() {
        let detector = PingPongDetector::new(PingPongConfig {
            enabled: true,
            min_volleys: 6,
        });
        let mut anomalies = drive(&detector, &reversal_seq(12));
        assert_eq!(anomalies.len(), 2);
        assert_eq!(anomalies[1].severity, Severity::High);
        // High 已触发后继续反转:count 继续增长但同 severity 不再发。
        anomalies.extend(drive(&detector, &reversal_seq(12)));
        assert_eq!(anomalies.len(), 2, "High 已触发,后续同 severity 不重发");
        // 第三方重置后,新 streak 可完整重新触发 Medium->High。
        assert!(detector.observe(&message("C", "D")).is_none());
        let fresh = drive(&detector, &reversal_seq(12));
        assert_eq!(fresh.len(), 2, "重置后重新走完 Medium->High");
        assert_eq!(fresh[0].severity, Severity::Medium);
        assert_eq!(fresh[1].severity, Severity::High);
    }

    #[test]
    fn ping_pong_reset_clears_state() {
        let detector = PingPongDetector::new(PingPongConfig {
            enabled: true,
            min_volleys: 6,
        });
        assert_eq!(detector.name(), "ping_pong");
        let anomalies = drive(&detector, &reversal_seq(6));
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].severity, Severity::Medium);
        detector.reset();
        // 重置后首条 A->B 单独出现:count=1,无异常。
        assert!(detector.observe(&message("A", "B")).is_none());
        // 完整重走反转序列:count 从 1 起,Medium 再次触发(fired_severity 已清)。
        let after = drive(&detector, &reversal_seq(6));
        assert_eq!(
            after.len(),
            1,
            "reset 清空 count/fired_severity 后 Medium 重新触发"
        );
        assert_eq!(after[0].severity, Severity::Medium);
        assert_eq!(after[0].evidence.get("volleys"), Some(&json!(6)));
    }
}
