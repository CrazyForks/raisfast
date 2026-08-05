//! `#[derive(EventMeta)]` — auto-generate event metadata methods on enums.
//!
//! This derive macro generates four methods on an event enum:
//!
//! - `name()` — returns the event's internal hook name (e.g., `"on_post_created"`).
//!   Used by the event bus for type-based routing/filtering and WASM function naming.
//! - `display_name()` — returns the PascalCase variant name (e.g., `"PostCreated"`).
//!   Used for human-readable logging.
//! - `table()` — returns `Some("table_name")` if the event is associated with a DB table,
//!   or `None` otherwise. Metadata only — no longer used for inference.
//! - `event_name()` — returns `Some("post.created")` if the variant declares
//!   `#[event(event_name = "...")]`. This is the stable external contract name used by
//!   webhooks, SSE, and audit logs. Variants without `event_name` do not produce
//!   external events.
//!
//! # Per-variant attributes
//!
//! ```ignore
//! #[derive(EventMeta)]
//! enum Event {
//!     #[event(table = "posts", event_name = "post.created")]
//!     PostCreated(Post),
//!
//!     #[event(table = "posts")]
//!     PostCreating,
//!
//!     #[event(table = "users", event_name = "password_reset.requested")]
//!     PasswordResetRequested { user: User, token: Token },
//!
//!     #[event(dynamic)]
//!     Custom { event_type: String, data: Value },
//! }
//! ```
//!
//! - `table = "..."` — associates this variant with a database table (metadata).
//! - `name = "..."` — overrides the default `on_variant_name` hook name.
//! - `event_name = "..."` — declares the stable external event name (e.g., for webhooks/SSE).
//!   Variants without this attribute do not produce external events.
//! - `dynamic` — the event name comes from a runtime `event_type` field instead of
//!   a static string. The variant must have a named field called `event_type`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Lit};

pub fn derive_event_meta(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Only support enums
    let variants = match &input.data {
        Data::Enum(data) => &data.variants,
        _ => {
            return syn::Error::new_spanned(&input, "EventMeta only supports enums")
                .to_compile_error()
                .into();
        }
    };

    let mut name_arms = Vec::new();
    let mut display_arms = Vec::new();
    let mut table_arms = Vec::new();
    let mut event_name_arms = Vec::new();
    let mut all_event_names: Vec<String> = Vec::new();

    for variant in variants {
        let ident = &variant.ident;
        let ident_str = ident.to_string();
        // Convert PascalCase to on_snake_case (e.g., PostCreated → on_post_created)
        let snake = pascal_to_on_snake(&ident_str);

        let mut table_val: Option<String> = None;
        let mut custom_name: Option<String> = None;
        let mut event_name_val: Option<String> = None;
        let mut is_dynamic = false;

        // Parse #[event(table = "...", name = "...", event_name = "...", dynamic)]
        for attr in &variant.attrs {
            if !attr.path().is_ident("event") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("table") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(lit) = value {
                        table_val = Some(lit.value());
                    }
                    Ok(())
                } else if meta.path.is_ident("name") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(lit) = value {
                        custom_name = Some(lit.value());
                    }
                    Ok(())
                } else if meta.path.is_ident("event_name") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(lit) = value {
                        event_name_val = Some(lit.value());
                    }
                    Ok(())
                } else if meta.path.is_ident("dynamic") {
                    is_dynamic = true;
                    Ok(())
                } else {
                    Err(meta.error("unsupported event attribute"))
                }
            })
            .unwrap_or(());
        }

        // Build match patterns that account for the variant's field style
        let pattern = match &variant.fields {
            Fields::Named(_fields) if is_dynamic => {
                // Dynamic: extract `event_type` from the variant's named fields
                quote! { #name::#ident { event_type, .. } }
            }
            Fields::Named(_) => {
                quote! { #name::#ident { .. } }
            }
            Fields::Unnamed(_) => {
                quote! { #name::#ident(_) }
            }
            Fields::Unit => {
                quote! { #name::#ident }
            }
        };

        // For non-dynamic variants, unit variants use a simple pattern
        let combined_pattern = match &variant.fields {
            Fields::Unit => quote! { #name::#ident },
            _ => pattern.clone(),
        };

        // name() arm: dynamic uses runtime event_type, custom uses override, default uses snake_case
        let name_arm = if is_dynamic {
            quote! { #pattern => ::std::borrow::Cow::Owned(event_type.clone()) }
        } else if let Some(ref custom) = custom_name {
            let lit = custom.as_str();
            quote! { #combined_pattern => ::std::borrow::Cow::Borrowed(#lit) }
        } else {
            quote! { #combined_pattern => ::std::borrow::Cow::Borrowed(#snake) }
        };
        name_arms.push(name_arm);

        // display_name() arm: always uses the PascalCase variant name (or dynamic event_type)
        let display_arm = if is_dynamic {
            quote! { #pattern => ::std::borrow::Cow::Owned(event_type.clone()) }
        } else {
            quote! { #combined_pattern => ::std::borrow::Cow::Borrowed(#ident_str) }
        };
        display_arms.push(display_arm);

        // table() arm: Some("table") if specified, None otherwise
        let table_arm = if let Some(ref table) = table_val {
            let lit = table.as_str();
            quote! { #combined_pattern => ::std::option::Option::Some(#lit) }
        } else {
            quote! { #combined_pattern => ::std::option::Option::None }
        };
        table_arms.push(table_arm);

        // event_name() arm: Some("...") if event_name attribute present, None otherwise.
        // Dynamic events use their runtime event_type as the external name.
        let event_name_arm = if is_dynamic {
            quote! { #pattern => ::std::option::Option::Some(::std::borrow::Cow::Owned(event_type.clone())) }
        } else if let Some(ref ev) = event_name_val {
            let lit = ev.as_str();
            all_event_names.push(ev.clone());
            quote! { #combined_pattern => ::std::option::Option::Some(::std::borrow::Cow::Borrowed(#lit)) }
        } else {
            quote! { #combined_pattern => ::std::option::Option::None }
        };
        event_name_arms.push(event_name_arm);
    }

    let expanded = quote! {
        impl #name {
            /// Returns the event's internal hook name (e.g., "on_post_created").
            /// For dynamic events, returns the runtime event_type string.
            pub fn name(&self) -> ::std::borrow::Cow<'static, str> {
                match self {
                    #(#name_arms),*
                }
            }

            /// Returns the display name (PascalCase variant name, e.g., "PostCreated").
            /// For dynamic events, returns the runtime event_type string.
            pub fn display_name(&self) -> ::std::borrow::Cow<'static, str> {
                match self {
                    #(#display_arms),*
                }
            }

            /// Returns Some("table_name") if this event is associated with a DB table.
            /// Returns None for events without a table association.
            pub fn table(&self) -> ::std::option::Option<&'static str> {
                match self {
                    #(#table_arms),*
                }
            }

            /// Returns the stable external event name (e.g., "post.created") if declared
            /// via `#[event(event_name = "...")]`.
            ///
            /// This is the canonical name consumed by webhooks, SSE, and audit logs.
            /// Variants without `event_name` return `None` and do not produce external events.
            /// Dynamic events return their runtime `event_type`.
            pub fn event_name(&self) -> ::std::option::Option<::std::borrow::Cow<'static, str>> {
                match self {
                    #(#event_name_arms),*
                }
            }

            /// Returns all declared external event names (e.g., `"post.created"`).
            ///
            /// This is the single source of truth — the frontend dropdown and TS export
            /// read from this list. Only variants with `#[event(event_name = "...")]`
            /// are included.
            pub fn all_event_names() -> &'static [&'static str] {
                &[#(#all_event_names),*]
            }
        }
    };

    TokenStream::from(expanded)
}

/// Convert PascalCase to `on_snake_case`.
///
/// Examples: `PostCreated` → `on_post_created`, `UserDeleted` → `on_user_deleted`.
fn pascal_to_on_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    result.push_str("on_");
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_ascii_lowercase());
    }
    result
}
