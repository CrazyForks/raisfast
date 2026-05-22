use raisfast::*;
use ts_rs::TS;

macro_rules! collect {
    ($($ty:ty),* $(,)?) => {{
        let mut seen = std::collections::HashSet::new();
        let mut decls: Vec<(String, String)> = Vec::new();
        let cfg = ts_rs::Config::default();
        $(
            let name = <$ty>::name(&cfg);
            if !seen.contains(&name) {
                seen.insert(name.clone());
                let decl = <$ty>::decl(&cfg);
                decls.push((name, decl));
            }
        )*
        decls.sort_by(|a, b| a.0.cmp(&b.0));
        decls
    }};
}

fn main() {
    let decls = collect![
        dto::CredentialResponse,
        dto::AuthConfigResponse,
        dto::UserResponse,
        dto::LoginResponse,
        dto::RegisterRequest,
        dto::LoginRequest,
        dto::RefreshRequest,
        dto::UpdateUserRequest,
        dto::UpdatePasswordRequest,
        dto::UpdateRoleRequest,
        dto::ForgotPasswordRequest,
        dto::ResetPasswordRequest,
        dto::SetPasswordRequest,
        dto::SendSmsCodeRequest,
        dto::VerifySmsRequest,
        dto::BindPhoneRequest,
        dto::BindEmailRequest,
        dto::VerifyEmailRequest,
        dto::ResendVerificationRequest,
        dto::PostResponse,
        dto::CreatePostRequest,
        dto::UpdatePostRequest,
        dto::CreateCommentRequest,
        dto::UpdateCommentStatusRequest,
        dto::WalletResponse,
        dto::WalletTransactionResponse,
        dto::AdminWalletOperationRequest,
        dto::ReversalRequest,
        dto::CurrencyResponse,
        dto::CreateCurrencyRequest,
        dto::UpdateCurrencyRequest,
        dto::BatchRequest,
        dto::BatchRequestWithRole,
        dto::BatchResponse,
        dto::CreateCategoryRequest,
        dto::UpdateCategoryRequest,
        dto::CreateTagRequest,
        dto::UpdateTagRequest,
        dto::MediaResponse,
        dto::MediaStatsResponse,
        dto::MediaTypeInfoResponse,
        dto::ProviderInfo,
        dto::CreatePageRequest,
        dto::UpdatePageRequest,
        dto::PageListQuery,
        dto::AdminPageListQuery,
        dto::UpdateStatusRequest,
        dto::ReorderRequest,
        dto::ReorderItem,
        dto::SitemapEntry,
        dto::AdminCommentListQuery,
        workflow::handler::CreateWorkflowRequest,
        workflow::handler::StartWorkflowRequest,
        workflow::handler::ExecuteStepRequest,
        workflow::handler::InstanceQuery,
        models::user::UserRole,
        models::user::UserStatus,
        models::user::RegisteredVia,
        models::user_credential::AuthType,
        models::post::PostStatus,
        models::post::CommentOpenStatus,
        models::comment::CommentStatus,
        models::page::PageStatus,
        models::wallet::WalletStatus,
        models::wallet_transaction::WalletEntryType,
        models::wallet_transaction::WalletTxType,
        models::wallet_transaction::WalletReferenceType,
        models::tenant::TenantStatus,
        models::options::OptionType,
        worker::JobStatus,
        worker::CronExecStatus,
        workflow::model::WorkflowInstanceStatus,
        workflow::model::WorkflowStepStatus,
        models::api_token::ApiTokenListItem,
        models::category::Category,
        models::comment::CommentResponse,
        models::comment::AdminCommentRow,
        models::content_revision::ContentRevision,
        models::content_revision::RevisionSummary,
        models::page::Page,
        models::reusable_block::ReusableBlock,
        models::page::PageBlock,
        models::page::GalleryImage,
        models::page::TestimonialItem,
        models::page::FaqItem,
        models::page::StatItem,
        models::page::TimelineItem,
        models::page::TeamMember,
        models::page::SocialLink,
        models::page::PricingPlan,
        models::page::FormFieldDef,
        models::page::ColumnDef,
        models::post::TagBrief,
        models::rbac::Role,
        models::rbac::Permission,
        models::tag::Tag,
        models::tenant::Tenant,
        workflow::StepType,
        workflow::StepDef,
        workflow::WorkflowDefinition,
        workflow::WorkflowInstance,
        workflow::StepLog,
        plugins::PluginHealth,
        plugins::PluginMetrics,
        plugins::PluginEvent,
        plugins::PluginInfoResponse,
        plugins::Permissions,
        webhook::model::WebhookSubscription,
        models::audit_log::AuditEntry,
        worker::CronSchedule,
        worker::CronExecutionLog,
        services::options::OptionGroup,
        services::options::OptionEntry,
        services::api_token::CreateTokenResult,
        services::oauth::OAuthBindingInfo,
        content_type::schema::ContentKind,
        content_type::schema::ContentTypeSchema,
        content_type::schema::FieldSchema,
        content_type::schema::FieldType,
        content_type::schema::RelationType,
        content_type::schema::RelationConfig,
        content_type::schema::MediaConfig,
        content_type::schema::IndexDef,
        content_type::schema::ApiAccess,
        content_type::schema::ApiEndpointConfig,
        content_type::schema::ApiConfig,
    ];

    let mut out = String::from(
        "// Auto-generated by `cargo run --bin export-types`\n// DO NOT EDIT MANUALLY\n\n",
    );

    for (_name, decl) in &decls {
        out.push_str(&format_type_decl(decl));
        out.push_str("\n\n");
    }

    let out_dir = "frontend/sdk/src/generated";
    std::fs::create_dir_all(out_dir).unwrap();
    let out_path = format!("{out_dir}/types.ts");
    std::fs::write(&out_path, out).unwrap();

    println!("Exported {} types to {}", decls.len(), out_path);
}

fn format_type_decl(decl: &str) -> String {
    let decl = decl.trim();
    if !decl.starts_with("type ") {
        return decl.to_string();
    }
    let rest = &decl[5..];

    let eq_pos = match rest.find(" = ") {
        Some(p) => p,
        None => return format!("export type {rest};"),
    };

    let name_part = &rest[..eq_pos];
    let body_raw = rest[(eq_pos + 3)..].trim_end_matches(';').trim();

    let variants = split_top_level(body_raw, " | ");

    if variants.len() > 1 {
        let all_strings = variants.iter().all(|v: &&str| v.trim().starts_with('"'));
        if all_strings {
            let keys: Vec<String> = variants
                .iter()
                .map(|v: &&str| {
                    let v = v.trim();
                    let key = v.trim_matches('"');
                    let key = key.replace('-', "_");
                    format!("  {key}: {v}")
                })
                .collect();
            let const_name = name_part.replace(" ", "");
            format!(
                "export const {const_name} = {{\n{}\n}} as const;\nexport type {const_name} = typeof {const_name}[keyof typeof {const_name}];",
                keys.join(",\n")
            )
        } else {
            let fmt: Vec<String> = variants
                .iter()
                .map(|v: &&str| {
                    let v = v.trim();
                    if v.starts_with('{') && v.ends_with('}') {
                        format_object(v, 2)
                    } else {
                        v.to_string()
                    }
                })
                .collect();
            format!("export type {name_part} =\n  | {};", fmt.join("\n  | "))
        }
    } else if body_raw.starts_with('{') {
        format!("export type {name_part} = {};", format_object(body_raw, 0))
    } else {
        format!("export type {name_part} = {body_raw};")
    }
}

fn format_object(obj: &str, base_indent: usize) -> String {
    let open = match obj.find('{') {
        Some(p) => p,
        None => return obj.to_string(),
    };
    let close = match obj.rfind('}') {
        Some(p) => p,
        None => return obj.to_string(),
    };
    if open >= close {
        return obj.to_string();
    }

    let inner = &obj[(open + 1)..close];
    let fields = split_top_level(inner, ",");

    let field_indent = " ".repeat(base_indent + 2);
    let close_indent = " ".repeat(base_indent);

    let mut result = String::from("{\n");
    for field in &fields {
        let field: &str = field.trim();
        if field.is_empty() {
            continue;
        }
        result.push_str(&reindent_block(field, &field_indent));
    }
    result.push_str(&close_indent);
    result.push('}');
    result
}

fn reindent_block(block: &str, indent: &str) -> String {
    let non_empty: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();

    if non_empty.is_empty() {
        return String::new();
    }

    let min_indent = non_empty
        .iter()
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    let mut result = String::new();
    for (idx, line) in non_empty.iter().enumerate() {
        let leading = line.len() - line.trim_start().len();
        let strip = leading.min(min_indent);
        result.push_str(indent);
        result.push_str(&line[strip..]);
        if idx == non_empty.len() - 1 {
            result.push(';');
        }
        result.push('\n');
    }
    result
}

fn split_top_level<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let mut result = Vec::new();
    let mut depth: i32 = 0;
    let mut angle_depth: i32 = 0;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut start = 0;
    let bytes = s.as_bytes();
    let sep_len = sep.len();

    let mut i = 0;
    while i < s.len() {
        if !in_line_comment && !in_block_comment && i + 1 < s.len() {
            if bytes[i] == b'/' && bytes[i + 1] == b'/' {
                in_line_comment = true;
                i += 2;
                continue;
            }
            if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                in_block_comment = true;
                i += 2;
                continue;
            }
        }

        if in_line_comment {
            if bytes[i] == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }

        if in_block_comment {
            if i + 1 < s.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }

        if bytes[i] == b'{' {
            depth += 1;
        } else if bytes[i] == b'}' {
            depth -= 1;
        } else if bytes[i] == b'<' {
            angle_depth += 1;
        } else if bytes[i] == b'>' {
            angle_depth -= 1;
        } else if depth == 0
            && angle_depth == 0
            && i + sep_len <= s.len()
            && &s[i..i + sep_len] == sep
        {
            result.push(&s[start..i]);
            i += sep_len;
            start = i;
            continue;
        }
        i += 1;
    }

    let remaining = &s[start..];
    if !remaining.trim().is_empty() {
        result.push(remaining);
    }
    result
}
