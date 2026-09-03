//! Flow engine models — one file per table (dev-docs/workflow db-schema.md).
//!
//! - [`flow`] — flow metadata
//! - [`flow_version`] — immutable definition snapshots
//! - [`flow_instance`] — one run
//! - [`flow_instance_snapshot`] — durable whole-snapshot (1:1)
//!
//! (flow_node_run added with P1.9 observability.)

pub mod flow;
pub mod flow_api_key;
pub mod flow_instance;
pub mod flow_instance_snapshot;
pub mod flow_node_run;
pub mod flow_version;

pub use flow::{
    Flow, delete_flow, extra_get, extra_set, find_flow_by_id, find_flows_by_tenant,
    find_flows_page, flow_api_token, flow_draft, insert_flow, set_flow_current_version,
    set_flow_draft, update_flow_meta,
};
pub use flow_api_key::{
    FlowApiKey, create as create_api_key, delete_by_flow as delete_api_key_by_flow,
    find_by_flow as find_api_key_by_flow, find_by_hash as find_api_key_by_hash,
    find_by_slug as find_api_key_by_slug, set_enabled as set_api_key_enabled,
    set_require_auth as set_api_key_require_auth, touch as touch_api_key,
    update_slug as update_api_key_slug, update_token as update_api_key_token,
};
pub use flow_instance::{
    FlowInstance, finalize_instance, find_instance_by_id, find_instances_by_trigger_page,
    find_instances_page, insert_flow_instance, update_instance_status,
};
pub use flow_instance_snapshot::{delete_snapshot, find_snapshot, upsert_snapshot};
pub use flow_node_run::{FlowNodeRun, find_node_runs, record_node_run};
pub use flow_version::{
    FlowVersion, current_version_number, find_version_by_id, insert_flow_version, latest_version,
    list_versions,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> crate::utils::tz::Timestamp {
        crate::utils::tz::now_utc()
    }

    #[tokio::test]
    async fn flow_version_instance_roundtrip() {
        let p = crate::test_pool!();
        let tenant = format!("wf-test-{}", crate::utils::id::new_id());

        let flow = Flow {
            id: crate::utils::id::new_snowflake_id(),
            tenant_id: tenant.clone(),
            name: "test flow".into(),
            description: None,
            enabled: true,
            current_version: None,
            extra: Some(json!({"tag": "x"})),
            created_at: now(),
            updated_at: now(),
        };
        insert_flow(&p, &flow).await.unwrap();
        let got = find_flow_by_id(&p, flow.id).await.unwrap();
        assert_eq!(got.name, "test flow");
        assert_eq!(got.extra.unwrap()["tag"], "x");

        let version = FlowVersion {
            id: crate::utils::id::new_snowflake_id(),
            flow_id: flow.id,
            version_number: 1,
            definition: json!({"nodes": []}),
            created_by: None,
            created_at: now(),
        };
        insert_flow_version(&p, &version).await.unwrap();
        let latest = latest_version(&p, flow.id).await.unwrap().unwrap();
        set_flow_current_version(&p, flow.id, latest.id)
            .await
            .unwrap();
        assert_eq!(
            find_flow_by_id(&p, flow.id)
                .await
                .unwrap()
                .current_version
                .unwrap(),
            latest.id
        );
        assert!(find_version_by_id(&p, latest.id).await.unwrap().is_some());

        let instance = FlowInstance {
            id: crate::utils::id::new_snowflake_id(),
            tenant_id: tenant.clone(),
            flow_id: flow.id,
            flow_version_id: latest.id,
            status: "running".into(),
            has_exceptions: false,
            trigger_kind: "api".into(),
            trigger_payload: Some(json!({"msg": "hi"})),
            inputs_summary: None,
            outputs: None,
            error: None,
            started_by: None,
            started_at: Some(now()),
            finished_at: None,
            waiting_kind: None,
            waiting_needed: None,
            waiting_received: 0,
            resume_until: None,
            created_at: now(),
        };
        insert_flow_instance(&p, &instance).await.unwrap();
        let got_inst = find_instance_by_id(&p, instance.id).await.unwrap();
        assert_eq!(got_inst.status, "running");

        update_instance_status(&p, instance.id, "success", false, None, Some(now()))
            .await
            .unwrap();
        assert_eq!(
            find_instance_by_id(&p, instance.id).await.unwrap().status,
            "success"
        );

        // snapshot upsert/find/delete
        upsert_snapshot(&p, instance.id, &json!({"pool": {}}))
            .await
            .unwrap();
        assert!(find_snapshot(&p, instance.id).await.unwrap().is_some());
        delete_snapshot(&p, instance.id).await.unwrap();
        assert!(find_snapshot(&p, instance.id).await.unwrap().is_none());
    }
}
