use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Lit, parse_macro_input};

#[proc_macro_derive(EventMeta, attributes(event))]
pub fn derive_event_meta(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

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
        let snake = pascal_to_on_snake(&ident_str);

        // Parse #[event(table = "posts", name = "custom", dynamic)]
        let mut table_val: Option<String> = None;
        let mut custom_name: Option<String> = None;
        let mut is_dynamic = false;

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

        // Build match pattern
        let pattern = match &variant.fields {
            Fields::Named(_fields) if is_dynamic => {
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

        // Combine unit + struct patterns (unit variant matches both)
        let combined_pattern = match &variant.fields {
            Fields::Unit => quote! { #name::#ident },
            _ => pattern.clone(),
        };

        // name() arm
        let name_arm = if is_dynamic {
            quote! { #pattern => ::std::borrow::Cow::Owned(event_type.clone()) }
        } else if let Some(ref custom) = custom_name {
            let lit = custom.as_str();
            quote! { #combined_pattern => ::std::borrow::Cow::Borrowed(#lit) }
        } else {
            quote! { #combined_pattern => ::std::borrow::Cow::Borrowed(#snake) }
        };
        name_arms.push(name_arm);

        // display_name() arm
        let display_arm = if is_dynamic {
            quote! { #pattern => ::std::borrow::Cow::Owned(event_type.clone()) }
        } else {
            quote! { #combined_pattern => ::std::borrow::Cow::Borrowed(#ident_str) }
        };
        display_arms.push(display_arm);

        // table() arm
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
            pub fn name(&self) -> ::std::borrow::Cow<'static, str> {
                match self {
                    #(#name_arms),*
                }
            }

            pub fn display_name(&self) -> ::std::borrow::Cow<'static, str> {
                match self {
                    #(#display_arms),*
                }
            }

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
/// `PostCreated` → `"on_post_created"`
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
