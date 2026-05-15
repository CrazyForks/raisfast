use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{Data, DeriveInput, Fields, ItemStruct, ItemTrait, Lit, TraitItem, parse_macro_input};

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

        let combined_pattern = match &variant.fields {
            Fields::Unit => quote! { #name::#ident },
            _ => pattern.clone(),
        };

        let name_arm = if is_dynamic {
            quote! { #pattern => ::std::borrow::Cow::Owned(event_type.clone()) }
        } else if let Some(ref custom) = custom_name {
            let lit = custom.as_str();
            quote! { #combined_pattern => ::std::borrow::Cow::Borrowed(#lit) }
        } else {
            quote! { #combined_pattern => ::std::borrow::Cow::Borrowed(#snake) }
        };
        name_arms.push(name_arm);

        let display_arm = if is_dynamic {
            quote! { #pattern => ::std::borrow::Cow::Owned(event_type.clone()) }
        } else {
            quote! { #combined_pattern => ::std::borrow::Cow::Borrowed(#ident_str) }
        };
        display_arms.push(display_arm);

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

struct DelegateAttr {
    fn_name: Option<String>,
    model: Option<String>,
    ok: bool,
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

fn parse_delegate_attr(attrs: &[syn::Attribute]) -> (Option<DelegateAttr>, Vec<syn::Attribute>) {
    let mut delegate = None;
    let mut retained = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("delegate") {
            let mut d = DelegateAttr {
                fn_name: None,
                model: None,
                ok: false,
            };
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("fn") {
                    let val: syn::LitStr = meta.value()?.parse()?;
                    d.fn_name = Some(val.value());
                } else if meta.path.is_ident("model") {
                    let val: syn::LitStr = meta.value()?.parse()?;
                    d.model = Some(val.value());
                } else if meta.path.is_ident("ok") {
                    d.ok = true;
                }
                Ok(())
            });
            delegate = Some(d);
        } else {
            retained.push(attr.clone());
        }
    }
    (delegate, retained)
}

#[proc_macro_attribute]
pub fn repository(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut model_val: Option<String> = None;
    let mut struct_name_val: Option<syn::Ident> = None;

    let parse_result = syn::parse::Parser::parse(
        |input: syn::parse::ParseStream| {
            while !input.is_empty() {
                let key: syn::Ident = input.parse()?;
                let _: syn::Token![=] = input.parse()?;
                if key == "model" {
                    let val: syn::LitStr = input.parse()?;
                    model_val = Some(val.value());
                } else if key == "struct_name" {
                    let val: syn::Ident = input.parse()?;
                    struct_name_val = Some(val);
                } else {
                    return Err(syn::Error::new(
                        key.span(),
                        "expected `model` or `struct_name`",
                    ));
                }
                if !input.is_empty() {
                    let _: syn::Token![,] = input.parse()?;
                }
            }
            Ok(())
        },
        attr,
    );

    if let Err(e) = parse_result {
        return e.to_compile_error().into();
    }

    let default_model = match model_val {
        Some(m) => m,
        None => {
            return syn::Error::new(Span::call_site(), "missing `model = \"...\"` attribute")
                .to_compile_error()
                .into();
        }
    };
    let struct_name = match struct_name_val {
        Some(s) => s,
        None => {
            return syn::Error::new(Span::call_site(), "missing `struct_name = Ident` attribute")
                .to_compile_error()
                .into();
        }
    };

    let mut input = parse_macro_input!(item as ItemTrait);
    let trait_name = &input.ident;

    let mut impl_methods = Vec::new();

    for item in &mut input.items {
        if let TraitItem::Fn(method) = item {
            let (delegate_attr, clean_attrs) = parse_delegate_attr(&method.attrs);
            method.attrs = clean_attrs;

            let sig = &method.sig;
            let method_name = &sig.ident;

            let mut arg_names: Vec<syn::Ident> = Vec::new();
            for input in &sig.inputs {
                if let syn::FnArg::Typed(pat_type) = input
                    && let syn::Pat::Ident(pat_ident) = &*pat_type.pat
                {
                    arg_names.push(pat_ident.ident.clone());
                }
            }

            let d = delegate_attr.unwrap_or(DelegateAttr {
                fn_name: None,
                model: None,
                ok: false,
            });

            let target_fn = d
                .fn_name
                .map(|s| syn::Ident::new(&s, method_name.span()))
                .unwrap_or_else(|| method_name.clone());

            let model_ident = d.model.unwrap_or_else(|| default_model.clone());
            let model_path: syn::Path =
                syn::parse_str(&format!("crate::models::{}", model_ident)).unwrap();

            let call = if d.ok {
                quote! {
                    #sig {
                        Ok(#model_path::#target_fn(&self.pool, #(#arg_names),*).await.ok())
                    }
                }
            } else {
                quote! {
                    #sig {
                        #model_path::#target_fn(&self.pool, #(#arg_names),*).await
                    }
                }
            };

            impl_methods.push(call);
        }
    }

    let expanded = quote! {
        #[async_trait::async_trait]
        #input

        pub struct #struct_name {
            pool: crate::db::Pool,
        }

        impl #struct_name {
            #[must_use]
            pub fn new(pool: crate::db::Pool) -> Self {
                Self { pool }
            }
        }

        #[async_trait::async_trait]
        impl #trait_name for #struct_name {
            #(#impl_methods)*
        }
    };

    TokenStream::from(expanded)
}

// ---------------------------------------------------------------------------
// #[aspect_service(entity = "tags", model = Tag)]
// ---------------------------------------------------------------------------
//
// Generates for a service struct:
//   - `new(...)` constructor from all fields
//   - `before_create<T>`, `before_update<T>`, `before_delete` hooks
//   - `after_created`, `after_updated`, `after_deleted` event emitters
//
// The `#[engine]` attribute marks which field is the AspectEngine.
// All other fields become plain constructor parameters.
//
// Usage:
//   #[aspect_service(entity = "tags", model = Tag)]
//   pub struct TagServiceImpl {
//       #[engine] aspect_engine: Arc<AspectEngine>,
//       repo: Arc<dyn TagRepository>,
//   }
// ---------------------------------------------------------------------------

#[proc_macro_attribute]
pub fn aspect_service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut entity: Option<String> = None;
    let mut model_ident: Option<syn::Ident> = None;

    let parse_result = syn::parse::Parser::parse(
        |input: syn::parse::ParseStream| {
            while !input.is_empty() {
                let key: syn::Ident = input.parse()?;
                let _: syn::Token![=] = input.parse()?;
                if key == "entity" {
                    let val: syn::LitStr = input.parse()?;
                    entity = Some(val.value());
                } else if key == "model" {
                    let val: syn::Ident = input.parse()?;
                    model_ident = Some(val);
                } else {
                    return Err(syn::Error::new(key.span(), "expected `entity` or `model`"));
                }
                if !input.is_empty() {
                    let _: syn::Token![,] = input.parse()?;
                }
            }
            Ok(())
        },
        attr,
    );

    if let Err(e) = parse_result {
        return e.to_compile_error().into();
    }

    let entity_str = match entity {
        Some(e) => e,
        None => {
            return syn::Error::new(Span::call_site(), "missing `entity = \"...\"` attribute")
                .to_compile_error()
                .into();
        }
    };
    let model = match model_ident {
        Some(m) => m,
        None => {
            return syn::Error::new(Span::call_site(), "missing `model = Ident` attribute")
                .to_compile_error()
                .into();
        }
    };

    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;

    // Find #[engine] field and collect all fields for constructor
    let mut engine_field: Option<syn::Ident> = None;
    let mut field_names: Vec<syn::Ident> = Vec::new();
    let mut field_types: Vec<syn::Type> = Vec::new();

    for field in input.fields.iter() {
        let fname = field.ident.clone().unwrap();
        let is_engine = field.attrs.iter().any(|a| a.path().is_ident("engine"));
        if is_engine {
            engine_field = Some(fname.clone());
        }
        field_names.push(fname);
        field_types.push(field.ty.clone());
    }

    let engine = match engine_field {
        Some(e) => e,
        None => {
            return syn::Error::new(Span::call_site(), "no field marked with `#[engine]` found")
                .to_compile_error()
                .into();
        }
    };

    // Strip #[engine] from field attributes for the clean struct output
    let mut clean_input = input.clone();
    for field in clean_input.fields.iter_mut() {
        field.attrs.retain(|a| !a.path().is_ident("engine"));
    }

    // Event variant names: TagCreated, TagUpdated, TagDeleted
    let model_str = model.to_string();
    let event_created = syn::Ident::new(&format!("{}Created", model_str), model.span());
    let event_updated = syn::Ident::new(&format!("{}Updated", model_str), model.span());
    let event_deleted = syn::Ident::new(&format!("{}Deleted", model_str), model.span());

    let expanded = quote! {
        #clean_input

        impl #struct_name {
            pub fn new(#(#field_names: #field_types),*) -> Self {
                Self { #(#field_names),* }
            }

            async fn before_create<T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send>(
                &self,
                auth: &crate::middleware::auth::AuthUser,
                req: T,
            ) -> crate::errors::app_error::AppResult<(T, crate::aspects::Dispatched)> {
                self.#engine.before_create(#entity_str, auth, req).await
            }

            async fn before_update<T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send>(
                &self,
                auth: &crate::middleware::auth::AuthUser,
                existing: &#model,
                req: T,
            ) -> crate::errors::app_error::AppResult<(T, crate::aspects::Dispatched)> {
                self.#engine.before_update(#entity_str, auth, existing, req).await
            }

            async fn before_delete(
                &self,
                auth: &crate::middleware::auth::AuthUser,
                existing: &#model,
            ) -> crate::errors::app_error::AppResult<crate::aspects::Dispatched> {
                self.#engine.before_delete(#entity_str, auth, existing).await
            }

            fn after_created(&self, entity: &#model) {
                self.#engine.emit(crate::event::Event::#event_created(entity.clone()));
            }

            fn after_updated(&self, entity: &#model) {
                self.#engine.emit(crate::event::Event::#event_updated(entity.clone()));
            }

            fn after_deleted(&self, entity: &#model) {
                self.#engine.emit(crate::event::Event::#event_deleted(entity.clone()));
            }
        }
    };

    TokenStream::from(expanded)
}
