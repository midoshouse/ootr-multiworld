#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::HashMap,
        env,
        fs,
        path::PathBuf,
    },
    itertools::Itertools as _,
    lazy_regex::regex_captures,
    proc_macro::TokenStream,
    proc_macro2::Span,
    quote::{
        quote,
        quote_spanned,
    },
    semver::Version,
    syn::{
        *,
        punctuated::Punctuated,
    },
};

fn csharp_extern_function_signatures() -> HashMap<String, (Vec<String>, String)> {
    let mut map = HashMap::default();
    let csharp_file = fs::read_to_string(PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("missing Cargo manifest dir envar")).parent().expect("Cargo manifest at file system root").join("multiworld-bizhawk").join("OotrMultiworld").join("src").join("MainForm.cs")).expect("failed to read MainForm.cs");
    for line in csharp_file.lines() {
        if let Some((_, return_type, name, args)) = regex_captures!("^    \\[DllImport\\(\"multiworld\"\\)\\] internal static extern ([0-9A-Za-z_]+) ([0-9a-z_]+)\\((.*)\\);$", line) {
            map.insert(name.to_owned(), (if args.is_empty() { Vec::default() } else { args.split(", ").map(|arg| arg.to_owned()).collect() }, return_type.to_owned()));
        }
    }
    map
}

fn csharp_ffi_type_check(rust_type: &Type, csharp_type: &str, span: Span) -> std::result::Result<(), TokenStream> {
    fn pointers(pointee: proc_macro2::TokenStream) -> Vec<Type> { vec![parse_quote!(HandleOwned<#pointee>), parse_quote!(*const #pointee), parse_quote!(*mut #pointee)] }

    let expected_rust_types = match csharp_type {
        "BoolResult" => pointers(quote!(Result<bool, Error>)),
        "Client" => pointers(quote!(Client)),
        "ClientResult" => pointers(quote!(Result<Client, Error>)),
        "Error" => pointers(quote!(Error)),
        "IntPtr" => return match rust_type {
            Type::Path(path) if path.path.segments.len() == 1 && matches!(&*path.path.segments[0].ident.to_string(), "HandleOwned" | "StringHandle") => Ok(()),
            Type::Ptr(_) => Ok(()),
            _ => Err(quote_spanned! {span=>
                compile_error!("expected IntPtr to map to HandleOwned or StringHandle");
            }.into()),
        },
        "OptMessageResult" => pointers(quote!(Result<Option<ServerMessage>, Error>)),
        "OwnedStringHandle" => vec![parse_quote!(*const c_char)],
        "StringHandle" => vec![parse_quote!(StringHandle)],
        "UnitResult" => pointers(quote!(Result<(), Error>)),
        "bool" => vec![parse_quote!(FfiBool)],
        "byte" => vec![parse_quote!(u8)],
        "sbyte" => vec![parse_quote!(i8)],
        "uint" => vec![parse_quote!(u32)],
        "ushort" => vec![parse_quote!(u16)],
        "void" => vec![parse_quote!(())],
        _ => return Err(quote_spanned! {span=>
            compile_error!(concat!("unknown C# FFI type: ", #csharp_type));
        }.into()),
    };
    if !expected_rust_types.contains(rust_type) {
        return Err(quote_spanned! {span=>
            compile_error!(concat!("found Rust type not matching C# type `", #csharp_type, "`"));
        }.into())
    }
    Ok(())
}

#[proc_macro_attribute]
pub fn csharp_ffi(_: TokenStream, item: TokenStream) -> TokenStream {
    let ItemFn { attrs, vis, sig, block } = parse_macro_input!(item);
    let fn_name = &sig.ident;
    if let Some((csharp_args, csharp_return_type)) = csharp_extern_function_signatures().get(&fn_name.to_string()) {
        for (rust_arg, csharp_arg) in sig.inputs.iter().zip_eq(csharp_args) {
            let FnArg::Typed(rust_arg) = rust_arg else { panic!("FFI function with receiver arg") };
            let (csharp_arg_type, _) = csharp_arg.split_once(' ').expect("missing space in C# argument type");
            if let Err(e) = csharp_ffi_type_check(&rust_arg.ty, csharp_arg_type, rust_arg.colon_token.spans[0]) { return e.into() }
        }
        if let Err(e) = match sig.output {
            ReturnType::Default => csharp_ffi_type_check(&parse_quote!(()), csharp_return_type, sig.ident.span()),
            ReturnType::Type(arrow, ref ty) => csharp_ffi_type_check(ty, csharp_return_type, arrow.spans[0].join(arrow.spans[1]).unwrap_or(sig.ident.span())),
        } { return e.into() }
    } else {
        return quote_spanned! {fn_name.span()=>
            compile_error!("not used in the C# code");
        }.into()
    }
    TokenStream::from(quote! {
        #[no_mangle] #(#attrs)* #vis #sig {
            if CONFIG.log {
                writeln!(&*LOG, concat!("{} called ", stringify!(#fn_name)), Utc::now().format("%Y-%m-%d %H:%M:%S")).expect("failed to write log entry");
            }
            #block
        }
    })
}

#[proc_macro]
pub fn latest(input: TokenStream) -> TokenStream {
    let version = Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse package version");
    let version = Ident::new(&format!("v{}", version.major), Span::call_site());
    if input.is_empty() {
        TokenStream::from(quote! {
            pub use self::#version as latest;
        })
    } else {
        TokenStream::from(quote! {
            compile_error!("multiworld_derive::latest does not take parameters");
        })
    }
}

#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input with Punctuated::<Ident, Token![,]>::parse_terminated);
    let version = Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse package version");
    let route_names = (10..=version.major).map(|version| Ident::new(&format!("v{version}"), Span::call_site()));
    TokenStream::from(quote!(rocket::routes![
        #input
        #(#route_names,)*
    ]))
}
