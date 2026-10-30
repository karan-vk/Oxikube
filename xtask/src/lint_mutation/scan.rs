//! The `syn` visitor behind the mutation-door lint.

use std::collections::BTreeSet;

use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{
    Attribute, Expr, ExprMethodCall, Field, ItemFn, ItemImpl, ItemMod, ItemUse, Local, Macro, Pat,
    PatType, Path, Token, Type, UseTree,
};

/// Methods of `ResourceWriter` that change a cluster: every method the trait declares (the
/// `mutating_methods_cover_the_writer_trait` test reads the trait to keep this in step).
pub(super) const MUTATING_METHODS: &[&str] = &[
    "create",
    "replace",
    "patch",
    "delete",
    "delete_collection",
    "scale",
    "evict",
    "create_subresource",
    "patch_subresource",
    "replace_subresource",
];

/// Methods that hand on the handle they are called on (or the value inside an `Option` or
/// `Result` holding it): `w.clone()`, `getter().unwrap()`.
const PASS_THROUGH_METHODS: &[&str] = &[
    "clone",
    "as_ref",
    "as_deref",
    "borrow",
    "unwrap",
    "expect",
    "unwrap_unchecked",
    "unwrap_or_else",
    "unwrap_or_default",
];

/// Names that produce a writer handle when called: `mutation.writer()`.
const WRITER_GETTERS: &[&str] = &["writer", "writer_handle"];

/// Names that may not be aliased on import.
const PROTECTED_NAMES: &[&str] = &["Mutation", "ResourceWriter", "ResourcePort"];

/// Scan one source file; one message per violation. A file that does not parse is reported
/// as a violation so it cannot hide a bypass.
pub(super) fn scan_source(src: &str) -> Vec<String> {
    let file = match syn::parse_file(src) {
        Ok(file) => file,
        Err(e) => return vec![format!("does not parse: {e}")],
    };
    let mut scanner = Scanner::default();
    // Pass one learns which names hold writer handles; pass two reports.
    scanner.visit_file(&file);
    scanner.reporting = true;
    scanner.visit_file(&file);
    scanner.found.into_iter().collect()
}

/// Whether an attribute list contains `#[cfg(test)]`.
pub(super) fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && a.parse_args::<Path>().is_ok_and(|p| p.is_ident("test"))
    })
}

#[derive(Default)]
struct Scanner {
    /// Bindings, parameters and fields whose declared type mentions a writer-capable trait.
    writer_names: BTreeSet<String>,
    /// Type aliases and generic parameters that stand for a writer-capable trait
    /// (`type Port = Arc<dyn ResourcePort>;`, `fn f<P: ResourcePort>(p: P)`).
    writer_types: BTreeSet<String>,
    reporting: bool,
    found: BTreeSet<String>,
}

impl Scanner {
    fn report(&mut self, message: String) {
        if self.reporting {
            self.found.insert(message);
        }
    }

    /// Whether `expr` evaluates to a writer handle.
    fn is_writer_handle(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Paren(p) => self.is_writer_handle(&p.expr),
            Expr::Reference(r) => self.is_writer_handle(&r.expr),
            Expr::Await(a) => self.is_writer_handle(&a.base),
            // `x.writer()?`: the handle inside the `Option` or `Result`.
            Expr::Try(t) => self.is_writer_handle(&t.expr),
            Expr::Path(p) => p
                .path
                .get_ident()
                .is_some_and(|i| self.writer_names.contains(&i.to_string())),
            Expr::Field(f) => match &f.member {
                syn::Member::Named(n) => self.writer_names.contains(&n.to_string()),
                syn::Member::Unnamed(_) => false,
            },
            Expr::MethodCall(m) => {
                let name = m.method.to_string();
                WRITER_GETTERS.contains(&name.as_str())
                    || (PASS_THROUGH_METHODS.contains(&name.as_str())
                        && self.is_writer_handle(&m.receiver))
            }
            _ => false,
        }
    }

    /// A renamed import (`use ...::ResourceWriter as W;`) would hide the trait from every later
    /// name check, so it is a violation. Plain imports are fine: their uses are checked.
    fn check_use_tree(&mut self, tree: &UseTree) {
        match tree {
            UseTree::Path(p) => self.check_use_tree(&p.tree),
            UseTree::Group(g) => {
                for item in &g.items {
                    self.check_use_tree(item);
                }
            }
            UseTree::Rename(r) => {
                let name = r.ident.to_string();
                if PROTECTED_NAMES.contains(&name.as_str()) {
                    self.report(format!(
                        "renames `{name}` on import (an alias would hide it from this lint)"
                    ));
                }
            }
            UseTree::Name(_) | UseTree::Glob(_) => {}
        }
    }

    fn learn(&mut self, pat: &Pat, ty: &Type) {
        if self.mentions_writer_trait(ty)
            && let Pat::Ident(id) = pat
        {
            self.writer_names.insert(id.ident.to_string());
        }
    }
}

/// Whether `ty` names `ResourceWriter` or `ResourcePort` anywhere, or one of `aliases`.
fn names_writer_trait(ty: &Type, aliases: &BTreeSet<String>) -> bool {
    struct Finder<'a> {
        aliases: &'a BTreeSet<String>,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Finder<'_> {
        fn visit_path_segment(&mut self, seg: &'ast syn::PathSegment) {
            if seg.ident == "ResourceWriter"
                || seg.ident == "ResourcePort"
                || self.aliases.contains(&seg.ident.to_string())
            {
                self.found = true;
            }
            syn::visit::visit_path_segment(self, seg);
        }
    }
    let mut finder = Finder {
        aliases,
        found: false,
    };
    finder.visit_type(ty);
    finder.found
}

/// Whether any of `bounds` names a writer-capable trait.
fn bounds_name_writer_trait<'a>(
    bounds: impl IntoIterator<Item = &'a syn::TypeParamBound>,
    aliases: &BTreeSet<String>,
) -> bool {
    bounds.into_iter().any(|bound| match bound {
        syn::TypeParamBound::Trait(t) => {
            let ty = Type::Path(syn::TypePath {
                qself: None,
                path: t.path.clone(),
            });
            names_writer_trait(&ty, aliases)
        }
        _ => false,
    })
}

impl Scanner {
    fn mentions_writer_trait(&self, ty: &Type) -> bool {
        names_writer_trait(ty, &self.writer_types)
    }
}

impl<'ast> Visit<'ast> for Scanner {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        if !is_cfg_test(&node.attrs) {
            syn::visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if !is_cfg_test(&node.attrs) {
            syn::visit::visit_item_fn(self, node);
        }
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        if is_cfg_test(&node.attrs) {
            return;
        }
        let implements_writer = node
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .is_some_and(|seg| seg.ident == "ResourceWriter");
        if implements_writer {
            // Implementing is allowed: skip the trait path, visit the rest.
            self.visit_generics(&node.generics);
            self.visit_type(&node.self_ty);
            for item in &node.items {
                self.visit_impl_item(item);
            }
        } else {
            syn::visit::visit_item_impl(self, node);
        }
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if !is_cfg_test(&node.attrs) {
            syn::visit::visit_impl_item_fn(self, node);
        }
    }

    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        if !is_cfg_test(&node.attrs) {
            self.check_use_tree(&node.tree);
        }
    }

    fn visit_path(&mut self, path: &'ast Path) {
        for seg in &path.segments {
            if seg.ident == "Mutation" {
                self.report(
                    "references `Mutation`, the app layer's capability token (route the action through a Command)"
                        .to_owned(),
                );
            } else if seg.ident == "ResourceWriter" {
                self.report(
                    "references `ResourceWriter` (only `impl ResourceWriter for T` is allowed outside the guard)"
                        .to_owned(),
                );
            }
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_field(&mut self, node: &'ast Field) {
        if let Some(ident) = &node.ident
            && self.mentions_writer_trait(&node.ty)
        {
            self.writer_names.insert(ident.to_string());
        }
        syn::visit::visit_field(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        if !is_cfg_test(&node.attrs) {
            if self.mentions_writer_trait(&node.ty) {
                self.writer_types.insert(node.ident.to_string());
            }
            syn::visit::visit_item_type(self, node);
        }
    }

    fn visit_type_param(&mut self, node: &'ast syn::TypeParam) {
        if bounds_name_writer_trait(&node.bounds, &self.writer_types) {
            self.writer_types.insert(node.ident.to_string());
        }
        syn::visit::visit_type_param(self, node);
    }

    fn visit_predicate_type(&mut self, node: &'ast syn::PredicateType) {
        if bounds_name_writer_trait(&node.bounds, &self.writer_types)
            && let Type::Path(p) = &node.bounded_ty
            && let Some(ident) = p.path.get_ident()
        {
            self.writer_types.insert(ident.to_string());
        }
        syn::visit::visit_predicate_type(self, node);
    }

    fn visit_pat_type(&mut self, node: &'ast PatType) {
        self.learn(&node.pat, &node.ty);
        syn::visit::visit_pat_type(self, node);
    }

    fn visit_local(&mut self, node: &'ast Local) {
        if let (Pat::Ident(id), Some(init)) = (&node.pat, &node.init)
            && self.is_writer_handle(&init.expr)
        {
            self.writer_names.insert(id.ident.to_string());
        }
        syn::visit::visit_local(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        let method = node.method.to_string();
        if MUTATING_METHODS.contains(&method.as_str()) && self.is_writer_handle(&node.receiver) {
            self.report(format!(
                "calls `.{method}()` on a ResourceWriter handle (mutations go through the guard)"
            ));
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_macro(&mut self, node: &'ast Macro) {
        // Best effort: macros such as `assert!(w.delete(..))` carry comma-separated expressions.
        if let Ok(args) = node.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            for expr in &args {
                self.visit_expr(expr);
            }
        }
        syn::visit::visit_macro(self, node);
    }
}
