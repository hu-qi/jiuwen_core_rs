//! Seam 标记。

/// Seam 标记 trait:可替换能力的契约。
///
/// 一个 Seam 由三方构成:
/// - **Service Definition**:本 trait(或一组方法)声明的接口;
/// - **Service Provider**:插件对该接口的实现;
/// - **Consumer**:消费方(通常是模型可见工具)。
///
/// 实现方与消费方只依赖契约 crate,彼此不直接依赖。
pub trait Seam: Send + Sync + 'static {}
