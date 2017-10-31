extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate synstructure;
#[macro_use]
extern crate quote;

use std::collections::{HashMap};

use quote::ToTokens;

enum ArgValue<'a> {
    Nothing,
    Str(String),
    List(ArgList<'a>),
}

impl<'a> ArgValue<'a> {
    fn get_str(&self) -> &String {
        match *self {
            ArgValue::Str(ref s) => s,
            _ => panic!("argument expects string value")
        }
    }

    fn get_str_ident(&self) -> syn::Ident {
        syn::Ident::from(self.get_str().as_str())
    }

    fn get_str_tokens(&self) -> quote::Tokens {
        let mut tokens = quote::Tokens::new();
        self.get_str_ident().to_tokens(&mut tokens);
        tokens
    }

    fn get_list(&self) -> &ArgList<'a> {
        match *self {
            ArgValue::List(ref l) => l,
            _ => panic!("argument expects list value")
        }
    }
}

type ArgList<'a> = HashMap<&'a str, ArgValue<'a>>;

fn get_item_args(mi: &syn::MetaItem) -> ArgList {
    if let syn::MetaItem::List(_, ref items) = *mi {
        items.iter().map(|item| {
            match *item {
                syn::NestedMetaItem::MetaItem(ref mi) => {
                    match *mi {
                        syn::MetaItem::Word(ref kw) => (kw.as_ref(), ArgValue::Nothing),

                        syn::MetaItem::NameValue(ref kw, ref val) => {
                            match *val {
                                syn::Lit::Str(ref s, syn::StrStyle::Cooked) =>
                                    (kw.as_ref(), ArgValue::Str(s.clone())),

                                _ => panic!("invalid tag value for by_value: {:?}", *val)
                            }
                        },

                        syn::MetaItem::List(ref kw, _) => {
                            (kw.as_ref(), ArgValue::List(get_item_args(mi)))
                        }
                    }
                },
                _ => panic!("unknown item passed to by_value: {:?}", *item)
            }
        }).collect()
    } else {
        Default::default()
    }
}

fn get_cross_check_args(attrs: &[syn::Attribute]) -> Option<ArgList> {
    attrs.iter()
         .find(|f| f.name() == "cross_check")
         .map(|attr| get_item_args(&attr.value))
}

// Extract the optional tag from a #[cross_check(by_value(...))] attribute
fn get_direct_item_config(args: &ArgList, default_filter_tokens: quote::Tokens)
        -> (syn::Ident, quote::Tokens) {
    // Process "tag = ..." argument
    let tag_ident = args.get("tag").map_or_else(
        || syn::Ident::from("UNKNOWN_TAG"), ArgValue::get_str_ident);
    // Process "filter = ..." argument
    let filter_tokens = args.get("filter").map_or(
        default_filter_tokens, ArgValue::get_str_tokens);
    (tag_ident, filter_tokens)
}

fn xcheck_hash_derive(s: synstructure::Structure) -> quote::Tokens {
    let top_args = get_cross_check_args(&s.ast().attrs[..]).unwrap_or_default();

    // Allow users to override __XCHA and __XCHS
    let ahasher_override = top_args.get("ahasher_override").map_or_else(
        || syn::Ident::from("__XCHA"), ArgValue::get_str_ident);
    let shasher_override = top_args.get("shasher_override").map_or_else(
        || syn::Ident::from("__XCHS"), ArgValue::get_str_ident);

    // Iterate through all fields, inserting the hash computation for each field
    let hash_fields = s.each(|f| {
        get_cross_check_args(&f.ast().attrs[..]).and_then(|args| {
            // FIXME: figure out the argument priorities here
            if args.contains_key("no") ||
               args.contains_key("never") ||
               args.contains_key("disable") {
                // Cross-checking is disabled
                Some(quote::Tokens::new())
            } else if let Some(ref sub_args) = args.get("check_value") {
                // Cross-check field directly by value
                // This has an optional tag parameter (tag="NNN_TAG")
                let (tag, filter) = get_direct_item_config(sub_args.get_list(),
                                                           quote::Tokens::new());
                Some(quote! { cross_check_value!(#tag, (#filter(#f)),
                                                 #ahasher_override,
                                                 #shasher_override) })
            } else if let Some(ref sub_args) = args.get("check_raw") {
                let (tag, filter) = get_direct_item_config(sub_args.get_list(),
                                                           quote! { * });
                Some(quote! { cross_check_raw!(#tag, (#filter(#f)) as u64) })
            } else if let Some(ref sub_arg) = args.get("custom_hash") {
                let id = sub_arg.get_str_ident();
                Some(quote! { #id(&mut h, #f) })
            } else {
                None
            }
        }).unwrap_or_else(|| {
            // Default implementation
            quote! {
                use cross_check_runtime::hash::CrossCheckHash;
                h.write_u64(CrossCheckHash::cross_check_hash_depth::<#ahasher_override, #shasher_override>(#f, _depth - 1));
            }
        })
    });

    let hash_code = top_args.get("custom_hash").map(|sub_arg| {
        // Hash this value by calling the specified function
        let id = sub_arg.get_str_ident();
        quote! { #id::<#ahasher_override, #shasher_override>(&self, _depth) }
    }).unwrap_or_else(|| {
        // Hash this value using the default algorithm
        let hasher = top_args.get("hasher").map_or(ahasher_override, ArgValue::get_str_ident);
        quote! {
            let mut h = #hasher::default();
            match *self { #hash_fields }
            h.finish()
        }
    });
    s.bound_impl("::cross_check_runtime::hash::CrossCheckHash", quote! {
        fn cross_check_hash_depth<__XCHA, __XCHS>(&self, _depth: usize) -> u64
                where __XCHA: ::cross_check_runtime::hash::CrossCheckHasher,
                      __XCHS: ::cross_check_runtime::hash::CrossCheckHasher {
            #hash_code
        }
    })
}
decl_derive!([CrossCheckHash] => xcheck_hash_derive);
