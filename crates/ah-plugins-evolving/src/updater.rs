//! Updater 协议与单维/多维更新编排。

use crate::dataset::EvaluatedCase;
use crate::signal::{EvolutionSignal, from_evaluated_case};
use ah_contracts::evolving::{
    Evaluation, Trajectory, UpdateEffect, UpdateMode, UpdateValue, Verdict,
};
use ah_contracts::optimizer::{Optimizer, TextualGradient};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub use ah_contracts::updater::{UpdateMap, UpdaterConfig, UpdaterError};

/// 统一单维/多维 updater 接口。
pub trait Updater: Send + Sync {
    /// 绑定可优化算子 ID,返回实际绑定数量。
    fn bind(&mut self, operator_ids: &[String], targets: Option<&[String]>) -> usize;

    /// 是否需要框架先执行 forward 数据。
    fn requires_forward_data(&self) -> bool;

    /// 直接消费已构造的演化信号。
    fn process(
        &self,
        trajectories: &[Trajectory],
        signals: &[EvolutionSignal],
        config: &UpdaterConfig,
    ) -> Result<UpdateMap, UpdaterError>;

    /// 兼容离线 EvaluatedCase 输入。
    fn update(
        &self,
        trajectories: &[Trajectory],
        evaluated_cases: &[EvaluatedCase],
        config: &UpdaterConfig,
    ) -> Result<UpdateMap, UpdaterError>;

    /// 可持久化状态。
    fn get_state(&self) -> Value;

    /// 加载同一 updater 的状态。
    fn load_state(&mut self, state: &Value) -> Result<(), UpdaterError>;
}

fn evaluation_from_signal(signal: &EvolutionSignal) -> Evaluation {
    let score = signal
        .context
        .as_ref()
        .and_then(|context| context.get("score"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    Evaluation {
        verdict: if score > 0.0 {
            Verdict::NeedsWork
        } else {
            Verdict::Fail
        },
        score,
        strengths: vec![],
        issues: vec![signal.excerpt.clone()],
        feedback: signal
            .context
            .as_ref()
            .and_then(|context| context.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn updates_from_gradients(gradients: Vec<TextualGradient>) -> UpdateMap {
    gradients
        .into_iter()
        .map(|gradient| {
            let key = (gradient.operator_id, gradient.parameter);
            let value = UpdateValue {
                payload: Value::String(gradient.gradient),
                mode: UpdateMode::Replace,
                effect: UpdateEffect::State,
                change_type: Some("optimizer_update".to_string()),
                metadata: Map::from_iter([(String::from("source"), json!("updater"))]),
            };
            (key, value)
        })
        .collect()
}

/// 单维 updater:所有已绑定算子交给同一个 optimizer。
pub struct SingleDimUpdater {
    optimizer: Arc<dyn Optimizer>,
    bound: BTreeSet<String>,
}

impl SingleDimUpdater {
    pub fn new(optimizer: Arc<dyn Optimizer>) -> Self {
        Self {
            optimizer,
            bound: BTreeSet::new(),
        }
    }

    fn signals_for_cases(
        &self,
        cases: &[EvaluatedCase],
        threshold: Option<f64>,
    ) -> Vec<EvolutionSignal> {
        let operator_id = self
            .bound
            .iter()
            .next()
            .map(String::as_str)
            .unwrap_or_default();
        cases
            .iter()
            .filter_map(|case| from_evaluated_case(case, operator_id, threshold))
            .collect()
    }
}

impl Updater for SingleDimUpdater {
    fn bind(&mut self, operator_ids: &[String], targets: Option<&[String]>) -> usize {
        let target_set = targets.map(|items| items.iter().collect::<BTreeSet<_>>());
        self.bound = operator_ids
            .iter()
            .filter(|id| {
                target_set
                    .as_ref()
                    .is_none_or(|targets| targets.contains(id))
            })
            .cloned()
            .collect();
        self.bound.len()
    }

    fn requires_forward_data(&self) -> bool {
        false
    }

    fn process(
        &self,
        _trajectories: &[Trajectory],
        signals: &[EvolutionSignal],
        _config: &UpdaterConfig,
    ) -> Result<UpdateMap, UpdaterError> {
        if self.bound.is_empty() {
            return Ok(BTreeMap::new());
        }
        let evaluations: Vec<_> = signals.iter().map(evaluation_from_signal).collect();
        let gradients = self
            .optimizer
            .backward(&evaluations)
            .into_iter()
            .filter(|gradient| self.bound.contains(&gradient.operator_id))
            .collect();
        Ok(updates_from_gradients(gradients))
    }

    fn update(
        &self,
        trajectories: &[Trajectory],
        evaluated_cases: &[EvaluatedCase],
        config: &UpdaterConfig,
    ) -> Result<UpdateMap, UpdaterError> {
        let signals = self.signals_for_cases(evaluated_cases, config.score_threshold);
        self.process(trajectories, &signals, config)
    }

    fn get_state(&self) -> Value {
        json!({"version": 1, "bound_operator_ids": self.bound.iter().collect::<Vec<_>>()})
    }

    fn load_state(&mut self, state: &Value) -> Result<(), UpdaterError> {
        let version = state.get("version").and_then(Value::as_u64).unwrap_or(0);
        if version != 1 {
            return Err(UpdaterError(format!(
                "unsupported updater state version: {version}"
            )));
        }
        let ids = state
            .get("bound_operator_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| UpdaterError("updater state missing bound_operator_ids".into()))?;
        self.bound = ids
            .iter()
            .map(|id| {
                id.as_str().map(str::to_string).ok_or_else(|| {
                    UpdaterError("updater state contains non-string operator id".into())
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(())
    }
}

fn domain_for_operator(operator_id: &str) -> Option<&'static str> {
    if operator_id.contains("tool") {
        Some("tool")
    } else if operator_id.contains("memory") {
        Some("memory")
    } else if operator_id.contains("llm") {
        Some("llm")
    } else {
        None
    }
}

fn domain_for_signal(signal: &EvolutionSignal) -> &'static str {
    if signal
        .context
        .as_ref()
        .and_then(|context| context.get("tool_name"))
        .is_some()
        || signal.excerpt.to_lowercase().contains("tool")
    {
        "tool"
    } else if signal.excerpt.to_lowercase().contains("memory") {
        "memory"
    } else {
        "llm"
    }
}

/// 多维 updater:按 LLM/tool/memory 域分发到各自 optimizer。
pub struct MultiDimUpdater {
    optimizers: BTreeMap<String, Arc<dyn Optimizer>>,
    bound: BTreeMap<String, BTreeSet<String>>,
}

impl MultiDimUpdater {
    pub fn new(optimizers: BTreeMap<String, Arc<dyn Optimizer>>) -> Self {
        Self {
            optimizers,
            bound: BTreeMap::new(),
        }
    }
}

impl Updater for MultiDimUpdater {
    fn bind(&mut self, operator_ids: &[String], targets: Option<&[String]>) -> usize {
        let target_set = targets.map(|items| items.iter().collect::<BTreeSet<_>>());
        self.bound.clear();
        for id in operator_ids {
            if target_set
                .as_ref()
                .is_some_and(|targets| !targets.contains(id))
            {
                continue;
            }
            if let Some(domain) = domain_for_operator(id)
                && self.optimizers.contains_key(domain)
            {
                self.bound
                    .entry(domain.to_string())
                    .or_default()
                    .insert(id.clone());
            }
        }
        self.bound.values().map(BTreeSet::len).sum()
    }

    fn requires_forward_data(&self) -> bool {
        false
    }

    fn process(
        &self,
        _trajectories: &[Trajectory],
        signals: &[EvolutionSignal],
        _config: &UpdaterConfig,
    ) -> Result<UpdateMap, UpdaterError> {
        let mut updates = BTreeMap::new();
        for (domain, optimizer) in &self.optimizers {
            let evaluations: Vec<_> = signals
                .iter()
                .filter(|signal| domain_for_signal(signal) == domain)
                .map(evaluation_from_signal)
                .collect();
            if !evaluations.is_empty() {
                let gradients = optimizer
                    .backward(&evaluations)
                    .into_iter()
                    .filter(|gradient| {
                        self.bound.get(domain).is_some_and(|operator_ids| {
                            operator_ids.contains(&gradient.operator_id)
                        })
                    })
                    .collect();
                updates.extend(updates_from_gradients(gradients));
            }
        }
        Ok(updates)
    }

    fn update(
        &self,
        trajectories: &[Trajectory],
        evaluated_cases: &[EvaluatedCase],
        config: &UpdaterConfig,
    ) -> Result<UpdateMap, UpdaterError> {
        let operator_id = self
            .bound
            .values()
            .flat_map(BTreeSet::iter)
            .next()
            .map(String::as_str)
            .unwrap_or_default();
        let signals = evaluated_cases
            .iter()
            .filter_map(|case| from_evaluated_case(case, operator_id, config.score_threshold))
            .collect::<Vec<_>>();
        self.process(trajectories, &signals, config)
    }

    fn get_state(&self) -> Value {
        json!({
            "version": 1,
            "bound": self.bound.iter().map(|(domain, ids)| {
                let values = ids.iter().cloned().map(Value::String).collect();
                (domain.clone(), Value::Array(values))
            }).collect::<Map<_, _>>()
        })
    }

    fn load_state(&mut self, state: &Value) -> Result<(), UpdaterError> {
        let version = state.get("version").and_then(Value::as_u64).unwrap_or(0);
        if version != 1 {
            return Err(UpdaterError(format!(
                "unsupported updater state version: {version}"
            )));
        }
        let bound = state
            .get("bound")
            .and_then(Value::as_object)
            .ok_or_else(|| UpdaterError("updater state missing bound".into()))?;
        let mut restored = BTreeMap::new();
        for (domain, values) in bound {
            if !self.optimizers.contains_key(domain) {
                return Err(UpdaterError(format!(
                    "updater state has unknown domain: {domain}"
                )));
            }
            let values = values.as_array().ok_or_else(|| {
                UpdaterError(format!("updater state domain is not an array: {domain}"))
            })?;
            let ids = values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        UpdaterError("updater state contains non-string operator id".into())
                    })
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            restored.insert(domain.clone(), ids);
        }
        self.bound = restored;
        Ok(())
    }
}

/// 将信号型 updater 适配为跨插件共享的 contracts seam。
pub struct UpdaterService {
    inner: std::sync::Mutex<SingleDimUpdater>,
}

impl UpdaterService {
    pub fn new(inner: SingleDimUpdater) -> Self {
        Self {
            inner: std::sync::Mutex::new(inner),
        }
    }
}

impl ah_contracts::seam::Seam for UpdaterService {}

fn signal_from_evaluation(evaluation: &Evaluation) -> EvolutionSignal {
    EvolutionSignal {
        signal_type: if evaluation.verdict == Verdict::Fail {
            "low_score"
        } else {
            "evaluated"
        }
        .to_string(),
        section: "Troubleshooting".to_string(),
        excerpt: evaluation.issues.join("; "),
        skill_name: None,
        context: Some(BTreeMap::from([
            ("score".to_string(), json!(evaluation.score)),
            (
                "reason".to_string(),
                Value::String(evaluation.feedback.clone()),
            ),
        ])),
    }
}

impl ah_contracts::updater::Updater for UpdaterService {
    fn bind(&self, operator_ids: &[String], targets: Option<&[String]>) -> usize {
        self.inner.lock().unwrap().bind(operator_ids, targets)
    }

    fn requires_forward_data(&self) -> bool {
        self.inner.lock().unwrap().requires_forward_data()
    }

    fn process(
        &self,
        trajectories: &[Trajectory],
        evaluations: &[Evaluation],
        config: &ah_contracts::updater::UpdaterConfig,
    ) -> Result<ah_contracts::updater::UpdateMap, ah_contracts::updater::UpdaterError> {
        let signals = evaluations
            .iter()
            .map(signal_from_evaluation)
            .collect::<Vec<_>>();
        self.inner
            .lock()
            .unwrap()
            .process(trajectories, &signals, config)
    }

    fn get_state(&self) -> Value {
        self.inner.lock().unwrap().get_state()
    }

    fn load_state(&self, state: &Value) -> Result<(), ah_contracts::updater::UpdaterError> {
        self.inner.lock().unwrap().load_state(state)
    }
}

/// 注册共享 updater seam,由 optimizer 与 operator registry 提供运行时依赖。
pub struct UpdaterPlugin;

impl ah_hub::plugin::Plugin for UpdaterPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-updater"
    }

    fn provides(&self) -> Vec<ah_contracts::service::ServiceKey> {
        vec![ah_contracts::keys::UPDATER]
    }

    fn inject(&self) -> Vec<ah_contracts::service::ServiceKey> {
        vec![ah_contracts::keys::OPTIMIZER, ah_contracts::keys::OPERATOR]
    }

    fn apply(
        &self,
        ctx: &ah_hub::context::Context,
    ) -> Result<Vec<ah_contracts::prelude::Effect>, ah_hub::plugin::PluginError> {
        let optimizer = ctx
            .service::<dyn ah_contracts::optimizer::Optimizer>(&ah_contracts::keys::OPTIMIZER)
            .ok_or_else(|| ah_hub::plugin::PluginError::Apply {
                plugin: self.name(),
                message: "optimizer seam not registered".to_string(),
            })?;
        let registry = ctx
            .service::<dyn ah_contracts::operator::OperatorRegistry>(&ah_contracts::keys::OPERATOR)
            .ok_or_else(|| ah_hub::plugin::PluginError::Apply {
                plugin: self.name(),
                message: "operator seam not registered".to_string(),
            })?;
        let mut updater = SingleDimUpdater::new(optimizer);
        updater.bind(&registry.ids(), None);
        let service: Arc<dyn ah_contracts::updater::Updater> =
            Arc::new(UpdaterService::new(updater));
        Ok(vec![ctx.register(ah_contracts::keys::UPDATER, service)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{Case, EvaluatedCase};
    use crate::signal::EvolutionSignal;
    use ah_contracts::evolving::Evaluation;
    use ah_contracts::operator::OperatorRegistry;
    use ah_contracts::optimizer::{Optimizer, TextualGradient, UpdateResult};
    use ah_contracts::seam::Seam;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    struct FakeOptimizer;

    impl Seam for FakeOptimizer {}

    impl Optimizer for FakeOptimizer {
        fn backward(&self, evaluations: &[Evaluation]) -> Vec<TextualGradient> {
            evaluations
                .iter()
                .map(|evaluation| TextualGradient {
                    operator_id: if evaluation.issues.iter().any(|issue| issue.contains("tool")) {
                        "agent/tool_call".to_string()
                    } else {
                        "agent/llm_call".to_string()
                    },
                    parameter: "system_prompt".to_string(),
                    gradient: format!("fix: {}", evaluation.issues.join(", ")),
                    issue: evaluation.issues.first().cloned().unwrap_or_default(),
                })
                .collect()
        }

        fn step(&self, _: &[TextualGradient], _: &dyn OperatorRegistry) -> Vec<UpdateResult> {
            vec![]
        }

        fn apply_updates(
            &self,
            _: &[Evaluation],
            _: &dyn OperatorRegistry,
        ) -> Result<Vec<UpdateResult>, ah_contracts::optimizer::OptimizerError> {
            Ok(vec![])
        }
    }

    fn signal(excerpt: &str) -> EvolutionSignal {
        EvolutionSignal {
            signal_type: "evaluated".into(),
            section: "Troubleshooting".into(),
            excerpt: excerpt.into(),
            skill_name: None,
            context: None,
        }
    }

    fn evaluated(reason: &str, score: f64) -> EvaluatedCase {
        EvaluatedCase::new(
            Case::new(BTreeMap::new(), BTreeMap::new(), None),
            Some(json!("answer")),
            score,
            reason,
            None,
        )
    }

    #[test]
    fn single_dim_binds_targets_and_converts_signals_to_updates() {
        let mut updater = SingleDimUpdater::new(Arc::new(FakeOptimizer));
        assert_eq!(
            updater.bind(
                &["agent/llm_call".into(), "op-b".into()],
                Some(&["agent/llm_call".into()])
            ),
            1
        );
        let updates = updater
            .process(&[], &[signal("bad answer")], &UpdaterConfig::default())
            .unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(
            updates[&("agent/llm_call".into(), "system_prompt".into())].payload,
            json!("fix: bad answer")
        );
    }

    #[test]
    fn single_dim_filters_evaluated_cases_by_threshold_and_restores_state() {
        let mut updater = SingleDimUpdater::new(Arc::new(FakeOptimizer));
        updater.bind(&["agent/llm_call".into()], None);
        let updates = updater
            .update(
                &[],
                &[evaluated("weak", 0.1), evaluated("good", 0.9)],
                &UpdaterConfig {
                    score_threshold: Some(0.5),
                },
            )
            .unwrap();
        assert_eq!(updates.len(), 1);
        let state = updater.get_state();
        let mut restored = SingleDimUpdater::new(Arc::new(FakeOptimizer));
        restored.load_state(&state).unwrap();
        assert_eq!(restored.get_state(), state);
    }

    #[test]
    fn multi_dim_routes_signals_to_domain_optimizers() {
        let mut optimizers = BTreeMap::new();
        optimizers.insert("llm".into(), Arc::new(FakeOptimizer) as Arc<dyn Optimizer>);
        optimizers.insert("tool".into(), Arc::new(FakeOptimizer) as Arc<dyn Optimizer>);
        let mut updater = MultiDimUpdater::new(optimizers);
        assert_eq!(
            updater.bind(&["agent/llm_call".into(), "agent/tool_call".into()], None),
            2
        );
        let mut tool = signal("tool failed");
        tool.context = Some(BTreeMap::from([("tool_name".into(), json!("shell"))]));
        let updates = updater
            .process(
                &[],
                &[signal("answer failed"), tool],
                &UpdaterConfig::default(),
            )
            .unwrap();
        assert_eq!(updates.len(), 2);
        assert!(updates.contains_key(&("agent/llm_call".into(), "system_prompt".into())));
    }

    #[test]
    fn updater_rejects_empty_bind_and_invalid_state() {
        let mut updater = SingleDimUpdater::new(Arc::new(FakeOptimizer));
        assert_eq!(updater.bind(&[], None), 0);
        assert!(updater.load_state(&json!({"version": 99})).is_err());
    }
    #[test]
    fn updater_plugin_registers_shared_service_and_generates_bound_update() {
        use ah_contracts::keys::UPDATER;
        use ah_contracts::updater::Updater as ContractUpdater;
        use ah_hub::context::Context;
        use ah_hub::plugin::DynPlugin;
        use std::sync::Arc as StdArc;

        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                StdArc::new(ah_plugins_operator::OperatorPlugin) as DynPlugin,
                StdArc::new(ah_plugins_optimizer::OptimizerPlugin) as DynPlugin,
                StdArc::new(UpdaterPlugin) as DynPlugin,
            ])
            .expect("mount updater provider");
        let updater = ctx
            .service::<dyn ContractUpdater>(&UPDATER)
            .expect("updater seam");
        let result = updater
            .process(
                &[],
                &[Evaluation {
                    verdict: Verdict::Fail,
                    score: 0.0,
                    strengths: vec![],
                    issues: vec!["tool output failed".into()],
                    feedback: "retry".into(),
                }],
                &UpdaterConfig::default(),
            )
            .expect("generate update");
        assert!(result.contains_key(&("agent/tool_call".into(), "tool_description".into())));
        drop(effects);
    }
}
