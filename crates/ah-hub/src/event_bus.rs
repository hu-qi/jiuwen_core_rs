//! 事件总线:emit / serial / parallel / waterfall 四种分发。

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use ah_contracts::event::Event;

use crate::effect::Effect;

type EmitListener = Arc<dyn Fn(&dyn Any) + Send + Sync>;
type AsyncListener =
    Arc<dyn Fn(&dyn Any) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
type ErasedValue = Box<dyn Any + Send>;
type ErasedFut = Pin<Box<dyn Future<Output = ErasedValue> + Send>>;
type ErasedNext = Box<dyn FnOnce(ErasedValue) -> ErasedFut + Send>;

/// waterfall 监听器的类型擦除载体(内部用)。
trait WaterfallHandler: Send + Sync {
    fn call(&self, event: &dyn Any, value: ErasedValue, next: ErasedNext) -> ErasedFut;
}

struct TypedWaterfall<E, R, F> {
    handler: F,
    _marker: PhantomData<fn(E) -> R>,
}

impl<E, R, F, Fut> WaterfallHandler for TypedWaterfall<E, R, F>
where
    E: Event,
    R: Send + 'static,
    F: Fn(E, R, Next<R>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
{
    fn call(&self, event: &dyn Any, value: ErasedValue, next: ErasedNext) -> ErasedFut {
        let event = event
            .downcast_ref::<E>()
            .expect("waterfall handler event type mismatch")
            .clone();
        let value = *value
            .downcast::<R>()
            .expect("waterfall handler value type mismatch");
        let next = Next::<R> {
            inner: next,
            _marker: PhantomData,
        };
        let fut = (self.handler)(event, value, next);
        Box::pin(async move { Box::new(fut.await) as ErasedValue })
    }
}

#[derive(Clone)]
struct EmitSlot {
    id: u64,
    listener: EmitListener,
}

#[derive(Clone)]
struct AsyncSlot {
    id: u64,
    listener: AsyncListener,
}

#[derive(Clone)]
struct WaterfallSlot {
    id: u64,
    handler: Arc<dyn WaterfallHandler>,
}

/// 事件总线。
///
/// 分发模式(对齐 DSH/Cordis):
/// - `EventBus::emit`:同步、按注册顺序通知(观察);
/// - `EventBus::serial`:异步、按注册顺序逐个 await(串行);
/// - `EventBus::parallel`:异步、并发执行所有监听器(扇出);
/// - `EventBus::waterfall`:异步、监听器可调用 `next()` 委托/短路(包装链)。
#[derive(Default)]
pub struct EventBus {
    next_id: AtomicU64,
    emit: RwLock<HashMap<TypeId, Vec<EmitSlot>>>,
    serial: RwLock<HashMap<TypeId, Vec<AsyncSlot>>>,
    parallel: RwLock<HashMap<TypeId, Vec<AsyncSlot>>>,
    waterfall: RwLock<HashMap<TypeId, Vec<WaterfallSlot>>>,
}

impl EventBus {
    /// 创建空事件总线。
    pub fn new() -> Self {
        Self::default()
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// 注册一个 emit 监听器(同步、按注册顺序通知)。
    pub fn on<E: Event>(self: &Arc<Self>, listener: impl Fn(E) + Send + Sync + 'static) -> Effect {
        let trampoline: EmitListener = Arc::new(move |any: &dyn Any| {
            let event = any
                .downcast_ref::<E>()
                .expect("emit listener event type mismatch")
                .clone();
            listener(event);
        });
        self.insert_emit(TypeId::of::<E>(), trampoline)
    }

    /// 注册一个 serial 监听器(异步、按注册顺序逐个 await)。
    pub fn on_serial<E, F, Fut>(self: &Arc<Self>, listener: F) -> Effect
    where
        E: Event,
        F: Fn(E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let trampoline: AsyncListener = Arc::new(move |any: &dyn Any| {
            let event = any
                .downcast_ref::<E>()
                .expect("serial listener event type mismatch")
                .clone();
            Box::pin(listener(event))
        });
        self.insert_async(TypeId::of::<E>(), &self.serial, trampoline)
    }

    /// 注册一个 parallel 监听器(异步、并发执行)。
    pub fn on_parallel<E, F, Fut>(self: &Arc<Self>, listener: F) -> Effect
    where
        E: Event,
        F: Fn(E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let trampoline: AsyncListener = Arc::new(move |any: &dyn Any| {
            let event = any
                .downcast_ref::<E>()
                .expect("parallel listener event type mismatch")
                .clone();
            Box::pin(listener(event))
        });
        self.insert_async(TypeId::of::<E>(), &self.parallel, trampoline)
    }

    /// 注册一个 waterfall 监听器(异步、`next()` 委托 / 短路)。
    pub fn on_waterfall<E, R, F, Fut>(self: &Arc<Self>, handler: F) -> Effect
    where
        E: Event,
        R: Send + 'static,
        F: Fn(E, R, Next<R>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        let wrapped: Arc<dyn WaterfallHandler> = Arc::new(TypedWaterfall::<E, R, F> {
            handler,
            _marker: PhantomData,
        });
        let bus = Arc::clone(self);
        let key = TypeId::of::<E>();
        let id = self.next_id();
        {
            let mut guard = self.waterfall.write().expect("bus poisoned");
            guard.entry(key).or_default().push(WaterfallSlot {
                id,
                handler: wrapped,
            });
        }
        Effect::new(move || {
            if let Some(slots) = bus.waterfall.write().expect("bus poisoned").get_mut(&key) {
                slots.retain(|slot| slot.id != id);
            }
        })
    }

    fn insert_emit(self: &Arc<Self>, key: TypeId, listener: EmitListener) -> Effect {
        let bus = Arc::clone(self);
        let id = self.next_id();
        {
            let mut guard = self.emit.write().expect("bus poisoned");
            guard
                .entry(key)
                .or_default()
                .push(EmitSlot { id, listener });
        }
        Effect::new(move || {
            if let Some(slots) = bus.emit.write().expect("bus poisoned").get_mut(&key) {
                slots.retain(|slot| slot.id != id);
            }
        })
    }

    fn insert_async(
        self: &Arc<Self>,
        key: TypeId,
        map: &RwLock<HashMap<TypeId, Vec<AsyncSlot>>>,
        listener: AsyncListener,
    ) -> Effect {
        let bus = Arc::clone(self);
        let id = self.next_id();
        {
            let mut guard = map.write().expect("bus poisoned");
            guard
                .entry(key)
                .or_default()
                .push(AsyncSlot { id, listener });
        }
        Effect::new(move || {
            if let Some(slots) = bus.serial.write().expect("bus poisoned").get_mut(&key) {
                // 两个 async 表都尝试清理,无害。
                slots.retain(|slot| slot.id != id);
            }
            if let Some(slots) = bus.parallel.write().expect("bus poisoned").get_mut(&key) {
                slots.retain(|slot| slot.id != id);
            }
        })
    }

    /// emit:同步通知所有监听器(按注册顺序)。
    pub fn emit<E: Event>(&self, event: E) {
        let slots = self
            .emit
            .read()
            .expect("bus poisoned")
            .get(&TypeId::of::<E>())
            .cloned()
            .unwrap_or_default();
        for slot in slots {
            (slot.listener)(&event);
        }
    }

    /// serial:按注册顺序逐个 await。
    pub async fn serial<E: Event>(&self, event: E) {
        let slots = self
            .serial
            .read()
            .expect("bus poisoned")
            .get(&TypeId::of::<E>())
            .cloned()
            .unwrap_or_default();
        for slot in slots {
            (slot.listener)(&event).await;
        }
    }

    /// parallel:并发执行所有监听器并等待全部完成。
    pub async fn parallel<E: Event>(&self, event: E) {
        let slots = self
            .parallel
            .read()
            .expect("bus poisoned")
            .get(&TypeId::of::<E>())
            .cloned()
            .unwrap_or_default();
        let futures: Vec<_> = slots
            .into_iter()
            .map(|slot| (slot.listener)(&event))
            .collect();
        futures_util::future::join_all(futures).await;
    }

    /// waterfall:监听器按注册顺序执行,收到 `(event, value, next)`;
    /// 调用 [`Next::next`] 把(可能被包装的)值委托给下游,或直接返回结果短路。
    pub async fn waterfall<E: Event, R: Send + 'static>(&self, event: E, initial: R) -> R {
        let handlers: Vec<Arc<dyn WaterfallHandler>> = self
            .waterfall
            .read()
            .expect("bus poisoned")
            .get(&TypeId::of::<E>())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|slot| slot.handler)
            .collect();

        // 从链尾构建:最后一个监听器的 next 是透传。
        let mut next: ErasedNext = Box::new(|value: ErasedValue| Box::pin(async move { value }));
        for slot in handlers.iter().rev() {
            let handler = slot.clone();
            let previous = next;
            let event = event.clone();
            next = Box::new(move |value: ErasedValue| {
                let handler = handler.clone();
                let previous = previous;
                let event = event.clone();
                Box::pin(async move { handler.call(&event, value, previous).await })
            });
        }

        let result = next(Box::new(initial)).await;
        *result
            .downcast::<R>()
            .expect("waterfall result type mismatch")
    }
}

/// waterfall 的委托句柄:监听器调用 [`Next::next`] 把(可能被包装的)值传给下游。
pub struct Next<R> {
    inner: ErasedNext,
    _marker: PhantomData<fn(R)>,
}

impl<R: Send + 'static> Next<R> {
    /// 委托给链上的下一个监听器;若已是链尾则原样返回。
    pub async fn next(self, value: R) -> R {
        let erased: ErasedValue = Box::new(value);
        let result = (self.inner)(erased).await;
        *result
            .downcast::<R>()
            .expect("waterfall next type mismatch")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone, Debug)]
    struct Ping;

    impl Event for Ping {
        const ID: &'static str = "ping";
    }

    #[test]
    fn emit_runs_listeners_in_order() {
        let bus = Arc::new(EventBus::new());
        let order = Arc::new(Mutex::new(Vec::new()));

        let o1 = order.clone();
        let _l1 = bus.on::<Ping>(move |_e| o1.lock().unwrap().push(1));
        let o2 = order.clone();
        let _l2 = bus.on::<Ping>(move |_e| o2.lock().unwrap().push(2));

        bus.emit(Ping);
        assert_eq!(*order.lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn emit_effect_removes_listener() {
        let bus = Arc::new(EventBus::new());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = calls.clone();
        let effect = bus.on::<Ping>(move |_e| {
            c.fetch_add(1, Ordering::SeqCst);
        });

        bus.emit(Ping);
        drop(effect);
        bus.emit(Ping);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn serial_awaits_in_order() {
        let bus = Arc::new(EventBus::new());
        let order = Arc::new(Mutex::new(Vec::new()));

        let o1 = order.clone();
        let _l1 = bus.on_serial::<Ping, _, _>(move |_e| {
            let o = o1.clone();
            async move { o.lock().unwrap().push(1) }
        });
        let o2 = order.clone();
        let _l2 = bus.on_serial::<Ping, _, _>(move |_e| {
            let o = o2.clone();
            async move { o.lock().unwrap().push(2) }
        });

        bus.serial(Ping).await;
        assert_eq!(*order.lock().unwrap(), vec![1, 2]);
    }

    #[tokio::test]
    async fn parallel_runs_all_listeners() {
        let bus = Arc::new(EventBus::new());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c1 = calls.clone();
        let _l1 = bus.on_parallel::<Ping, _, _>(move |_e| {
            let c = c1.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
            }
        });
        let c2 = calls.clone();
        let _l2 = bus.on_parallel::<Ping, _, _>(move |_e| {
            let c = c2.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
            }
        });

        bus.parallel(Ping).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn waterfall_delegates_through_chain() {
        let bus = Arc::new(EventBus::new());
        // h1(先注册):委托后 +1;h2:委托后 *2。
        let _h1 = bus.on_waterfall::<Ping, i32, _, _>(|_e, v, next| async move {
            let delegated = next.next(v).await;
            delegated + 1
        });
        let _h2 = bus.on_waterfall::<Ping, i32, _, _>(|_e, v, next| async move {
            let delegated = next.next(v).await;
            delegated * 2
        });

        // 链尾透传 1 → h2: 1*2=2 → h1: 2+1=3
        let result = bus.waterfall(Ping, 1).await;
        assert_eq!(result, 3);
    }

    #[tokio::test]
    async fn waterfall_short_circuits_without_next() {
        let bus = Arc::new(EventBus::new());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let _h1 = bus.on_waterfall::<Ping, i32, _, _>(|_e, _v, _next| async move { 42 });
        let c = calls.clone();
        let _h2 = bus.on_waterfall::<Ping, i32, _, _>(move |_e, v, next| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                next.next(v).await
            }
        });

        let result = bus.waterfall(Ping, 1).await;
        assert_eq!(result, 42);
        // h2 不应被执行(短路)。
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn waterfall_without_listeners_returns_initial() {
        let bus = Arc::new(EventBus::new());
        let result = bus.waterfall(Ping, 7).await;
        assert_eq!(result, 7);
    }
}
