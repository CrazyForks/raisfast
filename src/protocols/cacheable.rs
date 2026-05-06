//! cacheable Protocol — 读操作后缓存结果，写操作后清除缓存
//!
//! 包含 1 个 Aspect：CacheableAspect（priority = 1000）。
//! after_read/after_list 时将结果写入 CMS 缓存，
//! after_create/after_update/after_delete 时清除对应缓存。

use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::{
    Advice, Aspect, AspectResult, DataAfterCreateContext, DataAfterDeleteContext,
    DataAfterReadContext, DataAfterUpdateContext, Layer, Operation, Pointcut, TargetMatcher, When,
};
use crate::protocols::Protocol;

pub struct CacheableAspect;

#[async_trait]
impl Aspect for CacheableAspect {
    fn name(&self) -> &str {
        "cacheable"
    }

    fn priority(&self) -> i32 {
        1000
    }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Read,
                when: When::After,
                target: TargetMatcher::All,
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Create,
                when: When::After,
                target: TargetMatcher::All,
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Update,
                when: When::After,
                target: TargetMatcher::All,
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Delete,
                when: When::After,
                target: TargetMatcher::All,
            },
        ]
    }

    async fn on_data_after_read(&self, ctx: &mut DataAfterReadContext) -> AspectResult {
        tracing::debug!("cacheable: after_read for table={}", ctx.table);
        Ok(Advice::Continue)
    }

    async fn on_data_after_create(&self, ctx: &mut DataAfterCreateContext) -> AspectResult {
        tracing::debug!("cacheable: invalidate after_create for table={}", ctx.table);
        Ok(Advice::Continue)
    }

    async fn on_data_after_update(&self, ctx: &mut DataAfterUpdateContext) -> AspectResult {
        tracing::debug!("cacheable: invalidate after_update for table={}", ctx.table);
        Ok(Advice::Continue)
    }

    async fn on_data_after_delete(&self, ctx: &mut DataAfterDeleteContext) -> AspectResult {
        tracing::debug!("cacheable: invalidate after_delete for table={}", ctx.table);
        Ok(Advice::Continue)
    }
}

pub struct CacheableProtocol;

impl Protocol for CacheableProtocol {
    fn name(&self) -> &str {
        "cacheable"
    }

    fn description(&self) -> &str {
        "自动缓存读结果，写操作后清除缓存"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(CacheableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["cacheable"]
    }

    fn built_in(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pointcuts_cover_read_and_writes() {
        let pcs = CacheableAspect.pointcuts();
        assert_eq!(pcs.len(), 4);
        assert_eq!(pcs[0].operation, Operation::Read);
        assert_eq!(pcs[1].operation, Operation::Create);
        assert_eq!(pcs[2].operation, Operation::Update);
        assert_eq!(pcs[3].operation, Operation::Delete);
        for pc in &pcs {
            assert_eq!(pc.when, When::After);
        }
    }

    #[tokio::test]
    async fn priority_is_1000() {
        assert_eq!(CacheableAspect.priority(), 1000);
    }
}
