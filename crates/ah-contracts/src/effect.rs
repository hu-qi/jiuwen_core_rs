//! 可逆注册(RAII guard)。
//!
//! 定义在契约层:seam 接口(如 `ToolRegistry::register`)需要返回它。
//! 注册动作返回本 guard,guard 被 drop(插件卸载 / 作用域结束)时自动回滚。

/// 可逆注册的 RAII guard。
pub struct Effect {
    undo: Option<Box<dyn FnOnce() + Send>>,
}

impl Effect {
    /// 构造一个执行 `undo` 的注册 guard。
    pub fn new(undo: impl FnOnce() + Send + 'static) -> Self {
        Self {
            undo: Some(Box::new(undo)),
        }
    }

    /// 无操作 guard(注册没有需要回滚的副作用)。
    pub fn noop() -> Self {
        Self { undo: None }
    }

    /// 手动执行回滚并消费 guard。
    pub fn dispose(mut self) {
        if let Some(undo) = self.undo.take() {
            undo();
        }
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        if let Some(undo) = self.undo.take() {
            undo();
        }
    }
}

impl core::fmt::Debug for Effect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Effect")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn effect_runs_undo_on_drop() {
        let counter = Arc::new(AtomicUsize::new(0));
        {
            let c = counter.clone();
            let _effect = Effect::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
            assert_eq!(counter.load(Ordering::SeqCst), 0);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn effect_dispose_runs_undo_once() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let effect = Effect::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        effect.dispose();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn noop_effect_is_inert() {
        let effect = Effect::noop();
        effect.dispose();
    }
}
