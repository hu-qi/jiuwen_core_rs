//! updater service contract shared by RSI and evolving plugins.

use crate::evolving::{Evaluation, Trajectory, UpdateKey, UpdateValue};
use crate::seam::Seam;
use serde_json::Value;

/// updater 可选配置。
#[derive(Debug, Clone, Default)]
pub struct UpdaterConfig {
    /// 低于该分数的离线样本才生成更新信号。
    pub score_threshold: Option<f64>,
}

/// updater 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdaterError(pub String);

impl core::fmt::Display for UpdaterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UpdaterError {}

/// updater 生成的结构化更新集合。
pub type UpdateMap = std::collections::BTreeMap<UpdateKey, UpdateValue>;

/// RSI trainer 与 evolving provider 共享的 updater seam。
pub trait Updater: Seam + Send + Sync {
    /// 绑定可优化算子 ID,返回实际绑定数量。
    fn bind(&self, operator_ids: &[String], targets: Option<&[String]>) -> usize;

    /// 是否需要框架先执行 forward 数据。
    fn requires_forward_data(&self) -> bool;

    /// 直接消费已评估轨迹。
    fn process(
        &self,
        trajectories: &[Trajectory],
        evaluations: &[Evaluation],
        config: &UpdaterConfig,
    ) -> Result<UpdateMap, UpdaterError>;

    /// 可持久化状态。
    fn get_state(&self) -> Value;

    /// 加载同一 updater 的状态。
    fn load_state(&self, state: &Value) -> Result<(), UpdaterError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolving::{Evaluation, Trajectory};
    use crate::seam::Seam;

    struct Noop;
    impl Seam for Noop {}

    #[test]
    fn config_and_error_are_stable_contract_types() {
        assert_eq!(UpdaterConfig::default().score_threshold, None);
        assert_eq!(UpdaterError("x".into()).to_string(), "x");
        let _: &dyn Updater = &Noop;
    }

    impl Updater for Noop {
        fn bind(&self, _: &[String], _: Option<&[String]>) -> usize {
            0
        }

        fn requires_forward_data(&self) -> bool {
            false
        }

        fn process(
            &self,
            _: &[Trajectory],
            _: &[Evaluation],
            _: &UpdaterConfig,
        ) -> Result<UpdateMap, UpdaterError> {
            Ok(UpdateMap::new())
        }

        fn get_state(&self) -> serde_json::Value {
            serde_json::json!({"version": 1})
        }

        fn load_state(&self, _: &serde_json::Value) -> Result<(), UpdaterError> {
            Ok(())
        }
    }
}
