//! # nether_monomorphization
//!
//! Purpose: eliminate all generics between `hir` and `mir` — produce one
//! fully-substituted copy of every generic `fn`/method per concrete
//! instantiation actually reachable from the program's entry point
//! (`docs/architecture/crates.md` § `compiler/monomorphization`).
//!
//! Responsibilities:
//! - Starting from the entry function (`main`), walk `CallStatic`/`Call`
//!   sites reachable from it and discover the set of concrete
//!   instantiations of each generic `fn` actually needed, memoizing by
//!   `(HirFnId, concrete type arguments)` so repeated instantiations with
//!   the same arguments reuse one [`node::MonoFunction`].
//! - Substitute every `Type::Generic` occurring in a generic function's
//!   signature/body with the concrete `Type` determined at its call
//!   site(s), read directly off the (already fully-typed) `HirExpr`
//!   arguments — no re-inference needed, `typecheck` already did it.
//! - Resolve every [`nether_hir::HirExprKind::CallGenericMethod`] into a
//!   concrete [`node::MonoExprKind::CallStatic`], now that the receiver's
//!   substituted type identifies exactly one `impl`.
//!
//! Input: a [`nether_hir::HirModule`] plus the entry function's
//! [`nether_hir::HirFnId`] (the driver resolves the entry module's `main`
//! through `HirModule::fn_by_def`).
//!
//! Output: a [`node::MonoModule`] — every function/method actually
//! reachable from the entry point, with zero remaining generic
//! parameters.
//!
//! Dependencies: `nether_ast`, `nether_resolver`, `nether_typecheck`,
//! `nether_hir`.
//!
//! Consumers: `mir`.
//!
//! Invariants: no [`node::MonoFunction`] contains a `Type::Generic` or a
//! still-generic call; functions never reached from the entry point are
//! not included (dead-code elimination is a free side effect of this
//! walk, not a separate pass). Monomorphization always terminates for the
//! MVP's generics — the language has no way to express higher-kinded or
//! recursive generic instantiation, so the memoized call-graph walk below
//! is finite by construction (`docs/architecture/crates.md`'s own
//! invariant for this crate).
//!
//! Documented simplifications:
//! - A generic function referenced as a first-class value
//!   ([`nether_hir::HirExprKind::FnRef`]) without being called directly is
//!   not instantiated — there is no argument list to read concrete types
//!   from at that point. `typecheck` rejects this source form with a
//!   diagnostic; first-class non-generic functions and closures are
//!   supported.
//!
//! Generic types and their methods use the same substitution map; layout
//! specialization itself remains in codegen where concrete field layouts
//! are needed.

#[path = "mono/helpers.rs"]
mod helpers;
mod node;
#[path = "mono/substitute.rs"]
mod substitute;

use std::collections::HashMap;

use nether_ast::Symbol;
use nether_hir::{
    HirCapture, HirExpr, HirExprKind, HirFnId, HirFunction, HirLocalId, HirModule, HirParam,
    HirStmtKind,
};
use nether_resolver::DefId;
use nether_typecheck::{ReceiverDomain, Type};

pub use node::{
    MonoCapture, MonoExpr, MonoExprKind, MonoFnId, MonoFunction, MonoMatchArm, MonoModule,
    MonoParam, MonoStmt, MonoStmtKind,
};

use helpers::*;

/// Runs monomorphization starting from `entry` (the program's `main`).
///
/// # Panics
/// Panics if the reachable call graph references a generic function in a
/// way this crate's documented simplifications don't cover (an
/// unsubstitutable `FnRef` to a generic function, or a
/// `CallGenericMethod` whose substituted receiver type isn't a
/// user-defined `Struct`/`TupleStruct`/`Enum`) — both are unreachable for
/// any program `typecheck` accepts today; see this crate's module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericStrategy {
    Monomorphize,
    WitnessTables,
}

pub fn monomorphize(module: &HirModule, entry: HirFnId) -> MonoModule {
    monomorphize_with_strategy(module, entry, GenericStrategy::Monomorphize)
}

pub fn monomorphize_with_strategy(
    module: &HirModule,
    entry: HirFnId,
    strategy: GenericStrategy,
) -> MonoModule {
    let mut ctx = Mono {
        hir: module,
        strategy,
        functions: Vec::new(),
        plain: HashMap::new(),
        generic: HashMap::new(),
        adapters: HashMap::new(),
    };
    let entry_id = ctx.instantiate_plain(entry);
    MonoModule {
        functions: ctx.functions,
        entry: entry_id,
    }
}

struct Mono<'a> {
    hir: &'a HirModule,
    strategy: GenericStrategy,
    functions: Vec<MonoFunction>,
    /// Non-generic `HirFnId`s each ever expand to exactly one
    /// `MonoFunction` — memoized purely to avoid duplicating shared calls
    /// and to break recursion.
    plain: HashMap<HirFnId, MonoFnId>,
    /// Generic `HirFnId`s expand to one `MonoFunction` per distinct
    /// concrete type-argument list, in the function's own declared
    /// `generics` order.
    generic: HashMap<(HirFnId, Vec<Type>), MonoFnId>,
    /// Closure-ABI wrapper for each ordinary named function referenced as
    /// a first-class value.
    adapters: HashMap<MonoFnId, MonoFnId>,
}

impl<'a> Mono<'a> {
    fn subst_ty(&self, ty: &Type, subst: &HashMap<Symbol, Type>) -> Type {
        self.hir
            .signatures
            .normalize_associated(&subst_type(ty, subst))
    }
    fn reserve_slot(&mut self, placeholder: MonoFunction) -> MonoFnId {
        let id = MonoFnId(self.functions.len() as u32);
        self.functions.push(placeholder);
        id
    }

    fn instantiate_plain(&mut self, hir_id: HirFnId) -> MonoFnId {
        if let Some(&id) = self.plain.get(&hir_id) {
            return id;
        }
        let hir_fn = self.hir.get(hir_id);
        debug_assert!(
            hir_fn.generics.is_empty(),
            "instantiate_plain called on a generic function"
        );
        let id = self.reserve_slot(placeholder_for(hir_fn));
        self.plain.insert(hir_id, id);
        self.finish_instantiation(id, hir_fn, &HashMap::new());
        id
    }

    fn instantiate_generic(&mut self, hir_id: HirFnId, subst: &HashMap<Symbol, Type>) -> MonoFnId {
        let hir_fn = self.hir.get(hir_id);
        let subst = if self.strategy == GenericStrategy::WitnessTables {
            self.dev_erased_substitution(hir_fn)
                .unwrap_or_else(|| subst.clone())
        } else {
            subst.clone()
        };
        let key_types: Vec<Type> = hir_fn
            .generics
            .iter()
            .map(|(name, _)| subst.get(name).cloned().unwrap_or(Type::Error))
            .collect();
        let key = (hir_id, key_types);
        if let Some(&id) = self.generic.get(&key) {
            return id;
        }
        let id = self.reserve_slot(placeholder_for(hir_fn));
        self.generic.insert(key, id);
        self.finish_instantiation(id, hir_fn, &subst);
        id
    }

    /// Development builds share the body of the common dictionary-safe
    /// generic shape: a free function whose generic value parameters are
    /// passed directly and constrained by one trait. Layout-dependent
    /// generic code deliberately falls back to ordinary monomorphization.
    fn dev_erased_substitution(&self, f: &HirFunction) -> Option<HashMap<Symbol, Type>> {
        if f.owner.is_some() || f.generics.is_empty() || has_associated_const(&f.body) {
            return None;
        }
        let mut erased = HashMap::new();
        for (name, bounds) in &f.generics {
            let [bound] = bounds.as_slice() else {
                return None;
            };
            // The concrete value argument is what carries the runtime
            // dictionary. Explicit type-only parameters have no such value
            // at the call boundary and stay monomorphized.
            if !f
                .params
                .iter()
                .any(|p| matches!(&p.ty, Type::Generic(param) if param == name))
            {
                return None;
            }
            erased.insert(name.clone(), Type::Any(bound.trait_id, bound.args.clone()));
        }
        if contains_erased_generic(&f.ret, &erased)
            || f.params.iter().any(|p| {
                contains_erased_generic(&p.ty, &erased)
                    && !matches!(&p.ty, Type::Generic(name) if erased.contains_key(name))
            })
        {
            return None;
        }
        Some(erased)
    }

    fn finish_instantiation(
        &mut self,
        id: MonoFnId,
        hir_fn: &'a HirFunction,
        subst: &HashMap<Symbol, Type>,
    ) {
        let params = hir_fn
            .params
            .iter()
            .map(|p| MonoParam {
                local: p.local,
                name: p.name.clone(),
                mutable: p.mutable,
                ty: self.subst_ty(&p.ty, subst),
            })
            .collect();
        let ret = self.subst_ty(&hir_fn.ret, subst);
        let body = self.subst_expr(&hir_fn.body, subst);
        self.functions[id.0 as usize] = MonoFunction {
            id,
            name: hir_fn.name.clone(),
            owner: hir_fn.owner,
            is_closure: false,
            self_param: hir_fn.self_param,
            self_ty: hir_fn.self_ty.as_ref().map(|ty| self.subst_ty(ty, subst)),
            self_local: hir_fn.self_local,
            captures: Vec::new(),
            params,
            ret,
            body,
        };
    }

    /// Calls (whether to a generic or non-generic target) always route
    /// through here so a generic callee's substitution is derived
    /// uniformly from its already-substituted argument types.
    fn resolve_call(
        &mut self,
        hir_id: HirFnId,
        args: &[MonoExpr],
        generic_args: &[Type],
    ) -> MonoFnId {
        let hir_fn = self.hir.get(hir_id);
        if hir_fn.generics.is_empty() {
            return self.instantiate_plain(hir_id);
        }
        let mut subst: HashMap<Symbol, Type> = hir_fn
            .generics
            .iter()
            .map(|(name, _)| name.clone())
            .zip(generic_args.iter().cloned())
            .collect();
        let self_offset = usize::from(hir_fn.self_param.is_some());
        if let (Some(self_ty), Some(receiver)) =
            (&hir_fn.self_ty, args.first().filter(|_| self_offset == 1))
        {
            collect_generic_bindings(self_ty, &receiver.ty, &mut subst);
        }
        for (param, arg) in hir_fn.params.iter().zip(args.iter().skip(self_offset)) {
            collect_generic_bindings(&param.ty, &arg.ty, &mut subst);
        }
        self.instantiate_generic(hir_id, &subst)
    }
}

fn has_associated_const(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::AssociatedConst { .. } => true,
        HirExprKind::Tuple(xs) | HirExprKind::Array(xs) | HirExprKind::Concat(xs) => {
            xs.iter().any(has_associated_const)
        }
        HirExprKind::Borrow(x)
        | HirExprKind::Deref(x)
        | HirExprKind::PromoteUnique(x)
        | HirExprKind::CloneToUnique(x)
        | HirExprKind::Hash(x)
        | HirExprKind::ToString(x)
        | HirExprKind::Unary { expr: x, .. }
        | HirExprKind::Field { base: x, .. }
        | HirExprKind::Loop { body: x }
        | HirExprKind::PackExistential { value: x, .. } => has_associated_const(x),
        HirExprKind::Binary { lhs, rhs, .. }
        | HirExprKind::Assign {
            target: lhs,
            value: rhs,
        }
        | HirExprKind::Index {
            base: lhs,
            index: rhs,
        }
        | HirExprKind::While {
            cond: lhs,
            body: rhs,
        } => has_associated_const(lhs) || has_associated_const(rhs),
        HirExprKind::Call { callee, args } => {
            has_associated_const(callee) || args.iter().any(has_associated_const)
        }
        HirExprKind::CallStatic { args, .. }
        | HirExprKind::CallBuiltin { args, .. }
        | HirExprKind::Construct { fields: args, .. }
        | HirExprKind::ConstructVariant { payload: args, .. } => {
            args.iter().any(has_associated_const)
        }
        HirExprKind::CallGenericMethod { receiver, args, .. }
        | HirExprKind::CallWitness { receiver, args, .. }
        | HirExprKind::CallArrayMethod { receiver, args, .. }
        | HirExprKind::CallMethod { receiver, args, .. } => {
            has_associated_const(receiver) || args.iter().any(has_associated_const)
        }
        HirExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            has_associated_const(cond)
                || has_associated_const(then_branch)
                || else_branch.as_deref().is_some_and(has_associated_const)
        }
        HirExprKind::Match { scrutinee, arms } => {
            has_associated_const(scrutinee)
                || arms.iter().any(|arm| has_associated_const(&arm.body))
        }
        HirExprKind::Block(stmts, tail) => {
            stmts.iter().any(|stmt| match &stmt.kind {
                HirStmtKind::Let { value, .. } | HirStmtKind::Expr(value) => {
                    has_associated_const(value)
                }
            }) || tail.as_deref().is_some_and(has_associated_const)
        }
        HirExprKind::Break(value) | HirExprKind::Return(value) => {
            value.as_deref().is_some_and(has_associated_const)
        }
        HirExprKind::Closure { body, .. } => has_associated_const(body),
        _ => false,
    }
}

fn contains_erased_generic(ty: &Type, erased: &HashMap<Symbol, Type>) -> bool {
    match ty {
        Type::Generic(name) => erased.contains_key(name),
        Type::Struct(_, args)
        | Type::TupleStruct(_, args)
        | Type::Tuple(args)
        | Type::Enum(_, args)
        | Type::Any(_, args)
        | Type::Some(_, args) => args.iter().any(|ty| contains_erased_generic(ty, erased)),
        Type::Array(inner)
        | Type::Weak(inner)
        | Type::Unique(inner)
        | Type::Ref(inner)
        | Type::MutRef(inner) => contains_erased_generic(inner, erased),
        Type::FixedArray(element, length) => {
            contains_erased_generic(element, erased) || contains_erased_generic(length, erased)
        }
        Type::Associated(owner, _) => contains_erased_generic(owner, erased),
        Type::Function(params, ret) => {
            params.iter().any(|ty| contains_erased_generic(ty, erased))
                || contains_erased_generic(ret, erased)
        }
        _ => false,
    }
}
