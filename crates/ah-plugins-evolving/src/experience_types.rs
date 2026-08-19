//! 在线经验生命周期中间类型(对齐 experience/types.py + lifecycle.py 结果工厂)。
//!
//! 纯逻辑:EvolutionContext/PendingChange(创建快照)/ExperienceProposal/审批请求/在线状态/
//! ApplyResult(ok + host 视图:纯拒绝→rejected,否则 persisted/partial)+ HostFacing 结果工厂。

use crate::experience_query::RecordView;
use crate::protocols::{PENDING_CHANGE_EFFECT, SKILL_EXPERIENCE_ENTRY, STATE_EFFECT};
use crate::signal::EvolutionSignal;
use serde_json::Value;
use std::collections::BTreeMap;

/// 在线/离线经验生成的 canonical 输入(对齐 EvolutionContext)。
#[derive(Debug, Clone, PartialEq)]
pub struct EvolutionContext {
    pub skill_name: String,
    pub signals: Vec<EvolutionSignal>,
    pub skill_content: String,
    pub messages: Vec<Value>,
    pub existing_desc_records: Vec<RecordView>,
    pub existing_body_records: Vec<RecordView>,
    pub user_query: String,
    pub existing_script_records: Vec<RecordView>,
    pub tool_call_chain: String,
    pub metadata: BTreeMap<String, Value>,
}

/// UTC 时间生成(ISO-8601 近似:YYYY-MM-DDTHH:MM:SS.ffffff+00:00)。
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

fn unique_8hex() -> String {
    let hex = crate::tool_metadata::unique_hex();
    hex.chars().take(8).collect()
}

/// 待审批演进快照(对齐 PendingChange)。
#[derive(Debug, Clone, PartialEq)]
pub struct PendingChange {
    pub operator_id: String,
    pub skill_name: String,
    pub change_type: String,
    pub payload: Vec<RecordView>,
    pub created_at: String,
    pub change_id: String,
    pub subject_kind: Option<String>,
    pub is_shared_records: bool,
    pub messages: Option<Vec<Value>>,
}

impl PendingChange {
    /// 创建快照(对齐 PendingChange.make)。
    pub fn make(
        skill_name: &str,
        records: &[RecordView],
        subject_kind: Option<String>,
        messages: Option<Vec<Value>>,
    ) -> Self {
        Self {
            operator_id: format!("skill_experience_{skill_name}"),
            skill_name: skill_name.to_string(),
            change_type: SKILL_EXPERIENCE_ENTRY.to_string(),
            payload: records.to_vec(),
            created_at: utc_iso_now(),
            change_id: format!("skill_evolve_{}", unique_8hex()),
            subject_kind,
            is_shared_records: false,
            messages,
        }
    }

    /// 共享记录快照(对齐 make_for_shared_records)。
    pub fn make_for_shared_records(
        skill_name: &str,
        records: &[RecordView],
        subject_kind: Option<String>,
        messages: Option<Vec<Value>>,
    ) -> Self {
        let mut pending = Self::make(skill_name, records, subject_kind, messages);
        pending.is_shared_records = true;
        pending
    }
}

/// 生成的演进提案(对齐 ExperienceProposal)。
#[derive(Debug, Clone, PartialEq)]
pub struct ExperienceProposal {
    pub skill_name: String,
    pub records: Vec<RecordView>,
    pub requires_approval: bool,
    pub source: String,
    pub user_query: String,
    pub signal_type: Option<String>,
    pub signal_source: Option<String>,
}

impl ExperienceProposal {
    pub fn record_count(&self) -> usize {
        self.records.len()
    }
}

/// Host 面稳定结果契约(对齐 HostFacingExperienceResult)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HostFacingExperienceResult {
    pub skill_name: String,
    pub request_id: Option<String>,
    pub effect: String,
    pub change_type: String,
    #[serde(default)]
    pub applied_count: usize,
    #[serde(default)]
    pub rejected_count: usize,
    #[serde(default)]
    pub pending_count: usize,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

fn default_status() -> String {
    "pending_approval".to_string()
}

impl HostFacingExperienceResult {
    pub fn pending_approval(
        skill_name: &str,
        request_id: &str,
        change_type: &str,
        pending_count: usize,
    ) -> Self {
        Self {
            skill_name: skill_name.to_string(),
            request_id: Some(request_id.to_string()),
            effect: PENDING_CHANGE_EFFECT.to_string(),
            change_type: change_type.to_string(),
            pending_count,
            status: "pending_approval".to_string(),
            ..Self::default_for(skill_name)
        }
    }

    pub fn persisted(
        skill_name: &str,
        request_id: Option<&str>,
        change_type: &str,
        applied_count: usize,
        rejected_count: usize,
        pending_count: usize,
        errors: &[String],
    ) -> Self {
        let error_list = errors.to_vec();
        let has_partial = pending_count > 0 || rejected_count > 0 || !error_list.is_empty();
        let status = if has_partial { "partial" } else { "persisted" };
        Self {
            skill_name: skill_name.to_string(),
            request_id: request_id.map(|s| s.to_string()),
            effect: STATE_EFFECT.to_string(),
            change_type: change_type.to_string(),
            applied_count,
            rejected_count,
            pending_count,
            status: status.to_string(),
            errors: error_list,
            metadata: BTreeMap::new(),
        }
    }

    pub fn rejected(
        skill_name: &str,
        request_id: Option<&str>,
        change_type: &str,
        rejected_count: usize,
    ) -> Self {
        Self {
            skill_name: skill_name.to_string(),
            request_id: request_id.map(|s| s.to_string()),
            effect: STATE_EFFECT.to_string(),
            change_type: change_type.to_string(),
            rejected_count,
            status: "rejected".to_string(),
            ..Self::default_for(skill_name)
        }
    }

    fn default_for(skill_name: &str) -> Self {
        Self {
            skill_name: skill_name.to_string(),
            request_id: None,
            effect: String::new(),
            change_type: SKILL_EXPERIENCE_ENTRY.to_string(),
            applied_count: 0,
            rejected_count: 0,
            pending_count: 0,
            status: String::new(),
            errors: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }
}

/// 应用结果(对齐 ExperienceApplyResult)。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExperienceApplyResult {
    pub skill_name: String,
    #[serde(default)]
    pub applied_count: usize,
    #[serde(default)]
    pub rejected_count: usize,
    #[serde(default)]
    pub pending_count: usize,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl ExperienceApplyResult {
    pub fn ok(&self) -> bool {
        self.errors.is_empty() && self.pending_count == 0
    }

    /// Host 面视图(纯拒绝→rejected,否则 persisted/partial;对齐 to_host_result)。
    pub fn to_host_result(
        &self,
        request_id: Option<&str>,
        change_type: &str,
    ) -> HostFacingExperienceResult {
        let pure_rejection = self.rejected_count > 0
            && self.applied_count == 0
            && self.pending_count == 0
            && self.errors.is_empty();
        if pure_rejection {
            HostFacingExperienceResult::rejected(
                &self.skill_name,
                request_id,
                change_type,
                self.rejected_count,
            )
        } else {
            HostFacingExperienceResult::persisted(
                &self.skill_name,
                request_id,
                change_type,
                self.applied_count,
                self.rejected_count,
                self.pending_count,
                &self.errors,
            )
        }
    }
}

/// 在线演进状态(对齐 OnlineEvolutionStatus)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineEvolutionStatus {
    Staged,
    AutoApproved,
    NoEvolutionNoRecords,
    SkippedNoInput,
    SkippedSkillNotFound,
    SkippedSkillDefinitionNotFound,
    PersistenceFailed,
    GenerationFailed,
}

impl OnlineEvolutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::AutoApproved => "auto_approved",
            Self::NoEvolutionNoRecords => "no_evolution_no_records",
            Self::SkippedNoInput => "skipped_no_input",
            Self::SkippedSkillNotFound => "skipped_skill_not_found",
            Self::SkippedSkillDefinitionNotFound => "skipped_skill_definition_not_found",
            Self::PersistenceFailed => "persistence_failed",
            Self::GenerationFailed => "generation_failed",
        }
    }
}

/// 终结态集合(对齐 ONLINE_EVOLUTION_OUTCOME_STATUSES)。
pub fn online_evolution_outcome_statuses() -> std::collections::BTreeSet<&'static str> {
    [
        "no_evolution_no_records",
        "generation_failed",
        "persistence_failed",
        "skipped_skill_definition_not_found",
    ]
    .into_iter()
    .collect()
}

/// 在线结果的待审批请求暴露规则(对齐 request_for_online_evolution_result)。
pub fn request_for_online_evolution_result(status: OnlineEvolutionStatus) -> bool {
    let status_str = status.as_str();
    if online_evolution_outcome_statuses().contains(status_str)
        && status_str != "persistence_failed"
    {
        return false;
    }
    true
}

/// 审批请求(对齐 ExperienceApprovalRequest)。
#[derive(Debug, Clone, PartialEq)]
pub struct ExperienceApprovalRequest {
    pub skill_name: String,
    pub proposal: ExperienceProposal,
    pub pending_change: Option<PendingChange>,
    pub request_id: Option<String>,
}

impl ExperienceApprovalRequest {
    /// Host 面视图(对齐 to_host_result:pending_approval)。
    pub fn to_host_result(&self) -> HostFacingExperienceResult {
        let pending_count = self
            .pending_change
            .as_ref()
            .map(|p| p.payload.len())
            .unwrap_or(0);
        let change_type = self
            .pending_change
            .as_ref()
            .map(|p| p.change_type.clone())
            .unwrap_or_else(|| SKILL_EXPERIENCE_ENTRY.to_string());
        HostFacingExperienceResult::pending_approval(
            &self.skill_name,
            self.request_id.as_deref().unwrap_or(""),
            &change_type,
            pending_count,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recs(n: usize) -> Vec<RecordView> {
        (0..n)
            .map(|i| RecordView {
                id: format!("r{i}"),
                summary: Some(format!("s{i}")),
                score: 0.5,
                timestamp: "t".to_string(),
                target: "body".to_string(),
                section: "Troubleshooting".to_string(),
                content: "c".to_string(),
            })
            .collect()
    }

    #[test]
    fn pending_change_make_shape() {
        let pending = PendingChange::make("sk", &recs(2), None, None);
        assert_eq!(pending.operator_id, "skill_experience_sk");
        assert_eq!(pending.change_type, "skill_experience_entry");
        assert_eq!(pending.payload.len(), 2);
        assert!(pending.change_id.starts_with("skill_evolve_"));
        assert!(pending.created_at.contains('T'));
        assert!(pending.created_at.ends_with("+00:00"));
        assert!(!pending.is_shared_records);
        let shared = PendingChange::make_for_shared_records("sk", &recs(1), None, None);
        assert!(shared.is_shared_records);
    }

    #[test]
    fn apply_result_ok_and_host_views() {
        let ok = ExperienceApplyResult {
            skill_name: "sk".to_string(),
            applied_count: 2,
            ..Default::default()
        };
        assert!(ok.ok());
        let host = ok.to_host_result(Some("req1"), "skill_experience_entry");
        assert_eq!(host.status, "persisted");
        assert_eq!(host.effect, "state");
        assert_eq!(host.applied_count, 2);
        let partial = ExperienceApplyResult {
            skill_name: "sk".to_string(),
            applied_count: 1,
            pending_count: 1,
            ..Default::default()
        };
        assert_eq!(
            partial
                .to_host_result(None, "skill_experience_entry")
                .status,
            "partial"
        );
        let rejected = ExperienceApplyResult {
            skill_name: "sk".to_string(),
            rejected_count: 3,
            ..Default::default()
        };
        let rhost = rejected.to_host_result(Some("req2"), "skill_experience_entry");
        assert_eq!(rhost.status, "rejected");
        assert_eq!(rhost.effect, "state");
        assert_eq!(rhost.rejected_count, 3);
        // ok = 无错误且无 pending;纯拒绝同样为 ok(与 Python 属性一致)
        assert!(rejected.ok());
    }

    #[test]
    fn pending_approval_host_result() {
        let pending = PendingChange::make("sk", &recs(2), None, None);
        let req = ExperienceApprovalRequest {
            skill_name: "sk".to_string(),
            proposal: ExperienceProposal {
                skill_name: "sk".to_string(),
                records: recs(2),
                requires_approval: true,
                source: "experience_optimizer".to_string(),
                user_query: String::new(),
                signal_type: None,
                signal_source: None,
            },
            pending_change: Some(pending),
            request_id: Some("req1".to_string()),
        };
        assert_eq!(req.proposal.record_count(), 2);
        let host = req.to_host_result();
        assert_eq!(host.status, "pending_approval");
        assert_eq!(host.effect, "pending_change");
        assert_eq!(host.pending_count, 2);
        assert_eq!(host.request_id.as_deref(), Some("req1"));
    }

    #[test]
    fn online_status_values_and_request_rule() {
        assert_eq!(OnlineEvolutionStatus::Staged.as_str(), "staged");
        assert_eq!(
            OnlineEvolutionStatus::NoEvolutionNoRecords.as_str(),
            "no_evolution_no_records"
        );
        // 终结态(除 persistence_failed)不暴露请求
        assert!(!request_for_online_evolution_result(
            OnlineEvolutionStatus::NoEvolutionNoRecords
        ));
        assert!(!request_for_online_evolution_result(
            OnlineEvolutionStatus::GenerationFailed
        ));
        assert!(request_for_online_evolution_result(
            OnlineEvolutionStatus::PersistenceFailed
        ));
        assert!(request_for_online_evolution_result(
            OnlineEvolutionStatus::Staged
        ));
        assert_eq!(online_evolution_outcome_statuses().len(), 4);
    }

    #[test]
    fn civil_date_conversion() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }
}
