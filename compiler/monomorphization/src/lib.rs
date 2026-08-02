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
//! Generic types and their methods use the same substitution map; layout
//! specialization itself remains in codegen where concrete field layouts
//! are needed.

mod node;

use std::collections::HashMap;

use nether_ast::Symbol;
use nether_hir::{
    HirCapture, HirExpr, HirExprKind, HirFnId, HirFunction, HirModule, HirParam, HirStmtKind,
};
use nether_resolver::DefId;
use nether_typecheck::Type;

pub use node::{
    MonoCapture, MonoExpr, MonoExprKind, MonoFnId, MonoFunction, MonoMatchArm, MonoModule,
    MonoParam, MonoStmt, MonoStmtKind,
};

/// Runs monomorphization starting from `entry` (the program's `main`).
///
/// # Panics
/// Panics if the reachable call graph references a generic function in a
/// way this crate's documented simplifications don't cover (an
/// unsubstitutable `FnRef` to a generic function, or a
/// `CallGenericMethod` whose substituted receiver type isn't a
/// user-defined `Struct`/`TupleStruct`/`Enum`) — both are unreachable for
/// any program `typecheck` accepts today; see this crate's module docs.
pub fn monomorphize(module: &HirModule, entry: HirFnId) -> MonoModule {
    let mut ctx = Mono {
        hir: module,
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
        self.finish_instantiation(id, hir_fn, subst);
        id
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
                ty: subst_type(&p.ty, subst),
            })
            .collect();
        let ret = subst_type(&hir_fn.ret, subst);
        let body = self.subst_expr(&hir_fn.body, subst);
        self.functions[id.0 as usize] = MonoFunction {
            id,
            name: hir_fn.name.clone(),
            owner: hir_fn.owner,
            is_closure: false,
            self_param: hir_fn.self_param,
            self_ty: hir_fn.self_ty.as_ref().map(|ty| subst_type(ty, subst)),
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

    fn subst_expr(&mut self, expr: &HirExpr, subst: &HashMap<Symbol, Type>) -> MonoExpr {
        let ty = subst_type(&expr.ty, subst);
        let kind = match &expr.kind {
            HirExprKind::Literal(l) => MonoExprKind::Literal(l.clone()),
            HirExprKind::Local(id) => MonoExprKind::Local(*id),
            HirExprKind::FnRef(fn_id) => {
                let hir_fn = self.hir.get(*fn_id);
                assert!(hir_fn.generics.is_empty(), "monomorphization: cannot take a bare reference to generic function `{}` without a call site to infer its type arguments from (see this crate's module docs)", hir_fn.name);
                let target = self.instantiate_plain(*fn_id);
                MonoExprKind::Closure {
                    function: self.instantiate_adapter(target),
                    captures: Vec::new(),
                }
            }
            HirExprKind::Unit => MonoExprKind::Unit,
            HirExprKind::Tuple(items) => MonoExprKind::Tuple(self.subst_exprs(items, subst)),
            HirExprKind::Array(items) => MonoExprKind::Array(self.subst_exprs(items, subst)),
            HirExprKind::Concat(items) => MonoExprKind::Concat(self.subst_exprs(items, subst)),
            HirExprKind::ToString(inner) => {
                MonoExprKind::ToString(Box::new(self.subst_expr(inner, subst)))
            }
            HirExprKind::Unary { op, expr } => MonoExprKind::Unary {
                op: *op,
                expr: Box::new(self.subst_expr(expr, subst)),
            },
            HirExprKind::Binary { op, lhs, rhs } => MonoExprKind::Binary {
                op: *op,
                lhs: Box::new(self.subst_expr(lhs, subst)),
                rhs: Box::new(self.subst_expr(rhs, subst)),
            },
            HirExprKind::Assign { target, value } => MonoExprKind::Assign {
                target: Box::new(self.subst_expr(target, subst)),
                value: Box::new(self.subst_expr(value, subst)),
            },
            HirExprKind::Call { callee, args } => MonoExprKind::Call {
                callee: Box::new(self.subst_expr(callee, subst)),
                args: self.subst_exprs(args, subst),
            },
            HirExprKind::CallStatic {
                fn_id,
                generic_args,
                args,
            } => {
                let args = self.subst_exprs(args, subst);
                let generic_args: Vec<Type> = generic_args
                    .iter()
                    .map(|ty| subst_type(ty, subst))
                    .collect();
                let mono_id = self.resolve_call(*fn_id, &args, &generic_args);
                MonoExprKind::CallStatic {
                    fn_id: mono_id,
                    args,
                }
            }
            HirExprKind::CallBuiltin { name, args } => MonoExprKind::CallBuiltin {
                name: name.clone(),
                args: self.subst_exprs(args, subst),
            },
            HirExprKind::Field { base, index } => MonoExprKind::Field {
                base: Box::new(self.subst_expr(base, subst)),
                index: *index,
            },
            HirExprKind::Index { base, index } => MonoExprKind::Index {
                base: Box::new(self.subst_expr(base, subst)),
                index: Box::new(self.subst_expr(index, subst)),
            },
            HirExprKind::Construct { ty, fields } => MonoExprKind::Construct {
                ty: *ty,
                fields: self.subst_exprs(fields, subst),
            },
            HirExprKind::ConstructVariant {
                enum_id,
                variant,
                payload,
            } => MonoExprKind::ConstructVariant {
                enum_id: *enum_id,
                variant: *variant,
                payload: self.subst_exprs(payload, subst),
            },
            HirExprKind::CallGenericMethod {
                receiver,
                method_name,
                is_static,
                generic_args,
                args,
                ..
            } => {
                let receiver = self.subst_expr(receiver, subst);
                let args = self.subst_exprs(args, subst);
                let generic_args: Vec<Type> = generic_args
                    .iter()
                    .map(|ty| subst_type(ty, subst))
                    .collect();
                self.resolve_generic_method_call(
                    receiver,
                    method_name,
                    *is_static,
                    args,
                    &generic_args,
                    ty.clone(),
                )
            }
            HirExprKind::CallMethod {
                receiver,
                method_name,
                generic_args,
                args,
            } => {
                let receiver = self.subst_expr(receiver, subst);
                let args = self.subst_exprs(args, subst);
                let generic_args: Vec<Type> = generic_args
                    .iter()
                    .map(|ty| subst_type(ty, subst))
                    .collect();
                self.resolve_generic_method_call(
                    receiver,
                    method_name,
                    false,
                    args,
                    &generic_args,
                    ty.clone(),
                )
            }
            HirExprKind::CallArrayMethod {
                receiver,
                method,
                args,
            } => MonoExprKind::CallArrayMethod {
                receiver: Box::new(self.subst_expr(receiver, subst)),
                method: method.clone(),
                args: self.subst_exprs(args, subst),
            },
            HirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => MonoExprKind::If {
                cond: Box::new(self.subst_expr(cond, subst)),
                then_branch: Box::new(self.subst_expr(then_branch, subst)),
                else_branch: else_branch
                    .as_ref()
                    .map(|e| Box::new(self.subst_expr(e, subst))),
            },
            HirExprKind::Match { scrutinee, arms } => MonoExprKind::Match {
                scrutinee: Box::new(self.subst_expr(scrutinee, subst)),
                arms: arms
                    .iter()
                    .map(|arm| MonoMatchArm {
                        pattern: arm.pattern.clone(),
                        body: self.subst_expr(&arm.body, subst),
                    })
                    .collect(),
            },
            HirExprKind::Block(stmts, tail) => {
                let stmts = stmts
                    .iter()
                    .map(|s| MonoStmt {
                        kind: match &s.kind {
                            HirStmtKind::Let { local, ty, value } => MonoStmtKind::Let {
                                local: *local,
                                ty: subst_type(ty, subst),
                                value: self.subst_expr(value, subst),
                            },
                            HirStmtKind::Expr(e) => MonoStmtKind::Expr(self.subst_expr(e, subst)),
                        },
                    })
                    .collect();
                MonoExprKind::Block(
                    stmts,
                    tail.as_ref().map(|t| Box::new(self.subst_expr(t, subst))),
                )
            }
            HirExprKind::While { cond, body } => MonoExprKind::While {
                cond: Box::new(self.subst_expr(cond, subst)),
                body: Box::new(self.subst_expr(body, subst)),
            },
            HirExprKind::Loop { body } => MonoExprKind::Loop {
                body: Box::new(self.subst_expr(body, subst)),
            },
            HirExprKind::Break(v) => {
                MonoExprKind::Break(v.as_ref().map(|e| Box::new(self.subst_expr(e, subst))))
            }
            HirExprKind::Continue => MonoExprKind::Continue,
            HirExprKind::Return(v) => {
                MonoExprKind::Return(v.as_ref().map(|e| Box::new(self.subst_expr(e, subst))))
            }
            HirExprKind::Closure {
                params,
                captures,
                body,
            } => {
                let function = self.instantiate_closure(params, captures, body, &ty, subst);
                let captures = captures
                    .iter()
                    .map(|capture| MonoExpr {
                        kind: MonoExprKind::Local(capture.local),
                        ty: subst_type(&capture.ty, subst),
                    })
                    .collect();
                MonoExprKind::Closure { function, captures }
            }
        };
        MonoExpr { kind, ty }
    }

    fn instantiate_closure(
        &mut self,
        params: &[HirParam],
        captures: &[HirCapture],
        body: &HirExpr,
        closure_ty: &Type,
        subst: &HashMap<Symbol, Type>,
    ) -> MonoFnId {
        let id = MonoFnId(self.functions.len() as u32);
        let (param_tys, ret) = match closure_ty {
            Type::Function(params, ret) => (params.clone(), (**ret).clone()),
            _ => (Vec::new(), Type::Error),
        };
        let mono_params = params
            .iter()
            .zip(param_tys)
            .map(|(param, ty)| MonoParam {
                local: param.local,
                name: param.name.clone(),
                mutable: param.mutable,
                ty,
            })
            .collect();
        let mono_captures = captures
            .iter()
            .map(|capture| MonoCapture {
                local: capture.local,
                ty: subst_type(&capture.ty, subst),
            })
            .collect();
        self.functions.push(MonoFunction {
            id,
            name: Symbol::new(&format!("closure_{}", id.0)),
            owner: None,
            is_closure: true,
            self_param: None,
            self_ty: None,
            self_local: None,
            captures: mono_captures,
            params: mono_params,
            ret: ret.clone(),
            body: MonoExpr {
                kind: MonoExprKind::Unit,
                ty: ret,
            },
        });
        let mono_body = self.subst_expr(body, subst);
        self.functions[id.0 as usize].body = mono_body;
        id
    }

    fn instantiate_adapter(&mut self, target: MonoFnId) -> MonoFnId {
        if let Some(&adapter) = self.adapters.get(&target) {
            return adapter;
        }
        let target_fn = &self.functions[target.0 as usize];
        let params = target_fn.params.clone();
        let ret = target_fn.ret.clone();
        let args = params
            .iter()
            .map(|param| MonoExpr {
                kind: MonoExprKind::Local(param.local),
                ty: param.ty.clone(),
            })
            .collect();
        let id = MonoFnId(self.functions.len() as u32);
        self.functions.push(MonoFunction {
            id,
            name: Symbol::new(&format!("fn_adapter_{}", target.0)),
            owner: None,
            is_closure: true,
            self_param: None,
            self_ty: None,
            self_local: None,
            captures: Vec::new(),
            params,
            ret: ret.clone(),
            body: MonoExpr {
                kind: MonoExprKind::CallStatic {
                    fn_id: target,
                    args,
                },
                ty: ret,
            },
        });
        self.adapters.insert(target, id);
        id
    }

    fn subst_exprs(&mut self, exprs: &[HirExpr], subst: &HashMap<Symbol, Type>) -> Vec<MonoExpr> {
        exprs.iter().map(|e| self.subst_expr(e, subst)).collect()
    }

    /// `receiver`/`args` are already substituted; only the target method
    /// still needs resolving now that `receiver.ty` is concrete.
    fn resolve_generic_method_call(
        &mut self,
        receiver: MonoExpr,
        method_name: &Symbol,
        is_static: bool,
        args: Vec<MonoExpr>,
        generic_args: &[Type],
        result_ty: Type,
    ) -> MonoExprKind {
        if !is_static && method_name.as_str() == "into_string" && args.is_empty() {
            match &receiver.ty {
                Type::Primitive(_) => return MonoExprKind::ToString(Box::new(receiver)),
                Type::String => return receiver.kind,
                _ => {}
            }
        }
        let owner = owner_def_id(&receiver.ty, self.hir.array_owner).unwrap_or_else(|| {
            panic!("monomorphization: generic method call's receiver substituted to non-nominal type {:?} (see this crate's module docs)", receiver.ty)
        });
        // `receiver.ty` is already fully substituted here (the caller
        // always passes an already-`subst_expr`'d receiver) — its owner
        // arguments are the exact-match key an `impl Owner<ConcreteArgs>`
        // specialization was registered under, so this is the one place
        // that override actually takes effect, including for a call site
        // written inside another still-generic function: *that* function
        // only reaches here once monomorphized for one concrete
        // instantiation, at which point `receiver.ty` is concrete too
        // (`nether_hir::MethodFnSet::for_args`; mirrors `nether_typecheck`
        // picking the same override at typecheck time whenever the
        // receiver was already concrete there too).
        let receiver_owner_args: &[Type] = match &receiver.ty {
            Type::Struct(_, args) | Type::TupleStruct(_, args) | Type::Enum(_, args) => args,
            Type::Array(elem) => std::slice::from_ref(elem.as_ref()),
            _ => &[],
        };
        let target_hir_id = self
            .hir
            .methods
            .get(&(owner, method_name.clone()))
            .and_then(|set| set.for_args(receiver_owner_args))
            .unwrap_or_else(|| panic!("monomorphization: no impl of method `{method_name}` found for the substituted receiver type (typecheck should have rejected this earlier)"));
        let mut target_generic_args = receiver_owner_args.to_vec();
        target_generic_args.extend_from_slice(generic_args);
        let mut full_args = Vec::with_capacity(args.len() + usize::from(!is_static));
        if !is_static {
            full_args.push(receiver.clone());
        }
        full_args.extend(args);
        let mono_id = self.resolve_call(target_hir_id, &full_args, &target_generic_args);
        let call = MonoExprKind::CallStatic {
            fn_id: mono_id,
            args: full_args,
        };
        if is_static {
            MonoExprKind::Block(
                vec![MonoStmt {
                    kind: MonoStmtKind::Expr(receiver),
                }],
                Some(Box::new(MonoExpr {
                    kind: call,
                    ty: result_ty,
                })),
            )
        } else {
            call
        }
    }
}

fn placeholder_for(hir_fn: &HirFunction) -> MonoFunction {
    MonoFunction {
        id: MonoFnId(0),
        name: hir_fn.name.clone(),
        owner: hir_fn.owner,
        is_closure: false,
        self_param: hir_fn.self_param,
        self_ty: hir_fn.self_ty.clone(),
        self_local: hir_fn.self_local,
        captures: Vec::new(),
        params: Vec::new(),
        ret: Type::Error,
        body: MonoExpr {
            kind: MonoExprKind::Unit,
            ty: Type::unit(),
        },
    }
}

/// `Struct`/`TupleStruct`/`Enum`/`Array` are the only `Type` variants that
/// can own an `impl` block (language-spec §7/§8) — everything else
/// (primitives, tuples, strings, functions, interfaces-as-bounds) never
/// reaches here for a well-typed program, since `typecheck` only ever
/// produces a `CallGenericMethod`/`CallMethod` when the bound check
/// succeeded against a declared `impl`. Unlike the other three, a
/// `Type::Array(_)` carries no `DefId` of its own — `array_owner` (looked
/// up once in `hir::lower` and threaded through `HirModule`) supplies it.
fn owner_def_id(ty: &Type, array_owner: Option<DefId>) -> Option<DefId> {
    match ty {
        Type::Struct(id, _) | Type::TupleStruct(id, _) => Some(*id),
        Type::Enum(id, _) => Some(*id),
        Type::Array(_) => array_owner,
        _ => None,
    }
}

fn subst_type(ty: &Type, subst: &HashMap<Symbol, Type>) -> Type {
    match ty {
        Type::Generic(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Struct(id, args) => {
            Type::Struct(*id, args.iter().map(|t| subst_type(t, subst)).collect())
        }
        Type::TupleStruct(id, args) => {
            Type::TupleStruct(*id, args.iter().map(|t| subst_type(t, subst)).collect())
        }
        Type::Tuple(items) => Type::Tuple(items.iter().map(|t| subst_type(t, subst)).collect()),
        Type::Enum(id, args) => {
            Type::Enum(*id, args.iter().map(|t| subst_type(t, subst)).collect())
        }
        Type::Array(elem) => Type::Array(Box::new(subst_type(elem, subst))),
        Type::Function(params, ret) => Type::Function(
            params.iter().map(|t| subst_type(t, subst)).collect(),
            Box::new(subst_type(ret, subst)),
        ),
        Type::Weak(inner) => Type::Weak(Box::new(subst_type(inner, subst))),
        Type::Primitive(_) | Type::String | Type::Interface(_) | Type::Never | Type::Error => {
            ty.clone()
        }
    }
}

/// Walks `declared` (a generic function's param type, possibly containing
/// `Type::Generic`) alongside `concrete` (the same param's actual argument
/// type at a call site, already fully substituted) and records every
/// `Type::Generic(name) -> concrete-subtree` binding found. Mirrors
/// `subst_type`'s own recursion shape so every position a substitution
/// could be written back into is also a position this can read one out of.
fn collect_generic_bindings(declared: &Type, concrete: &Type, out: &mut HashMap<Symbol, Type>) {
    match declared {
        Type::Generic(name) => {
            out.entry(name.clone()).or_insert_with(|| concrete.clone());
        }
        Type::Tuple(items) => {
            if let Type::Tuple(concrete_items) = concrete {
                for (d, c) in items.iter().zip(concrete_items) {
                    collect_generic_bindings(d, c, out);
                }
            }
        }
        Type::Enum(_, args) => {
            if let Type::Enum(_, concrete_args) = concrete {
                for (d, c) in args.iter().zip(concrete_args) {
                    collect_generic_bindings(d, c, out);
                }
            }
        }
        Type::Struct(_, args) => {
            if let Type::Struct(_, concrete_args) = concrete {
                for (d, c) in args.iter().zip(concrete_args) {
                    collect_generic_bindings(d, c, out);
                }
            }
        }
        Type::TupleStruct(_, args) => {
            if let Type::TupleStruct(_, concrete_args) = concrete {
                for (d, c) in args.iter().zip(concrete_args) {
                    collect_generic_bindings(d, c, out);
                }
            }
        }
        Type::Array(elem) => {
            if let Type::Array(concrete_elem) = concrete {
                collect_generic_bindings(elem, concrete_elem, out);
            }
        }
        Type::Function(params, ret) => {
            if let Type::Function(concrete_params, concrete_ret) = concrete {
                for (d, c) in params.iter().zip(concrete_params) {
                    collect_generic_bindings(d, c, out);
                }
                collect_generic_bindings(ret, concrete_ret, out);
            }
        }
        Type::Weak(inner) => {
            if let Type::Weak(concrete_inner) = concrete {
                collect_generic_bindings(inner, concrete_inner, out);
            }
        }
        Type::Primitive(_) | Type::String | Type::Interface(_) | Type::Never | Type::Error => {}
    }
}
