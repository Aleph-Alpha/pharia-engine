use proc_macro::TokenStream;
use quote::quote;
use std::collections::HashSet;
use syn::{ImplItem, ItemImpl, parse_macro_input};

/// Generates unimplemented!() for all CsiForSkills methods except those you implement
#[proc_macro_attribute]
pub fn csi_double(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemImpl);
    let struct_name = &input.self_ty;

    let implemented_methods: HashSet<String> = input
        .items
        .iter()
        .filter_map(|item| {
            if let ImplItem::Fn(method) = item {
                Some(method.sig.ident.to_string())
            } else {
                None
            }
        })
        .collect();

    let user_methods = &input.items;
    let unimplemented_methods = generate_unimplemented_methods(&implemented_methods);

    let expanded = quote! {
        #[async_trait::async_trait]
        impl CsiForSkills for #struct_name {
            #(#user_methods)*
            #unimplemented_methods
        }
    };

    TokenStream::from(expanded)
}

fn generate_unimplemented_methods(implemented: &HashSet<String>) -> proc_macro2::TokenStream {
    let mut methods = proc_macro2::TokenStream::new();
    if !implemented.contains("explain") {
        methods.extend(quote! {
            async fn explain(&mut self, _requests: Vec<ExplanationRequest>) -> Vec<Explanation> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("complete") {
        methods.extend(quote! {
            async fn complete(&mut self, _requests: Vec<CompletionRequest>) -> Vec<Completion> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("completion_stream_new") {
        methods.extend(quote! {
            async fn completion_stream_new(&mut self, _request: CompletionRequest) -> CompletionStreamId {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("completion_stream_next") {
        methods.extend(quote! {
            async fn completion_stream_next(&mut self, _id: &CompletionStreamId) -> Option<CompletionEvent> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("completion_stream_drop") {
        methods.extend(quote! {
            async fn completion_stream_drop(&mut self, _id: CompletionStreamId) {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("chunk") {
        methods.extend(quote! {
            async fn chunk(&mut self, _requests: Vec<ChunkRequest>) -> Vec<Vec<Chunk>> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("select_language") {
        methods.extend(quote! {
            async fn select_language(&mut self, _requests: Vec<SelectLanguageRequest>) -> Vec<Option<Language>> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("chat") {
        methods.extend(quote! {
            async fn chat(&mut self, _requests: Vec<ChatRequest>) -> Vec<ChatResponse> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("chat_stream_new") {
        methods.extend(quote! {
            async fn chat_stream_new(&mut self, _request: ChatRequest) -> ChatStreamId {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("chat_stream_next") {
        methods.extend(quote! {
            async fn chat_stream_next(&mut self, _id: &ChatStreamId) -> Option<ChatEvent> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("chat_stream_drop") {
        methods.extend(quote! {
            async fn chat_stream_drop(&mut self, _id: ChatStreamId) {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("search") {
        methods.extend(quote! {
            async fn search(&mut self, _requests: Vec<SearchRequest>) -> Vec<Vec<SearchResult>> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("document_metadata") {
        methods.extend(quote! {
            async fn document_metadata(&mut self, _document_paths: Vec<DocumentPath>) -> Vec<Option<Value>> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("documents") {
        methods.extend(quote! {
            async fn documents(&mut self, _document_paths: Vec<DocumentPath>) -> Vec<Document> {
                unimplemented!()
            }
        });
    }

    if !implemented.contains("invoke_tool") {
        methods.extend(quote! {
            async fn invoke_tool(&mut self, _request: Vec<InvokeRequest>) -> Vec<Vec<u8>> {
                unimplemented!()
            }
        });
    }

    methods
}
