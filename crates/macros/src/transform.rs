use std::collections::{BTreeMap, HashSet};

use proc_macro2::TokenStream;
use quote::quote;
use syn::meta::ParseNestedMeta;
use syn::parse::Parse;
use syn::{Error, ImplItem, ImplItemFn, ItemImpl, LitInt, LitStr, Result, Type};

struct Transform {
    method: syn::Ident,
    event: String,
    from: u32,
    to: u32,
    rename: Option<String>,
}

impl Transform {
    fn destination(&self) -> &str {
        self.rename.as_deref().unwrap_or(&self.event)
    }
}

pub(super) fn optional_argument<T: Parse>(
    slot: &mut Option<T>,
    meta: ParseNestedMeta<'_>,
) -> Result<()> {
    if slot.is_some() {
        return Err(meta.error("duplicate argument"));
    }
    *slot = Some(meta.value()?.parse()?);
    Ok(())
}

fn error_type(args: TokenStream) -> Result<Type> {
    let mut aggregate: Option<Type> = None;
    let mut error: Option<Type> = None;
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("aggregate") {
            optional_argument(&mut aggregate, meta)
        } else if meta.path.is_ident("error") {
            optional_argument(&mut error, meta)
        } else {
            Err(meta.error("expected `aggregate` or `error`"))
        }
    });
    syn::parse::Parser::parse2(parser, args)?;
    aggregate
        .ok_or_else(|| Error::new(proc_macro2::Span::call_site(), "`aggregate` is required"))?;
    error.ok_or_else(|| Error::new(proc_macro2::Span::call_site(), "`error` is required"))
}

fn marker_name(ast: &ItemImpl) -> Result<&syn::Ident> {
    if !ast.generics.params.is_empty() || ast.generics.where_clause.is_some() {
        return Err(Error::new_spanned(
            &ast.generics,
            "transform marker impls cannot have generics or where clauses",
        ));
    }
    if ast.trait_.is_some() || ast.unsafety.is_some() || ast.defaultness.is_some() {
        return Err(Error::new_spanned(
            ast,
            "transforms requires a plain inherent impl",
        ));
    }
    let Type::Path(path) = &*ast.self_ty else {
        return Err(Error::new_spanned(
            &ast.self_ty,
            "transform marker must be a single unqualified type name",
        ));
    };
    let mut segments = path.path.segments.iter();
    let Some(segment) = segments.next() else {
        return Err(Error::new_spanned(
            path,
            "transform marker requires a type name",
        ));
    };
    if path.qself.is_some()
        || path.path.leading_colon.is_some()
        || segments.next().is_some()
        || !matches!(segment.arguments, syn::PathArguments::None)
    {
        return Err(Error::new_spanned(
            path,
            "transform marker must be a single unqualified type name",
        ));
    }
    Ok(&segment.ident)
}

fn validate_method(method: &ImplItemFn) -> Result<()> {
    let signature = &method.sig;
    if !signature.generics.params.is_empty() || signature.generics.where_clause.is_some() {
        return Err(Error::new_spanned(
            &signature.generics,
            "transform functions cannot have generics or where clauses",
        ));
    }
    if signature.asyncness.is_some()
        || signature.unsafety.is_some()
        || signature.constness.is_some()
        || signature.abi.is_some()
        || signature.variadic.is_some()
    {
        return Err(Error::new_spanned(
            signature,
            "transform functions must be ordinary synchronous safe Rust functions",
        ));
    }
    if signature.inputs.len() != 1 || signature.receiver().is_some() {
        return Err(Error::new_spanned(
            &signature.inputs,
            "transform functions require exactly one payload argument and no receiver",
        ));
    }
    if matches!(signature.output, syn::ReturnType::Default) {
        return Err(Error::new_spanned(
            signature,
            "transform functions must return Result<Vec<u8>, the configured error type>",
        ));
    }
    for attribute in &method.attrs {
        if attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr") {
            return Err(Error::new_spanned(
                attribute,
                "conditional transform functions are unsupported; condition the entire transform impl instead",
            ));
        }
    }
    Ok(())
}

fn transform(method: &ImplItemFn) -> Result<Option<Transform>> {
    let mut definition = None;
    for attribute in &method.attrs {
        if !attribute.path().is_ident("transform") {
            continue;
        }
        if definition.is_some() {
            return Err(Error::new_spanned(
                attribute,
                "duplicate #[transform] attribute",
            ));
        }
        validate_method(method)?;
        let mut event: Option<LitStr> = None;
        let mut from: Option<LitInt> = None;
        let mut to: Option<LitInt> = None;
        let mut rename: Option<LitStr> = None;
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("event") {
                optional_argument(&mut event, meta)
            } else if meta.path.is_ident("from") {
                optional_argument(&mut from, meta)
            } else if meta.path.is_ident("to") {
                optional_argument(&mut to, meta)
            } else if meta.path.is_ident("rename") {
                optional_argument(&mut rename, meta)
            } else {
                Err(meta.error("expected `event`, `from`, `to`, or `rename`"))
            }
        })?;
        let event = event.ok_or_else(|| {
            Error::new_spanned(attribute, "`event` is required in #[transform(...)]")
        })?;
        let from = from.ok_or_else(|| {
            Error::new_spanned(attribute, "`from` is required in #[transform(...)]")
        })?;
        let to = to.ok_or_else(|| {
            Error::new_spanned(attribute, "`to` is required in #[transform(...)]")
        })?;
        let source: u32 = from.base10_parse()?;
        if source == 0 {
            return Err(Error::new_spanned(from, "from version must be >= 1"));
        }
        let destination: u32 = to.base10_parse()?;
        if destination == 0 {
            return Err(Error::new_spanned(to, "to version must be >= 1"));
        }
        let next = source.checked_add(1).ok_or_else(|| {
            Error::new_spanned(&from, "schema version cannot advance past u32::MAX")
        })?;
        if destination != next {
            return Err(Error::new_spanned(
                to,
                format!("non-contiguous version: to ({destination}) must equal from + 1 ({next})"),
            ));
        }
        definition = Some(Transform {
            method: method.sig.ident.clone(),
            event: event.value(),
            from: source,
            to: destination,
            rename: rename.map(|name| name.value()),
        });
    }
    Ok(definition)
}

fn definitions(ast: &ItemImpl) -> Result<Vec<Transform>> {
    let mut definitions = Vec::new();
    for item in &ast.items {
        let name = match item {
            ImplItem::Fn(method) => &method.sig.ident,
            ImplItem::Const(constant) => &constant.ident,
            _ => {
                return Err(Error::new_spanned(
                    item,
                    "transform impls support only functions and constants",
                ));
            }
        };
        if name == "upcast" || name == "current_version" {
            return Err(Error::new_spanned(
                name,
                "name is reserved for the generated transform API",
            ));
        }
        if let ImplItem::Fn(method) = item
            && let Some(definition) = transform(method)?
        {
            definitions.push(definition);
        }
    }
    Ok(definitions)
}

fn validate_graph(definitions: &[Transform]) -> Result<BTreeMap<&str, u32>> {
    let mut sources = HashSet::new();
    let mut versions = BTreeMap::new();
    let targets: HashSet<_> = definitions
        .iter()
        .map(|edge| (edge.destination(), edge.to))
        .collect();
    for edge in definitions {
        if !sources.insert((edge.event.as_str(), edge.from)) {
            return Err(Error::new_spanned(
                &edge.method,
                format!(
                    "duplicate transform for event '{}' at source version {}",
                    edge.event, edge.from
                ),
            ));
        }
        for (name, schema) in [
            (edge.event.as_str(), edge.from),
            (edge.destination(), edge.to),
        ] {
            let latest = versions.entry(name).or_insert(schema);
            *latest = (*latest).max(schema);
        }
    }
    // Versions increase by exactly one. Every non-root source must have an
    // incoming edge, so induction proves reachability from schema 1 without
    // enumerating the potentially enormous schema range. Each source has one
    // outgoing edge; converging migrations have no ambiguous dispatch.
    for edge in definitions {
        if edge.from != 1 && !targets.contains(&(edge.event.as_str(), edge.from)) {
            return Err(Error::new_spanned(
                &edge.method,
                format!(
                    "transform chain gap: no transform produces '{}@{}'; chains must start at schema 1",
                    edge.event, edge.from
                ),
            ));
        }
    }
    for edge in definitions {
        for node in [
            (edge.event.as_str(), edge.from),
            (edge.destination(), edge.to),
        ] {
            if versions.get(node.0).is_some_and(|latest| node.1 < *latest)
                && !sources.contains(&node)
            {
                return Err(Error::new_spanned(
                    &edge.method,
                    format!(
                        "transform chain gap: '{}@{}' stops before its latest declared schema",
                        node.0, node.1
                    ),
                ));
            }
        }
    }
    Ok(versions)
}

fn match_arms(definitions: &[Transform]) -> Vec<TokenStream> {
    definitions.iter().map(|edge| {
        let method = &edge.method;
        let event = &edge.event;
        let from = edge.from;
        let to = edge.to;
        let destination = edge.destination();
        quote! {
            (#event, v) if v.get() == #from => {
                let payload = Self::#method(morsel.payload())
                    .map_err(::mnesis_store::TransformError::Transform)?;
                ::mnesis_store::EventMorsel::new(#destination,
                    ::mnesis_store::SchemaVersion::from_u32(#to).expect("validated nonzero schema"), payload)
            }
        }
    }).collect()
}

fn version_arms(versions: &BTreeMap<&str, u32>) -> Vec<TokenStream> {
    // Ordered emission makes identical input produce identical token ordering.
    versions.iter().map(|(event, version)| quote! {
        #event => ::core::option::Option::Some(
            ::mnesis_store::SchemaVersion::from_u32(#version).expect("validated nonzero schema"))
    }).collect()
}

fn schema_guards(versions: &BTreeMap<&str, u32>) -> Vec<TokenStream> {
    versions.iter().map(|(event, latest)| quote! {
        (#event, schema) => {
            if schema.get() == #latest { break; }
            return ::core::result::Result::Err(::mnesis_store::TransformError::UnsupportedSchema {
                event_type: ::mnesis::ErrorId::from_display(&morsel.event_type()), schema,
            });
        }
    }).collect()
}

pub(super) fn expand(ast: &ItemImpl, args: TokenStream) -> Result<TokenStream> {
    let error = error_type(args)?;
    let name = marker_name(ast)?;
    let definitions = definitions(ast)?;
    let schemas = validate_graph(&definitions)?;
    let matches = match_arms(&definitions);
    let versions = version_arms(&schemas);
    let guards = schema_guards(&schemas);
    let methods = ast.items.iter().map(|item| {
        let mut item = item.clone();
        if let ImplItem::Fn(method) = &mut item {
            method
                .attrs
                .retain(|attribute| !attribute.path().is_ident("transform"));
        }
        item
    });
    let attributes = &ast.attrs;
    Ok(quote! {
        #(#attributes)*
        pub struct #name;
        #(#attributes)*
        impl #name {
            #(#methods)*
            /// Apply the deterministic schema transition chain. Schema increases
            /// on every step, including when an event name is revisited.
            pub fn upcast<'a>(mut morsel: ::mnesis_store::EventMorsel<'a>)
                -> ::core::result::Result<::mnesis_store::EventMorsel<'a>, ::mnesis_store::TransformError<#error>>
            {
                loop {
                    morsel = match (morsel.event_type(), morsel.schema_version()) {
                        #(#matches,)*
                        #(#guards,)*
                        _ => break,
                    };
                }
                ::core::result::Result::Ok(morsel)
            }
            /// Latest declared schema for this event name. A rename destination
            /// has its own schema; retiring a source name does not advance it.
            /// Unknown names return `None`.
            #[must_use]
            pub fn current_version(event_type: &str)
                -> ::core::option::Option<::mnesis_store::SchemaVersion>
            {
                match event_type {
                    #(#versions,)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::expand;
    use quote::quote;

    #[test]
    fn invalid_schema_expansion_returns_errors_at_literals_without_unwinding() {
        let cases = [
            ("0", "1", "0", "from version must be >= 1"),
            ("1", "0", "0", "to version must be >= 1"),
            (
                "4294967295",
                "1",
                "4294967295",
                "schema version cannot advance past u32::MAX",
            ),
            (
                "18446744073709551615",
                "2",
                "18446744073709551615",
                "number too large to fit in target type",
            ),
            (
                "1",
                "4294967296",
                "4294967296",
                "number too large to fit in target type",
            ),
        ];
        for (from, to, token, expected) in cases {
            let source = format!(
                "impl Marker {{ #[transform(event = \"E\", from = {from}, to = {to})] fn step(_: &[u8]) -> Result<Vec<u8>, ()> {{ Ok(vec![]) }} }}"
            );
            let outcome = std::panic::catch_unwind(|| {
                let ast = syn::parse_str(&source).expect("valid fixture syntax");
                expand(&ast, quote!(aggregate = (), error = ()))
            });
            let error = outcome
                .expect("untrusted schema values must not panic")
                .expect_err("invalid schemas must reject expansion");
            assert_eq!(error.to_string(), expected);
            let location = error.span().start();
            assert_eq!(location.line, 1);
            assert_eq!(
                location.column,
                source.find(token).expect("literal occurs in source")
            );
        }
    }
}
