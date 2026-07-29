use std::collections::{HashMap, HashSet};

use nether_ast::{
    Block, EnumDecl, Expr, ExprKind, FnDecl, GenericParam, ImplBlock, Item, Module, NodeId, Param,
    Path, Pattern, Stmt, Symbol, TypeDecl, TypeExpr, UseDecl,
};
use nether_diagnostics::Diagnostic;

use crate::def::{self, DefId, DefKind, Definitions};
use crate::scope::{LocalId, LocalIdGen, Scopes};

/// What one [`Path`] (or, in type position, the leading segments of a
/// [`TypeExpr::Named`]) turned out to name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// A local variable — `self`, a parameter, a `let` binding, or a
    /// pattern binding.
    Local(LocalId),
    /// The whole path names this definition directly, with no trailing
    /// member segment (a bare type/enum/interface/fn/primitive name).
    Def(DefId),
    /// `Color.Red`, `Option.Some` — the enum and the variant's index
    /// within [`def::Def::variants`].
    EnumVariant(DefId, u32),
    /// `Dog.new` — the type/enum and the method's index within
    /// [`def::Def::methods`].
    StaticMember(DefId, u32),
    /// Resolved to an in-scope generic type parameter (type position only;
    /// `typecheck` substitutes the concrete type during monomorphization).
    GenericParam,
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
    let (defs, mut diags) = def::collect(module);
    let type_generics = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Type(decl) => {
                Some((
                    (decl.span.file, decl.name.name.clone()),
                    decl.generics.clone(),
                ))
            }
            Item::Enum(decl) => {
                Some((
                    (decl.span.file, decl.name.name.clone()),
                    decl.generics.clone(),
                ))
            }
            _ => None,
        })
        .collect();

    let mut resolver = Resolver {
        defs: &defs,
        scopes: Scopes::new(),
        generic_scopes: Vec::new(),
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
    let Resolver { path_res, local_sites, mut diagnostics, .. } = resolver;
    diags.append(&mut diagnostics);

    (ResolvedNames { definitions: defs, path_res, locals: local_sites }, diags)
}

struct Resolver<'a> {
    defs: &'a Definitions,
    scopes: Scopes,
    /// Stack of in-scope generic-parameter name sets, pushed on entering a
    /// generic item's signature/body and popped on leaving. Checked before
    /// falling back to `defs` when resolving a type-position path.
    generic_scopes: Vec<HashSet<Symbol>>,
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
        self.diagnostics.push(Diagnostic::error(message).with_label(span, "here"));
    }

    fn bind_local(&mut self, site: NodeId, name: Symbol) {
        let id = self.locals.next_id();
        self.scopes.bind(name, id);
        self.local_sites.insert(site, id);
    }

    fn push_generics(&mut self, generics: &[GenericParam]) {
        let names = generics.iter().map(|g| g.name.name.clone()).collect();
        self.generic_scopes.push(names);
    }

    fn pop_generics(&mut self) {
        self.generic_scopes.pop();
    }

    fn is_generic_param(&self, name: &Symbol) -> bool {
        self.generic_scopes.iter().any(|scope| scope.contains(name))
    }

    // ---- items ----

    fn resolve_item(&mut self, item: &Item) {
        match item {
            Item::Type(t) => self.resolve_type_decl(t),
            Item::Enum(e) => self.resolve_enum_decl(e),
            Item::Interface(i) => {
                self.push_generics(&i.generics);
                for generic in &i.generics {
                    if let Some(bound) = &generic.bound {
                        self.resolve_type_expr(bound);
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
        }
    }

    fn resolve_type_decl(&mut self, t: &TypeDecl) {
        self.push_generics(&t.generics);
        for g in &t.generics {
            if let Some(bound) = &g.bound {
                self.resolve_type_expr(bound);
            }
        }
        match &t.kind {
            nether_ast::TypeDeclKind::Struct(fields) => {
                for field in fields {
                    self.resolve_type_expr(&field.ty);
                }
            }
            nether_ast::TypeDeclKind::TupleStruct(tys) => {
                for ty in tys {
                    self.resolve_type_expr(ty);
                }
            }
            nether_ast::TypeDeclKind::Unit => {}
        }
        self.pop_generics();
    }

    fn resolve_enum_decl(&mut self, e: &EnumDecl) {
        self.push_generics(&e.generics);
        for g in &e.generics {
            if let Some(bound) = &g.bound {
                self.resolve_type_expr(bound);
            }
        }
        for variant in &e.variants {
            for ty in &variant.payload {
                self.resolve_type_expr(ty);
            }
        }
        self.pop_generics();
    }

    fn resolve_impl_block(&mut self, b: &ImplBlock) {
        let owner_generics = self
            .type_generics
            .get(&(b.span.file, b.target.name.clone()))
            .cloned()
            .unwrap_or_default();
        self.push_generics(&owner_generics);
        if let Some(interface) = &b.interface {
            self.resolve_type_expr(interface);
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
            if let Some(bound) = &g.bound {
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
            TypeExpr::Array(inner, _) | TypeExpr::Weak(inner, _) => self.resolve_type_expr(inner),
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
        if path.segments.len() == 1 && self.is_generic_param(&first.name) {
            self.path_res.insert(path.id, PathResolution { base: Resolution::GenericParam, consumed: 1 });
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
            Some((index, id)) if index + 1 == path.segments.len() => {
                self.path_res.insert(path.id, PathResolution { base: Resolution::Def(id), consumed: index + 1 });
            }
            Some(_) => {
                self.error(path.span, "a type path cannot contain value/member segments after the type name");
                self.path_res.insert(path.id, PathResolution { base: Resolution::Error, consumed: path.segments.len() });
            }
            None => {
                let name = path.segments.last().unwrap_or(first);
                self.error(name.span, format!("cannot find type `{}` in this scope", name.name));
                self.path_res.insert(path.id, PathResolution { base: Resolution::Error, consumed: path.segments.len() });
            }
        }
    }

    // ---- statements / expressions ----

    fn resolve_block_in_current_scope(&mut self, block: &Block) {
        // Caller already pushed a scope (function bodies push one scope
        // that also holds `self`/params); nested `{}` uses `resolve_block`.
        for stmt in &block.stmts {
            self.resolve_stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            self.resolve_expr(tail);
        }
    }

    fn resolve_block(&mut self, block: &Block) {
        self.scopes.push();
        self.resolve_block_in_current_scope(block);
        self.scopes.pop();
    }

    fn resolve_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(let_stmt) => {
                // Resolve the value *before* introducing the binding, so
                // `let x = x;` resolves the right-hand `x` to any outer
                // binding, matching Rust's shadowing semantics.
                self.resolve_expr(&let_stmt.value);
                if let Some(ty) = &let_stmt.ty {
                    self.resolve_type_expr(ty);
                }
                self.bind_local(let_stmt.id, let_stmt.name.name.clone());
            }
            Stmt::Expr(expr) => self.resolve_expr(expr),
        }
    }

    fn resolve_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Literal(_) | ExprKind::Continue => {}
            ExprKind::Path(path) => self.resolve_value_path(path),
            ExprKind::Tuple(elems) | ExprKind::Array(elems) => {
                for e in elems {
                    self.resolve_expr(e);
                }
            }
            ExprKind::StringTemplate(parts) => {
                for part in parts {
                    if let nether_ast::TemplatePart::Expr(e) = part {
                        self.resolve_expr(e);
                    }
                }
            }
            ExprKind::Unary { expr, .. } | ExprKind::MutArg(expr) => self.resolve_expr(expr),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.resolve_expr(lhs);
                self.resolve_expr(rhs);
            }
            ExprKind::Assign { target, value } => {
                self.resolve_expr(target);
                self.resolve_expr(value);
            }
            ExprKind::Call { callee, args } => {
                self.resolve_expr(callee);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.resolve_expr(receiver);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            ExprKind::Field { base, .. } => self.resolve_expr(base),
            ExprKind::Index { base, index } => {
                self.resolve_expr(base);
                self.resolve_expr(index);
            }
            ExprKind::If { cond, then_branch, else_branch } => {
                self.resolve_expr(cond);
                self.resolve_block(then_branch);
                if let Some(e) = else_branch {
                    self.resolve_expr(e);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.resolve_expr(scrutinee);
                for arm in arms {
                    self.scopes.push();
                    self.resolve_pattern(&arm.pattern);
                    self.resolve_expr(&arm.body);
                    self.scopes.pop();
                }
            }
            ExprKind::Block(block) => self.resolve_block(block),
            ExprKind::While { cond, body } => {
                self.resolve_expr(cond);
                self.resolve_block(body);
            }
            ExprKind::ForIn { pattern, iter, body } => {
                self.resolve_expr(iter);
                self.scopes.push();
                self.resolve_pattern(pattern);
                self.resolve_block_in_current_scope(body);
                self.scopes.pop();
            }
            ExprKind::Loop { body } => self.resolve_block(body),
            ExprKind::Break(value) | ExprKind::Return(value) => {
                if let Some(v) = value {
                    self.resolve_expr(v);
                }
            }
            ExprKind::Closure { params, body } => {
                self.scopes.push();
                for param in params {
                    self.resolve_type_expr(&param.ty);
                    self.bind_param(param);
                }
                self.resolve_expr(body);
                self.scopes.pop();
            }
            ExprKind::StructLit { path, fields } => {
                self.resolve_struct_lit_path(path);
                for (_, value) in fields {
                    self.resolve_expr(value);
                }
            }
        }
    }

    fn resolve_struct_lit_path(&mut self, path: &Path) {
        // A struct literal's path names a *type*, never a local — resolve
        // it the same way a type-position path is resolved.
        self.resolve_type_path(path);
    }

    fn resolve_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard(_) | Pattern::Literal(_, _) => {}
            Pattern::Binding(id, ident) => self.resolve_binding_or_unit_variant(*id, ident),
            Pattern::Tuple(elems, _) => {
                for e in elems {
                    self.resolve_pattern(e);
                }
            }
            Pattern::Variant { path, payload, .. } => {
                self.resolve_variant_pattern_path(path);
                for p in payload {
                    self.resolve_pattern(p);
                }
            }
        }
    }

    /// A bare identifier in pattern position (`Same => ...`) is
    /// syntactically ambiguous between introducing a fresh binding and
    /// matching an existing unit variant by name — `nether_parser` can't
    /// tell without name information, so it always produces
    /// [`Pattern::Binding`], and disambiguation happens here, the same way
    /// Rust's own resolver treats a bare path pattern that happens to name
    /// a unit variant/const as that item rather than a new binding.
    fn resolve_binding_or_unit_variant(&mut self, id: NodeId, ident: &nether_ast::Ident) {
        match find_unique_variant(
            self.defs,
            ident.span.file,
            &ident.name,
        ) {
            Ok((enum_id, idx)) => {
                self.path_res.insert(id, PathResolution { base: Resolution::EnumVariant(enum_id, idx), consumed: 1 });
            }
            Err(0) => self.bind_local(id, ident.name.clone()),
            Err(_) => {
                self.error(
                    ident.span,
                    format!("`{}` is ambiguous: more than one enum defines a variant with this name", ident.name),
                );
            }
        }
    }

    fn resolve_variant_pattern_path(&mut self, path: &Path) {
        if path.segments.len() >= 2 {
            let variant_name = path.segments.last().unwrap();
            let enum_name = &path.segments[path.segments.len() - 2];
            match self
                .defs
                .lookup_in(path.span.file, &enum_name.name)
            {
                Some(id) if self.defs.get(id).kind == DefKind::Enum => {
                    match variant_index(self.defs.get(id), &variant_name.name) {
                        Some(idx) => {
                            self.path_res.insert(
                                path.id,
                                PathResolution { base: Resolution::EnumVariant(id, idx), consumed: path.segments.len() },
                            );
                        }
                        None => {
                            self.error(
                                variant_name.span,
                                format!("enum `{}` has no variant named `{}`", enum_name.name, variant_name.name),
                            );
                            self.path_res.insert(path.id, PathResolution { base: Resolution::Error, consumed: 2 });
                        }
                    }
                }
                _ => {
                    self.error(enum_name.span, format!("cannot find enum `{}` in this scope", enum_name.name));
                    self.path_res.insert(
                        path.id,
                        PathResolution { base: Resolution::Error, consumed: path.segments.len() },
                    );
                }
            }
            return;
        }

        // A bare `Custom(x)` pattern with no enum-name qualifier: search
        // every known enum for a unique variant with this name.
        let name = &path.segments[0];
        match find_unique_variant(
            self.defs,
            name.span.file,
            &name.name,
        ) {
            Ok((enum_id, idx)) => {
                self.path_res
                    .insert(path.id, PathResolution { base: Resolution::EnumVariant(enum_id, idx), consumed: 1 });
            }
            Err(0) => {
                self.error(name.span, format!("no enum variant named `{}` found", name.name));
                self.path_res.insert(path.id, PathResolution { base: Resolution::Error, consumed: 1 });
            }
            Err(_) => {
                self.error(
                    name.span,
                    format!("`{}` is ambiguous: more than one enum defines a variant with this name", name.name),
                );
                self.path_res.insert(path.id, PathResolution { base: Resolution::Error, consumed: 1 });
            }
        }
    }

    fn resolve_value_path(&mut self, path: &Path) {
        let first = &path.segments[0];

        if let Some(local_id) = self.scopes.lookup(&first.name) {
            self.path_res.insert(path.id, PathResolution { base: Resolution::Local(local_id), consumed: 1 });
            return;
        }

        let Some(id) = self.defs.lookup_in(path.span.file, &first.name) else {
            self.unresolved_value(first);
            self.path_res.insert(path.id, PathResolution { base: Resolution::Error, consumed: path.segments.len() });
            return;
        };

        let def = self.defs.get(id);
        if path.segments.len() == 1 {
            self.path_res.insert(path.id, PathResolution { base: Resolution::Def(id), consumed: 1 });
            return;
        }

        let second = &path.segments[1];
        match def.kind {
            DefKind::Enum => {
                if let Some(idx) = variant_index(def, &second.name) {
                    self.path_res
                        .insert(path.id, PathResolution { base: Resolution::EnumVariant(id, idx), consumed: 2 });
                } else if let Some(idx) = method_index(def, &second.name) {
                    self.path_res
                        .insert(path.id, PathResolution { base: Resolution::StaticMember(id, idx), consumed: 2 });
                } else {
                    self.error(
                        second.span,
                        format!("enum `{}` has no variant or method named `{}`", first.name, second.name),
                    );
                    self.path_res.insert(path.id, PathResolution { base: Resolution::Error, consumed: 2 });
                }
            }
            DefKind::Type => {
                if let Some(idx) = method_index(def, &second.name) {
                    self.path_res
                        .insert(path.id, PathResolution { base: Resolution::StaticMember(id, idx), consumed: 2 });
                } else {
                    self.error(second.span, format!("type `{}` has no method named `{}`", first.name, second.name));
                    self.path_res.insert(path.id, PathResolution { base: Resolution::Error, consumed: 2 });
                }
            }
            DefKind::Interface | DefKind::Fn | DefKind::Primitive => {
                self.error(second.span, format!("`{}` has no member named `{}`", first.name, second.name));
                self.path_res.insert(path.id, PathResolution { base: Resolution::Error, consumed: 2 });
            }
            DefKind::Imported => {
                // Compatibility mode for callers that resolve a single
                // parsed file without asking the driver to load imports.
                // The imported declaration is opaque in that API.
                self.path_res.insert(path.id, PathResolution { base: Resolution::Def(id), consumed: 1 });
            }
        }
    }

    fn unresolved_value(&mut self, name: &nether_ast::Ident) {
        let mut candidates = self.scopes.visible_names();
        candidates.extend(
            self.defs
                .visible_in(name.span.file)
                .into_iter()
                .map(|id| self.defs.get(id).name.clone()),
        );
        candidates.sort();
        candidates.dedup();

        let mut diagnostic =
            Diagnostic::error(format!("cannot find `{}` in this scope", name.name))
                .with_label(name.span, "unknown name");
        if let Some(candidate) = closest_name(&name.name, &candidates) {
            diagnostic = diagnostic.with_suggestion(
                name.span,
                candidate.to_string(),
                format!("did you mean `{candidate}`?"),
            );
        }
        self.diagnostics.push(diagnostic);
    }
}

fn closest_name<'a>(needle: &Symbol, candidates: &'a [Symbol]) -> Option<&'a Symbol> {
    let needle_len = needle.as_str().chars().count();
    let max_distance = match needle_len {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    };
    candidates
        .iter()
        .filter(|candidate| *candidate != needle)
        .map(|candidate| {
            (
                edit_distance(needle.as_str(), candidate.as_str()),
                candidate,
            )
        })
        .filter(|(distance, _)| *distance <= max_distance)
        .min_by(|(left_distance, left), (right_distance, right)| {
            left_distance
                .cmp(right_distance)
                .then_with(|| left.cmp(right))
        })
        .map(|(_, candidate)| candidate)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(left_index + 1);
        for (right_index, right_char) in right.iter().enumerate() {
            let substitution = previous[right_index]
                + usize::from(left_char != *right_char);
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            current.push(substitution.min(insertion).min(deletion));
        }
        previous = current;
    }
    previous[right.len()]
}

fn variant_index(def: &def::Def, name: &Symbol) -> Option<u32> {
    def.variants.iter().position(|v| v == name).map(|i| i as u32)
}

fn method_index(def: &def::Def, name: &Symbol) -> Option<u32> {
    def.methods.iter().position(|m| m == name).map(|i| i as u32)
}

/// Searches every enum definition for a variant named `name`. Returns
/// `Err(count)` when the match isn't unique (`0` = not found, `2+` =
/// ambiguous) — used for unqualified variant patterns like bare `Custom(x)`.
fn find_unique_variant(
    defs: &Definitions,
    file: nether_diagnostics::FileId,
    name: &Symbol,
) -> Result<(DefId, u32), usize> {
    let mut found = None;
    let mut count = 0usize;
    for id in defs.visible_in(file) {
        let def = defs.get(id);
        if def.kind != DefKind::Enum {
            continue;
        }
        if let Some(idx) = variant_index(def, name) {
            count += 1;
            found = Some((id, idx));
        }
    }
    match count {
        1 => Ok(found.unwrap()),
        n => Err(n),
    }
}
