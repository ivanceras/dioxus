extern crate proc_macro;

use std::collections::HashSet;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, ItemImpl, Type,  TypeTuple, TypePath};

/// A helper attribute for deriving `State` for a struct.
#[proc_macro_attribute]
pub fn partial_derive_state(_: TokenStream, input: TokenStream) -> TokenStream {
    let impl_block: syn::ItemImpl = parse_macro_input!(input as syn::ItemImpl);

    let parent_dependencies = impl_block
        .items
        .iter()
        .find_map(|item| {
            if let syn::ImplItem::Type(syn::ImplItemType { ident, ty, .. }) = item {
                (ident == "ParentDependencies").then_some(ty)
            } else {
                None
            }
        })
        .expect("ParentDependencies must be defined");
    let child_dependencies = impl_block
        .items
        .iter()
        .find_map(|item| {
            if let syn::ImplItem::Type(syn::ImplItemType { ident, ty, .. }) = item {
                (ident == "ChildDependencies").then_some(ty)
            } else {
                None
            }
        })
        .expect("ChildDependencies must be defined");
    let node_dependencies = impl_block
        .items
        .iter()
        .find_map(|item| {
            if let syn::ImplItem::Type(syn::ImplItemType { ident, ty, .. }) = item {
                (ident == "NodeDependencies").then_some(ty)
            } else {
                None
            }
        })
        .expect("NodeDependencies must be defined");

    let this_type = &impl_block.self_ty;
    let this_type = extract_type_path(this_type).unwrap_or_else(|| panic!("Self must be a type path, found {}", quote!(#this_type)));

    let mut combined_dependencies = HashSet::new();

    let self_path: TypePath = syn::parse_quote!(Self);

    let parent_dependencies = match extract_tuple(parent_dependencies) {
        Some(tuple) => {
            let mut parent_dependencies = Vec::new();
            for type_ in &tuple.elems {
                let mut type_ = extract_type_path(type_).unwrap_or_else(|| panic!("ParentDependencies must be a tuple of type paths, found {}", quote!(#type_)));
                if type_ == self_path {
                    type_ = this_type.clone();
                }
                combined_dependencies.insert(type_.clone());
                parent_dependencies.push(type_);
            }
            parent_dependencies
        }
        _ => panic!("ParentDependencies must be a tuple, found {}", quote!(#parent_dependencies)),
    };
    let child_dependencies = match extract_tuple(child_dependencies) {
        Some(tuple) => {
            let mut child_dependencies = Vec::new();
            for type_ in &tuple.elems {
                let mut type_ = extract_type_path(type_).unwrap_or_else(|| panic!("ChildDependencies must be a tuple of type paths, found {}", quote!(#type_)));
                if type_ == self_path {
                    type_ = this_type.clone();
                }
                combined_dependencies.insert(type_.clone());
                child_dependencies.push(type_);
            }
            child_dependencies
        }
        _ => panic!("ChildDependencies must be a tuple, found {}", quote!(#child_dependencies)),
    };
    let node_dependencies = match extract_tuple(node_dependencies) {
        Some(tuple) => {
            let mut node_dependencies = Vec::new();
            for type_ in &tuple.elems {
                let mut type_ = extract_type_path(type_).unwrap_or_else(|| panic!("NodeDependencies must be a tuple of type paths, found {}", quote!(#type_)));
                if type_ == self_path {
                    type_ = this_type.clone();
                }
                combined_dependencies.insert(type_.clone());
                node_dependencies.push(type_);
            }
            node_dependencies
        }
        _ => panic!("NodeDependencies must be a tuple, found {}", quote!(#node_dependencies)),
    };
    combined_dependencies.insert(this_type.clone());

    let combined_dependencies: Vec<_> = combined_dependencies.into_iter().collect();
    let parent_dependancies_idxes: Vec<_> = parent_dependencies
        .iter()
        .filter_map(|ident| combined_dependencies.iter().position(|i| i == ident))
        .collect();
    let child_dependencies_idxes: Vec<_> = child_dependencies
        .iter()
        .filter_map(|ident| combined_dependencies.iter().position(|i| i == ident))
        .collect();
    let node_dependencies_idxes: Vec<_> = node_dependencies
        .iter()
        .filter_map(|ident| combined_dependencies.iter().position(|i| i == ident))
        .collect();
    let this_type_idx = combined_dependencies
        .iter()
        .enumerate()
        .find_map(|(i, ident)| (this_type == *ident).then_some(i))
        .unwrap();
    let this_view = format_ident!("__data{}", this_type_idx);

    let combined_dependencies_quote = combined_dependencies.iter().map(|ident| {
        if ident == &this_type {
            quote! {shipyard::ViewMut<#ident>}
        } else {
            quote! {shipyard::View<#ident>}
        }
    });
    let combined_dependencies_quote = quote!((#(#combined_dependencies_quote,)*));

    let ItemImpl {
        attrs,
        defaultness,
        unsafety,
        impl_token,
        generics,
        trait_,
        self_ty,
        items,
        ..
    } = impl_block;
    let for_ = trait_.as_ref().map(|t| t.2);
    let trait_ = trait_.map(|t| t.1);

    let split_views: Vec<_> = (0..combined_dependencies.len())
        .map(|i| {
            let ident = format_ident!("__data{}", i);
            if i == this_type_idx {
                quote!{mut #ident}
            } else {
                quote!{#ident}
            }
    })
        .collect();

    let node_view = node_dependencies_idxes
        .iter()
        .map(|i| format_ident!("__data{}", i))
        .collect::<Vec<_>>();
    let get_node_view = {
        if node_dependencies.is_empty() {
            quote! {
                let raw_node = ();
            }
        } else {
            let temps = (0..node_dependencies.len())
                .map(|i| format_ident!("__temp{}", i))
                .collect::<Vec<_>>();
            quote! {
                let raw_node: (#(*const #node_dependencies,)*) = {
                    let (#(#temps,)*) = (#(&#node_view,)*).get(id).unwrap_or_else(|err| panic!("Failed to get node view {:?}", err));
                    (#(#temps as *const _,)*)
                };
            }
        }
    };
    let parent_view = parent_dependancies_idxes
        .iter()
        .map(|i| format_ident!("__data{}", i))
        .collect::<Vec<_>>();
    let get_parent_view = {
        if parent_dependencies.is_empty() {
            quote! {
                let raw_parent = tree.parent_id(id).map(|_| ());
            }
        } else {
            let temps = (0..parent_dependencies.len())
                .map(|i| format_ident!("__temp{}", i))
                .collect::<Vec<_>>();
            quote! {
                let raw_parent = tree.parent_id(id).and_then(|parent_id| {
                    let raw_parent: Option<(#(*const #parent_dependencies,)*)> = (#(&#parent_view,)*).get(parent_id).ok().map(|c| {
                        let (#(#temps,)*) = c;
                        (#(#temps as *const _,)*)
                    });
                    raw_parent
                });
            }
        }
    };
    let child_view = child_dependencies_idxes
    .iter()
    .map(|i| format_ident!("__data{}", i))
    .collect::<Vec<_>>();
let get_child_view = {
    if child_dependencies.is_empty() {
        quote! {
            let raw_children: Vec<_> = tree.children_ids(id).into_iter().map(|_| ()).collect();
        }
    } else {
            let temps = (0..child_dependencies.len())
                .map(|i| format_ident!("__temp{}", i))
                .collect::<Vec<_>>();
            quote! {
                let raw_children: Vec<_> = tree.children_ids(id).into_iter().filter_map(|id| {
                    let raw_children: Option<(#(*const #child_dependencies,)*)> = (#(&#child_view,)*).get(id).ok().map(|c| {
                        let (#(#temps,)*) = c;
                        (#(#temps as *const _,)*)
                    });
                    raw_children
                }).collect();
            }
        }
    };

    let trait_generics = trait_
        .as_ref()
        .unwrap()
        .segments
        .last()
        .unwrap()
        .arguments
        .clone();

    quote!(
        #(#attrs)*
        #defaultness #unsafety #impl_token #generics #trait_ #for_ #self_ty {
            #(#items)*

            fn workload_system(type_id: std::any::TypeId, dependants: dioxus_native_core::exports::FxHashSet<std::any::TypeId>, pass_direction: dioxus_native_core::PassDirection) -> dioxus_native_core::exports::shipyard::WorkloadSystem {
                use dioxus_native_core::exports::shipyard::{IntoWorkloadSystem, Get, AddComponent};
                use dioxus_native_core::tree::TreeRef;

                (move |data: #combined_dependencies_quote, run_view: dioxus_native_core::RunPassView #trait_generics| {
                    println!("Running system for {:?}", type_id);
                    let (#(#split_views,)*) = data;
                    let (tree, types, _, _, _) = &run_view;
                    let tree = tree.clone();
                    let node_mask = Self::NODE_MASK.build();
                    let node_types = types.clone();
                    dioxus_native_core::run_pass(type_id, dependants.clone(), pass_direction, run_view, |id, context| {
                        let node_data: &NodeType<_> = node_types.get(id).unwrap_or_else(|err| panic!("Failed to get node type {:?}", err));
                        // get all of the states from the tree view
                        // Safety: No node has itself as a parent or child.
                        let raw_myself: Option<*mut Self> = (&mut #this_view).get(id).ok().map(|c| c as *mut _);
                        #get_node_view
                        #get_parent_view
                        #get_child_view

                        let myself: Option<&mut Self> = unsafe { raw_myself.map(|val| std::mem::transmute(val)) };
                        let node = unsafe { std::mem::transmute(raw_node) };
                        let parent = unsafe { raw_parent.map(|val| std::mem::transmute(val)) };
                        let children = unsafe { std::mem::transmute(raw_children) };

                        let view = NodeView::new(id, node_data, &node_mask);
                        if let Some(myself) = myself { 
                            myself
                                .update(view, node, parent, children, context)
                        }
                        else {
                            (&mut #this_view).add_component_unchecked(
                                id,
                                Self::create(view, node, parent, children, context));
                            true
                        } 
                    })
                }).into_workload_system().unwrap()
            }
        }
    )
    .into()
}

fn extract_tuple(ty: &Type) -> Option<TypeTuple> {
    match ty {
        Type::Tuple(tuple) => Some(tuple.clone()),
        Type::Group(group) => extract_tuple(&group.elem),
        _ => None,
    }
}

fn extract_type_path(ty: &Type) -> Option<TypePath> {
    match ty {
        Type::Path(path) => Some(path.clone()),
        Type::Group(group) => extract_type_path(&group.elem),
        _ => None,
    }
}