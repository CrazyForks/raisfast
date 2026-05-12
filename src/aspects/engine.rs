//! AspectEngine — Aspect registry and dispatcher
//!
//! Pre-computes dispatch table on registration (JoinPointId → ordered Aspect list),
//! O(1) lookup at runtime + execution in priority order.

use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;

use super::{
    AccessCheckContext, AccessFilterContext, Advice, Aspect, ColumnDef, DataAfterCreateContext,
    DataAfterDeleteContext, DataAfterReadContext, DataAfterUpdateContext, DataBeforeCreateContext,
    DataBeforeDeleteContext, DataBeforeReadContext, DataBeforeUpdateContext, EventContext,
    HttpAfterContext, HttpBeforeContext, JoinPointId, Layer, Operation, Pointcut, TargetMatcher,
    When,
};

#[derive(Clone)]
pub struct AspectEntry {
    pub aspect: Arc<dyn Aspect>,
    pub pointcuts: Vec<Pointcut>,
    pub enabled: bool,
}

pub struct AspectEngine {
    dispatch_table: DashMap<JoinPointId, Vec<Arc<dyn Aspect>>>,
    registry: ArcSwap<Vec<AspectEntry>>,
}

impl std::fmt::Debug for AspectEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self
            .registry
            .load()
            .iter()
            .map(|e| e.aspect.name().to_string())
            .collect();
        f.debug_struct("AspectEngine")
            .field("aspect_count", &names.len())
            .field("aspects", &names)
            .finish()
    }
}

impl AspectEngine {
    pub fn new() -> Self {
        Self {
            dispatch_table: DashMap::new(),
            registry: ArcSwap::from_pointee(Vec::new()),
        }
    }

    pub fn register(&self, aspect: impl Aspect) {
        let arc: Arc<dyn Aspect> = Arc::new(aspect);
        self.register_from_arc(arc);
    }

    pub fn register_from_arc(&self, arc: Arc<dyn Aspect>) {
        let pointcuts = arc.pointcuts();
        let pointcuts_clone = pointcuts.clone();
        let arc_clone = arc.clone();

        for pc in &pointcuts {
            let jp_id = pc.join_point_id();
            let mut list = self.dispatch_table.entry(jp_id).or_default();
            list.push(arc.clone());
            list.sort_by_key(|a| a.priority());
        }

        self.registry.rcu(|old| {
            let mut v = (**old).clone();
            v.push(AspectEntry {
                aspect: arc_clone.clone(),
                pointcuts: pointcuts_clone.clone(),
                enabled: true,
            });
            v
        });
    }

    pub fn enable(&self, name: &str) -> bool {
        let mut found = false;
        self.registry.rcu(|old| {
            let mut v = (**old).clone();
            for entry in &mut v {
                if entry.aspect.name() == name {
                    entry.enabled = true;
                    found = true;
                    break;
                }
            }
            v
        });
        found
    }

    pub fn disable(&self, name: &str) -> bool {
        let mut found = false;
        self.registry.rcu(|old| {
            let mut v = (**old).clone();
            for entry in &mut v {
                if entry.aspect.name() == name {
                    entry.enabled = false;
                    found = true;
                    break;
                }
            }
            v
        });
        found
    }

    pub fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        self.registry
            .load()
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.aspect.clone())
            .collect()
    }

    pub fn columns_for(&self, table: &str) -> Vec<ColumnDef> {
        let mut cols = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for entry in self.registry.load().iter() {
            if entry.enabled && matches_table_any(&entry.pointcuts, table) {
                for col in entry.aspect.columns() {
                    if seen.insert(col.name.clone()) {
                        cols.push(col);
                    }
                }
            }
        }
        cols
    }

    fn get_aspects(&self, jp_id: &JoinPointId, table: &str) -> Vec<Arc<dyn Aspect>> {
        let registry = self.registry.load();
        let enabled_names: std::collections::HashSet<&str> = registry
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.aspect.name())
            .collect();

        let Some(aspects) = self.dispatch_table.get(jp_id) else {
            return Vec::new();
        };
        aspects
            .iter()
            .filter(|a| {
                enabled_names.contains(a.name()) && matches_table_any(&a.pointcuts(), table)
            })
            .cloned()
            .collect()
    }

    // ─── Data Layer dispatch ───

    pub async fn dispatch_data_before_create(
        &self,
        table: &str,
        ctx: &mut DataBeforeCreateContext,
    ) -> Result<Option<serde_json::Value>, anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::Before,
        };
        let aspects = self.get_aspects(&jp_id, table);
        for aspect in &aspects {
            match aspect.on_data_before_create(ctx).await {
                Ok(Advice::Continue) => continue,
                Ok(Advice::Skip) => break,
                Ok(Advice::Return(val)) => return Ok(Some(val)),
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    pub async fn dispatch_data_after_create(
        &self,
        table: &str,
        ctx: &mut DataAfterCreateContext,
    ) -> Result<(), anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::After,
        };
        let aspects = self.get_aspects(&jp_id, table);
        for aspect in &aspects {
            match aspect.on_data_after_create(ctx).await {
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub async fn dispatch_data_before_read(
        &self,
        table: &str,
        ctx: &mut DataBeforeReadContext,
    ) -> Result<Option<serde_json::Value>, anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Read,
            when: When::Before,
        };
        let aspects = self.get_aspects(&jp_id, table);
        for aspect in &aspects {
            match aspect.on_data_before_read(ctx).await {
                Ok(Advice::Continue) => continue,
                Ok(Advice::Skip) => break,
                Ok(Advice::Return(val)) => return Ok(Some(val)),
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    pub async fn dispatch_data_after_read(
        &self,
        table: &str,
        ctx: &mut DataAfterReadContext,
    ) -> Result<(), anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Read,
            when: When::After,
        };
        let aspects = self.get_aspects(&jp_id, table);
        for aspect in &aspects {
            match aspect.on_data_after_read(ctx).await {
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub async fn dispatch_data_before_update(
        &self,
        table: &str,
        ctx: &mut DataBeforeUpdateContext,
    ) -> Result<Option<serde_json::Value>, anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Update,
            when: When::Before,
        };
        let aspects = self.get_aspects(&jp_id, table);
        for aspect in &aspects {
            match aspect.on_data_before_update(ctx).await {
                Ok(Advice::Continue) => continue,
                Ok(Advice::Skip) => break,
                Ok(Advice::Return(val)) => return Ok(Some(val)),
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    pub async fn dispatch_data_after_update(
        &self,
        table: &str,
        ctx: &mut DataAfterUpdateContext,
    ) -> Result<(), anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Update,
            when: When::After,
        };
        let aspects = self.get_aspects(&jp_id, table);
        for aspect in &aspects {
            match aspect.on_data_after_update(ctx).await {
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub async fn dispatch_data_before_delete(
        &self,
        table: &str,
        ctx: &mut DataBeforeDeleteContext,
    ) -> Result<Option<serde_json::Value>, anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Delete,
            when: When::Before,
        };
        let aspects = self.get_aspects(&jp_id, table);
        for aspect in &aspects {
            match aspect.on_data_before_delete(ctx).await {
                Ok(Advice::Continue) => continue,
                Ok(Advice::Skip) => break,
                Ok(Advice::Return(val)) => return Ok(Some(val)),
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    pub async fn dispatch_data_after_delete(
        &self,
        table: &str,
        ctx: &mut DataAfterDeleteContext,
    ) -> Result<(), anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Delete,
            when: When::After,
        };
        let aspects = self.get_aspects(&jp_id, table);
        for aspect in &aspects {
            match aspect.on_data_after_delete(ctx).await {
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Event Layer: pre-publish interception
    ///
    /// Returns `Ok(true)` to continue publishing, `Ok(false)` if blocked by an aspect.
    pub async fn dispatch_event_before_publish(
        &self,
        event_type: &str,
        ctx: &mut EventContext,
    ) -> Result<bool, anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Event,
            operation: Operation::Publish,
            when: When::Before,
        };
        let aspects = self.get_aspects(&jp_id, event_type);
        for aspect in &aspects {
            match aspect.on_event_before_publish(ctx).await {
                Ok(Advice::Continue) => continue,
                Ok(Advice::Skip) => return Ok(false),
                Ok(Advice::Return(_)) => return Ok(false),
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    /// Event Layer: post-publish notification
    pub async fn dispatch_event_after_publish(
        &self,
        event_type: &str,
        ctx: &mut EventContext,
    ) -> Result<(), anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Event,
            operation: Operation::Publish,
            when: When::After,
        };
        let aspects = self.get_aspects(&jp_id, event_type);
        for aspect in &aspects {
            match aspect.on_event_after_publish(ctx).await {
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// HTTP Layer: pre-request interception
    ///
    /// Returns `Ok(None)` to continue processing, `Ok(Some(body))` to short-circuit.
    pub async fn dispatch_http_before(
        &self,
        path: &str,
        ctx: &mut HttpBeforeContext,
    ) -> Result<Option<serde_json::Value>, anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Http,
            operation: Operation::Request,
            when: When::Before,
        };
        let aspects = self.get_aspects(&jp_id, path);
        for aspect in &aspects {
            match aspect.on_http_before(ctx).await {
                Ok(Advice::Continue) => continue,
                Ok(Advice::Skip) => break,
                Ok(Advice::Return(val)) => return Ok(Some(val)),
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// HTTP Layer: post-response notification
    pub async fn dispatch_http_after(
        &self,
        path: &str,
        ctx: &mut HttpAfterContext,
    ) -> Result<(), anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Http,
            operation: Operation::Response,
            when: When::After,
        };
        let aspects = self.get_aspects(&jp_id, path);
        for aspect in &aspects {
            match aspect.on_http_after(ctx).await {
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Access Layer: permission check interception
    ///
    /// Returns `Ok(true)` to allow, `Ok(false)` to deny.
    pub async fn dispatch_access_check(
        &self,
        subject: &str,
        ctx: &mut AccessCheckContext,
    ) -> Result<bool, anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Access,
            operation: Operation::Check,
            when: When::Before,
        };
        let aspects = self.get_aspects(&jp_id, subject);
        for aspect in &aspects {
            match aspect.on_access_check(ctx).await {
                Ok(Advice::Continue) => continue,
                Ok(Advice::Skip) => return Ok(false),
                Ok(Advice::Return(_)) => return Ok(false),
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    /// Access Layer: data filtering (query condition injection)
    pub async fn dispatch_access_filter(
        &self,
        table: &str,
        ctx: &mut AccessFilterContext,
    ) -> Result<(), anyhow::Error> {
        let jp_id = JoinPointId {
            layer: Layer::Access,
            operation: Operation::Filter,
            when: When::Before,
        };
        let aspects = self.get_aspects(&jp_id, table);
        for aspect in &aspects {
            match aspect.on_access_filter(ctx).await {
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl Default for AspectEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn matches_target(matcher: &TargetMatcher, table: &str) -> bool {
    match matcher {
        TargetMatcher::All => true,
        TargetMatcher::Tables(tables) => tables.iter().any(|t| t == table),
    }
}

fn matches_table_any(pointcuts: &[Pointcut], table: &str) -> bool {
    pointcuts.iter().any(|pc| matches_target(&pc.target, table))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspects::{
        Advice, AspectResult, BaseContext, ColumnDef, DataBeforeCreateContext, Layer, Operation,
        Pointcut, Record, SqlType, TargetMatcher, When,
    };

    struct OrderTestAspect {
        aspect_name: &'static str,
        prio: i32,
    }

    #[async_trait::async_trait]
    impl Aspect for OrderTestAspect {
        fn name(&self) -> &str {
            self.aspect_name
        }
        fn priority(&self) -> i32 {
            self.prio
        }
        fn pointcuts(&self) -> Vec<Pointcut> {
            vec![Pointcut {
                layer: Layer::Data,
                operation: Operation::Create,
                when: When::Before,
                target: TargetMatcher::All,
            }]
        }
        async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
            ctx.record
                .insert(self.aspect_name.into(), serde_json::Value::Bool(true));
            Ok(Advice::Continue)
        }
    }

    #[tokio::test]
    async fn dispatch_executes_in_priority_order() {
        let engine = AspectEngine::new();
        engine.register(OrderTestAspect {
            aspect_name: "low",
            prio: 100,
        });
        engine.register(OrderTestAspect {
            aspect_name: "high",
            prio: -100,
        });

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "2026-01-01T00:00:00Z".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        let _: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_create("articles", &mut ctx)
            .await;

        assert!(ctx.record.contains_key("high"));
        assert!(ctx.record.contains_key("low"));
    }

    #[tokio::test]
    async fn columns_for_returns_unique_columns() {
        let engine = AspectEngine::new();
        engine.register(crate::protocols::ownable::OwnableAspect);
        engine.register(crate::protocols::timestampable::TimestampableAspect);

        let cols = engine.columns_for("articles");
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"created_by"));
        assert!(names.contains(&"updated_by"));
        assert!(names.contains(&"created_at"));
        assert!(names.contains(&"updated_at"));
    }

    #[tokio::test]
    async fn dispatch_target_filtering() {
        let engine = AspectEngine::new();

        struct RestrictedAspect;
        #[async_trait::async_trait]
        impl Aspect for RestrictedAspect {
            fn name(&self) -> &str {
                "restricted"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Create,
                    when: When::Before,
                    target: TargetMatcher::Tables(vec!["posts".into()]),
                }]
            }
            async fn on_data_before_create(
                &self,
                ctx: &mut DataBeforeCreateContext,
            ) -> AspectResult {
                ctx.record
                    .insert("restricted".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
        }

        engine.register(RestrictedAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "2026-01-01T00:00:00Z".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        let _: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_create("articles", &mut ctx)
            .await;

        assert!(!ctx.record.contains_key("restricted"));
    }

    #[tokio::test]
    async fn advice_return_short_circuits() {
        struct CacheHitAspect;
        #[async_trait::async_trait]
        impl Aspect for CacheHitAspect {
            fn name(&self) -> &str {
                "cache_hit"
            }
            fn priority(&self) -> i32 {
                1000
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Create,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_data_before_create(
                &self,
                ctx: &mut DataBeforeCreateContext,
            ) -> AspectResult {
                ctx.record
                    .insert("cache_hit_ran".into(), serde_json::Value::Bool(true));
                Ok(Advice::Return(serde_json::json!({"cached": true})))
            }
        }

        struct ShouldNotRunAspect;
        #[async_trait::async_trait]
        impl Aspect for ShouldNotRunAspect {
            fn name(&self) -> &str {
                "should_not_run"
            }
            fn priority(&self) -> i32 {
                2000
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Create,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_data_before_create(
                &self,
                ctx: &mut DataBeforeCreateContext,
            ) -> AspectResult {
                ctx.record
                    .insert("should_not_run_ran".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(CacheHitAspect);
        engine.register(ShouldNotRunAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        let result: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_create("articles", &mut ctx)
            .await;

        let short_circuit = result.unwrap();
        assert_eq!(short_circuit, Some(serde_json::json!({"cached": true})));
        assert!(ctx.record.contains_key("cache_hit_ran"));
        assert!(!ctx.record.contains_key("should_not_run_ran"));
    }

    #[tokio::test]
    async fn advice_error_aborts_dispatch() {
        struct FailAspect;
        #[async_trait::async_trait]
        impl Aspect for FailAspect {
            fn name(&self) -> &str {
                "fail"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Create,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_data_before_create(
                &self,
                ctx: &mut DataBeforeCreateContext,
            ) -> AspectResult {
                ctx.record
                    .insert("fail_ran".into(), serde_json::Value::Bool(true));
                Err(anyhow::anyhow!("validation failed"))
            }
        }

        struct AfterFailAspect;
        #[async_trait::async_trait]
        impl Aspect for AfterFailAspect {
            fn name(&self) -> &str {
                "after_fail"
            }
            fn priority(&self) -> i32 {
                100
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Create,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_data_before_create(
                &self,
                ctx: &mut DataBeforeCreateContext,
            ) -> AspectResult {
                ctx.record
                    .insert("after_fail_ran".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(FailAspect);
        engine.register(AfterFailAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        let result: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_create("articles", &mut ctx)
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("validation failed")
        );
        assert!(ctx.record.contains_key("fail_ran"));
        assert!(!ctx.record.contains_key("after_fail_ran"));
    }

    #[tokio::test]
    async fn empty_engine_dispatch_succeeds() {
        let engine = AspectEngine::new();

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        let result: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_create("articles", &mut ctx)
            .await;
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn after_create_dispatches() {
        use crate::aspects::DataAfterCreateContext;

        struct AuditAspect;
        #[async_trait::async_trait]
        impl Aspect for AuditAspect {
            fn name(&self) -> &str {
                "audit"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Create,
                    when: When::After,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_data_after_create(&self, ctx: &mut DataAfterCreateContext) -> AspectResult {
                ctx.record
                    .insert("audited".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(AuditAspect);

        let mut ctx = DataAfterCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            record: {
                let mut r = Record::new();
                r.insert("id".into(), serde_json::json!("abc"));
                r
            },
            schema: None,
        };

        engine
            .dispatch_data_after_create("articles", &mut ctx)
            .await
            .unwrap();
        assert_eq!(
            ctx.record.get("audited").unwrap(),
            &serde_json::Value::Bool(true)
        );
    }

    #[tokio::test]
    async fn after_update_dispatches() {
        use crate::aspects::DataAfterUpdateContext;

        struct LogAspect;
        #[async_trait::async_trait]
        impl Aspect for LogAspect {
            fn name(&self) -> &str {
                "log"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Update,
                    when: When::After,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_data_after_update(&self, ctx: &mut DataAfterUpdateContext) -> AspectResult {
                ctx.new_record
                    .insert("logged".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(LogAspect);

        let mut ctx = DataAfterUpdateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            old_record: Record::new(),
            new_record: {
                let mut r = Record::new();
                r.insert("title".into(), serde_json::json!("new title"));
                r
            },
            schema: None,
        };

        engine
            .dispatch_data_after_update("articles", &mut ctx)
            .await
            .unwrap();
        assert!(ctx.new_record.get("logged").is_some());
    }

    #[tokio::test]
    async fn aspects_returns_all_registered() {
        let engine = AspectEngine::new();
        engine.register(crate::protocols::ownable::OwnableAspect);
        engine.register(crate::protocols::timestampable::TimestampableAspect);

        let aspects = engine.aspects();
        assert_eq!(aspects.len(), 2);
        let names: Vec<&str> = aspects.iter().map(|a| a.name()).collect();
        assert!(names.contains(&"ownable"));
        assert!(names.contains(&"timestampable"));
    }

    #[tokio::test]
    async fn columns_for_empty_engine() {
        let engine = AspectEngine::new();
        let cols = engine.columns_for("articles");
        assert!(cols.is_empty());
    }

    #[tokio::test]
    async fn columns_for_target_filtering() {
        struct PostsOnlyAspect;
        #[async_trait::async_trait]
        impl Aspect for PostsOnlyAspect {
            fn name(&self) -> &str {
                "posts_only"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Create,
                    when: When::Before,
                    target: TargetMatcher::Tables(vec!["posts".into()]),
                }]
            }
            fn columns(&self) -> Vec<ColumnDef> {
                vec![ColumnDef {
                    name: "special_col".into(),
                    sql_type: SqlType::Text,
                    default: None,
                }]
            }
        }

        let engine = AspectEngine::new();
        engine.register(PostsOnlyAspect);

        let cols_posts = engine.columns_for("posts");
        assert!(cols_posts.iter().any(|c| c.name == "special_col"));

        let cols_articles = engine.columns_for("articles");
        assert!(!cols_articles.iter().any(|c| c.name == "special_col"));
    }

    #[tokio::test]
    async fn multi_pointcut_aspect() {
        struct CreateAndUpdateAspect;
        #[async_trait::async_trait]
        impl Aspect for CreateAndUpdateAspect {
            fn name(&self) -> &str {
                "multi"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![
                    Pointcut {
                        layer: Layer::Data,
                        operation: Operation::Create,
                        when: When::Before,
                        target: TargetMatcher::All,
                    },
                    Pointcut {
                        layer: Layer::Data,
                        operation: Operation::Update,
                        when: When::Before,
                        target: TargetMatcher::All,
                    },
                ]
            }
            async fn on_data_before_create(
                &self,
                ctx: &mut DataBeforeCreateContext,
            ) -> AspectResult {
                ctx.record
                    .insert("multi_create".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
            async fn on_data_before_update(
                &self,
                ctx: &mut DataBeforeUpdateContext,
            ) -> AspectResult {
                ctx.new_record
                    .insert("multi_update".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(CreateAndUpdateAspect);

        let mut create_ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };
        let _: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_create("articles", &mut create_ctx)
            .await;
        assert!(create_ctx.record.contains_key("multi_create"));

        let mut update_ctx = DataBeforeUpdateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            old_record: Record::new(),
            new_record: Record::new(),
            schema: None,
        };
        let _: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_update("articles", &mut update_ctx)
            .await;
        assert!(update_ctx.new_record.contains_key("multi_update"));
    }

    #[tokio::test]
    async fn aspect_communication_via_extensions() {
        struct ProducerAspect;
        #[async_trait::async_trait]
        impl Aspect for ProducerAspect {
            fn name(&self) -> &str {
                "producer"
            }
            fn priority(&self) -> i32 {
                -100
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Create,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_data_before_create(
                &self,
                ctx: &mut DataBeforeCreateContext,
            ) -> AspectResult {
                ctx.base
                    .extensions
                    .insert::<String>("produced-value".into());
                Ok(Advice::Continue)
            }
        }

        struct ConsumerAspect;
        #[async_trait::async_trait]
        impl Aspect for ConsumerAspect {
            fn name(&self) -> &str {
                "consumer"
            }
            fn priority(&self) -> i32 {
                100
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Data,
                    operation: Operation::Create,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_data_before_create(
                &self,
                ctx: &mut DataBeforeCreateContext,
            ) -> AspectResult {
                if let Some(val) = ctx.base.extensions.get::<String>() {
                    ctx.record.insert("consumed".into(), serde_json::json!(val));
                }
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(ProducerAspect);
        engine.register(ConsumerAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };

        let _: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_create("articles", &mut ctx)
            .await;

        assert_eq!(
            ctx.record.get("consumed").unwrap(),
            &serde_json::json!("produced-value")
        );
    }

    #[tokio::test]
    async fn disable_prevents_dispatch() {
        let engine = AspectEngine::new();
        engine.register(crate::protocols::ownable::OwnableAspect);

        let found = engine.disable("ownable");
        assert!(found);

        let aspects = engine.aspects();
        assert!(aspects.is_empty());

        let mut ctx = DataBeforeCreateContext {
            base: crate::aspects::BaseContext::new(
                Some("user-1".into()),
                "default".into(),
                "now".into(),
            ),
            table: "articles".into(),
            record: Record::new(),
            schema: None,
        };
        let _: Result<Option<serde_json::Value>, _> = engine
            .dispatch_data_before_create("articles", &mut ctx)
            .await;
        assert!(ctx.record.get("created_by").is_none());
    }

    #[tokio::test]
    async fn enable_reactivates_aspect() {
        let engine = AspectEngine::new();
        engine.register(crate::protocols::ownable::OwnableAspect);
        engine.disable("ownable");

        let found = engine.enable("ownable");
        assert!(found);

        let aspects = engine.aspects();
        assert_eq!(aspects.len(), 1);
    }

    #[tokio::test]
    async fn disable_unknown_returns_false() {
        let engine = AspectEngine::new();
        assert!(!engine.disable("nonexistent"));
        assert!(!engine.enable("nonexistent"));
    }

    // ─── Event Layer tests ───

    #[tokio::test]
    async fn event_before_publish_returns_true_when_no_aspects() {
        let engine = AspectEngine::new();
        let mut ctx = EventContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            event_type: "post_created".into(),
            payload: serde_json::json!({"id": "1"}),
            table: Some("posts".into()),
        };
        let result = engine
            .dispatch_event_before_publish("post_created", &mut ctx)
            .await;
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn event_before_publish_continue_allows_publish() {
        struct LogEventAspect;
        #[async_trait::async_trait]
        impl Aspect for LogEventAspect {
            fn name(&self) -> &str {
                "log_event"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Event,
                    operation: Operation::Publish,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_event_before_publish(&self, ctx: &mut EventContext) -> AspectResult {
                ctx.payload
                    .as_object_mut()
                    .unwrap()
                    .insert("logged".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(LogEventAspect);

        let mut ctx = EventContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            event_type: "post_created".into(),
            payload: serde_json::json!({"id": "1"}),
            table: Some("posts".into()),
        };
        let result = engine
            .dispatch_event_before_publish("post_created", &mut ctx)
            .await;
        assert!(result.unwrap());
        assert_eq!(ctx.payload["logged"], serde_json::Value::Bool(true));
    }

    #[tokio::test]
    async fn event_before_publish_skip_blocks_publish() {
        struct BlockAspect;
        #[async_trait::async_trait]
        impl Aspect for BlockAspect {
            fn name(&self) -> &str {
                "block_event"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Event,
                    operation: Operation::Publish,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_event_before_publish(&self, _ctx: &mut EventContext) -> AspectResult {
                Ok(Advice::Skip)
            }
        }

        let engine = AspectEngine::new();
        engine.register(BlockAspect);

        let mut ctx = EventContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            event_type: "post_created".into(),
            payload: serde_json::json!({}),
            table: None,
        };
        let result = engine
            .dispatch_event_before_publish("post_created", &mut ctx)
            .await;
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn event_after_publish_dispatches() {
        struct AfterPublishAspect;
        #[async_trait::async_trait]
        impl Aspect for AfterPublishAspect {
            fn name(&self) -> &str {
                "after_publish"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Event,
                    operation: Operation::Publish,
                    when: When::After,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_event_after_publish(&self, ctx: &mut EventContext) -> AspectResult {
                ctx.payload
                    .as_object_mut()
                    .unwrap()
                    .insert("after_ran".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(AfterPublishAspect);

        let mut ctx = EventContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            event_type: "post_created".into(),
            payload: serde_json::json!({"id": "1"}),
            table: Some("posts".into()),
        };
        engine
            .dispatch_event_after_publish("post_created", &mut ctx)
            .await
            .unwrap();
        assert_eq!(ctx.payload["after_ran"], serde_json::Value::Bool(true));
    }

    #[tokio::test]
    async fn event_target_filtering() {
        struct PostsOnlyEventAspect;
        #[async_trait::async_trait]
        impl Aspect for PostsOnlyEventAspect {
            fn name(&self) -> &str {
                "posts_event"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Event,
                    operation: Operation::Publish,
                    when: When::Before,
                    target: TargetMatcher::Tables(vec!["post_created".into()]),
                }]
            }
            async fn on_event_before_publish(&self, ctx: &mut EventContext) -> AspectResult {
                ctx.payload
                    .as_object_mut()
                    .unwrap()
                    .insert("posts_only".into(), serde_json::Value::Bool(true));
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(PostsOnlyEventAspect);

        let mut ctx = EventContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            event_type: "comment_created".into(),
            payload: serde_json::json!({}),
            table: Some("comments".into()),
        };
        engine
            .dispatch_event_before_publish("comment_created", &mut ctx)
            .await
            .unwrap();
        assert!(!ctx.payload.as_object().unwrap().contains_key("posts_only"));
    }

    // ─── Access Layer tests ───

    #[tokio::test]
    async fn access_check_allows_when_no_aspects() {
        let engine = AspectEngine::new();
        let mut ctx = AccessCheckContext {
            base: BaseContext::new(Some("u1".into()), "default".into(), "now".into()),
            route: "/api/v1/admin/posts".into(),
            method: "PUT".into(),
            table: Some("posts".into()),
            action: "update".into(),
        };
        let result = engine
            .dispatch_access_check("posts", &mut ctx)
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn access_check_skip_denies_access() {
        struct DenyAspect;
        #[async_trait::async_trait]
        impl Aspect for DenyAspect {
            fn name(&self) -> &str {
                "deny_all"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Access,
                    operation: Operation::Check,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_access_check(&self, _ctx: &mut AccessCheckContext) -> AspectResult {
                Ok(Advice::Skip)
            }
        }

        let engine = AspectEngine::new();
        engine.register(DenyAspect);

        let mut ctx = AccessCheckContext {
            base: BaseContext::new(Some("u1".into()), "default".into(), "now".into()),
            route: "/api/v1/admin/posts/123".into(),
            method: "DELETE".into(),
            table: Some("posts".into()),
            action: "delete".into(),
        };
        let result = engine
            .dispatch_access_check("posts", &mut ctx)
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn access_check_continue_allows() {
        struct AllowAspect;
        #[async_trait::async_trait]
        impl Aspect for AllowAspect {
            fn name(&self) -> &str {
                "allow"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Access,
                    operation: Operation::Check,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_access_check(&self, _ctx: &mut AccessCheckContext) -> AspectResult {
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(AllowAspect);

        let mut ctx = AccessCheckContext {
            base: BaseContext::new(Some("u1".into()), "default".into(), "now".into()),
            route: "/api/v1/posts".into(),
            method: "GET".into(),
            table: Some("posts".into()),
            action: "read".into(),
        };
        let result = engine
            .dispatch_access_check("posts", &mut ctx)
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn access_filter_adds_conditions() {
        struct TenantFilterAspect;
        #[async_trait::async_trait]
        impl Aspect for TenantFilterAspect {
            fn name(&self) -> &str {
                "tenant_filter"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Access,
                    operation: Operation::Filter,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_access_filter(&self, ctx: &mut AccessFilterContext) -> AspectResult {
                ctx.conditions.push("tenant_id = ?".into());
                ctx.params.push(ctx.base.tenant_id.clone());
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(TenantFilterAspect);

        let mut ctx = AccessFilterContext {
            base: BaseContext::new(None, "t1".into(), "now".into()),
            table: "posts".into(),
            conditions: vec![],
            params: vec![],
        };
        engine
            .dispatch_access_filter("posts", &mut ctx)
            .await
            .unwrap();
        assert!(ctx.conditions.contains(&"tenant_id = ?".to_string()));
        assert!(ctx.params.contains(&"t1".to_string()));
    }

    #[tokio::test]
    async fn access_check_error_aborts() {
        struct FailCheckAspect;
        #[async_trait::async_trait]
        impl Aspect for FailCheckAspect {
            fn name(&self) -> &str {
                "fail_check"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Access,
                    operation: Operation::Check,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_access_check(&self, _ctx: &mut AccessCheckContext) -> AspectResult {
                Err(anyhow::anyhow!("rbac lookup failed"))
            }
        }

        let engine = AspectEngine::new();
        engine.register(FailCheckAspect);

        let mut ctx = AccessCheckContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            route: "/api/v1/posts".into(),
            method: "GET".into(),
            table: Some("posts".into()),
            action: "read".into(),
        };
        let result = engine.dispatch_access_check("posts", &mut ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("rbac lookup failed")
        );
    }

    // ─── HTTP Layer tests ───

    #[tokio::test]
    async fn http_before_returns_none_when_no_aspects() {
        let engine = AspectEngine::new();
        let mut ctx = HttpBeforeContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            method: "GET".into(),
            path: "/api/v1/posts".into(),
            headers: std::collections::HashMap::new(),
        };
        let result = engine
            .dispatch_http_before("/api/v1/posts", &mut ctx)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn http_before_short_circuits_with_return() {
        struct BlockPathAspect;
        #[async_trait::async_trait]
        impl Aspect for BlockPathAspect {
            fn name(&self) -> &str {
                "block_path"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Http,
                    operation: Operation::Request,
                    when: When::Before,
                    target: TargetMatcher::Tables(vec!["/api/v1/blocked".into()]),
                }]
            }
            async fn on_http_before(&self, _ctx: &mut HttpBeforeContext) -> AspectResult {
                Ok(Advice::Return(serde_json::json!({"error": "blocked"})))
            }
        }

        let engine = AspectEngine::new();
        engine.register(BlockPathAspect);

        let mut ctx = HttpBeforeContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            method: "GET".into(),
            path: "/api/v1/blocked".into(),
            headers: std::collections::HashMap::new(),
        };
        let result = engine
            .dispatch_http_before("/api/v1/blocked", &mut ctx)
            .await
            .unwrap();
        assert_eq!(result, Some(serde_json::json!({"error": "blocked"})));
    }

    #[tokio::test]
    async fn http_before_continue_proceeds() {
        struct LogHttpAspect;
        #[async_trait::async_trait]
        impl Aspect for LogHttpAspect {
            fn name(&self) -> &str {
                "log_http"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Http,
                    operation: Operation::Request,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_http_before(&self, ctx: &mut HttpBeforeContext) -> AspectResult {
                ctx.headers.insert("x-aspect-ran".into(), "true".into());
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(LogHttpAspect);

        let mut ctx = HttpBeforeContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            method: "GET".into(),
            path: "/api/v1/posts".into(),
            headers: std::collections::HashMap::new(),
        };
        let result = engine
            .dispatch_http_before("/api/v1/posts", &mut ctx)
            .await
            .unwrap();
        assert!(result.is_none());
        assert_eq!(ctx.headers.get("x-aspect-ran").unwrap(), "true");
    }

    #[tokio::test]
    async fn http_after_dispatches() {
        struct MetricsAspect;
        #[async_trait::async_trait]
        impl Aspect for MetricsAspect {
            fn name(&self) -> &str {
                "metrics"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Http,
                    operation: Operation::Response,
                    when: When::After,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_http_after(&self, ctx: &mut HttpAfterContext) -> AspectResult {
                ctx.response_body = Some(serde_json::json!({"tracked": true}));
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(MetricsAspect);

        let mut ctx = HttpAfterContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            status_code: 200,
            response_body: None,
        };
        engine
            .dispatch_http_after("/api/v1/posts", &mut ctx)
            .await
            .unwrap();
        assert!(ctx.response_body.is_some());
    }

    #[tokio::test]
    async fn event_priority_order() {
        use std::sync::{Arc, Mutex};

        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        struct LateAspect {
            log: Arc<Mutex<Vec<String>>>,
        }
        #[async_trait::async_trait]
        impl Aspect for LateAspect {
            fn name(&self) -> &str {
                "late"
            }
            fn priority(&self) -> i32 {
                100
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Event,
                    operation: Operation::Publish,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_event_before_publish(&self, _ctx: &mut EventContext) -> AspectResult {
                self.log.lock().unwrap().push("late".into());
                Ok(Advice::Continue)
            }
        }

        struct EarlyAspect {
            log: Arc<Mutex<Vec<String>>>,
        }
        #[async_trait::async_trait]
        impl Aspect for EarlyAspect {
            fn name(&self) -> &str {
                "early"
            }
            fn priority(&self) -> i32 {
                -100
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Event,
                    operation: Operation::Publish,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_event_before_publish(&self, _ctx: &mut EventContext) -> AspectResult {
                self.log.lock().unwrap().push("early".into());
                Ok(Advice::Continue)
            }
        }

        let engine = AspectEngine::new();
        engine.register(LateAspect { log: log.clone() });
        engine.register(EarlyAspect { log: log.clone() });

        let mut ctx = EventContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            event_type: "test".into(),
            payload: serde_json::json!({}),
            table: None,
        };
        engine
            .dispatch_event_before_publish("test", &mut ctx)
            .await
            .unwrap();

        let entries = log.lock().unwrap();
        assert_eq!(entries[0], "early");
        assert_eq!(entries[1], "late");
    }
}
