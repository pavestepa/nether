use std::collections::{HashMap, HashSet};

use nether_ast::{
    BinaryOp, Block, Expr, ExprKind, FieldAccessor, FnDecl, Ident, InterfaceDecl, Item, Literal,
    MatchArm, Module, NodeId, Param, Path, Pattern, Stmt, Symbol, TemplatePart, TypeExpr,
};
use nether_resolver::{DefId, LocalId as ResolverLocalId, Resolution, ResolvedNames};
use nether_typecheck::{
    FnSig, GenericBound, PrimitiveKind, Signatures, Type, TypeShape, TypedTables,
};

use crate::node::{
    HirCapture, HirExpr, HirExprKind, HirFnId, HirFunction, HirLocalId, HirMatchArm, HirModule, HirParam,
    HirPattern, HirStmt, HirStmtKind,
};

/// Lowers a fully resolved, fully type-checked [`Module`] into a
/// [`HirModule`]. See this crate's module docs for the desugaring rules
/// applied and the documented simplifications.
///
/// Consumes `tables` (rather than borrowing it) because
/// [`nether_typecheck::Signatures`] moves straight into the resulting
/// [`HirModule`] unchanged — it is already exactly the structural,
/// desugared type information `monomorphization`/`mir` need, so this
/// crate carries it forward rather than re-deriving or re-wrapping it.
pub fn lower(module: &Module, resolved: &ResolvedNames, tables: TypedTables) -> HirModule {
    let TypedTables { expr_types, local_types, signatures } = tables;

    // `resolver::LocalId` -> `Type`, built once by cross-referencing
    // `resolved.locals` (binding site -> LocalId) against `local_types`
    // (binding site -> Type) — see `nether_typecheck::TypedTables::local_types`
    // docs for why this indirection exists.
    let local_types_by_id: HashMap<ResolverLocalId, Type> = resolved
        .locals
        .iter()
        .filter_map(|(node_id, local_id)| local_types.get(node_id).map(|ty| (*local_id, ty.clone())))
        .collect();

    let interface_decls = index_interfaces(module, resolved);
    let pending = collect_pending_fns(module, resolved, &signatures, &interface_decls);

    let mut fn_by_name = HashMap::new();
    let mut fn_by_def = HashMap::new();
    let mut methods = HashMap::new();
    for (i, p) in pending.iter().enumerate() {
        let id = HirFnId(i as u32);
        match p.owner {
            None => {
                fn_by_name.insert(p.name.clone(), id);
                if let Some(def_id) = p.def_id {
                    fn_by_def.insert(def_id, id);
                }
            }
            Some(owner) => {
                methods.insert((owner, p.name.clone()), id);
            }
        }
    }

    let mut fns = Vec::with_capacity(pending.len());
    for (i, p) in pending.iter().enumerate() {
        let id = HirFnId(i as u32);
        let mut lowerer = Lowerer {
            resolved,
            expr_types: &expr_types,
            local_types_by_id: &local_types_by_id,
            sigs: &signatures,
            fn_by_def: &fn_by_def,
            methods: &methods,
            locals_map: HashMap::new(),
            next_local: 0,
            generics: p.sig.generics.iter().cloned().collect(),
            type_subst: p.type_subst.clone(),
            self_override: None,
        };
        fns.push(lowerer.lower_fn(p, id));
    }

    HirModule {
        signatures,
        fns,
        fn_by_name,
        fn_by_def,
        methods,
    }
}

fn index_interfaces<'a>(module: &'a Module, resolved: &ResolvedNames) -> HashMap<DefId, &'a InterfaceDecl> {
    let mut map = HashMap::new();
    for item in &module.items {
        if let Item::Interface(i) = item {
            if let Some(id) = resolved
                .definitions
                .lookup_in(i.span.file, &i.name.name)
            {
                map.insert(id, i);
            }
        }
    }
    map
}

fn type_expr_def_id(ty: &TypeExpr, resolved: &ResolvedNames) -> Option<DefId> {
    if let TypeExpr::Named { path, .. } = ty {
        if let Some(res) = resolved.path_res.get(&path.id) {
            if let Resolution::Def(id) = res.base {
                return Some(id);
            }
        }
    }
    None
}

fn owner_type(owner: DefId, sigs: &Signatures) -> Type {
    if let Some(sig) = sigs.enum_sigs.get(&owner) {
        Type::Enum(
            owner,
            sig.generics
                .iter()
                .cloned()
                .map(Type::Generic)
                .collect(),
        )
    } else if matches!(sigs.type_shapes.get(&owner), Some(TypeShape::TupleStruct(_))) {
        Type::TupleStruct(
            owner,
            sigs.type_generics
                .get(&owner)
                .into_iter()
                .flatten()
                .cloned()
                .map(Type::Generic)
                .collect(),
        )
    } else {
        Type::Struct(
            owner,
            sigs.type_generics
                .get(&owner)
                .into_iter()
                .flatten()
                .cloned()
                .map(Type::Generic)
                .collect(),
        )
    }
}

fn subst_type(ty: &Type, subst: &HashMap<Symbol, Type>) -> Type {
    match ty {
        Type::Generic(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Struct(id, args) => {
            Type::Struct(*id, args.iter().map(|arg| subst_type(arg, subst)).collect())
        }
        Type::TupleStruct(id, args) => {
            Type::TupleStruct(*id, args.iter().map(|arg| subst_type(arg, subst)).collect())
        }
        Type::Tuple(items) => {
            Type::Tuple(items.iter().map(|item| subst_type(item, subst)).collect())
        }
        Type::Enum(id, args) => {
            Type::Enum(*id, args.iter().map(|arg| subst_type(arg, subst)).collect())
        }
        Type::Array(inner) => Type::Array(Box::new(subst_type(inner, subst))),
        Type::Function(params, ret) => Type::Function(
            params
                .iter()
                .map(|param| subst_type(param, subst))
                .collect(),
            Box::new(subst_type(ret, subst)),
        ),
        Type::Weak(inner) => Type::Weak(Box::new(subst_type(inner, subst))),
        _ => ty.clone(),
    }
}

/// One function/method still to be lowered, with its already-built
/// [`FnSig`] and a reference to the AST node supplying its body — which,
/// for an inherited interface default (not overridden by an `impl`), is
/// the interface's own [`FnDecl`], not anything in the `impl` block.
struct PendingFn<'a> {
    name: Symbol,
    def_id: Option<DefId>,
    owner: Option<DefId>,
    decl: &'a FnDecl,
    sig: FnSig,
    type_subst: HashMap<Symbol, Type>,
}

fn collect_pending_fns<'a>(
    module: &'a Module,
    resolved: &ResolvedNames,
    sigs: &Signatures,
    interface_decls: &HashMap<DefId, &'a InterfaceDecl>,
) -> Vec<PendingFn<'a>> {
    let mut pending = Vec::new();
    for item in &module.items {
        match item {
            Item::Fn(f) => {
                if let Some(id) = resolved
                    .definitions
                    .lookup_in(f.span.file, &f.name.name)
                {
                    if let Some(sig) = sigs.fns.get(&id).cloned() {
                        pending.push(PendingFn {
                            name: f.name.name.clone(),
                            def_id: Some(id),
                            owner: None,
                            decl: f,
                            sig,
                            type_subst: HashMap::new(),
                        });
                    }
                }
            }
            Item::Impl(b) => {
                let Some(owner) = resolved
                    .definitions
                    .lookup_in(b.span.file, &b.target.name)
                else { continue };
                for m in &b.methods {
                    if let Some(sig) = sigs.method(owner, &m.name.name).cloned() {
                        pending.push(PendingFn {
                            name: m.name.name.clone(),
                            def_id: None,
                            owner: Some(owner),
                            decl: m,
                            sig,
                            type_subst: HashMap::new(),
                        });
                    }
                }
                let Some(iface_ty) = &b.interface else { continue };
                let Some(iface_id) = type_expr_def_id(iface_ty, resolved) else { continue };
                let Some(iface_decl) = interface_decls.get(&iface_id) else { continue };
                for m in &iface_decl.methods {
                    if m.body.is_none() {
                        continue;
                    }
                    if b.methods.iter().any(|om| om.name.name == m.name.name) {
                        continue;
                    }
                    if let Some(sig) = sigs.method(owner, &m.name.name).cloned() {
                        let type_subst = sigs
                            .default_method_substitutions
                            .get(&(owner, m.name.name.clone()))
                            .cloned()
                            .unwrap_or_default();
                        pending.push(PendingFn {
                            name: m.name.name.clone(),
                            def_id: None,
                            owner: Some(owner),
                            decl: m,
                            sig,
                            type_subst,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    pending
}

fn local_ref(local: HirLocalId, ty: Type) -> HirExpr {
    HirExpr { kind: HirExprKind::Local(local), ty }
}

struct Lowerer<'a> {
    resolved: &'a ResolvedNames,
    expr_types: &'a HashMap<NodeId, Type>,
    local_types_by_id: &'a HashMap<ResolverLocalId, Type>,
    sigs: &'a Signatures,
    fn_by_def: &'a HashMap<DefId, HirFnId>,
    methods: &'a HashMap<(DefId, Symbol), HirFnId>,
    locals_map: HashMap<ResolverLocalId, HirLocalId>,
    next_local: u32,
    generics: HashMap<Symbol, Option<GenericBound>>,
    type_subst: HashMap<Symbol, Type>,
    self_override: Option<(ResolverLocalId, Type)>,
}

impl Lowerer<'_> {
    fn fresh_local(&mut self) -> HirLocalId {
        let id = HirLocalId(self.next_local);
        self.next_local += 1;
        id
    }

    fn local_for(&mut self, orig: ResolverLocalId) -> HirLocalId {
        if let Some(id) = self.locals_map.get(&orig) {
            return *id;
        }
        let id = self.fresh_local();
        self.locals_map.insert(orig, id);
        id
    }

    fn ty_of(&self, node_id: NodeId) -> Type {
        let ty = self.expr_types.get(&node_id).cloned().unwrap_or(Type::Error);
        subst_type(&ty, &self.type_subst)
    }

    fn local_ty(&self, orig: ResolverLocalId) -> Type {
        if let Some((self_local, ty)) = &self.self_override {
            if *self_local == orig {
                return ty.clone();
            }
        }
        let ty = self.local_types_by_id.get(&orig).cloned().unwrap_or(Type::Error);
        subst_type(&ty, &self.type_subst)
    }

    fn lower_fn(&mut self, p: &PendingFn, id: HirFnId) -> HirFunction {
        self.locals_map.clear();
        self.next_local = 0;
        self.type_subst.clone_from(&p.type_subst);
        self.self_override = self
            .resolved
            .locals
            .get(&p.decl.id)
            .copied()
            .zip(p.owner.map(|owner| owner_type(owner, self.sigs)));

        let mut params = Vec::new();
        for (param_ast, param_sig) in p.decl.params.iter().zip(&p.sig.params) {
            let local = match self.resolved.locals.get(&param_ast.id) {
                Some(orig) => self.local_for(*orig),
                None => self.fresh_local(),
            };
            params.push(HirParam { local, name: param_sig.name.clone(), mutable: param_sig.mutable, ty: param_sig.ty.clone() });
        }
        // `self` isn't in `params` (matching FnSig's own self/params
        // split) but still needs its translated id reserved up front so
        // body references resolve consistently.
        let self_local = self.resolved.locals.get(&p.decl.id).map(|orig| self.local_for(*orig));

        let body =
            p.decl.body.as_ref().expect("standalone fns, impl methods, and inherited interface defaults always have a body");
        let body_hir = self.lower_block_as_expr(body);

        HirFunction {
            id,
            name: p.name.clone(),
            owner: p.owner,
            self_param: p.decl.self_param,
            self_ty: p.owner.map(|owner| owner_type(owner, self.sigs)),
            self_local,
            generics: p.sig.generics.clone(),
            params,
            ret: p.sig.ret.clone(),
            body: body_hir,
        }
    }

    fn lower_block_as_expr(&mut self, block: &Block) -> HirExpr {
        let ty = block.tail.as_ref().map(|t| self.ty_of(t.id)).unwrap_or_else(Type::unit);
        let (stmts, tail) = self.lower_block(block);
        HirExpr { kind: HirExprKind::Block(stmts, tail), ty }
    }

    fn lower_block(&mut self, block: &Block) -> (Vec<HirStmt>, Option<Box<HirExpr>>) {
        let mut stmts = Vec::new();
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let(let_stmt) => {
                    let value = self.lower_expr(&let_stmt.value);
                    let (local, ty) = match self.resolved.locals.get(&let_stmt.id) {
                        Some(orig) => (self.local_for(*orig), self.local_ty(*orig)),
                        None => (self.fresh_local(), value.ty.clone()),
                    };
                    stmts.push(HirStmt { kind: HirStmtKind::Let { local, ty, value } });
                }
                Stmt::Expr(e) => stmts.push(HirStmt { kind: HirStmtKind::Expr(self.lower_expr(e)) }),
            }
        }
        let tail = block.tail.as_ref().map(|t| Box::new(self.lower_expr(t)));
        (stmts, tail)
    }

    fn lower_expr(&mut self, expr: &Expr) -> HirExpr {
        let ty = self.ty_of(expr.id);
        match &expr.kind {
            ExprKind::Literal(lit) => HirExpr { kind: HirExprKind::Literal(lit.clone()), ty },
            ExprKind::Path(path) => self.lower_value_path(path, None, ty),
            ExprKind::Tuple(elems) => {
                HirExpr { kind: HirExprKind::Tuple(elems.iter().map(|e| self.lower_expr(e)).collect()), ty }
            }
            ExprKind::Array(elems) => {
                HirExpr { kind: HirExprKind::Array(elems.iter().map(|e| self.lower_expr(e)).collect()), ty }
            }
            ExprKind::StringTemplate(parts) => self.lower_template(parts, ty),
            ExprKind::Unary { op, expr: inner } => {
                HirExpr { kind: HirExprKind::Unary { op: *op, expr: Box::new(self.lower_expr(inner)) }, ty }
            }
            ExprKind::Binary { op, lhs, rhs } => HirExpr {
                kind: HirExprKind::Binary { op: *op, lhs: Box::new(self.lower_expr(lhs)), rhs: Box::new(self.lower_expr(rhs)) },
                ty,
            },
            ExprKind::Assign { target, value } => HirExpr {
                kind: HirExprKind::Assign { target: Box::new(self.lower_assign_target(target)), value: Box::new(self.lower_expr(value)) },
                ty,
            },
            ExprKind::Call { callee, args } => self.lower_call(callee, args, ty),
            ExprKind::MutArg(inner) => self.lower_expr(inner),
            ExprKind::MethodCall { receiver, method, args } => self.lower_method_call(receiver, method, args, ty),
            ExprKind::Field { base, field } => self.lower_field(base, field, ty),
            ExprKind::Index { base, index } => HirExpr {
                kind: HirExprKind::Index { base: Box::new(self.lower_expr(base)), index: Box::new(self.lower_expr(index)) },
                ty,
            },
            ExprKind::If { cond, then_branch, else_branch } => HirExpr {
                kind: HirExprKind::If {
                    cond: Box::new(self.lower_expr(cond)),
                    then_branch: Box::new(self.lower_block_as_expr(then_branch)),
                    else_branch: else_branch.as_ref().map(|e| Box::new(self.lower_expr(e))),
                },
                ty,
            },
            ExprKind::Match { scrutinee, arms } => HirExpr {
                kind: HirExprKind::Match {
                    scrutinee: Box::new(self.lower_expr(scrutinee)),
                    arms: arms.iter().map(|a| self.lower_match_arm(a)).collect(),
                },
                ty,
            },
            ExprKind::Block(block) => self.lower_block_as_expr(block),
            ExprKind::While { cond, body } => HirExpr {
                kind: HirExprKind::While { cond: Box::new(self.lower_expr(cond)), body: Box::new(self.lower_block_as_expr(body)) },
                ty,
            },
            ExprKind::ForIn { pattern, iter, body } => self.lower_for_in(pattern, iter, body),
            ExprKind::Loop { body } => HirExpr { kind: HirExprKind::Loop { body: Box::new(self.lower_block_as_expr(body)) }, ty },
            ExprKind::Break(value) => {
                HirExpr { kind: HirExprKind::Break(value.as_ref().map(|v| Box::new(self.lower_expr(v)))), ty }
            }
            ExprKind::Continue => HirExpr { kind: HirExprKind::Continue, ty },
            ExprKind::Return(value) => {
                HirExpr { kind: HirExprKind::Return(value.as_ref().map(|v| Box::new(self.lower_expr(v)))), ty }
            }
            ExprKind::Closure { params, body } => self.lower_closure(params, body, ty),
            ExprKind::StructLit { path, fields } => self.lower_struct_lit(path, fields, ty),
        }
    }

    fn lower_args(&mut self, args: &[Expr]) -> Vec<HirExpr> {
        args.iter()
            .map(|a| match &a.kind {
                ExprKind::MutArg(inner) => self.lower_expr(inner),
                _ => self.lower_expr(a),
            })
            .collect()
    }

    /// Mirrors `nether_typecheck`'s `check_value_path`: walks
    /// `resolver`'s [`Resolution`] for the path's base, then any leftover
    /// segments as a field/method-call chain (language-spec §10), except
    /// this builds [`HirExpr`] nodes instead of computing a [`Type`].
    fn lower_value_path(&mut self, path: &Path, call_args: Option<&[Expr]>, result_ty: Type) -> HirExpr {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return HirExpr { kind: HirExprKind::Unit, ty: result_ty };
        };
        let total = path.segments.len();
        let direct_call_args = if res.consumed == total { call_args } else { None };

        let (mut current, mut current_ty) = match res.base {
            Resolution::Local(id) => {
                let ty = self.local_ty(id);
                let local = self.local_for(id);
                let expr = local_ref(local, ty.clone());
                if res.consumed == total {
                    if let Some(args) = call_args {
                        return self.lower_call_value(expr, args, result_ty);
                    }
                }
                let upgraded_ty = self.upgrade_weak(ty.clone());
                let kind = self.weak_upgrade_kind(expr, &ty);
                (HirExpr { kind, ty: upgraded_ty.clone() }, upgraded_ty)
            }
            Resolution::Def(id) => self.lower_def_value(id, direct_call_args, &result_ty),
            Resolution::EnumVariant(enum_id, idx) => self.lower_enum_variant_value(enum_id, idx, direct_call_args, &result_ty),
            Resolution::StaticMember(owner_id, idx) => self.lower_static_member(owner_id, idx, direct_call_args, &result_ty),
            Resolution::GenericParam | Resolution::Error => (HirExpr { kind: HirExprKind::Unit, ty: Type::Error }, Type::Error),
        };

        for i in res.consumed..total {
            let seg = &path.segments[i];
            let is_last = i + 1 == total;
            if is_last {
                if let Some(args) = call_args {
                    return self.lower_method_call_on(current, &current_ty, seg, args, &result_ty);
                }
            }
            let (next, next_ty) = self.lower_field_access_named(current, &current_ty, seg);
            current = next;
            current_ty = next_ty;
        }
        current
    }

    fn lower_def_value(&mut self, id: DefId, call_args: Option<&[Expr]>, result_ty: &Type) -> (HirExpr, Type) {
        let def = self.resolved.definitions.get(id);
        match def.kind {
            nether_resolver::DefKind::Fn => {
                let name = def.name.clone();
                if name.as_str() == "println" || name.as_str() == "print" {
                    let args = self
                        .lower_args(call_args.unwrap_or(&[]))
                        .into_iter()
                        .map(|arg| self.into_string_expr(arg))
                        .collect();
                    let expr = HirExpr { kind: HirExprKind::CallBuiltin { name, args }, ty: result_ty.clone() };
                    return (expr, result_ty.clone());
                }
                let fn_id = self.fn_by_def.get(&id).copied();
                match (fn_id, call_args) {
                    (Some(fid), Some(args)) => {
                        let args = self.lower_args(args);
                        let expr = HirExpr { kind: HirExprKind::CallStatic { fn_id: fid, args }, ty: result_ty.clone() };
                        (expr, result_ty.clone())
                    }
                    (Some(fid), None) => {
                        let expr = HirExpr { kind: HirExprKind::FnRef(fid), ty: result_ty.clone() };
                        (expr, result_ty.clone())
                    }
                    (None, _) => (HirExpr { kind: HirExprKind::Unit, ty: Type::Error }, Type::Error),
                }
            }
            nether_resolver::DefKind::Type => {
                let fields = match call_args {
                    Some(args) => self.lower_args(args),
                    None => Vec::new(),
                };
                let expr = HirExpr { kind: HirExprKind::Construct { ty: id, fields }, ty: result_ty.clone() };
                (expr, result_ty.clone())
            }
            _ => (HirExpr { kind: HirExprKind::Unit, ty: Type::Error }, Type::Error),
        }
    }

    fn lower_enum_variant_value(&mut self, enum_id: DefId, idx: u32, call_args: Option<&[Expr]>, result_ty: &Type) -> (HirExpr, Type) {
        let has_payload = self
            .sigs
            .enum_sigs
            .get(&enum_id)
            .and_then(|s| s.variants.get(idx as usize))
            .map(|(_, p)| !p.is_empty())
            .unwrap_or(false);
        let payload = if has_payload { self.lower_args(call_args.unwrap_or(&[])) } else { Vec::new() };
        let expr = HirExpr { kind: HirExprKind::ConstructVariant { enum_id, variant: idx, payload }, ty: result_ty.clone() };
        (expr, result_ty.clone())
    }

    fn lower_static_member(&mut self, owner_id: DefId, idx: u32, call_args: Option<&[Expr]>, result_ty: &Type) -> (HirExpr, Type) {
        let Some(name) = self.resolved.definitions.get(owner_id).methods.get(idx as usize).cloned() else {
            return (HirExpr { kind: HirExprKind::Unit, ty: Type::Error }, Type::Error);
        };
        let Some(fn_id) = self.methods.get(&(owner_id, name)).copied() else {
            return (HirExpr { kind: HirExprKind::Unit, ty: Type::Error }, Type::Error);
        };
        match call_args {
            Some(args) => {
                let args = self.lower_args(args);
                let expr = HirExpr { kind: HirExprKind::CallStatic { fn_id, args }, ty: result_ty.clone() };
                (expr, result_ty.clone())
            }
            None => {
                let expr = HirExpr { kind: HirExprKind::FnRef(fn_id), ty: result_ty.clone() };
                (expr, result_ty.clone())
            }
        }
    }

    fn lower_method_call_on(&mut self, receiver: HirExpr, receiver_ty: &Type, method: &Ident, args: &[Expr], result_ty: &Type) -> HirExpr {
        if let Type::Array(_) = receiver_ty {
            let lowered = self.lower_args(args);
            return HirExpr {
                kind: HirExprKind::CallArrayMethod { receiver: Box::new(receiver), method: method.name.clone(), args: lowered },
                ty: result_ty.clone(),
            };
        }
        if let Type::Generic(name) = receiver_ty {
            let bound = self.generics.get(name).cloned().flatten();
            let lowered = self.lower_args(args);
            return match bound {
                Some(bound) => HirExpr {
                    kind: HirExprKind::CallGenericMethod {
                        receiver: Box::new(receiver),
                        bound_interface: bound.interface,
                        method_name: method.name.clone(),
                        args: lowered,
                    },
                    ty: result_ty.clone(),
                },
                None => HirExpr { kind: HirExprKind::Unit, ty: Type::Error },
            };
        }
        let owner_id = match receiver_ty {
            Type::Struct(id, _) | Type::TupleStruct(id, _) => Some(*id),
            Type::Enum(id, _) => Some(*id),
            _ => None,
        };
        let fn_id = owner_id.and_then(|id| self.methods.get(&(id, method.name.clone())).copied());
        let mut lowered = self.lower_args(args);
        match fn_id {
            Some(fid) => {
                let mut call_args = Vec::with_capacity(lowered.len() + 1);
                call_args.push(receiver);
                call_args.append(&mut lowered);
                HirExpr { kind: HirExprKind::CallStatic { fn_id: fid, args: call_args }, ty: result_ty.clone() }
            }
            None => HirExpr { kind: HirExprKind::Unit, ty: Type::Error },
        }
    }

    fn lower_field_access_named(&mut self, base: HirExpr, base_ty: &Type, ident: &Ident) -> (HirExpr, Type) {
        let (raw, field_ty) = self.raw_field_access(base, base_ty, ident);
        if field_ty.is_error() {
            return (raw, field_ty);
        }
        let upgraded_ty = self.upgrade_weak(field_ty.clone());
        let kind = self.weak_upgrade_kind(raw, &field_ty);
        (HirExpr { kind, ty: upgraded_ty.clone() }, upgraded_ty)
    }

    /// Reading a `weak T`-typed place *as a value* never yields a bare
    /// `weak T` — see `nether_typecheck::check::TypeChecker::upgrade_weak`
    /// (this is the same rule, one stage later: that decided the *type*
    /// every such read has; this decides the *shape* — wrapping the raw
    /// place-read in a call to the `__weak_upgrade` builtin, which
    /// `nether_mir`/`nether_codegen` lower into a real
    /// `nether_rt_arc_weak_upgrade` call producing the `Option<T>` this
    /// expression's type promises).
    fn upgrade_weak(&self, ty: Type) -> Type {
        match ty {
            Type::Weak(inner) => match self.resolved.definitions.lookup(&Symbol::new("Option")) {
                Some(option_id) => Type::Enum(option_id, vec![*inner]),
                None => Type::Error,
            },
            other => other,
        }
    }

    /// `expr`'s own kind, wrapped in a `__weak_upgrade` builtin call if
    /// `raw_ty` (its *un-upgraded* declared type) is `weak T` — see
    /// [`Self::upgrade_weak`].
    fn weak_upgrade_kind(&self, expr: HirExpr, raw_ty: &Type) -> HirExprKind {
        if matches!(raw_ty, Type::Weak(_)) {
            HirExprKind::CallBuiltin { name: Symbol::new("__weak_upgrade"), args: vec![expr] }
        } else {
            expr.kind
        }
    }

    fn lower_call(&mut self, callee: &Expr, args: &[Expr], result_ty: Type) -> HirExpr {
        if let ExprKind::Path(path) = &callee.kind {
            self.lower_value_path(path, Some(args), result_ty)
        } else {
            let callee_hir = self.lower_expr(callee);
            self.lower_call_value(callee_hir, args, result_ty)
        }
    }

    fn lower_call_value(&mut self, callee: HirExpr, args: &[Expr], result_ty: Type) -> HirExpr {
        let args = self.lower_args(args);
        HirExpr { kind: HirExprKind::Call { callee: Box::new(callee), args }, ty: result_ty }
    }

    fn lower_method_call(&mut self, receiver: &Expr, method: &Ident, args: &[Expr], result_ty: Type) -> HirExpr {
        let receiver_hir = self.lower_expr(receiver);
        let receiver_ty = receiver_hir.ty.clone();
        self.lower_method_call_on(receiver_hir, &receiver_ty, method, args, &result_ty)
    }

    fn lower_field(&mut self, base: &Expr, field: &FieldAccessor, result_ty: Type) -> HirExpr {
        let base_hir = self.lower_expr(base);
        match field {
            FieldAccessor::Named(ident) => {
                let base_ty = base_hir.ty.clone();
                let (expr, _) = self.lower_field_access_named(base_hir, &base_ty, ident);
                HirExpr { kind: expr.kind, ty: result_ty }
            }
            FieldAccessor::Index(idx, _) => {
                HirExpr { kind: HirExprKind::Field { base: Box::new(base_hir), index: *idx }, ty: result_ty }
            }
        }
    }

    /// Lowers an assignment's target place — deliberately bypasses the
    /// `weak T` → `Option<T>` upgrade a normal read applies
    /// (`Self::weak_upgrade_kind`): the target must stay a plain,
    /// assignable place (`nether_mir::build::FnBuilder::lower_place`'s own
    /// invariant — it panics on anything else), carrying its *real*
    /// `weak T` type, not an upgrade-call expression, which produces a
    /// value, not a location. Mirrors `nether_typecheck::check::TypeChecker::
    /// check_assign_target_type`'s own doc for why this needs to exist at
    /// all.
    fn lower_assign_target(&mut self, target: &Expr) -> HirExpr {
        let ty = self.ty_of(target.id);
        match &target.kind {
            // `self.field`/`x.field` is one multi-segment `Path` node, not
            // `ExprKind::Field` — see `nether_typecheck::check::TypeChecker::
            // check_assign_target_type`'s own matching note.
            ExprKind::Path(path) => self.lower_assign_path_target(path, ty),
            ExprKind::Field { base, field: FieldAccessor::Named(ident) } => {
                let base_hir = self.lower_expr(base);
                let base_ty = base_hir.ty.clone();
                let (expr, _) = self.raw_field_access(base_hir, &base_ty, ident);
                HirExpr { kind: expr.kind, ty }
            }
            _ => self.lower_expr(target),
        }
    }

    /// [`Self::lower_assign_target`]'s handling for a `Path` target —
    /// mirrors [`Self::lower_value_path`]'s own segment-walking loop, but
    /// the *last* segment uses the raw, non-upgrading
    /// [`Self::raw_field_access`] rather than
    /// [`Self::lower_field_access_named`]; every earlier segment is an
    /// ordinary read on the way there, so it upgrades as normal.
    fn lower_assign_path_target(&mut self, path: &Path, ty: Type) -> HirExpr {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return HirExpr { kind: HirExprKind::Unit, ty: Type::Error };
        };
        let total = path.segments.len();
        let (mut current, mut current_ty) = match res.base {
            Resolution::Local(id) => {
                let local_ty = self.local_ty(id);
                (local_ref(self.local_for(id), local_ty.clone()), local_ty)
            }
            _ => return self.lower_value_path(path, None, ty),
        };
        for i in res.consumed..total {
            let seg = &path.segments[i];
            if i + 1 == total {
                let (expr, _) = self.raw_field_access(current, &current_ty, seg);
                return HirExpr { kind: expr.kind, ty };
            }
            let (next, next_ty) = self.lower_field_access_named(current, &current_ty, seg);
            current = next;
            current_ty = next_ty;
        }
        current
    }

    /// The raw, un-upgraded field projection — shared by
    /// [`Self::lower_assign_target`]/[`Self::lower_assign_path_target`]
    /// (an assignment target's final segment, which must stay a plain
    /// place) and [`Self::lower_field_access_named`] (which wraps this in
    /// the `weak`-upgrade check for a normal read).
    fn raw_field_access(&mut self, base: HirExpr, base_ty: &Type, ident: &Ident) -> (HirExpr, Type) {
        if let Type::Struct(_, _) = base_ty {
            if let Some(fields) = self.sigs.named_type_fields(base_ty) {
                if let Some(idx) = fields.iter().position(|(n, _)| n == &ident.name) {
                    let field_ty = fields[idx].1.clone();
                    let expr = HirExpr { kind: HirExprKind::Field { base: Box::new(base), index: idx as u32 }, ty: field_ty.clone() };
                    return (expr, field_ty);
                }
            }
        }
        (HirExpr { kind: HirExprKind::Unit, ty: Type::Error }, Type::Error)
    }

    fn lower_struct_lit(&mut self, path: &Path, fields: &[(Ident, Expr)], result_ty: Type) -> HirExpr {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return HirExpr { kind: HirExprKind::Unit, ty: Type::Error };
        };
        let Resolution::Def(id) = res.base else {
            return HirExpr { kind: HirExprKind::Unit, ty: Type::Error };
        };
        let decl_fields = match self.sigs.type_shapes.get(&id) {
            Some(TypeShape::Struct(f)) => f.clone(),
            _ => Vec::new(),
        };
        let mut by_name: HashMap<Symbol, &Expr> = HashMap::new();
        for (name, value) in fields {
            by_name.insert(name.name.clone(), value);
        }
        let ordered: Vec<HirExpr> = decl_fields
            .iter()
            .map(|(name, _)| match by_name.get(name) {
                Some(e) => self.lower_expr(e),
                None => HirExpr { kind: HirExprKind::Unit, ty: Type::Error },
            })
            .collect();
        HirExpr { kind: HirExprKind::Construct { ty: id, fields: ordered }, ty: result_ty }
    }

    fn lower_pattern(&mut self, pattern: &Pattern) -> HirPattern {
        match pattern {
            Pattern::Wildcard(_) => HirPattern::Wildcard,
            Pattern::Binding(id, _ident) => {
                // Per `resolver`, a bare identifier pattern is either a
                // fresh binding or (if it uniquely names an enum variant)
                // a variant match — see
                // `nether_resolver`'s bare-pattern disambiguation.
                if let Some(orig) = self.resolved.locals.get(id) {
                    HirPattern::Binding(self.local_for(*orig))
                } else if let Some(Resolution::EnumVariant(enum_id, idx)) =
                    self.resolved.path_res.get(id).map(|r| r.base)
                {
                    HirPattern::Variant { enum_id, variant: idx, payload: Vec::new() }
                } else {
                    HirPattern::Wildcard
                }
            }
            Pattern::Literal(lit, _) => HirPattern::Literal(lit.clone()),
            Pattern::Tuple(elems, _) => HirPattern::Tuple(elems.iter().map(|p| self.lower_pattern(p)).collect()),
            Pattern::Variant { path, payload, .. } => {
                if let Some(Resolution::EnumVariant(enum_id, idx)) = self.resolved.path_res.get(&path.id).map(|r| r.base) {
                    HirPattern::Variant {
                        enum_id,
                        variant: idx,
                        payload: payload.iter().map(|p| self.lower_pattern(p)).collect(),
                    }
                } else {
                    HirPattern::Wildcard
                }
            }
        }
    }

    fn lower_match_arm(&mut self, arm: &MatchArm) -> HirMatchArm {
        HirMatchArm { pattern: self.lower_pattern(&arm.pattern), body: self.lower_expr(&arm.body) }
    }

    fn lower_template(&mut self, parts: &[TemplatePart], result_ty: Type) -> HirExpr {
        let mut pieces = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                TemplatePart::Literal(s) => {
                    pieces.push(HirExpr { kind: HirExprKind::Literal(Literal::Str(s.clone())), ty: Type::String });
                }
                TemplatePart::Expr(e) => {
                    let hir = self.lower_expr(e);
                    pieces.push(self.into_string_expr(hir));
                }
            }
        }
        HirExpr { kind: HirExprKind::Concat(pieces), ty: result_ty }
    }

    fn into_string_expr(&self, expr: HirExpr) -> HirExpr {
        match &expr.ty {
            Type::String => expr,
            Type::Primitive(_) => HirExpr {
                ty: Type::String,
                kind: HirExprKind::ToString(Box::new(expr)),
            },
            Type::Struct(owner, _) | Type::TupleStruct(owner, _) | Type::Enum(owner, _) => {
                match self.methods.get(&(*owner, Symbol::new("into_string"))).copied() {
                    Some(fn_id) => HirExpr {
                        ty: Type::String,
                        kind: HirExprKind::CallStatic { fn_id, args: vec![expr] },
                    },
                    None => HirExpr { ty: Type::Error, kind: HirExprKind::Unit },
                }
            }
            Type::Generic(name) => match self.generics.get(name).cloned().flatten() {
                Some(bound) => HirExpr {
                    ty: Type::String,
                    kind: HirExprKind::CallGenericMethod {
                        receiver: Box::new(expr),
                        bound_interface: bound.interface,
                        method_name: Symbol::new("into_string"),
                        args: Vec::new(),
                    },
                },
                None => HirExpr { ty: Type::Error, kind: HirExprKind::Unit },
            },
            _ => HirExpr { ty: Type::Error, kind: HirExprKind::Unit },
        }
    }

    /// Desugars `for pattern in iter { body }` into an index-based `while`
    /// loop over a synthesized index/length pair, matching the pattern
    /// against each element via a single-arm `Match` (which reuses the
    /// same pattern-binding machinery as a real `match`, so an arbitrarily
    /// complex `for`-pattern, not just a bare binding, is handled for
    /// free). This is the "underlying iteration primitive" the
    /// architecture docs call for — there is no `ForIn` node left in HIR.
    fn lower_for_in(&mut self, pattern: &Pattern, iter: &Expr, body: &Block) -> HirExpr {
        let iter_hir = self.lower_expr(iter);
        let iter_ty = iter_hir.ty.clone();
        let elem_ty = match &iter_ty {
            Type::Array(inner) => (**inner).clone(),
            _ => Type::Error,
        };

        let iter_local = self.fresh_local();
        let idx_local = self.fresh_local();
        let elem_local = self.fresh_local();

        let iter_let = HirStmt {
            kind: HirStmtKind::Let { local: iter_local, ty: iter_ty.clone(), value: iter_hir },
        };
        let idx_init = HirExpr { kind: HirExprKind::Literal(Literal::Int(0)), ty: Type::Primitive(PrimitiveKind::Usize) };
        let idx_let = HirStmt {
            kind: HirStmtKind::Let {
                local: idx_local,
                ty: Type::Primitive(PrimitiveKind::Usize),
                value: idx_init,
            },
        };

        let len_call = HirExpr {
            kind: HirExprKind::CallArrayMethod {
                receiver: Box::new(local_ref(iter_local, iter_ty.clone())),
                method: Symbol::new("len"),
                args: Vec::new(),
            },
            ty: Type::Primitive(PrimitiveKind::Usize),
        };
        let cond = HirExpr {
            kind: HirExprKind::Binary {
                op: BinaryOp::Lt,
                lhs: Box::new(local_ref(idx_local, Type::Primitive(PrimitiveKind::Usize))),
                rhs: Box::new(len_call),
            },
            ty: Type::Primitive(PrimitiveKind::Bool),
        };

        let index_expr = HirExpr {
            kind: HirExprKind::Index {
                base: Box::new(local_ref(iter_local, iter_ty.clone())),
                index: Box::new(local_ref(idx_local, Type::Primitive(PrimitiveKind::Usize))),
            },
            ty: elem_ty.clone(),
        };
        let elem_let = HirStmt {
            kind: HirStmtKind::Let { local: elem_local, ty: elem_ty.clone(), value: index_expr },
        };

        let hir_pattern = self.lower_pattern(pattern);
        let (body_stmts, body_tail) = self.lower_block(body);
        let mut arm_stmts = body_stmts;
        if let Some(tail) = body_tail {
            arm_stmts.push(HirStmt { kind: HirStmtKind::Expr(*tail) });
        }
        let arm_body = HirExpr { kind: HirExprKind::Block(arm_stmts, None), ty: Type::unit() };
        let match_expr = HirExpr {
            kind: HirExprKind::Match {
                scrutinee: Box::new(local_ref(elem_local, elem_ty)),
                arms: vec![HirMatchArm { pattern: hir_pattern, body: arm_body }],
            },
            ty: Type::unit(),
        };

        let incremented = HirExpr {
            kind: HirExprKind::Binary {
                op: BinaryOp::Add,
                lhs: Box::new(local_ref(idx_local, Type::Primitive(PrimitiveKind::Usize))),
                rhs: Box::new(HirExpr { kind: HirExprKind::Literal(Literal::Int(1)), ty: Type::Primitive(PrimitiveKind::Usize) }),
            },
            ty: Type::Primitive(PrimitiveKind::Usize),
        };
        let increment_stmt = HirStmt {
            kind: HirStmtKind::Expr(HirExpr {
                kind: HirExprKind::Assign {
                    target: Box::new(local_ref(idx_local, Type::Primitive(PrimitiveKind::Usize))),
                    value: Box::new(incremented),
                },
                ty: Type::unit(),
            }),
        };

        let while_body = HirExpr {
            kind: HirExprKind::Block(vec![elem_let, HirStmt { kind: HirStmtKind::Expr(match_expr) }, increment_stmt], None),
            ty: Type::unit(),
        };
        let while_expr = HirExpr { kind: HirExprKind::While { cond: Box::new(cond), body: Box::new(while_body) }, ty: Type::unit() };

        HirExpr { kind: HirExprKind::Block(vec![iter_let, idx_let, HirStmt { kind: HirStmtKind::Expr(while_expr) }], None), ty: Type::unit() }
    }

    fn lower_closure(&mut self, params: &[Param], body: &Expr, result_ty: Type) -> HirExpr {
        let mut hir_params = Vec::new();
        for p in params {
            let (local, ty) = match self.resolved.locals.get(&p.id) {
                Some(orig) => (self.local_for(*orig), self.local_ty(*orig)),
                None => (self.fresh_local(), Type::Error),
            };
            hir_params.push(HirParam { local, name: p.name.name.clone(), mutable: p.mutable, ty });
        }
        let body_hir = self.lower_expr(body);
        let captures = closure_captures(&body_hir, &hir_params);
        HirExpr { kind: HirExprKind::Closure { params: hir_params, captures, body: Box::new(body_hir) }, ty: result_ty }
    }
}

fn closure_captures(body: &HirExpr, params: &[HirParam]) -> Vec<HirCapture> {
    let mut used = HashMap::new();
    let mut bound: HashSet<HirLocalId> = params.iter().map(|param| param.local).collect();
    collect_closure_locals(body, &mut used, &mut bound);
    let mut captures: Vec<_> = used
        .into_iter()
        .filter(|(local, _)| !bound.contains(local))
        .map(|(local, ty)| HirCapture { local, ty })
        .collect();
    captures.sort_by_key(|capture| capture.local.0);
    captures
}

fn collect_closure_locals(
    expr: &HirExpr,
    used: &mut HashMap<HirLocalId, Type>,
    bound: &mut HashSet<HirLocalId>,
) {
    match &expr.kind {
        HirExprKind::Local(local) => {
            used.entry(*local).or_insert_with(|| expr.ty.clone());
        }
        HirExprKind::Tuple(items) | HirExprKind::Array(items) | HirExprKind::Concat(items) => {
            for item in items {
                collect_closure_locals(item, used, bound);
            }
        }
        HirExprKind::ToString(inner)
        | HirExprKind::Unary { expr: inner, .. }
        | HirExprKind::Field { base: inner, .. }
        | HirExprKind::Loop { body: inner } => collect_closure_locals(inner, used, bound),
        HirExprKind::Binary { lhs, rhs, .. }
        | HirExprKind::Assign { target: lhs, value: rhs }
        | HirExprKind::Index { base: lhs, index: rhs }
        | HirExprKind::While { cond: lhs, body: rhs } => {
            collect_closure_locals(lhs, used, bound);
            collect_closure_locals(rhs, used, bound);
        }
        HirExprKind::Call { callee, args } => {
            collect_closure_locals(callee, used, bound);
            for arg in args {
                collect_closure_locals(arg, used, bound);
            }
        }
        HirExprKind::CallStatic { args, .. }
        | HirExprKind::CallBuiltin { args, .. }
        | HirExprKind::Construct { fields: args, .. }
        | HirExprKind::ConstructVariant { payload: args, .. } => {
            for arg in args {
                collect_closure_locals(arg, used, bound);
            }
        }
        HirExprKind::CallGenericMethod { receiver, args, .. }
        | HirExprKind::CallArrayMethod { receiver, args, .. } => {
            collect_closure_locals(receiver, used, bound);
            for arg in args {
                collect_closure_locals(arg, used, bound);
            }
        }
        HirExprKind::If { cond, then_branch, else_branch } => {
            collect_closure_locals(cond, used, bound);
            collect_closure_locals(then_branch, used, bound);
            if let Some(branch) = else_branch {
                collect_closure_locals(branch, used, bound);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            collect_closure_locals(scrutinee, used, bound);
            for arm in arms {
                collect_pattern_locals(&arm.pattern, bound);
                collect_closure_locals(&arm.body, used, bound);
            }
        }
        HirExprKind::Block(stmts, tail) => {
            for stmt in stmts {
                match &stmt.kind {
                    HirStmtKind::Let { local, value, .. } => {
                        collect_closure_locals(value, used, bound);
                        bound.insert(*local);
                    }
                    HirStmtKind::Expr(expr) => collect_closure_locals(expr, used, bound),
                }
            }
            if let Some(tail) = tail {
                collect_closure_locals(tail, used, bound);
            }
        }
        HirExprKind::Break(value) | HirExprKind::Return(value) => {
            if let Some(value) = value {
                collect_closure_locals(value, used, bound);
            }
        }
        // Creating a nested closure uses its captured outer locals, but
        // the nested closure's own body/parameters are a separate scope.
        HirExprKind::Closure { captures, .. } => {
            for capture in captures {
                used.entry(capture.local).or_insert_with(|| capture.ty.clone());
            }
        }
        HirExprKind::Literal(_) | HirExprKind::FnRef(_) | HirExprKind::Unit | HirExprKind::Continue => {}
    }
}

fn collect_pattern_locals(pattern: &HirPattern, bound: &mut HashSet<HirLocalId>) {
    match pattern {
        HirPattern::Binding(local) => {
            bound.insert(*local);
        }
        HirPattern::Tuple(items) | HirPattern::Variant { payload: items, .. } => {
            for item in items {
                collect_pattern_locals(item, bound);
            }
        }
        HirPattern::Wildcard | HirPattern::Literal(_) => {}
    }
}
