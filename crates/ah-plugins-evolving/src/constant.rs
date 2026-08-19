//! 自进化超参默认值与合法边界(对齐 agent_evolving/constant.py)。
//!
//! 纯常量:训练/推理/采样默认值 + 迭代/并行/样例数量边界;校验复用 utils 的数值边界检查。

use crate::utils::validate_digital_parameter;

/// 超参默认值与校验边界(对齐 TuneConstant)。
pub struct TuneConstant;

impl TuneConstant {
    // ---- 默认值 ----
    pub const DEFAULT_EXAMPLE_NUM: i64 = 1;
    pub const DEFAULT_ITERATION_NUM: i64 = 3;
    pub const DEFAULT_MAX_SAMPLED_EXAMPLE_NUM: i64 = 10;
    pub const DEFAULT_PARALLEL_NUM: i64 = 1;
    pub const DEFAULT_MAX_NUM_SAMPLE_ERROR_CASES: i64 = 10;
    pub const DEFAULT_EARLY_STOP_SCORE: f64 = 1.0;

    // ---- 合法边界 ----
    pub const MIN_ITERATION_NUM: i64 = 1;
    pub const MAX_ITERATION_NUM: i64 = 20;
    pub const MIN_PARALLEL_NUM: i64 = 1;
    pub const MAX_PARALLEL_NUM: i64 = 20;
    pub const MIN_EXAMPLE_NUM: i64 = 0;
    pub const MAX_EXAMPLE_NUM: i64 = 20;
}

/// 整数参数边界校验(错误消息对齐 Python 的 "should be between X and Y")。
fn validate_int(param: i64, param_name: &str, lower: i64, upper: i64) -> Result<(), String> {
    validate_digital_parameter(param as f64, param_name, lower as f64, upper as f64)
}

/// num_parallel ∈ [MIN_PARALLEL_NUM, MAX_PARALLEL_NUM](对齐 evaluator 的 batch_evaluate 校验)。
pub fn validate_num_parallel(num_parallel: i64) -> Result<(), String> {
    validate_int(
        num_parallel,
        "num_parallel",
        TuneConstant::MIN_PARALLEL_NUM,
        TuneConstant::MAX_PARALLEL_NUM,
    )
}

/// num_iterations ∈ [MIN_ITERATION_NUM, MAX_ITERATION_NUM]。
pub fn validate_num_iterations(num_iterations: i64) -> Result<(), String> {
    validate_int(
        num_iterations,
        "num_iterations",
        TuneConstant::MIN_ITERATION_NUM,
        TuneConstant::MAX_ITERATION_NUM,
    )
}

/// num_examples ∈ [MIN_EXAMPLE_NUM, MAX_EXAMPLE_NUM](对齐 example_optimizer 的样例数校验)。
pub fn validate_example_num(example_num: i64) -> Result<(), String> {
    validate_int(
        example_num,
        "num_examples",
        TuneConstant::MIN_EXAMPLE_NUM,
        TuneConstant::MAX_EXAMPLE_NUM,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_python() {
        assert_eq!(TuneConstant::DEFAULT_EXAMPLE_NUM, 1);
        assert_eq!(TuneConstant::DEFAULT_ITERATION_NUM, 3);
        assert_eq!(TuneConstant::DEFAULT_MAX_SAMPLED_EXAMPLE_NUM, 10);
        assert_eq!(TuneConstant::DEFAULT_PARALLEL_NUM, 1);
        assert_eq!(TuneConstant::DEFAULT_MAX_NUM_SAMPLE_ERROR_CASES, 10);
        assert_eq!(TuneConstant::DEFAULT_EARLY_STOP_SCORE, 1.0);
    }

    #[test]
    fn bounds_match_python() {
        assert_eq!(
            (
                TuneConstant::MIN_ITERATION_NUM,
                TuneConstant::MAX_ITERATION_NUM
            ),
            (1, 20)
        );
        assert_eq!(
            (
                TuneConstant::MIN_PARALLEL_NUM,
                TuneConstant::MAX_PARALLEL_NUM
            ),
            (1, 20)
        );
        assert_eq!(
            (TuneConstant::MIN_EXAMPLE_NUM, TuneConstant::MAX_EXAMPLE_NUM),
            (0, 20)
        );
    }

    #[test]
    fn num_parallel_validation() {
        assert!(validate_num_parallel(1).is_ok());
        assert!(validate_num_parallel(20).is_ok());
        assert!(validate_num_parallel(0).is_err());
        assert!(validate_num_parallel(21).is_err());
        let err = validate_num_parallel(0).unwrap_err();
        assert_eq!(err, "num_parallel should be between 1 and 20");
    }

    #[test]
    fn num_iterations_validation() {
        assert!(validate_num_iterations(3).is_ok());
        assert!(validate_num_iterations(20).is_ok());
        assert!(validate_num_iterations(21).is_err());
        assert!(validate_num_iterations(0).is_err());
    }

    #[test]
    fn example_num_validation_allows_zero() {
        assert!(validate_example_num(0).is_ok());
        assert!(validate_example_num(20).is_ok());
        assert!(validate_example_num(21).is_err());
        assert!(validate_example_num(-1).is_err());
    }
}
