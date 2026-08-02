use super::*;

pub(super) fn closure_captures(body: &HirExpr, params: &[HirParam]) -> Vec<HirCapture> {
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

pub(super) fn collect_closure_locals(
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
        } => {
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
        | HirExprKind::CallArrayMethod { receiver, args, .. }
        | HirExprKind::CallMethod { receiver, args, .. } => {
            collect_closure_locals(receiver, used, bound);
            for arg in args {
                collect_closure_locals(arg, used, bound);
            }
        }
        HirExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
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
                used.entry(capture.local)
                    .or_insert_with(|| capture.ty.clone());
            }
        }
        HirExprKind::Literal(_)
        | HirExprKind::FnRef(_)
        | HirExprKind::Unit
        | HirExprKind::Continue => {}
    }
}

pub(super) fn collect_pattern_locals(pattern: &HirPattern, bound: &mut HashSet<HirLocalId>) {
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
