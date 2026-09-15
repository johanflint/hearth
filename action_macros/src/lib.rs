use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, DeriveInput, Expr, ItemFn};

#[proc_macro_attribute]
pub fn register_action(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let name = &input.ident;

    let fn_name = format_ident!("register_{}", name.to_string().to_case(Case::Snake));

    let expanded = quote! {
        #input

        #[ctor::ctor(unsafe)]
        fn #fn_name() {
            crate::flow_engine::action_registry::register_action::<#name>();
        }
    };

    expanded.into()
}

#[proc_macro_attribute]
pub fn track_failures(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Accept any expression yielding &'static str — a literal, a const fn
    let metric_name = parse_macro_input!(attr as Expr);
    let input_fn = parse_macro_input!(item as ItemFn);

    let sig = &input_fn.sig;
    if sig.asyncness.is_none() {
        return syn::Error::new_spanned(sig.fn_token, "#[track_failures] requires an `async fn`")
            .to_compile_error()
            .into();
    }

    let attrs = &input_fn.attrs;
    let vis = &input_fn.vis;
    let block = &input_fn.block;
    let expanded = quote! {
        #(#attrs)*
        #vis #sig {
            let __track_failures_result = async move #block.await;

            if let ::std::result::Result::Err(ref __track_failures_error) = __track_failures_result {
                // `.metric_reason()` resolves via ordinary method lookup at the
                // call site: the caller's crate must have `MetricReason` in scope.
                ::metrics::counter!(#metric_name, "reason" => __track_failures_error.metric_reason()).increment(1);
            }

            __track_failures_result
        }
    };

    expanded.into()
}