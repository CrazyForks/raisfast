//! `#[derive(EventMeta)]` — auto-generate event metadata methods on enums.
//!
//! This derive macro generates three methods on an event enum:
//!
//! - `name()` — returns the event's canonical name (e.g., `"on_post_created"`).
//!   Used by the event bus for type-based routing/filtering.
//! - `display_name()` — returns the PascalCase variant name (e.g., `"PostCreated"`).
//!   Used for human-readable logging and SSE event type tags.
//! - `table()` — returns `Some("table_name")` if the event is associated with a DB table,
//!   or `None` otherwise. Used by the plugin host to determine which table a query affects.
//!
//! # Per-variant attributes
//!
//! ```ignore
//! #[derive(EventMeta)]
//! enum Event {
//!     #[event(table = "posts")]
//!     PostCreated(Post),
//!
//!     #[event(table = "posts", name = "post_published")]
//!     PostUpdated(Post),
//!
//!     #[event(table = "posts")]
//!     PostDeleted(Post),
//!
//!     #[event(dynamic)]
//!     PluginEvent { event_type: String, payload: String },
//! }
//! ```
//!
//! - `table = "..."` — associates this variant with a database table.
//! - `name = "..."` — overrides the default `on_variant_name` event name.
//! - `dynamic` — the event name comes from a runtime `event_type` field instead of
//!   a static string. The variant must have a named field called `event_type`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Lit, parse_macro_input};

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

    for variant in variants {
        let ident = &variant.ident;
        let ident_str = ident.to_string();
        // Convert PascalCase to on_snake_case (e.g., PostCreated → on_post_created)
        let snake = pascal_to_on_snake(&ident_str);

        let mut table_val: Option<String> = None;
        let mut custom_name: Option<String> = None;
        let mut is_dynamic = false;

        // Parse #[event(table = "...", name = "...", dynamic)] attributes
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
    }

    let expanded = quote! {
        impl #name {
            /// Returns the event's canonical name (e.g., "on_post_created").
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
