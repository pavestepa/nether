use super::*;

pub(super) fn build_type_shapes(
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    for (&id, t) in &decls.type_decls {
        sigs.type_generics
            .insert(id, t.generics.iter().map(|g| g.name.name.clone()).collect());
        sigs.generic_type_bounds.insert(
            id,
            t.generics
                .iter()
                .map(|generic| {
                    generic
                        .bound
                        .as_ref()
                        .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags))
                })
                .collect(),
        );
        let shape = match &t.kind {
            TypeDeclKind::Struct(fields) => TypeShape::Struct(
                fields
                    .iter()
                    .map(|f| {
                        (
                            f.name.name.clone(),
                            lower_type_expr(&f.ty, resolved, decls, diags),
                        )
                    })
                    .collect(),
            ),
            TypeDeclKind::TupleStruct(tys) => TypeShape::TupleStruct(
                tys.iter()
                    .map(|ty| lower_type_expr(ty, resolved, decls, diags))
                    .collect(),
            ),
            TypeDeclKind::Unit => TypeShape::Unit,
        };
        sigs.type_shapes.insert(id, shape);
    }
}

pub(super) fn build_enum_sigs(
    module: &Module,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    // `Option`/`Result` need no seeding here — they're ordinary `enum`
    // items in the bundled prelude (`stdlib/option.nt`/`result.nt`), so
    // the loop below already covers them exactly like any user-declared
    // generic enum.
    for item in &module.items {
        let Item::Enum(e) = item else { continue };
        let Some(id) = resolved.definitions.lookup_in(e.span.file, &e.name.name) else {
            continue;
        };
        let generics = e.generics.iter().map(|g| g.name.name.clone()).collect();
        sigs.generic_type_bounds.insert(
            id,
            e.generics
                .iter()
                .map(|generic| {
                    generic
                        .bound
                        .as_ref()
                        .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags))
                })
                .collect(),
        );
        let variants = e
            .variants
            .iter()
            .map(|v| {
                (
                    v.name.name.clone(),
                    v.payload
                        .iter()
                        .map(|t| lower_type_expr(t, resolved, decls, diags))
                        .collect(),
                )
            })
            .collect();
        sigs.enum_sigs.insert(id, EnumSig { generics, variants });
    }
}

pub(super) fn build_fn_sigs(
    module: &Module,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    for item in &module.items {
        if let Item::Fn(f) = item {
            if let Some(id) = resolved.definitions.lookup_in(f.span.file, &f.name.name) {
                let sig = build_fn_sig(f, resolved, decls, diags);
                sigs.fns.insert(id, sig);
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct InterfaceDefault {
    pub(super) source: DefId,
    /// Source-interface generic parameters expressed in the current
    /// interface's generic parameters.
    pub(super) subst: HashMap<Symbol, Type>,
}

#[derive(Clone)]
pub(super) struct InterfaceMethod {
    pub(super) sig: FnSig,
    pub(super) default: Option<InterfaceDefault>,
    pub(super) ambiguous_default: bool,
}

pub(super) type InterfaceMethodTable = HashMap<DefId, HashMap<Symbol, InterfaceMethod>>;

pub(super) fn specialize_fn_sig(sig: &FnSig, subst: &HashMap<Symbol, Type>) -> FnSig {
    FnSig {
        self_param: sig.self_param,
        params: sig
            .params
            .iter()
            .map(|param| crate::sig::ParamSig {
                name: param.name.clone(),
                mutable: param.mutable,
                variadic: param.variadic,
                ty: substitute_generic(&param.ty, subst),
            })
            .collect(),
        ret: substitute_generic(&sig.ret, subst),
        generics: sig
            .generics
            .iter()
            .map(|(name, bound)| {
                (
                    name.clone(),
                    bound.as_ref().map(|bound| GenericBound {
                        interface: bound.interface,
                        args: bound
                            .args
                            .iter()
                            .map(|arg| substitute_generic(arg, subst))
                            .collect(),
                    }),
                )
            })
            .collect(),
    }
}

pub(super) fn method_signatures_match(actual: &FnSig, required: &FnSig) -> bool {
    actual.self_param == required.self_param
        && actual.params.len() == required.params.len()
        && actual.generics.len() == required.generics.len()
        && actual
            .params
            .iter()
            .zip(&required.params)
            .all(|(actual, required)| {
                actual.mutable == required.mutable && actual.ty == required.ty
            })
        && actual.ret == required.ret
}

pub(super) fn owner_generic_params(
    owner: DefId,
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    diags: &mut Vec<Diagnostic>,
) -> Vec<(Symbol, Option<GenericBound>)> {
    let generics = decls
        .type_decls
        .get(&owner)
        .map(|decl| decl.generics.as_slice())
        .or_else(|| {
            decls
                .enum_decls
                .get(&owner)
                .map(|decl| decl.generics.as_slice())
        })
        .unwrap_or(&[]);
    generics
        .iter()
        .map(|generic| {
            (
                generic.name.name.clone(),
                generic
                    .bound
                    .as_ref()
                    .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags)),
            )
        })
        .collect()
}

/// The number of type arguments `owner` takes — a declared type's or
/// enum's own `generics.len()` (`Option`/`Result` included: ordinary
/// prelude `enum`s, not builtins, so they always have a declaration to
/// read here), `0` for anything else (a primitive, an interface, ...).
pub(super) fn owner_arity(owner: DefId, decls: &DeclIndex) -> usize {
    decls
        .type_decls
        .get(&owner)
        .map(|decl| decl.generics.len())
        .or_else(|| decls.enum_decls.get(&owner).map(|decl| decl.generics.len()))
        .unwrap_or(0)
}

/// Owner generics for `impl<T, ...> Target<...> { ... }` — the explicit
/// Rust-like form ([`nether_ast::ImplBlock::generics`] non-empty). Unlike
/// the implicit form (whose scope is read straight off the target's own
/// declaration, see [`owner_generic_params`]), this form is the only way
/// to bind a name for a builtin owner such as `Option`/`Result` that has
/// no declaration to read parameters from.
///
/// `target_args` is deliberately restricted to a bare permutation of the
/// impl's own declared generic names, in the owner's positional order —
/// pure renaming, not specialization. `impl Option<i32>` (a concrete
/// argument) or `impl<T, U> Option<T>` (an unused impl parameter) are
/// rejected; only `impl<T> Option<T>` is accepted here (MVP scope —
/// concrete/specialized impls are a separate, not-yet-supported feature).
pub(super) fn explicit_impl_owner_generics(
    block: &ImplBlock,
    owner: DefId,
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    diags: &mut Vec<Diagnostic>,
) -> Vec<(Symbol, Option<GenericBound>)> {
    let owner_name = resolved.definitions.get(owner).name.clone();
    let expected = owner_arity(owner, decls);

    let fallback = |diags: &mut Vec<Diagnostic>| {
        block
            .generics
            .iter()
            .map(|g| {
                (
                    g.name.name.clone(),
                    g.bound
                        .as_ref()
                        .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags)),
                )
            })
            .collect::<Vec<_>>()
    };

    if block.target_args.len() != expected {
        diags.push(
            Diagnostic::error(format!(
                "`{owner_name}` takes {expected} type argument(s), found {}",
                block.target_args.len()
            ))
            .with_label(block.span, "in this `impl` block"),
        );
        return fallback(diags);
    }

    let mut used = HashSet::new();
    let mut ordered_names = Vec::with_capacity(block.target_args.len());
    let mut ok = true;
    for arg in &block.target_args {
        let named = match arg {
            TypeExpr::Named { path, generics, .. }
                if generics.is_empty() && path.segments.len() == 1 =>
            {
                block
                    .generics
                    .iter()
                    .find(|g| g.name.name == path.segments[0].name)
                    .map(|g| g.name.name.clone())
            }
            _ => None,
        };
        match named {
            Some(name) if used.insert(name.clone()) => ordered_names.push(name),
            _ => {
                ok = false;
                diags.push(
                    Diagnostic::error(
                        "an explicit `impl<...> Target<...>` argument list must name each of \
                         the impl's own generic parameters exactly once (no concrete types, no \
                         repeats)",
                    )
                    .with_label(arg.span(), "not one of this impl's generic parameters"),
                );
            }
        }
    }
    if ok && ordered_names.len() != block.generics.len() {
        ok = false;
        diags.push(
            Diagnostic::error(format!(
                "every one of this `impl`'s generic parameters must appear in `{owner_name}<...>`"
            ))
            .with_label(block.span, "in this `impl` block"),
        );
    }
    if !ok {
        return fallback(diags);
    }

    ordered_names
        .into_iter()
        .map(|name| {
            let bound = block
                .generics
                .iter()
                .find(|g| g.name.name == name)
                .and_then(|g| g.bound.as_ref())
                .and_then(|bound| lower_generic_bound(bound, resolved, decls, diags));
            (name, bound)
        })
        .collect()
}

/// Dispatches to the implicit, explicit-generic, or concrete-specialization
/// form depending on what `block` wrote — the one place `build_impl_methods`
/// needs to look up an impl block's generic scope. A concrete
/// specialization (`impl Option<i32> { ... }`, [`impl_specialization_args`])
/// has no generic parameters at all in scope for its own methods: every
/// field the `self` receiver could mention is already a concrete type.
pub(super) fn owner_generics_for_impl(
    block: &ImplBlock,
    owner: DefId,
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    diags: &mut Vec<Diagnostic>,
) -> Vec<(Symbol, Option<GenericBound>)> {
    if !block.generics.is_empty() {
        explicit_impl_owner_generics(block, owner, decls, resolved, diags)
    } else if block.target_args.is_empty() {
        owner_generic_params(owner, decls, resolved, diags)
    } else {
        Vec::new()
    }
}

/// Whether `block` is a concrete specialization — `impl Option<i32> {
/// ... }`, no `impl<...>` generics of its own but a fully concrete
/// `target_args` list (necessarily concrete: with no generics pushed into
/// scope for this block, `resolver` could only have resolved each
/// `target_args` entry to an actual declared type, never a generic
/// parameter — see `resolver::resolve_impl_block`'s explicit-form
/// branch). `None` for every other block shape, including on an arity
/// mismatch (already diagnosed here; the caller falls back to treating it
/// as an ordinary non-specialized block).
pub(super) fn impl_specialization_args(
    block: &ImplBlock,
    owner: DefId,
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    diags: &mut Vec<Diagnostic>,
) -> Option<Vec<Type>> {
    if !block.generics.is_empty() || block.target_args.is_empty() {
        return None;
    }
    let owner_name = resolved.definitions.get(owner).name.clone();
    let expected = owner_arity(owner, decls);
    if block.target_args.len() != expected {
        diags.push(
            Diagnostic::error(format!(
                "`{owner_name}` takes {expected} type argument(s), found {}",
                block.target_args.len()
            ))
            .with_label(block.span, "in this `impl` block"),
        );
        return None;
    }
    Some(
        block
            .target_args
            .iter()
            .map(|arg| lower_type_expr(arg, resolved, decls, diags))
            .collect(),
    )
}

/// Whether a concrete specialization's method signature (`concrete_sig`,
/// declared with no generics of its own beyond the method's own extra
/// ones) matches the generic impl's signature for the same method name,
/// once the generic impl's own owner parameters are substituted with
/// `concrete_args`. Specialization may only override a method's body, not
/// its externally observable type — required so that a still-generic
/// caller (which can only ever see the generic signature, since it
/// doesn't know which concrete override will apply until monomorphized)
/// never disagrees with what actually runs.
pub(super) fn specialization_matches_generic(
    owner: DefId,
    decls: &DeclIndex,
    generic_sig: &FnSig,
    concrete_args: &[Type],
    concrete_sig: &FnSig,
) -> bool {
    let owner_arity = owner_arity(owner, decls);
    if generic_sig.self_param != concrete_sig.self_param
        || generic_sig.params.len() != concrete_sig.params.len()
        || generic_sig.generics.len() != owner_arity + concrete_sig.generics.len()
    {
        return false;
    }
    let subst: HashMap<Symbol, Type> = generic_sig.generics[..owner_arity]
        .iter()
        .map(|(name, _)| name.clone())
        .zip(concrete_args.iter().cloned())
        .collect();
    let expected = specialize_fn_sig(generic_sig, &subst);
    expected
        .params
        .iter()
        .zip(&concrete_sig.params)
        .all(|(expected, actual)| expected.mutable == actual.mutable && expected.ty == actual.ty)
        && expected.ret == concrete_sig.ret
}
