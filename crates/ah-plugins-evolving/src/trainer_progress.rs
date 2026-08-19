//! 训练进度状态机(对齐 trainer/progress.py)。
//!
//! 纯逻辑:Progress(当前/最大 epoch、批迭代、最优分数、当前轮分数)+ run_epoch/run_batch
//! 迭代序列 + Callbacks 生命周期钩子 trait(默认空实现)。

use crate::constant::TuneConstant;

/// 训练进度(对齐 Progress)。
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub start_epoch: usize,
    pub current_epoch: usize,
    pub max_epoch: usize,
    pub current_batch_iter: usize,
    pub max_batch_iter: usize,
    pub best_score: f64,
    pub best_batch_score: f64,
    pub current_epoch_score: f64,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            start_epoch: 0,
            current_epoch: 0,
            max_epoch: TuneConstant::DEFAULT_ITERATION_NUM as usize,
            current_batch_iter: 0,
            max_batch_iter: 1,
            best_score: 0.0,
            best_batch_score: 0.0,
            current_epoch_score: 0.0,
        }
    }
}

impl Progress {
    /// 迭代 start_epoch+1 ..= max_epoch,每轮更新 current_epoch(对齐 run_epoch)。
    /// 结束后 current_epoch 收敛到 max_epoch。
    pub fn run_epoch(&mut self) -> Vec<usize> {
        let mut epochs = Vec::new();
        let start = self.start_epoch + 1;
        for epoch in start..=self.max_epoch {
            self.current_epoch = epoch;
            epochs.push(epoch);
        }
        if self.current_epoch < self.max_epoch {
            self.current_epoch = self.max_epoch;
        }
        epochs
    }

    /// 迭代批步骤,先重置 best_batch_score(对齐 run_batch)。
    pub fn run_batch(&mut self) -> Vec<usize> {
        self.best_batch_score = 0.0;
        let mut steps = Vec::new();
        for batch_iter in 0..self.max_batch_iter {
            self.current_batch_iter = batch_iter;
            steps.push(batch_iter);
        }
        steps
    }
}

/// 训练生命周期钩子(对齐 Callbacks;默认空实现,可覆写)。
pub trait TrainCallbacks {
    fn on_train_begin(&self) {}
    fn on_train_end(&self) {}
    fn on_train_epoch_begin(&self) {}
    fn on_train_epoch_end(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_progress_values() {
        let p = Progress::default();
        assert_eq!(p.start_epoch, 0);
        assert_eq!(p.current_epoch, 0);
        assert_eq!(p.max_epoch, 3); // TuneConstant 默认迭代数
        assert_eq!(p.max_batch_iter, 1);
        assert_eq!(p.best_score, 0.0);
    }

    #[test]
    fn run_epoch_sequence_and_final_state() {
        let mut p = Progress::default();
        let epochs = p.run_epoch();
        assert_eq!(epochs, vec![1, 2, 3]);
        assert_eq!(p.current_epoch, 3);
    }

    #[test]
    fn run_epoch_from_start_epoch() {
        let mut p = Progress {
            start_epoch: 1,
            max_epoch: 4,
            ..Default::default()
        };
        let epochs = p.run_epoch();
        assert_eq!(epochs, vec![2, 3, 4]);
        assert_eq!(p.current_epoch, 4);
    }

    #[test]
    fn run_epoch_keeps_max_when_already_at_max() {
        let mut p = Progress {
            current_epoch: 5,
            max_epoch: 3,
            ..Default::default()
        };
        // start=0 → 迭代 1..=3,current_epoch 已 ≥ max 不调整
        let epochs = p.run_epoch();
        assert_eq!(epochs, vec![1, 2, 3]);
        assert_eq!(p.current_epoch, 3);
    }

    #[test]
    fn run_batch_resets_best_and_yields_steps() {
        let mut p = Progress {
            best_batch_score: 0.9,
            max_batch_iter: 3,
            ..Default::default()
        };
        let steps = p.run_batch();
        assert_eq!(steps, vec![0, 1, 2]);
        assert_eq!(p.current_batch_iter, 2);
        assert_eq!(p.best_batch_score, 0.0);
    }

    #[test]
    fn callbacks_default_noop() {
        struct Noop;
        impl TrainCallbacks for Noop {}
        let c = Noop;
        c.on_train_begin();
        c.on_train_end();
        c.on_train_epoch_begin();
        c.on_train_epoch_end();
    }
}
