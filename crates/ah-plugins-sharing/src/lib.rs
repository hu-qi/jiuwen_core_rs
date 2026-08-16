//! # ah-plugins-sharing
//!
//! Real experience sharing (aligned with openjiuwen/agent_evolving/sharing):
//! - LocalSharingBackend: local filesystem hub (packages/bundles/index/outbox layout),
//!   Jaccard keyword dedup and retrieval;
//! - ExperienceSharerImpl: skill-scoped facade (stage/dedup queue, flush with retry &
//!   backoff, download mirror cache, skill package sync, keyword search).
//!
//! Hub layout (dir injectable; tests use temp dirs):
//! packages/{skill_id}/skill.tar.gz + meta.json; bundles/{skill_id}/{bundle_id}.json;
//! index/{skill_id}.jsonl bundle keyword index + index/global.jsonl global skill index;
//! .outbox reserved.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use ah_contracts::keys::SHARING;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::sharing::{
    ExperienceSharer, QueryKeywords, SharedBackend, SharedExperience, SharedSkillBundle,
    SharedSkillProvider, SharingError, SkillPackageMeta, SkillSearchResult, UploadResult,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// Bundle id generator (process-local counter, sb_ prefix aligned with Python).
static BUNDLE_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_bundle_id() -> String {
    let seq = BUNDLE_SEQ.fetch_add(1, Ordering::SeqCst);
    format!("sb_{:010x}", seq)
}

/// Jaccard keyword similarity (aligned with Python: lowercase sets; empty -> 0;
/// substring containment fallback 0.5/|union|).
pub fn jaccard(a: &[String], b: &[String]) -> f64 {
    let set_a: HashSet<String> = a
        .iter()
        .filter(|k| !k.is_empty())
        .map(|k| k.to_lowercase())
        .collect();
    let set_b: HashSet<String> = b
        .iter()
        .filter(|k| !k.is_empty())
        .map(|k| k.to_lowercase())
        .collect();
    if set_a.is_empty() && set_b.is_empty() {
        return 0.0;
    }
    let intersection: HashSet<&String> = set_a.intersection(&set_b).collect();
    let union_len = set_a.union(&set_b).count();
    if !intersection.is_empty() {
        return intersection.len() as f64 / union_len as f64;
    }
    for ka in &set_a {
        for kb in &set_b {
            if ka.contains(kb.as_str()) || kb.contains(ka.as_str()) {
                return 0.5 / union_len.max(1) as f64;
            }
        }
    }
    0.0
}

/// Local filesystem hub: real persistence (bundle JSON / package bytes / index JSONL).
pub struct LocalSharingBackend {
    hub_path: PathBuf,
    dedup_jaccard_threshold: f64,
}

impl LocalSharingBackend {
    /// Create backend at hub root (creates directory structure).
    pub fn new(hub_path: impl Into<PathBuf>, dedup_jaccard_threshold: f64) -> Self {
        let hub_path = hub_path.into();
        for sub in ["packages", "bundles", "index", ".outbox"] {
            let _ = fs::create_dir_all(hub_path.join(sub));
        }
        Self {
            hub_path,
            dedup_jaccard_threshold,
        }
    }

    pub fn hub_path(&self) -> &Path {
        &self.hub_path
    }

    fn package_dir(&self, skill_id: &str) -> PathBuf {
        self.hub_path.join("packages").join(skill_id)
    }

    fn package_archive(&self, skill_id: &str) -> PathBuf {
        self.package_dir(skill_id).join("skill.tar.gz")
    }

    fn package_meta_path(&self, skill_id: &str) -> PathBuf {
        self.package_dir(skill_id).join("meta.json")
    }

    fn bundle_dir(&self, skill_id: &str) -> PathBuf {
        self.hub_path.join("bundles").join(skill_id)
    }

    fn index_path(&self, skill_id: &str) -> PathBuf {
        self.hub_path
            .join("index")
            .join(format!("{skill_id}.jsonl"))
    }

    fn global_index_path(&self) -> PathBuf {
        self.hub_path.join("index").join("global.jsonl")
    }

    /// Read per-skill bundle index lines (each {bundle_id, keywords}).
    fn read_index(&self, skill_id: &str) -> Vec<serde_json::Value> {
        let Ok(text) = fs::read_to_string(self.index_path(skill_id)) else {
            return Vec::new();
        };
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .collect()
    }

    /// Read global index lines (each {skill_id, bundle_id, keywords}).
    fn read_global_index(&self) -> Vec<serde_json::Value> {
        let Ok(text) = fs::read_to_string(self.global_index_path()) else {
            return Vec::new();
        };
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .collect()
    }

    /// Duplicate rejection: Jaccard >= threshold with any existing bundle rejects.
    fn duplicate_rejection_reason(&self, skill_id: &str, keywords: &[String]) -> Option<String> {
        if keywords.is_empty() {
            return None;
        }
        for entry in self.read_index(skill_id) {
            let existing: Vec<String> = entry
                .get("keywords")
                .and_then(|k| k.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let score = jaccard(keywords, &existing);
            if score >= self.dedup_jaccard_threshold {
                let existing_id = entry
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                return Some(format!(
                    "keywords overlap existing bundle {existing_id} (jaccard={score:.2}, threshold={:.2})",
                    self.dedup_jaccard_threshold
                ));
            }
        }
        None
    }
}

impl Seam for LocalSharingBackend {}

#[async_trait::async_trait]
impl ah_contracts::sharing::SharingBackend for LocalSharingBackend {
    async fn upload_bundle(
        &self,
        mut bundle: SharedSkillBundle,
    ) -> Result<UploadResult, SharingError> {
        let skill_id = bundle.skill_id.trim().to_string();
        if skill_id.is_empty() {
            return Ok(UploadResult::rejected(
                "bundle.skill_id is required for upload",
                false,
            ));
        }
        if let Some(reason) = self.duplicate_rejection_reason(&skill_id, &bundle.keywords_aggregate)
        {
            return Ok(UploadResult::rejected(reason, false));
        }
        if bundle.bundle_id.is_empty() {
            bundle.bundle_id = next_bundle_id();
        }
        let dir = self.bundle_dir(&skill_id);
        fs::create_dir_all(&dir)
            .map_err(|e| SharingError(format!("create bundle dir failed: {e}")))?;
        let file = dir.join(format!("{}.json", bundle.bundle_id));
        let json = serde_json::to_string_pretty(&bundle)
            .map_err(|e| SharingError(format!("serialize bundle failed: {e}")))?;
        fs::write(&file, json).map_err(|e| SharingError(format!("write bundle failed: {e}")))?;
        // Append bundle index and global index.
        let index_line = serde_json::json!({
            "bundle_id": bundle.bundle_id,
            "keywords": bundle.keywords_aggregate,
        });
        append_jsonl(&self.index_path(&skill_id), &index_line)?;
        let global_line = serde_json::json!({
            "skill_id": skill_id,
            "bundle_id": bundle.bundle_id,
            "keywords": bundle.keywords_aggregate,
        });
        append_jsonl(&self.global_index_path(), &global_line)?;
        Ok(UploadResult::ok(bundle.bundle_id))
    }

    async fn download_bundles(
        &self,
        skill_id: &str,
        query: &QueryKeywords,
        top_k: usize,
    ) -> Result<Vec<SharedSkillBundle>, SharingError> {
        let dir = self.bundle_dir(skill_id);
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut scored: Vec<(f64, SharedSkillBundle)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(bundle) = serde_json::from_str::<SharedSkillBundle>(&text) else {
                continue;
            };
            let score = jaccard(&query.keywords, &bundle.keywords_aggregate);
            scored.push((score, bundle));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(top_k).map(|(_, b)| b).collect())
    }

    async fn has_skill_package(&self, skill_id: &str) -> Result<bool, SharingError> {
        Ok(self.package_meta_path(skill_id).exists())
    }

    async fn upload_skill_package(
        &self,
        skill_id: &str,
        package_bytes: &[u8],
        meta: &SkillPackageMeta,
    ) -> Result<(), SharingError> {
        // Existing -> no-op (hub keeps only the first version).
        if self.package_meta_path(skill_id).exists() {
            return Ok(());
        }
        let dir = self.package_dir(skill_id);
        fs::create_dir_all(&dir)
            .map_err(|e| SharingError(format!("create package dir failed: {e}")))?;
        fs::write(self.package_archive(skill_id), package_bytes)
            .map_err(|e| SharingError(format!("write package failed: {e}")))?;
        let meta_json = serde_json::to_string_pretty(meta)
            .map_err(|e| SharingError(format!("serialize meta failed: {e}")))?;
        fs::write(self.package_meta_path(skill_id), meta_json)
            .map_err(|e| SharingError(format!("write meta failed: {e}")))?;
        Ok(())
    }

    async fn download_skill_package(
        &self,
        skill_id: &str,
    ) -> Result<Option<Vec<u8>>, SharingError> {
        let path = self.package_archive(skill_id);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(_) => Ok(None),
        }
    }

    async fn get_skill_package_meta(
        &self,
        skill_id: &str,
    ) -> Result<Option<SkillPackageMeta>, SharingError> {
        let path = self.package_meta_path(skill_id);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| SharingError(format!("parse meta failed: {e}"))),
            Err(_) => Ok(None),
        }
    }

    async fn search_skills(
        &self,
        query: &QueryKeywords,
        top_k: usize,
    ) -> Result<Vec<SkillSearchResult>, SharingError> {
        // Aggregate global index by skill_id: experience count, keyword union, max Jaccard.
        let mut by_skill: HashMap<String, (u64, Vec<String>, f64)> = HashMap::new();
        for line in self.read_global_index() {
            let Some(skill_id) = line
                .get("skill_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
            else {
                continue;
            };
            let keywords: Vec<String> = line
                .get("keywords")
                .and_then(|k| k.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let score = jaccard(&query.keywords, &keywords);
            let entry = by_skill.entry(skill_id).or_insert((0, Vec::new(), 0.0));
            entry.0 += 1;
            for kw in keywords {
                if !entry.1.contains(&kw) {
                    entry.1.push(kw);
                }
            }
            entry.2 = entry.2.max(score);
        }
        let mut results: Vec<SkillSearchResult> = by_skill
            .into_iter()
            .map(|(skill_id, (experience_count, keywords, score))| {
                // Enrich with package metadata when available.
                let (skill_name, description) =
                    fs::read_to_string(self.package_meta_path(&skill_id))
                        .ok()
                        .and_then(|text| serde_json::from_str::<SkillPackageMeta>(&text).ok())
                        .map(|m| (m.skill_name, m.description))
                        .unwrap_or_default();
                SkillSearchResult {
                    skill_id,
                    skill_name,
                    description,
                    experience_count,
                    keywords,
                    score,
                }
            })
            .collect();
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(results.into_iter().take(top_k).collect())
    }
}

fn append_jsonl(path: &Path, value: &serde_json::Value) -> Result<(), SharingError> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| SharingError(format!("open index failed: {e}")))?;
    let line = serde_json::to_string(value)
        .map_err(|e| SharingError(format!("serialize index line failed: {e}")))?;
    writeln!(file, "{line}").map_err(|e| SharingError(format!("append index failed: {e}")))?;
    Ok(())
}
/// Skill-scoped sharing facade: stage queue -> flush bundled upload (retry/backoff)
/// plus download mirror cache.
pub struct ExperienceSharerImpl {
    backend: SharedBackend,
    local_cache_dir: Option<PathBuf>,
    max_upload_retries: u32,
    backoff_base_secs: f64,
    skill_provider: Mutex<Option<SharedSkillProvider>>,
    pending: Mutex<HashMap<String, Vec<SharedExperience>>>,
    pending_keys: Mutex<HashMap<String, HashSet<(String, String)>>>,
}

impl ExperienceSharerImpl {
    /// Create with backend + local cache dir; retries/backoff default 3 / 0.5s.
    pub fn new(
        backend: SharedBackend,
        local_cache_dir: Option<PathBuf>,
        max_upload_retries: u32,
        backoff_base_secs: f64,
    ) -> Self {
        Self {
            backend,
            local_cache_dir,
            max_upload_retries: max_upload_retries.max(1),
            backoff_base_secs: backoff_base_secs.max(0.0),
            skill_provider: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            pending_keys: Mutex::new(HashMap::new()),
        }
    }

    /// Late-bind the skill package provider.
    pub fn set_skill_sharing_context_provider(&self, provider: Option<SharedSkillProvider>) {
        *self.skill_provider.lock().unwrap() = provider;
    }

    /// Backend reference.
    pub fn backend(&self) -> &SharedBackend {
        &self.backend
    }

    fn mirror_bundle(&self, bundle: &SharedSkillBundle, kind: &str) {
        let Some(cache) = &self.local_cache_dir else {
            return;
        };
        if bundle.bundle_id.is_empty() || bundle.skill_id.trim().is_empty() {
            return;
        }
        if kind != "uploaded" && kind != "downloaded" {
            return;
        }
        let target_dir = cache.join(kind).join(bundle.skill_id.trim());
        let _ = fs::create_dir_all(&target_dir);
        if let Ok(json) = serde_json::to_string_pretty(bundle) {
            let _ = fs::write(target_dir.join(format!("{}.json", bundle.bundle_id)), json);
        }
    }

    /// Sync skill package: resolve skill_id via provider and upload initial package
    /// when the hub does not have it yet.
    async fn sync_skill_package(&self, bundle: &mut SharedSkillBundle, skill_name: &str) {
        let provider = self.skill_provider.lock().unwrap().clone();
        let Some(provider) = provider else {
            return;
        };
        let Ok((skill_id, package_bytes, resolved_name, description)) =
            provider.provide(skill_name)
        else {
            return;
        };
        let skill_id = skill_id.trim().to_string();
        if skill_id.is_empty() {
            return;
        }
        bundle.skill_id = skill_id.clone();
        if !resolved_name.is_empty() {
            bundle.skill_name = resolved_name.clone();
        }
        let Ok(already) = self.backend.has_skill_package(&skill_id).await else {
            return;
        };
        if already || package_bytes.is_empty() {
            return;
        }
        let meta = SkillPackageMeta {
            skill_id: skill_id.clone(),
            skill_name: if resolved_name.is_empty() {
                skill_name.to_string()
            } else {
                resolved_name
            },
            description,
            uploaded_at: String::new(),
        };
        let _ = self
            .backend
            .upload_skill_package(&skill_id, &package_bytes, &meta)
            .await;
    }
}

impl Seam for ExperienceSharerImpl {}

#[async_trait::async_trait]
impl ExperienceSharer for ExperienceSharerImpl {
    fn resolve_skill_id(&self, skill_name: &str) -> String {
        let provider = self.skill_provider.lock().unwrap().clone();
        let Some(provider) = provider else {
            return String::new();
        };
        if skill_name.is_empty() {
            return String::new();
        }
        match provider.provide(skill_name) {
            Ok((skill_id, _, _, _)) => skill_id.trim().to_string(),
            Err(_) => String::new(),
        }
    }

    fn has_pending(&self, skill_name: &str) -> bool {
        self.pending
            .lock()
            .unwrap()
            .get(skill_name)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    fn stage_for_upload(&self, skill_name: &str, exp: SharedExperience) {
        if skill_name.is_empty() {
            return;
        }
        let record_id = exp.record.id.clone();
        let dedup_key = (skill_name.to_string(), record_id);
        let mut keys = self.pending_keys.lock().unwrap();
        let mut pending = self.pending.lock().unwrap();
        let skill_keys = keys.entry(skill_name.to_string()).or_default();
        if skill_keys.contains(&dedup_key) {
            return;
        }
        skill_keys.insert(dedup_key);
        pending.entry(skill_name.to_string()).or_default().push(exp);
    }

    fn discard_pending_uploads(&self, skill_name: &str) -> usize {
        let count = self
            .pending
            .lock()
            .unwrap()
            .get(skill_name)
            .map(|v| v.len())
            .unwrap_or(0);
        self.pending.lock().unwrap().remove(skill_name);
        self.pending_keys.lock().unwrap().remove(skill_name);
        count
    }

    async fn flush_pending_uploads(&self, skill_name: &str) -> Result<UploadResult, SharingError> {
        let experiences = self.pending.lock().unwrap().remove(skill_name);
        self.pending_keys.lock().unwrap().remove(skill_name);
        let Some(experiences) = experiences else {
            return Ok(UploadResult::ok(""));
        };
        if experiences.is_empty() {
            return Ok(UploadResult::ok(""));
        }
        let mut bundle = SharedSkillBundle::make(skill_name, experiences, "", "");
        self.sync_skill_package(&mut bundle, skill_name).await;
        if bundle.skill_id.trim().is_empty() {
            return Ok(UploadResult::rejected("skill_id unavailable", false));
        }
        let mut attempt = 0u32;
        let mut last = UploadResult::rejected("upload not attempted", false);
        while attempt < self.max_upload_retries {
            attempt += 1;
            let result = self.backend.upload_bundle(bundle.clone()).await?;
            if result.ok {
                bundle.bundle_id = result.bundle_id.clone();
                self.mirror_bundle(&bundle, "uploaded");
                return Ok(result);
            }
            last = result;
            if !last.retryable {
                return Ok(last);
            }
            if attempt < self.max_upload_retries && self.backoff_base_secs > 0.0 {
                let backoff = self.backoff_base_secs * (2u32.pow(attempt - 1)) as f64;
                tokio::time::sleep(std::time::Duration::from_secs_f64(backoff)).await;
            }
        }
        Ok(last)
    }

    async fn download_relevant(
        &self,
        skill_id: &str,
        query: &QueryKeywords,
        top_k: usize,
    ) -> Result<Vec<SharedSkillBundle>, SharingError> {
        let resolved = skill_id.trim().to_string();
        if resolved.is_empty() {
            return Ok(Vec::new());
        }
        let bundles = self
            .backend
            .download_bundles(&resolved, query, top_k)
            .await?;
        for bundle in &bundles {
            self.mirror_bundle(bundle, "downloaded");
        }
        Ok(bundles)
    }

    async fn search_skills(
        &self,
        query: &QueryKeywords,
        top_k: usize,
    ) -> Result<Vec<SkillSearchResult>, SharingError> {
        self.backend.search_skills(query, top_k).await
    }

    async fn download_skill_package(
        &self,
        skill_id: &str,
    ) -> Result<Option<Vec<u8>>, SharingError> {
        self.backend.download_skill_package(skill_id).await
    }

    async fn get_skill_package_meta(
        &self,
        skill_id: &str,
    ) -> Result<Option<SkillPackageMeta>, SharingError> {
        self.backend.get_skill_package_meta(skill_id).await
    }

    fn list_cached_bundles(&self, skill_id: &str) -> Vec<SharedSkillBundle> {
        let Some(cache) = &self.local_cache_dir else {
            return Vec::new();
        };
        let skill_dir = cache.join("downloaded").join(skill_id.trim());
        let Ok(entries) = fs::read_dir(&skill_dir) else {
            return Vec::new();
        };
        let mut bundles = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(bundle) = serde_json::from_str::<SharedSkillBundle>(&text) else {
                continue;
            };
            bundles.push(bundle);
        }
        bundles
    }
}

/// Sharing plugin: registers the `sharing` seam (ExperienceSharer facade).
pub struct SharingPlugin {
    backend: SharedBackend,
    local_cache_dir: Option<PathBuf>,
}

impl SharingPlugin {
    /// Create plugin with backend + local cache dir.
    pub fn new(backend: SharedBackend, local_cache_dir: Option<PathBuf>) -> Self {
        Self {
            backend,
            local_cache_dir,
        }
    }
}

impl Plugin for SharingPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-sharing"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SHARING]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let sharer: Arc<dyn ExperienceSharer> = Arc::new(ExperienceSharerImpl::new(
            self.backend.clone(),
            self.local_cache_dir.clone(),
            3,
            0.5,
        ));
        Ok(vec![ctx.register(SHARING, sharer)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::evolving::{Experience, Verdict};
    use ah_contracts::sharing::{SharingBackend, SkillSharingContextProvider};
    use ah_hub::plugin::DynPlugin;

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ah-sharing-{tag}-{}", std::process::id()))
    }

    fn experience(id: &str, task: &str) -> Experience {
        Experience {
            id: id.to_string(),
            task: task.to_string(),
            verdict: Verdict::Pass,
            score: 0.9,
            issues: vec![],
            saved_ms: 1000,
        }
    }

    fn shared_exp(id: &str, task: &str, keywords: &[&str]) -> SharedExperience {
        let mut exp = SharedExperience::new(experience(id, task));
        exp.keywords = keywords.iter().map(|s| s.to_string()).collect();
        exp.summary = format!("summary-{id}");
        exp
    }

    fn bundle(skill_id: &str, id: &str, keywords: &[&str]) -> SharedSkillBundle {
        let mut b =
            SharedSkillBundle::make("skill-a", vec![shared_exp(id, "task", keywords)], "", "");
        b.skill_id = skill_id.to_string();
        b.bundle_id = id.to_string();
        b
    }

    #[tokio::test]
    async fn backend_upload_persists_bundle_and_indexes() {
        let dir = temp_dir("upload");
        let _ = fs::remove_dir_all(&dir);
        let backend = LocalSharingBackend::new(&dir, 0.85);

        let b = bundle("sk-1", "sb_test1", &["rust", "compiler"]);
        let result = backend.upload_bundle(b).await.expect("upload");
        assert!(result.ok, "upload ok");
        assert_eq!(result.bundle_id, "sb_test1");

        // 束 JSON 真实落盘。
        let file = dir.join("bundles").join("sk-1").join("sb_test1.json");
        assert!(file.exists(), "bundle json persisted");
        let parsed: SharedSkillBundle =
            serde_json::from_str(&fs::read_to_string(&file).unwrap()).expect("parse bundle");
        assert_eq!(parsed.bundle_id, "sb_test1");
        assert_eq!(
            parsed.keywords_aggregate,
            vec!["rust".to_string(), "compiler".to_string()]
        );

        // 索引行真实追加。
        let index = fs::read_to_string(dir.join("index").join("sk-1.jsonl")).unwrap();
        assert_eq!(index.lines().count(), 1);
        let global = fs::read_to_string(dir.join("index").join("global.jsonl")).unwrap();
        assert_eq!(global.lines().count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn backend_rejects_empty_skill_id_and_duplicate_keywords() {
        let dir = temp_dir("reject");
        let _ = fs::remove_dir_all(&dir);
        let backend = LocalSharingBackend::new(&dir, 0.85);

        let no_id = bundle("", "sb_x", &["a"]);
        let result = backend.upload_bundle(no_id.clone()).await.expect("upload");
        assert!(!result.ok);
        assert!(result.reason.contains("skill_id is required"));

        // 正常上传第一个束。
        let b1 = bundle("sk-2", "sb_1", &["rust", "compiler", "borrow"]);
        assert!(backend.upload_bundle(b1).await.unwrap().ok);
        // 高度重叠关键词 → 拒绝(非可重试)。
        let b2 = bundle("sk-2", "sb_2", &["rust", "compiler", "borrow"]);
        let dup = backend.upload_bundle(b2).await.expect("upload");
        assert!(!dup.ok);
        assert!(
            dup.reason.contains("keywords overlap"),
            "reason: {}",
            dup.reason
        );
        assert!(!dup.retryable);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn backend_download_ranks_by_relevance() {
        let dir = temp_dir("rank");
        let _ = fs::remove_dir_all(&dir);
        let backend = LocalSharingBackend::new(&dir, 0.85);
        backend
            .upload_bundle(bundle("sk-3", "sb_a", &["database", "index", "query"]))
            .await
            .unwrap();
        backend
            .upload_bundle(bundle("sk-3", "sb_b", &["cooking", "recipe"]))
            .await
            .unwrap();
        let query = QueryKeywords::new(vec!["database".to_string(), "query".to_string()]);
        let results = backend
            .download_bundles("sk-3", &query, 2)
            .await
            .expect("download");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].bundle_id, "sb_a", "database 束排最前");
        assert_eq!(results[1].bundle_id, "sb_b");
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn backend_skill_package_lifecycle() {
        let dir = temp_dir("pkg");
        let _ = fs::remove_dir_all(&dir);
        let backend = LocalSharingBackend::new(&dir, 0.85);

        assert!(!backend.has_skill_package("sk-9").await.unwrap());
        let meta = SkillPackageMeta::new("sk-9");
        backend
            .upload_skill_package("sk-9", b"package-bytes-1", &meta)
            .await
            .unwrap();
        assert!(backend.has_skill_package("sk-9").await.unwrap());
        // 重传 → no-op(只保留第一版)。
        backend
            .upload_skill_package("sk-9", b"package-bytes-2", &meta)
            .await
            .unwrap();
        let bytes = backend
            .download_skill_package("sk-9")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bytes, b"package-bytes-1", "第一版保留");
        let fetched_meta = backend
            .get_skill_package_meta("sk-9")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched_meta.skill_id, "sk-9");
        assert!(
            backend
                .download_skill_package("sk-missing")
                .await
                .unwrap()
                .is_none()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn backend_search_skills_aggregates_and_ranks() {
        let dir = temp_dir("search");
        let _ = fs::remove_dir_all(&dir);
        let backend = LocalSharingBackend::new(&dir, 0.85);
        backend
            .upload_bundle(bundle("sk-10", "sb_1", &["rust", "compiler"]))
            .await
            .unwrap();
        backend
            .upload_bundle(bundle("sk-10", "sb_2", &["rust", "lifetime"]))
            .await
            .unwrap();
        backend
            .upload_bundle(bundle("sk-11", "sb_3", &["cooking"]))
            .await
            .unwrap();
        let query = QueryKeywords::new(vec!["rust".to_string()]);
        let results = backend.search_skills(&query, 5).await.expect("search");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].skill_id, "sk-10", "rust 相关排最前");
        assert_eq!(results[0].experience_count, 2);
        assert!(results[0].keywords.contains(&"rust".to_string()));
        assert!(results[0].score > 0.0);
        let _ = fs::remove_dir_all(&dir);
    }

    struct TestProvider;

    impl SkillSharingContextProvider for TestProvider {
        fn provide(&self, _skill_name: &str) -> Result<(String, Vec<u8>, String, String), String> {
            Ok((
                "sk-provided".to_string(),
                b"skill-package".to_vec(),
                "Skill A".to_string(),
                "desc".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn sharer_stage_dedup_discard_and_flush_with_mirror() {
        let dir = temp_dir("sharer");
        let _ = fs::remove_dir_all(&dir);
        let hub = dir.join("hub");
        let cache = dir.join("cache");
        let backend: SharedBackend = Arc::new(LocalSharingBackend::new(&hub, 0.85));
        let sharer = ExperienceSharerImpl::new(backend.clone(), Some(cache.clone()), 3, 0.0);
        sharer.set_skill_sharing_context_provider(Some(Arc::new(TestProvider)));

        // stage 去重:同 record.id 只入队一次。
        sharer.stage_for_upload("skill-a", shared_exp("r1", "t1", &["rust"]));
        sharer.stage_for_upload("skill-a", shared_exp("r1", "t1-dup", &["rust"]));
        sharer.stage_for_upload("skill-a", shared_exp("r2", "t2", &["database"]));
        assert!(sharer.has_pending("skill-a"));

        // 丢弃。
        let dropped = sharer.discard_pending_uploads("skill-a");
        assert_eq!(dropped, 2);
        assert!(!sharer.has_pending("skill-a"));

        // 重新 stage 并 flush:打包上传 + 镜像 uploaded + skill 包同步。
        sharer.stage_for_upload("skill-a", shared_exp("r1", "t1", &["rust", "compiler"]));
        let result = sharer
            .flush_pending_uploads("skill-a")
            .await
            .expect("flush");
        assert!(result.ok, "flush ok: {:?}", result.reason);
        assert!(!result.bundle_id.is_empty());

        // hub 上束真实存在 + skill 包已同步。
        assert!(
            hub.join("bundles")
                .join("sk-provided")
                .read_dir()
                .unwrap()
                .next()
                .is_some()
        );
        assert!(backend.has_skill_package("sk-provided").await.unwrap());
        // uploaded 镜像。
        let uploaded = cache.join("uploaded").join("sk-provided");
        assert!(uploaded.read_dir().unwrap().next().is_some());
        assert!(!sharer.has_pending("skill-a"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn sharer_flush_without_provider_reports_skill_id_unavailable() {
        let dir = temp_dir("noprov");
        let _ = fs::remove_dir_all(&dir);
        let backend: SharedBackend = Arc::new(LocalSharingBackend::new(dir.join("hub"), 0.85));
        let sharer = ExperienceSharerImpl::new(backend, None, 3, 0.0);
        sharer.stage_for_upload("skill-b", shared_exp("r1", "t1", &["x"]));
        let result = sharer
            .flush_pending_uploads("skill-b")
            .await
            .expect("flush");
        assert!(!result.ok);
        assert_eq!(result.reason, "skill_id unavailable");
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn sharer_download_mirrors_and_lists_cached_bundles() {
        let dir = temp_dir("download");
        let _ = fs::remove_dir_all(&dir);
        let hub = dir.join("hub");
        let cache = dir.join("cache");
        let backend: SharedBackend = Arc::new(LocalSharingBackend::new(&hub, 0.85));
        // 直接经后端上传两个束。
        backend
            .upload_bundle(bundle("sk-20", "sb_d1", &["rust", "borrow"]))
            .await
            .unwrap();
        backend
            .upload_bundle(bundle("sk-20", "sb_d2", &["database"]))
            .await
            .unwrap();

        let sharer = ExperienceSharerImpl::new(backend, Some(cache.clone()), 3, 0.0);
        let query = QueryKeywords::new(vec!["rust".to_string()]);
        let bundles = sharer
            .download_relevant("sk-20", &query, 2)
            .await
            .expect("download");
        assert_eq!(bundles.len(), 2);
        assert_eq!(bundles[0].bundle_id, "sb_d1", "rust 相关最相关");

        // 下载镜像 + list_cached_bundles 读回。
        let cached = sharer.list_cached_bundles("sk-20");
        assert_eq!(cached.len(), 2, "下载束已镜像到本地缓存");
        let ids: Vec<String> = cached.iter().map(|b| b.bundle_id.clone()).collect();
        assert!(ids.contains(&"sb_d1".to_string()));
        assert!(ids.contains(&"sb_d2".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plugin_registers_sharing_seam() {
        let dir = temp_dir("plugin");
        let _ = fs::remove_dir_all(&dir);
        let backend: SharedBackend = Arc::new(LocalSharingBackend::new(dir.join("hub"), 0.85));
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(SharingPlugin::new(backend, Some(dir.join("cache"))));
        let effects = ctx.mount(&plugin).expect("mount");
        let sharer = ctx
            .service::<dyn ExperienceSharer>(&SHARING)
            .expect("sharing seam");
        assert!(!sharer.has_pending("anything"));
        drop(effects);
        assert!(!ctx.has_service(&SHARING), "卸载后反注册");
        let _ = fs::remove_dir_all(&dir);
    }
}
