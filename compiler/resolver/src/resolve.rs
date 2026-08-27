use std::collections::{HashMap, HashSet};

use nether_ast::{
    Block, EnumDecl, Expr, ExprKind, FnDecl, GenericParam, ImplBlock, Item, Module, NodeId, Param,
    Path, Pattern, Stmt, StructDecl, Symbol, TypeExpr, UseDecl,
};
use nether_diagnostics::Diagnostic;

use crate::def::{self, DefId, DefKind, Definitions};
use crate::scope::{LocalId, LocalIdGen, Scopes};

mod expr;
mod helpers;
mod path;

pub(crate) use helpers::variant_index;
use helpers::*;

/// What one [`Path`] (or, in type position, the leading segments of a
/// [`TypeExpr::Named`]) turned out to name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// A local variable — `self`, a parameter, a `let` binding, or a
    /// pattern binding.
    Local(LocalId),
    /// The whole path names this definition directly, with no trailing
    /// member segment (a bare type/enum/trait/fn/primitive name).
    Def(DefId),
    /// `Color.Red`, `Option.Some` — the enum and the variant's index
    /// within [`def::Def::variants`].
    EnumVariant(DefId, u32),
    /// `Dog.new` — the type/enum and the method's index within
    /// [`def::Def::methods`].
    StaticMember(DefId, u32),
    /// `Dog.LEGS` — a type/enum-associated constant.
    StaticConst(DefId, u32),
    /// Resolved to an in-scope generic type parameter (type position only;
    /// `typecheck` substitutes the concrete type during monomorphization).
    GenericParam,
    /// A compile-time integer parameter used in value position.
    ConstParam,
    /// Could not be resolved; a diagnostic has already been emitted for
    /// this path, so downstream stages should not report it again.
    Error,
}

/// The result of resolving one [`Path`]: `base` is what the leading
/// segment(s) named, and `consumed` says how many of `path.segments` that
/// accounts for.
///
/// When `consumed < path.segments.len()`, the remaining segments are left
/// for `typecheck`/`hir` to interpret as a field/method access chain on
/// the resolved base — e.g. `self.name` resolves segment 0 (`self`) to a
/// `Local`, `consumed == 1`, leaving `name` for `typecheck` to resolve as
/// a field access once it knows `self`'s type (language-spec §10;
/// `docs/architecture/crates.md` § `resolver`).
#[derive(Debug, Clone, Copy)]
pub struct PathResolution {
    pub base: Resolution,
    pub consumed: usize,
}

/// Every result of one [`resolve`] call.
pub struct ResolvedNames {
    pub definitions: Definitions,
    /// Keyed by a [`Path`]'s own [`NodeId`] (not the enclosing
    /// [`Expr`]/[`TypeExpr`]'s) — see [`nether_ast::Path`]'s docs for why
    /// it carries one.
    pub path_res: HashMap<NodeId, PathResolution>,
    /// Keyed by a binding site's [`NodeId`] (a [`nether_ast::LetStmt`]'s,
    /// a [`Param`]'s, or a [`Pattern::Binding`]'s) — `self` is keyed by its
    /// owning [`FnDecl`]'s `id`, since it has no binding-site node of its
    /// own.
    pub locals: HashMap<NodeId, LocalId>,
}

/// Resolves every name in `module`: builds the top-level namespace
/// (`def::collect`), then walks every item resolving local variable
/// bindings/uses and every [`Path`] (language-spec §10).
///
/// `use` aliases are resolved through the driver's module graph and kept
/// in per-file namespaces. Field-existence checks remain deliberately
/// deferred to `typecheck`, the first stage with enough type information.
pub fn resolve(module: &Module) -> (ResolvedNames, Vec<Diagnostic>) {
    resolve_with_prelude(module, None)
}

/// Like [`resolve`], but additionally re-exports every top-level `use` in
/// `prelude_file` (the driver's `stdlib/mod.nr`, when it loaded one) to
/// every other file with no `use` of their own — see
/// [`def::Definitions::promote_to_prelude`]. `resolve` itself is `None`'s
/// case, kept as the ordinary entry point for every caller that isn't the
/// driver (this crate's own tests, `typecheck`/`hir`/`monomorphization`'s
/// tests) since they construct a bare `Module` with no bundled prelude.
pub fn resolve_with_prelude(
    module: &Module,
    prelude_file: Option<nether_diagnostics::FileId>,
) -> (ResolvedNames, Vec<Diagnostic>) {
    let (defs, mut diags) = def::collect(module, prelude_file);
    let type_generics = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some((
                (decl.span.file, decl.name.name.clone()),
                decl.generics.clone(),
            )),
            Item::Enum(decl) => Some((
                (decl.span.file, decl.name.name.clone()),
                decl.generics.clone(),
            )),
            _ => None,
        })
        .collect();

    let mut resolver = Resolver {
        defs: &defs,
        scopes: Scopes::new(),
        generic_scopes: Vec::new(),
        const_generic_scopes: Vec::new(),
        type_generics,
        locals: LocalIdGen::default(),
        path_res: HashMap::new(),
        local_sites: HashMap::new(),
        diagnostics: Vec::new(),
    };
    for item in &module.items {
        resolver.resolve_item(item);
    }
    // Fully destructure `resolver` so its borrow of `defs` ends here,
    // before `defs` is moved into the return value below.
    let Resolver {
        path_res,
        local_sites,
        mut diagnostics,
        ..
    } = resolver;
    diags.append(&mut diagnostics);

    (
        ResolvedNames {
            definitions: defs,
            path_res,
            locals: local_sites,
        },
        diags,
    )
}

struct Resolver<'a> {
    defs: &'a Definitions,
    scopes: Scopes,
    /// Stack of in-scope generic-parameter name sets, pushed on entering a
    /// generic item's signature/body and popped on leaving. Checked before
    /// falling back to `defs` when resolving a type-position path.
    generic_scopes: Vec<HashSet<Symbol>>,
    const_generic_scopes: Vec<HashSet<Symbol>>,
    /// Generic parameters implicitly in scope inside `impl Type { ... }`.
    /// The grammar deliberately writes the target as a bare name; the
    /// declaration's parameters supply the impl's generic environment.
    type_generics: HashMap<(nether_diagnostics::FileId, Symbol), Vec<GenericParam>>,
    locals: LocalIdGen,
    path_res: HashMap<NodeId, PathResolution>,
    local_sites: HashMap<NodeId, LocalId>,
    diagnostics: Vec<Diagnostic>,
}

impl Resolver<'_> {
    fn error(&mut self, span: nether_diagnostics::Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(message).with_label(span, "here"));
    }

    fn bind_local(&mut self, site: NodeId, name: Symbol) {
        let id = self.locals.next_id();
        self.scopes.bind(name, id);
        self.local_sites.insert(site, id);
    }

    fn push_generics(&mut self, generics: &[GenericParam]) {
        let names = generics.iter().map(|g| g.name.name.clone()).collect();
        let const_names = generics
            .iter()
            .filter(|generic| generic.const_ty.is_some())
            .map(|generic| generic.name.name.clone())
            .collect();
        self.generic_scopes.push(names);
        self.const_generic_scopes.push(const_names);
    }

    fn pop_generics(&mut self) {
        self.generic_scopes.pop();
        self.const_generic_scopes.pop();
    }

    fn is_generic_param(&self, name: &Symbol) -> bool {
        self.generic_scopes.iter().any(|scope| scope.contains(name))
    }

    fn is_const_generic_param(&self, name: &Symbol) -> bool {
        self.const_generic_scopes
            .iter()
            .any(|scope| scope.contains(name))
    }

    // ---- items ----

    fn resolve_item(&mut self, item: &Item) {
        match item {
            Item::Struct(t) => self.resolve_struct_decl(t),
            Item::Enum(e) => self.resolve_enum_decl(e),
            Item::Trait(i) => {
                self.push_generics(&i.generics);
                for generic in &i.generics {
                    if let Some(const_ty) = &generic.const_ty {
                        self.resolve_type_expr(const_ty);
                    }
                    for bound in &generic.bounds {
                        self.resolve_type_expr(bound);
                    }
                }
                for parent in &i.parents {
                    self.resolve_type_expr(parent);
                }
                for constant in &i.associated_consts {
                    self.resolve_type_expr(&constant.ty);
                    if let Some(value) = &constant.value {
                        self.resolve_expr(value);
                    }
                }
                for associated in &i.associated_types {
                    if let Some(value) = &associated.value {
                        self.resolve_type_expr(value);
                    }
                }
                for method in &i.methods {
                    // Resolve each bound in the method's own generics too.
                    self.resolve_fn_decl(method);
                }
                self.pop_generics();
            }
            Item::Fn(f) => self.resolve_fn_decl(f),
            Item::Impl(b) => self.resolve_impl_block(b),
            Item::Use(u) => self.resolve_use_decl(u),
            Item::Mod(_) => {}
            Item::TypeAlias(alias) => self.resolve_type_expr(&alias.ty),
            Item::Extern(block) => {
                for f in &block.functions {
                    // `resolve_fn_decl` already tolerates `f.body: None`
                    // (a trait method with no default) — an extern
                    // signature is the same shape.
                    self.resolve_fn_decl(f);
                }
            }
        }
    }

    fn resolve_struct_decl(&mut self, t: &StructDecl) {
        self.push_generics(&t.generics);
        for g in &t.generics {
            if let Some(const_ty) = &g.const_ty {
                self.resolve_type_expr(const_ty);
            }
            for bound in &g.bounds {
                self.resolve_type_expr(bound);
            }
        }
        for trait_ref in &t.traits {
            self.resolve_type_expr(trait_ref);
        }
        match &t.kind {
            nether_ast::StructDeclKind::Struct(fields) => {
                for field in fields {
                    self.resolve_type_expr(&field.ty);
                }
            }
            nether_ast::StructDeclKind::TupleStruct(tys) => {
                for ty in tys {
                    self.resolve_type_expr(ty);
                }
            }
            nether_ast::StructDeclKind::Unit => {}
        }
        self.pop_generics();
    }

    fn resolve_enum_decl(&mut self, e: &EnumDecl) {
        self.push_generics(&e.generics);
        for g in &e.generics {
            if let Some(const_ty) = &g.const_ty {
                self.resolve_type_expr(const_ty);
            }
            for bound in &g.bounds {
                self.resolve_type_expr(bound);
            }
        }
        for trait_ref in &e.traits {
            self.resolve_type_expr(trait_ref);
        }
        for variant in &e.variants {
            for ty in &variant.payload {
                self.resolve_type_expr(ty);
            }
        }
        self.pop_generics();
    }

    fn resolve_impl_block(&mut self, b: &ImplBlock) {
        // `impl<T> Option<T> { ... }` — the explicit Rust-like form. Unlike
        // the implicit form below, the impl block's own `<T>` supplies the
        // generic environment (and `target_args` is resolved within it),
        // which is the only way to name a type parameter for a builtin
        // owner such as `Option`/`Result` that has no local declaration to
        // read parameters from.
        if !b.generics.is_empty() || !b.target_args.is_empty() {
            self.push_generics(&b.generics);
            for g in &b.generics {
                if let Some(const_ty) = &g.const_ty {
                    self.resolve_type_expr(const_ty);
                }
                for bound in &g.bounds {
                    self.resolve_type_expr(bound);
                }
            }
            for arg in &b.target_args {
                self.resolve_type_expr(arg);
            }
            for trait_ref in &b.traits {
                self.resolve_type_expr(trait_ref);
            }
            for constant in &b.associated_consts {
                self.resolve_type_expr(&constant.ty);
                if let Some(value) = &constant.value {
                    self.resolve_expr(value);
                }
            }
            for associated in &b.associated_types {
                if let Some(value) = &associated.value {
                    self.resolve_type_expr(value);
                }
            }
            for method in &b.methods {
                self.resolve_fn_decl(method);
            }
            self.pop_generics();
            return;
        }
        let owner_generics = self
            .type_generics
            .get(&(b.span.file, b.target.name.clone()))
            .cloned()
            .unwrap_or_default();
        self.push_generics(&owner_generics);
        for trait_ref in &b.traits {
            self.resolve_type_expr(trait_ref);
        }
        for constant in &b.associated_consts {
            self.resolve_type_expr(&constant.ty);
            if let Some(value) = &constant.value {
                self.resolve_expr(value);
            }
        }
        for associated in &b.associated_types {
            if let Some(value) = &associated.value {
                self.resolve_type_expr(value);
            }
        }
        for method in &b.methods {
            self.resolve_fn_decl(method);
        }
        self.pop_generics();
    }

    fn resolve_use_decl(&mut self, _u: &UseDecl) {
        // `build_definitions` already validated this use edge and bound
        // its imported declaration in the source file's namespace.
    }

    fn resolve_fn_decl(&mut self, f: &FnDecl) {
        self.push_generics(&f.generics);
        for g in &f.generics {
            if let Some(const_ty) = &g.const_ty {
                self.resolve_type_expr(const_ty);
            }
            for bound in &g.bounds {
                self.resolve_type_expr(bound);
            }
        }
        self.scopes.push();
        if f.self_param.is_some() {
            // `self` has no binding-site node of its own; keyed by the
            // owning `FnDecl`'s id instead (see `ResolvedNames::locals`).
            self.bind_local(f.id, Symbol::new("self"));
        }
        for param in &f.params {
            self.resolve_type_expr(&param.ty);
            self.bind_param(param);
        }
        if let Some(ret) = &f.ret {
            self.resolve_type_expr(ret);
        }
        if let Some(body) = &f.body {
            self.resolve_block_in_current_scope(body);
        }
        self.scopes.pop();
        self.pop_generics();
    }

    fn bind_param(&mut self, param: &Param) {
        self.bind_local(param.id, param.name.name.clone());
    }

    // ---- types ----

    fn resolve_type_expr(&mut self, ty: &TypeExpr) {
        match ty {
            TypeExpr::Const(_, _) => {}
            TypeExpr::Named { path, generics, .. } => {
                self.resolve_type_path(path);
                for g in generics {
                    self.resolve_type_expr(g);
                }
            }
            TypeExpr::Tuple(elems, _) => {
                for e in elems {
                    self.resolve_type_expr(e);
                }
            }
            TypeExpr::Array(inner, _)
            | TypeExpr::Weak(inner, _)
            | TypeExpr::Any(inner, _)
            | TypeExpr::Some(inner, _)
            | TypeExpr::Unique(inner, _)
            | TypeExpr::Ref(inner, _)
            | TypeExpr::MutRef(inner, _)
            | TypeExpr::RawConstPtr(inner, _)
            | TypeExpr::RawMutPtr(inner, _) => self.resolve_type_expr(inner),
            TypeExpr::FixedArray {
                element, length, ..
            } => {
                self.resolve_type_expr(element);
                self.resolve_type_expr(length);
            }
            TypeExpr::Function { params, ret, .. } => {
                for p in params {
                    self.resolve_type_expr(p);
                }
                self.resolve_type_expr(ret);
            }
        }
    }

    fn resolve_type_path(&mut self, path: &Path) {
        let first = &path.segments[0];
        if self.is_generic_param(&first.name) {
            self.path_res.insert(
                path.id,
                PathResolution {
                    base: Resolution::GenericParam,
                    consumed: 1,
                },
            );
            return;
        }
        let named = path
            .segments
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, segment)| {
                self.defs
                    .lookup_in(path.span.file, &segment.name)
                    .map(|id| (index, id))
            });
        match named {
            Some((index, id))
                if index + 1 == path.segments.len() || (index == 0 && path.segments.len() == 2) =>
            {
                self.path_res.insert(
                    path.id,
                    PathResolution {
                        base: Resolution::Def(id),
                        consumed: index + 1,
                    },
                );
            }
            Some(_) => {
                self.error(
                    path.span,
                    "a type path cannot contain value/member segments after the type name",
                );
                self.path_res.insert(
                    path.id,
                    PathResolution {
                        base: Resolution::Error,
                        consumed: path.segments.len(),
                    },
                );
            }
            None => {
                let name = path.segments.last().unwrap_or(first);
                self.error(
                    name.span,
                    format!("cannot find type `{}` in this scope", name.name),
                );
                self.path_res.insert(
                    path.id,
                    PathResolution {
                        base: Resolution::Error,
                        consumed: path.segments.len(),
                    },
                );
            }
        }
    }

    // ---- statements / expressions ----
}
