mod transform;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Result, Type, parse_macro_input};

/// Attribute macro that transforms a unit struct into an aggregate newtype.
///
/// Usage:
/// ```ignore
/// #[mnesis::aggregate(state = MyState, error = MyError, id = MyId)]
/// struct MyAggregate;
/// ```
///
/// Generates `impl Aggregate` for the unit struct (a type-level marker) plus a
/// convenience `Name::new(id) -> AggregateRoot<Self>` constructor. Only nongeneric
/// unit structs without where clauses are supported; duplicate arguments are
/// rejected and user attributes/visibility are preserved. Implement
/// `Handle<C>` on the marker as `handle(state, cmd) -> events`.
#[proc_macro_attribute]
pub fn aggregate(attr: TokenStream, item: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(item as DeriveInput);
    let args = attr;
    match parse_aggregate(&ast, args.into()) {
        Ok(code) => code,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

/// Derive event names for a nonempty enum, retaining type, lifetime and const
/// parameters and where clauses. The enum must satisfy `Message`'s existing
/// `Send + Sync + Debug + 'static` bounds; declare those bounds on generic input.
#[proc_macro_derive(DomainEvent)]
pub fn domain_event(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    match parse_domain_event(&ast) {
        Ok(code) => code,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

fn parse_domain_event(ast: &DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();
    match &ast.data {
        Data::Enum(data_enum) => {
            if data_enum.variants.is_empty() {
                return Err(Error::new(
                    name.span(),
                    "DomainEvent enum must have at least one variant.",
                ));
            }

            let variant_arms: Vec<_> = data_enum
                .variants
                .iter()
                .map(|variant| {
                    let variant_ident = &variant.ident;
                    let variant_name = variant_ident.to_string();
                    match &variant.fields {
                        syn::Fields::Unit => {
                            quote! { #name::#variant_ident => #variant_name }
                        }
                        syn::Fields::Unnamed(_) => {
                            quote! { #name::#variant_ident(..) => #variant_name }
                        }
                        syn::Fields::Named(_) => {
                            quote! { #name::#variant_ident { .. } => #variant_name }
                        }
                    }
                })
                .collect();

            let expanded = quote! {
                impl #impl_generics ::mnesis::Message for #name #ty_generics #where_clause {}

                impl #impl_generics ::mnesis::DomainEvent for #name #ty_generics #where_clause {
                    fn name(&self) -> &'static str {
                        match self {
                            #(#variant_arms),*
                        }
                    }
                }
            };

            Ok(expanded)
        }
        Data::Struct(_) => Err(Error::new(
            name.span(),
            "DomainEvent derive requires an enum. Wrap event structs in an enum: `enum MyEvent { Created(Created), ... }`",
        )),
        Data::Union(_) => Err(Error::new(name.span(), "Unions are not supported.")),
    }
}

/// Generates a unit struct with inherent `upcast` and `current_version`
/// functions from annotated transform functions.
///
/// # Attributes
///
/// - `aggregate = Type` — a documentation label; parsed as a Rust type but does
///   not create an aggregate trait implementation or check payload compatibility
/// - `error = Type` — the error type returned by transform functions
///
/// Each method must be annotated with `#[transform(...)]`:
/// - `event = "EventName"` — the event type this transform handles
/// - `from = N` — source schema version (>= 1)
/// - `to = N` — target schema version (must be `from + 1`)
/// - `rename = "NewName"` — optional event type rename
///
/// # Compile-time validation
///
/// - `from >= 1`
/// - `to == from + 1` for each transform (contiguity per step)
/// - No duplicate `(event, from)` pairs
/// - Each non-initial source `(name, schema)` has a predecessor, including
///   renamed destinations; each declared node below its name's latest schema
///   has a successor. Gaps and ambiguous outgoing edges are rejected.
/// - Every step increases schema by one; revisiting a name is allowed and cannot
///   create a cycle in the `(name, schema)` graph.
/// - Schema IDs fit nonzero u32; successor arithmetic is checked.
///
/// # Supported syntax
///
/// Use a nongeneric inherent impl of a single unqualified marker name, containing
/// functions and constants. Annotated transforms must be nongeneric, synchronous,
/// safe Rust functions with one payload argument, no receiver and an explicit
/// return type. Rust checks the payload/result types against the generated call.
/// Conditional individual transforms are rejected; condition the entire impl.
/// Helper functions and constants are preserved. `upcast` and `current_version`
/// are reserved. Duplicate arguments and transform attributes are rejected.
///
/// # Emitted output
///
/// The macro emits a `pub struct <Name>;` plus an inherent impl block
/// carrying the user's transform functions (with `#[transform]` attrs
/// stripped) and two associated functions:
///
/// - `pub fn upcast<'a>(EventMorsel<'a>) -> Result<EventMorsel<'a>, TransformError<Error>>` —
///   runs the chain to current schema version. Associated (no `&self`)
///   so call sites are `OrderTransforms::upcast(morsel)` — a `'static`
///   function pointer pluggable into [`EventStore::load_with`](https://docs.rs/mnesis-store/latest/mnesis_store/struct.EventStore.html#method.load_with). Known names with
///   undeclared schemas return `TransformError::UnsupportedSchema`; user failures
///   retain their source in `TransformError::Transform`. Unknown names pass through.
/// - `pub fn current_version(event_type: &str) -> Option<SchemaVersion>` — the
///   latest schema declared for that exact name, used on writes. A rename
///   destination has its own schema; the retired source name keeps its
///   pre-rename schema. Unknown names return `None`. Also associated; call as
///   `OrderTransforms::current_version("EventName")`.
///
/// # Example
///
/// ```ignore
/// #[mnesis::transforms(aggregate = Order, error = MyError)]
/// impl OrderTransforms {
///     #[transform(event = "OrderCreated", from = 1, to = 2)]
///     fn v1_to_v2(payload: &[u8]) -> Result<Vec<u8>, MyError> {
///         Ok(payload.to_vec())
///     }
/// }
///
/// // Direct call:
/// let upgraded = OrderTransforms::upcast(morsel)?;
///
/// // Plugged into the facade:
/// let root = store.load_with(id, |_| Ok::<_, core::convert::Infallible>(()), OrderTransforms::upcast).await?;
/// ```
#[proc_macro_attribute]
pub fn transforms(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = attr;
    let ast = parse_macro_input!(item as syn::ItemImpl);
    match transform::expand(&ast, args.into()) {
        Ok(code) => code,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

fn parse_aggregate(
    ast: &DeriveInput,
    args: proc_macro2::TokenStream,
) -> Result<proc_macro2::TokenStream> {
    let name = &ast.ident;
    let vis = &ast.vis;
    // Preserve user attributes (#[cfg(...)], #[doc = "..."], etc.)
    let user_attrs = &ast.attrs;

    if !ast.generics.params.is_empty() || ast.generics.where_clause.is_some() {
        return Err(Error::new_spanned(
            &ast.generics,
            "aggregate markers cannot have generics or where clauses",
        ));
    }

    // Only unit structs allowed
    match &ast.data {
        Data::Struct(data) => {
            if !matches!(data.fields, syn::Fields::Unit) {
                return Err(Error::new(
                    name.span(),
                    "aggregate macro requires a unit struct (no fields).",
                ));
            }
        }
        _ => {
            return Err(Error::new(
                name.span(),
                "aggregate macro only works on unit structs.",
            ));
        }
    }

    // Parse state = ..., error = ..., id = ... from attribute args
    let mut state_type: Option<Type> = None;
    let mut error_type: Option<Type> = None;
    let mut id_type: Option<Type> = None;

    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("state") {
            transform::optional_argument(&mut state_type, meta)?;
        } else if meta.path.is_ident("error") {
            transform::optional_argument(&mut error_type, meta)?;
        } else if meta.path.is_ident("id") {
            transform::optional_argument(&mut id_type, meta)?;
        } else {
            return Err(meta.error("expected `state`, `error`, or `id`"));
        }
        Ok(())
    });

    syn::parse::Parser::parse2(parser, args)?;

    let state_type = state_type.ok_or_else(|| Error::new(name.span(), "`state` is required"))?;
    let error_type = error_type.ok_or_else(|| Error::new(name.span(), "`error` is required"))?;
    let id_type = id_type.ok_or_else(|| Error::new(name.span(), "`id` is required"))?;

    let expanded = quote! {
        #(#user_attrs)*
        #vis struct #name;

        impl ::mnesis::Aggregate for #name {
            type State = #state_type;
            type Error = #error_type;
            type Id = #id_type;
        }

        impl #name {
            /// Create a fresh aggregate at initial state (version `None`).
            ///
            /// Returns the live [`AggregateRoot`](::mnesis::AggregateRoot) — the
            /// stateful container. The aggregate type itself is a marker; this
            /// is a convenience entry point for
            /// `AggregateRoot::<Self>::new(id)`.
            #[must_use]
            #vis fn new(id: #id_type) -> ::mnesis::AggregateRoot<Self> {
                ::mnesis::AggregateRoot::new(id)
            }
        }
    };

    Ok(expanded)
}
