use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::spanned::Spanned;

#[derive(Clone, Copy, PartialEq)]
enum RuntimeFlavor {
    InPlace,
    WorkStealing,
}

impl RuntimeFlavor {
    fn from_str(s: &str) -> Result<RuntimeFlavor, String> {
        match s {
            "in_place" => Ok(RuntimeFlavor::InPlace),
            "work_stealing" => Ok(RuntimeFlavor::WorkStealing),
            "single_threaded" => Err(format!("The single threaded runtime flavor is called `in_place`.")),
            "basic_scheduler" => Err(format!("The `basic_scheduler` runtime flavor has been renamed to `in_place`.")),
            "threaded_scheduler" => Err(format!("The `threaded_scheduler` runtime flavor has been renamed to `work_stealing`.")),
            _ => Err(format!("No such runtime flavor `{}`.", s)),
        }
    }
}

struct FinalConfig {
    flavor: RuntimeFlavor,
    num_workers: Option<usize>,
    max_threads: Option<usize>,
}

struct Configuration {
    rt_threaded_available: bool,
    default_flavor: RuntimeFlavor,
    flavor: Option<RuntimeFlavor>,
    num_workers: Option<(usize, Span)>,
    max_threads: Option<(usize, Span)>,
}
impl Configuration {
    fn new(is_test: bool, rt_threaded: bool) -> Self {
        Configuration {
            rt_threaded_available: rt_threaded,
            default_flavor: match is_test {
                true => RuntimeFlavor::InPlace,
                false => RuntimeFlavor::WorkStealing,
            },
            flavor: None,
            num_workers: None,
            max_threads: None,
        }
    }
    fn set_flavor(
        &mut self,
        runtime: syn::Lit,
        span: Span,
    ) -> Result<(), syn::Error> {
        if self.flavor.is_some() {
            return Err(syn::Error::new(span, "`flavor` set multiple times."));
        }

        let runtime_str = parse_string(runtime, span, "flavor")?;
        let runtime = RuntimeFlavor::from_str(&runtime_str)
            .map_err(|err| syn::Error::new(span, err))?;
        self.flavor = Some(runtime);
        Ok(())
    }
    fn set_num_workers(
        &mut self,
        num_workers: syn::Lit,
        span: Span,
    ) -> Result<(), syn::Error> {
        if self.num_workers.is_some() {
            return Err(syn::Error::new(span, "`num_workers` set multiple times."));
        }

        let num_workers = parse_int(num_workers, span, "num_workers")?;
        if num_workers == 0 {
            return Err(syn::Error::new(span, "`num_workers` may not be 0."));
        }
        self.num_workers = Some((num_workers, span));
        Ok(())
    }
    fn set_max_threads(
        &mut self,
        max_threads: syn::Lit,
        span: Span,
    ) -> Result<(), syn::Error> {
        if self.max_threads.is_some() {
            return Err(syn::Error::new(span, "`max_threads` set multiple times."));
        }

        let max_threads = parse_int(max_threads, span, "max_threads")?;
        if max_threads == 0 {
            return Err(syn::Error::new(span, "`max_threads` may not be 0."));
        }
        self.max_threads = Some((max_threads, span));
        Ok(())
    }
    fn build(&self) -> Result<FinalConfig, syn::Error> {
        let flavor = self.flavor.unwrap_or(self.default_flavor);
        match flavor {
            RuntimeFlavor::InPlace => {
                if let Some((_, num_workers_span)) = &self.num_workers {
                    return Err(syn::Error::new(
                        num_workers_span.clone(),
                        "The `num_workers` option requires the `work_stealing` runtime flavor."
                    ));
                }
                assert!(self.num_workers.is_none());
                Ok(FinalConfig {
                    flavor,
                    num_workers: None,
                    max_threads: self.max_threads.map(|(val, _span)| val),
                })
            },
            RuntimeFlavor::WorkStealing => {
                if !self.rt_threaded_available {
                    let msg = if self.flavor.is_none() {
                        "The default runtime flavor is `work_stealing`, but the `rt-threaded` feature is disabled."
                    } else {
                        "The runtime flavor `work_stealing` requires the `rt-threaded` feature."
                    };
                    return Err(syn::Error::new(Span::call_site(), msg));
                }
                match (self.num_workers, self.max_threads) {
                    (None, Some((_, span))) => {
                        return Err(syn::Error::new(
                                span,
                                "`num_workers` must also be set when using `max_threads`"
                        ));
                    },
                    (Some((num_workers, _)), Some((max_threads, span))) if max_threads <= num_workers => {
                        return Err(syn::Error::new(
                                span,
                                "`max_threads` must be larger than `num_workers`",
                        ));
                    },
                    _ => {},
                }
                Ok(FinalConfig {
                    flavor,
                    num_workers: self.num_workers.map(|(val, _span)| val),
                    max_threads: self.max_threads.map(|(val, _span)| val),
                })
            },
        }
    }
}

fn parse_int(int: syn::Lit, span: Span, field: &str) -> Result<usize, syn::Error> {
    match int {
        syn::Lit::Int(lit) => {
            match lit.base10_parse::<usize>() {
                Ok(value) => Ok(value),
                Err(e) => Err(syn::Error::new(
                    span,
                    format!("Failed to parse {} as integer: {}", field, e),
                )),
            }
        },
        _ => Err(syn::Error::new(span, format!("Failed to parse {} as integer.", field))),
    }
}
fn parse_string(int: syn::Lit, span: Span, field: &str) -> Result<String, syn::Error> {
    match int {
        syn::Lit::Str(s) => Ok(s.value()),
        syn::Lit::Verbatim(s) => Ok(s.to_string()),
        _ => Err(syn::Error::new(span, format!("Failed to parse {} as string.", field))),
    }
}

fn parse_knobs(
    mut input: syn::ItemFn,
    args: syn::AttributeArgs,
    is_test: bool,
    rt_threaded: bool,
) -> Result<TokenStream, syn::Error> {
    let sig = &mut input.sig;
    let body = &input.block;
    let attrs = &input.attrs;
    let vis = input.vis;

    if sig.asyncness.is_none() {
        let msg = "the async keyword is missing from the function declaration";
        return Err(syn::Error::new_spanned(sig.fn_token, msg));
    }

    sig.asyncness = None;

    let macro_name = if is_test { "tokio::test" } else { "tokio::main" };
    let mut config = Configuration::new(is_test, rt_threaded);

    for arg in args {
        match arg {
            syn::NestedMeta::Meta(syn::Meta::NameValue(namevalue)) => {
                let ident = namevalue.path.get_ident();
                if ident.is_none() {
                    let msg = "Must have specified ident";
                    return Err(syn::Error::new_spanned(namevalue, msg));
                }
                match ident.unwrap().to_string().to_lowercase().as_str() {
                    "num_workers" => {
                        config.set_num_workers(namevalue.lit.clone(), namevalue.span())?;
                    }
                    "max_threads" => {
                        config.set_max_threads(namevalue.lit.clone(), namevalue.span())?;
                    },
                    "flavor" => {
                        config.set_flavor(namevalue.lit.clone(), namevalue.span())?;
                    },
                    "core_threads" => {
                        let msg = "Attribute `core_threads` is renamed to `num_workers`";
                        return Err(syn::Error::new_spanned(namevalue, msg));
                    }
                    name => {
                        let msg = format!("Unknown attribute {} is specified; expected one of: `flavor`, `num_workers`, `max_threads`", name);
                        return Err(syn::Error::new_spanned(namevalue, msg));
                    }
                }
            }
            syn::NestedMeta::Meta(syn::Meta::Path(path)) => {
                let ident = path.get_ident();
                if ident.is_none() {
                    let msg = "Must have specified ident";
                    return Err(syn::Error::new_spanned(path, msg));
                }
                let name = ident.unwrap().to_string().to_lowercase();
                let msg = match name.as_str() {
                    "threaded_scheduler" | "work_stealing" => {
                        format!("Set the runtime flavor with #[{}(flavor = \"work_stealing\")].", macro_name)
                    },
                    "basic_scheduler" | "in_place" => {
                        format!("Set the runtime flavor with #[{}(flavor = \"in_place\")].", macro_name)
                    },
                    "flavor" | "num_workers" | "max_threads" => {
                        format!("The `{}` attribute requires an argument.", name)
                    },
                    name => {
                        format!("Unkown attribute {} is specified; expected one of: `flavor`, `num_workers`, `max_threads`", name)
                    },
                };
                return Err(syn::Error::new_spanned(path, msg));
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "Unknown attribute inside the macro",
                ));
            }
        }
    }

    let config = config.build()?;

    let mut rt = match config.flavor {
        RuntimeFlavor::InPlace => quote! {
            tokio::runtime::Builder::new().basic_scheduler()
        },
        RuntimeFlavor::WorkStealing => quote! {
            tokio::runtime::Builder::new().threaded_scheduler()
        },
    };
    if let Some(v) = config.num_workers {
        rt = quote! { #rt.core_threads(#v) };
    }
    if let Some(v) = config.max_threads {
        rt = quote! { #rt.max_threads(#v) };
    }

    let header = {
        if is_test {
            quote! {
                #[::core::prelude::v1::test]
            }
        } else {
            quote! {}
        }
    };

    let result = quote! {
        #header
        #(#attrs)*
        #vis #sig {
            #rt
                .enable_all()
                .build()
                .unwrap()
                .block_on(async { #body })
        }
    };

    Ok(result.into())
}

#[cfg(not(test))] // Work around for rust-lang/rust#62127
pub(crate) fn main(args: TokenStream, item: TokenStream, rt_threaded: bool) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::ItemFn);
    let args = syn::parse_macro_input!(args as syn::AttributeArgs);

    if input.sig.ident == "main" && !input.sig.inputs.is_empty() {
        let msg = "the main function cannot accept arguments";
        return syn::Error::new_spanned(&input.sig.ident, msg)
            .to_compile_error()
            .into();
    }

    parse_knobs(input, args, false, rt_threaded)
        .unwrap_or_else(|e| e.to_compile_error().into())
}

pub(crate) fn test(args: TokenStream, item: TokenStream, rt_threaded: bool) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::ItemFn);
    let args = syn::parse_macro_input!(args as syn::AttributeArgs);

    for attr in &input.attrs {
        if attr.path.is_ident("test") {
            let msg = "second test attribute is supplied";
            return syn::Error::new_spanned(&attr, msg)
                .to_compile_error()
                .into();
        }
    }

    if !input.sig.inputs.is_empty() {
        let msg = "the test function cannot accept arguments";
        return syn::Error::new_spanned(&input.sig.inputs, msg)
            .to_compile_error()
            .into();
    }

    parse_knobs(input, args, true, rt_threaded)
        .unwrap_or_else(|e| e.to_compile_error().into())
}
