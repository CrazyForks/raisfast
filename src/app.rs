//! Service Locator — type-safe, lock-free component registry for AppState.
//!
//! Two-phase design:
//! 1. **Build phase**: [`ServiceRegistryBuilder`] accepts `Arc<T>` registrations.
//! 2. **Runtime phase**: [`ServiceRegistry`] is frozen, read-only, lock-free.
//!
//! # Example
//!
//! ```ignore
//! let mut builder = ServiceRegistryBuilder::new();
//! builder.register(Arc::new(my_service) as Arc<dyn MyTrait>);
//! let registry = builder.build();
//!
//! // Lock-free, infallible resolve:
//! let svc: Arc<dyn MyTrait> = registry.resolve::<dyn MyTrait>();
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Build-phase registry — mutable, not `Clone`, consumed by [`build`](ServiceRegistryBuilder::build).
pub struct ServiceRegistryBuilder {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Default for ServiceRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRegistryBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Register a service. The key is `T` (the outermost type of the `Arc`).
    ///
    /// For trait objects, use the `as Arc<dyn Trait>` cast at the call site.
    pub fn register<T: Send + Sync + 'static + ?Sized>(&mut self, service: Arc<T>) {
        self.map.insert(TypeId::of::<T>(), Box::new(service));
    }

    /// Freeze into a read-only [`ServiceRegistry`].
    pub fn build(self) -> ServiceRegistry {
        ServiceRegistry {
            map: Arc::new(self.map),
        }
    }
}

/// Read-only, lock-free type-safe heterogeneous container for `Arc<T>` services.
///
/// Stores `Arc<T>` values keyed by `TypeId::of::<T>()`. Both concrete types
/// and trait objects (`Arc<dyn Trait>`) are supported. Cheaply `Clone`able
/// (inner data is behind `Arc`).
///
/// Use [`ServiceRegistryBuilder`] to construct, then call [`resolve`] at runtime.
/// Resolve is infallible — if a type was registered, it will be found.
/// For optional resolution, use [`try_resolve`].
#[derive(Clone, Default)]
pub struct ServiceRegistry {
    map: Arc<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
}

impl ServiceRegistry {
    /// Create an empty (frozen) registry.
    ///
    /// Prefer [`ServiceRegistryBuilder::build`] for populated registries.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a previously registered service.
    ///
    /// Returns `None` if `T` was not registered.
    pub fn try_resolve<T: 'static + ?Sized>(&self) -> Option<Arc<T>> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref::<Arc<T>>().cloned())
    }

    /// Resolve a previously registered service.
    ///
    /// Infallible when used with types registered during build phase.
    /// For optional resolution, use [`try_resolve`].
    ///
    /// # Panics
    ///
    /// Only panics if `T` was never registered — a programming error
    /// that should be caught during development.
    pub fn resolve<T: 'static + ?Sized>(&self) -> Arc<T> {
        self.try_resolve().unwrap_or_else(|| {
            let type_name = std::any::type_name::<T>();
            panic!("ServiceRegistry: no service registered for type `{type_name}`")
        })
    }

    /// Check if a service of type `T` has been registered.
    pub fn contains<T: 'static + ?Sized>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Number of registered services.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Convenience macro to resolve a service from `AppState.services`.
///
/// ```ignore
/// let svc: Arc<PostService> = resolve!(state, PostService);
/// ```
#[macro_export]
macro_rules! resolve {
    ($state:expr, $ty:ty) => {
        $state.services.resolve::<$ty>()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Greeting: Send + Sync {
        fn greet(&self) -> &str;
    }

    struct English;
    impl Greeting for English {
        fn greet(&self) -> &str {
            "hello"
        }
    }

    #[test]
    fn insert_and_resolve_concrete() {
        let mut builder = ServiceRegistryBuilder::new();
        builder.register(Arc::new(42i32));
        let reg = builder.build();
        let val: Arc<i32> = reg.resolve::<i32>();
        assert_eq!(*val, 42);
    }

    #[test]
    fn insert_and_resolve_trait_object() {
        let mut builder = ServiceRegistryBuilder::new();
        builder.register(Arc::new(English) as Arc<dyn Greeting>);
        let reg = builder.build();
        let svc: Arc<dyn Greeting> = reg.resolve::<dyn Greeting>();
        assert_eq!(svc.greet(), "hello");
    }

    #[test]
    fn clone_shares_data() {
        let mut builder = ServiceRegistryBuilder::new();
        builder.register(Arc::new("shared".to_string()));
        let reg = builder.build();
        let cloned = reg.clone();
        let val: Arc<String> = cloned.resolve::<String>();
        assert_eq!(&*val, "shared");
    }

    #[test]
    fn contains_check() {
        let reg = ServiceRegistryBuilder::new().build();
        assert!(!reg.contains::<i32>());
        let mut builder = ServiceRegistryBuilder::new();
        builder.register(Arc::new(1i32));
        let reg = builder.build();
        assert!(reg.contains::<i32>());
    }

    #[test]
    fn len_and_is_empty() {
        let reg = ServiceRegistryBuilder::new().build();
        assert!(reg.is_empty());
        let mut builder = ServiceRegistryBuilder::new();
        builder.register(Arc::new(1i32));
        let reg = builder.build();
        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());
    }

    #[test]
    #[should_panic(expected = "no service registered")]
    fn resolve_missing_panics() {
        let reg = ServiceRegistryBuilder::new().build();
        let _: Arc<i32> = reg.resolve::<i32>();
    }

    #[test]
    fn try_resolve_missing_returns_none() {
        let reg = ServiceRegistryBuilder::new().build();
        assert!(reg.try_resolve::<i32>().is_none());
    }

    #[test]
    fn insert_overwrites() {
        let mut builder = ServiceRegistryBuilder::new();
        builder.register(Arc::new(1i32));
        builder.register(Arc::new(2i32));
        let reg = builder.build();
        let val: Arc<i32> = reg.resolve::<i32>();
        assert_eq!(*val, 2);
    }
}
