use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_attribute]
pub fn register_action(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let name = &input.ident;

    let fn_name = format_ident!("register_{}", name.to_string().to_case(Case::Snake));

    let expanded = quote! {
        #input

        #[ctor::ctor]
        fn #fn_name() {
            register_action::<#name>();
        }
    };

    expanded.into()
}
