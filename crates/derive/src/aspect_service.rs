//! `#[aspect_service(entity = "...", model = Type)]` — service struct boilerplate generator.
//!
//! This attribute macro is applied to a service struct and generates:
//!
//! 1. **`new(...)` constructor** — takes all fields as parameters.
//! 2. **`after_created(entity)`** — emits a `{Model}Created` domain event.
//! 3. **`after_updated(entity)`** — emits a `{Model}Updated` domain event.
//! 4. **`after_deleted(entity)`** — emits a `{Model}Deleted` domain event.
//!
//! The struct must have an `emitter: EventEmitter` field for event emission.
//! The `entity` attribute is accepted for backwards compatibility but no longer
//! used (aspects are now handled by protocols in the content-type layer).
//! The `model` attribute specifies the domain model type (used for event variant
//! names and method signatures).
//!
//! # Example
//!
//! ```ignore
//! #[aspect_service(entity = "posts", model = Post)]
//! pub struct PostService {
//!     emitter: EventEmitter,
//!     pool: SqlitePool,
//! }
//! ```
//!
//! This generates:
//! - `PostService::new(emitter, pool)` constructor
//! - `post_service.after_created(&post)` → emits `Event::PostCreated(post.clone())`

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{ItemStruct, parse_macro_input};

pub fn aspect_service(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse the attribute: `entity = "posts", model = Post`
    // (`entity` is accepted for backwards compatibility but no longer used.)
    let mut model_ident: Option<syn::Ident> = None;

    let parse_result = syn::parse::Parser::parse(
        |input: syn::parse::ParseStream| {
            while !input.is_empty() {
                let key: syn::Ident = input.parse()?;
                let _: syn::Token![=] = input.parse()?;
                if key == "entity" {
                    let _: syn::LitStr = input.parse()?;
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

    // Extract field info (the legacy `#[engine]` attribute is stripped but no
    // longer required).
    let mut field_names: Vec<syn::Ident> = Vec::new();
    let mut field_types: Vec<syn::Type> = Vec::new();

    for field in input.fields.iter() {
        let fname = field.ident.clone().unwrap();
        field_names.push(fname);
        field_types.push(field.ty.clone());
    }

    // Create a clean copy of the struct without the `#[engine]` attribute
    // (the attribute was a legacy macro hint for the old aspect-engine field).
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
        // Emit the original struct definition (without legacy #[engine] attrs)
        #clean_input

        impl #struct_name {
            /// Constructor — takes all fields as parameters in declaration order.
            pub fn new(#(#field_names: #field_types),*) -> Self {
                Self { #(#field_names),* }
            }

            /// After-created hook — emits a {Model}Created domain event.
            fn after_created(&self, entity: &#model) {
                self.emitter.emit(crate::event::Event::#event_created(entity.clone()));
            }

            /// After-updated hook — emits a {Model}Updated domain event.
            fn after_updated(&self, entity: &#model) {
                self.emitter.emit(crate::event::Event::#event_updated(entity.clone()));
            }

            /// After-deleted hook — emits a {Model}Deleted domain event.
            fn after_deleted(&self, entity: &#model) {
                self.emitter.emit(crate::event::Event::#event_deleted(entity.clone()));
            }
        }
    };

    TokenStream::from(expanded)
}
