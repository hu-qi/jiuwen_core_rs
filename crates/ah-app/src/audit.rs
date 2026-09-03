//! Structured capability audit ledger and deterministic Markdown summary.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AuditLedger {
    pub schema_version: u64,
    pub ledger_id: String,
    pub recorded_at: String,
    pub source: AuditSource,
    pub work_packages: Vec<WorkPackage>,
}

#[derive(Debug, Deserialize)]
pub struct AuditSource {
    pub repository: String,
    pub code_revision: String,
    pub reference_revision: String,
    pub status_policy: String,
    pub percentage_policy: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditStatus {
    Done,
    Partial,
    Missing,
    Excluded,
}

impl AuditStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Partial => "partial",
            Self::Missing => "missing",
            Self::Excluded => "excluded",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkPackage {
    pub id: String,
    pub phase: String,
    pub domain: String,
    pub title: String,
    pub status: AuditStatus,
    #[serde(default)]
    pub implementation: Vec<String>,
    #[serde(default)]
    pub verification: Vec<String>,
    pub production: String,
    pub differential: String,
}

#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    done: usize,
    partial: usize,
    missing: usize,
    excluded: usize,
}

impl Counts {
    fn add(&mut self, status: AuditStatus) {
        match status {
            AuditStatus::Done => self.done += 1,
            AuditStatus::Partial => self.partial += 1,
            AuditStatus::Missing => self.missing += 1,
            AuditStatus::Excluded => self.excluded += 1,
        }
    }

    fn total(self) -> usize {
        self.done + self.partial + self.missing + self.excluded
    }
}

pub fn load(path: &Path) -> Result<AuditLedger, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read audit ledger {}: {error}", path.display()))?;
    let ledger: AuditLedger = serde_json::from_str(&text)
        .map_err(|error| format!("parse audit ledger {}: {error}", path.display()))?;
    validate(&ledger)?;
    Ok(ledger)
}

pub fn validate(ledger: &AuditLedger) -> Result<(), String> {
    if ledger.schema_version != 1 {
        return Err(format!(
            "unsupported audit ledger schema version: {}",
            ledger.schema_version
        ));
    }
    if ledger.ledger_id.trim().is_empty() || ledger.recorded_at.trim().is_empty() {
        return Err("audit ledger id and recorded_at are required".to_string());
    }
    if ledger.source.repository.trim().is_empty()
        || ledger.source.code_revision.trim().is_empty()
        || ledger.source.status_policy.trim().is_empty()
        || ledger.source.percentage_policy.trim().is_empty()
    {
        return Err("audit ledger source metadata is incomplete".to_string());
    }
    if ledger.work_packages.is_empty() {
        return Err("audit ledger must contain at least one work package".to_string());
    }

    let mut ids = std::collections::BTreeSet::new();
    for package in &ledger.work_packages {
        if package.id.trim().is_empty()
            || package.phase.trim().is_empty()
            || package.domain.trim().is_empty()
            || package.title.trim().is_empty()
        {
            return Err("audit work package id, phase, domain, and title are required".to_string());
        }
        if !ids.insert(&package.id) {
            return Err(format!("duplicate audit work package id: {}", package.id));
        }
        if package.production.trim().is_empty() || package.differential.trim().is_empty() {
            return Err(format!("audit evidence is incomplete for {}", package.id));
        }
    }
    Ok(())
}

pub fn render_summary(ledger: &AuditLedger) -> String {
    let mut overall = Counts::default();
    let mut domains: BTreeMap<&str, Counts> = BTreeMap::new();
    for package in &ledger.work_packages {
        overall.add(package.status);
        domains
            .entry(package.domain.as_str())
            .or_default()
            .add(package.status);
    }

    let mut output = String::new();
    output.push_str("# Generated capability audit summary\n\n");
    output.push_str(&format!(
        "> Generated from `audit/ledger.json`; recorded at {}; code revision `{}`; reference revision `{}`.\n\n",
        ledger.recorded_at, ledger.source.code_revision, ledger.source.reference_revision
    ));
    output.push_str("Percentages below are status shares, not weighted capability completion.\n\n");
    output.push_str("## Overall\n\n| Status | Count | Share |\n| --- | ---: | ---: |\n");
    for (label, count) in [
        ("done", overall.done),
        ("partial", overall.partial),
        ("missing", overall.missing),
        ("excluded", overall.excluded),
    ] {
        output.push_str(&format!(
            "| {label} | {count} | {} |\n",
            share(count, overall.total())
        ));
    }
    output.push_str(&format!(
        "| **total** | **{}** | **100.0%** |\n",
        overall.total()
    ));

    output.push_str("\n## Domain status shares\n\n| Domain | Total | Done | Partial | Missing | Excluded | Done share |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for (domain, counts) in domains {
        output.push_str(&format!(
            "| {domain} | {} | {} | {} | {} | {} | {} |\n",
            counts.total(),
            counts.done,
            counts.partial,
            counts.missing,
            counts.excluded,
            share(counts.done, counts.total())
        ));
    }

    output.push_str("\n## Incomplete work packages\n\n| ID | Phase | Domain | Title | Status |\n| --- | --- | --- | --- | --- |\n");
    for package in &ledger.work_packages {
        if package.status != AuditStatus::Done {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                package.id,
                package.phase,
                package.domain,
                package.title,
                package.status.as_str()
            ));
        }
    }

    output.push_str("\n## Evidence ledger\n\n");
    for package in &ledger.work_packages {
        output.push_str(&format!(
            "### {} — {}\n\n- Status: `{}`\n- Implementation: {}\n- Verification: {}\n- Production: {}\n- Differential: {}\n\n",
            package.id,
            package.title,
            package.status.as_str(),
            list_or_none(&package.implementation),
            list_or_none(&package.verification),
            package.production,
            package.differential
        ));
    }
    output
}

pub fn generate(input: &Path, output: &Path) -> Result<(), String> {
    let ledger = load(input)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create audit summary directory: {error}"))?;
    }
    std::fs::write(output, render_summary(&ledger))
        .map_err(|error| format!("write audit summary {}: {error}", output.display()))
}

fn share(value: usize, total: usize) -> String {
    if total == 0 {
        return "0.0%".to_string();
    }
    format!("{:.1}%", value as f64 * 100.0 / total as f64)
}

fn list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values
            .iter()
            .map(|value| format!("`{value}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AuditLedger {
        serde_json::from_str(
            r#"{
              "schema_version": 1,
              "ledger_id": "test",
              "recorded_at": "2026-09-03",
              "source": {
                "repository": "agent-harness",
                "code_revision": "abc",
                "reference_revision": "def",
                "status_policy": "done|partial|missing|excluded",
                "percentage_policy": "status shares"
              },
              "work_packages": [
                {"id":"P0-01","phase":"P0","domain":"core","title":"one","status":"done","production":"ok","differential":"ok"},
                {"id":"P1-01","phase":"P1","domain":"core","title":"two","status":"partial","production":"pending","differential":"pending"},
                {"id":"P3-01","phase":"P3","domain":"rsi","title":"three","status":"missing","production":"none","differential":"n/a"}
              ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn validation_rejects_duplicate_ids() {
        let mut ledger = sample();
        ledger.work_packages[1].id = ledger.work_packages[0].id.clone();
        let error = validate(&ledger).expect_err("duplicate accepted");
        assert!(error.contains("duplicate"));
    }

    #[test]
    fn summary_counts_statuses_and_domains() {
        let ledger = sample();
        validate(&ledger).expect("valid ledger");
        let summary = render_summary(&ledger);
        assert!(summary.contains("| done | 1 | 33.3% |"));
        assert!(summary.contains("| core | 2 | 1 | 1 | 0 | 0 | 50.0% |"));
        assert!(summary.contains("| P3-01 | P3 | rsi | three | missing |"));
    }

    #[test]
    fn zero_length_evidence_is_represented_explicitly() {
        let ledger = sample();
        assert!(render_summary(&ledger).contains("- Implementation: none"));
    }
}
