//! Service Locator — type-safe component registry for AppState.
//!
//! Provides [`ServiceRegistry`] for registering and resolving `Arc<T>` components
//! by type. Used to construct `AppState` in a declarative style.
//!
//! # Example
//!
//! ```ignore
//! let mut registry = ServiceRegistry::new();
//! registry.insert(Arc::new(SqlxUserRepository::new(pool)) as Arc<dyn UserRepository>);
//! let repo: Arc<dyn UserRepository> = registry.resolve::<dyn UserRepository>();
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
struct RegistryInner {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

/// Type-safe heterogeneous container for `Arc<T>` services.
///
/// Stores `Arc<T>` values keyed by `TypeId::of::<T>()`. Both concrete types
/// and trait objects (`Arc<dyn Trait>`) are supported. Cheaply `Clone`able
/// (inner data is behind `Arc`).
#[derive(Default, Clone)]
pub struct ServiceRegistry {
    inner: Arc<std::sync::RwLock<RegistryInner>>,
}

impl ServiceRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a service. The key is `T` (the outermost type of the `Arc`).
    ///
    /// For trait objects, use the `as Arc<dyn Trait>` cast at the call site.
    pub fn insert<T: Send + Sync + 'static + ?Sized>(&self, service: Arc<T>) {
        self.inner
            .write()
            .expect("ServiceRegistry lock poisoned")
            .map
            .insert(TypeId::of::<T>(), Box::new(service));
    }

    /// Resolve a previously registered service.
    ///
    /// # Panics
    ///
    /// Panics if `T` was not registered or if the lock is poisoned.
    pub fn resolve<T: 'static + ?Sized>(&self) -> Arc<T> {
        let guard = self.inner.read().expect("ServiceRegistry lock poisoned");
        let Some(boxed) = guard.map.get(&TypeId::of::<T>()) else {
            let type_name = std::any::type_name::<T>();
            panic!("ServiceRegistry: no service registered for type `{type_name}`");
        };
        boxed
            .downcast_ref::<Arc<T>>()
            .cloned()
            .expect("ServiceRegistry: type mismatch — stored Arc<T> does not match requested T")
    }

    /// Check if a service of type `T` has been registered.
    pub fn contains<T: 'static + ?Sized>(&self) -> bool {
        self.inner
            .read()
            .expect("ServiceRegistry lock poisoned")
            .map
            .contains_key(&TypeId::of::<T>())
    }

    /// Number of registered services.
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .expect("ServiceRegistry lock poisoned")
            .map
            .len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Convenience macro to resolve a service from `AppState.services`.
///
/// ```ignore
/// let repo: Arc<dyn UserRepository> = resolve!(state, dyn UserRepository);
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
        let reg = ServiceRegistry::new();
        reg.insert(Arc::new(42i32));
        let val: Arc<i32> = reg.resolve::<i32>();
        assert_eq!(*val, 42);
    }

    #[test]
    fn insert_and_resolve_trait_object() {
        let reg = ServiceRegistry::new();
        reg.insert(Arc::new(English) as Arc<dyn Greeting>);
        let svc: Arc<dyn Greeting> = reg.resolve::<dyn Greeting>();
        assert_eq!(svc.greet(), "hello");
    }

    #[test]
    fn clone_shares_data() {
        let reg = ServiceRegistry::new();
        reg.insert(Arc::new("shared".to_string()));
        let cloned = reg.clone();
        let val: Arc<String> = cloned.resolve::<String>();
        assert_eq!(&*val, "shared");
    }

    #[test]
    fn contains_check() {
        let reg = ServiceRegistry::new();
        assert!(!reg.contains::<i32>());
        reg.insert(Arc::new(1i32));
        assert!(reg.contains::<i32>());
    }

    #[test]
    fn len_and_is_empty() {
        let reg = ServiceRegistry::new();
        assert!(reg.is_empty());
        reg.insert(Arc::new(1i32));
        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());
    }

    #[test]
    #[should_panic(expected = "no service registered")]
    fn resolve_missing_panics() {
        let reg = ServiceRegistry::new();
        let _: Arc<i32> = reg.resolve::<i32>();
    }

    #[test]
    fn insert_overwrites() {
        let reg = ServiceRegistry::new();
        reg.insert(Arc::new(1i32));
        reg.insert(Arc::new(2i32));
        let val: Arc<i32> = reg.resolve::<i32>();
        assert_eq!(*val, 2);
    }
}
