use {
    proc_macro::TokenStream,
    quote::quote,
    syn::*,
};

#[proc_macro_attribute]
pub fn csharp_ffi(_: TokenStream, item: TokenStream) -> TokenStream {
    let ItemFn { attrs, vis, sig, block } = parse_macro_input!(item);
    let fn_name = &sig.ident;
    TokenStream::from(quote! {
        #[no_mangle] #(#attrs)* #vis #sig {
            if CONFIG.log { writeln!(&*LOG, concat!("called ", stringify!(#fn_name))).expect("failed to write log entry"); }
            #block
        }
    })
}
