//! `#[aspect_service(entity = "...", model = Type)]` — service struct boilerplate generator.
//!
//! This attribute macro is applied to a service struct and generates:
//!
//! 1. **`new(...)` constructor** — takes all fields as parameters.
//! 2. **`before_create(auth, req)`** — delegates to the aspect engine's `before_create` hook.
//! 3. **`before_update(auth, existing, req)`** — delegates to the aspect engine's `before_update` hook.
//! 4. **`before_delete(auth, existing)`** — delegates to the aspect engine's `before_delete` hook.
//! 5. **`after_created(entity)`** — emits a `{Model}Created` domain event.
//! 6. **`after_updated(entity)`** — emits a `{Model}Updated` domain event.
//! 7. **`after_deleted(entity)`** — emits a `{Model}Deleted` domain event.
//!
//! # Requirements
//!
//! - The struct must have exactly one field annotated with `#[engine]` — this is the
//!   `AspectEngine` instance that handles aspect dispatching.
//! - The `entity` attribute specifies the entity name used for aspect dispatching.
//! - The `model` attribute specifies the domain model type (used for event variant names
//!   and method signatures).
//!
//! # Example
//!
//! ```ignore
//! #[aspect_service(entity = "posts", model = Post)]
//! pub struct PostService {
//!     #[engine]
//!     engine: AspectEngine,
//!     pool: SqlitePool,
//! }
//! ```
//!
//! This generates:
//! - `PostService::new(engine, pool)` constructor
//! - `post_service.before_create(auth, cmd)` → delegates to `engine.before_create("posts", auth, cmd)`
//! - `post_service.after_created(&post)` → emits `Event::PostCreated(post.clone())`

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{ItemStruct, parse_macro_input};

pub fn aspect_service(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse the attribute: `entity = "posts", model = Post`
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

    // Parse the struct definition
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;

    // Extract field info and find the #[engine]-annotated field
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

    // Create a clean copy of the struct without the #[engine] attribute
    // (the attribute is a macro hint, not meant for the runtime struct)
    let mut clean_input = input.clone();
    for field in clean_input.fields.iter_mut() {
        field.attrs.retain(|a| !a.path().is_ident("engine"));
    }

    // Derive event variant names from the model ident:
    // model = Post → PostCreated, PostUpdated, PostDeleted
    let model_str = model.to_string();
    let event_created = syn::Ident::new(&format!("{}Created", model_str), model.span());
    let event_updated = syn::Ident::new(&format!("{}Updated", model_str), model.span());
    let event_deleted = syn::Ident::new(&format!("{}Deleted", model_str), model.span());

    let expanded = quote! {
        // Emit the original struct definition (without #[engine] attrs)
        #clean_input

        impl #struct_name {
            /// Constructor — takes all fields as parameters in declaration order.
            pub fn new(#(#field_names: #field_types),*) -> Self {
                Self { #(#field_names),* }
            }

            /// Before-create hook — dispatches to the aspect engine.
            /// Returns the (possibly modified) request and a Dispatched result.
            async fn before_create<T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send>(
                &self,
                auth: &crate::middleware::auth::AuthUser,
                req: T,
            ) -> crate::errors::app_error::AppResult<(T, crate::aspects::Dispatched)> {
                self.#engine.before_create(#entity_str, auth, req).await
            }

            /// Before-update hook — dispatches to the aspect engine with the existing entity.
            async fn before_update<T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send>(
                &self,
                auth: &crate::middleware::auth::AuthUser,
                existing: &#model,
                req: T,
            ) -> crate::errors::app_error::AppResult<(T, crate::aspects::Dispatched)> {
                self.#engine.before_update(#entity_str, auth, existing, req).await
            }

            /// Before-delete hook — dispatches to the aspect engine with the existing entity.
            async fn before_delete(
                &self,
                auth: &crate::middleware::auth::AuthUser,
                existing: &#model,
            ) -> crate::errors::app_error::AppResult<crate::aspects::Dispatched> {
                self.#engine.before_delete(#entity_str, auth, existing).await
            }

            /// After-created hook — emits a {Model}Created domain event.
            fn after_created(&self, entity: &#model) {
                self.#engine.emit(crate::event::Event::#event_created(entity.clone()));
            }

            /// After-updated hook — emits a {Model}Updated domain event.
            fn after_updated(&self, entity: &#model) {
                self.#engine.emit(crate::event::Event::#event_updated(entity.clone()));
            }

            /// After-deleted hook — emits a {Model}Deleted domain event.
            fn after_deleted(&self, entity: &#model) {
                self.#engine.emit(crate::event::Event::#event_deleted(entity.clone()));
            }
        }
    };

    TokenStream::from(expanded)
}
