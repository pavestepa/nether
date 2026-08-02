use super::*;

pub(super) fn escaping_local(op: &Operand) -> Option<Local> {
    match op {
        Operand::Local(l) => Some(*l),
        _ => None,
    }
}

pub(super) fn pattern_has_bindings(pattern: &HirPattern) -> bool {
    match pattern {
        HirPattern::Binding(_) => true,
        HirPattern::Tuple(items) => items.iter().any(pattern_has_bindings),
        HirPattern::Variant { payload, .. } => payload.iter().any(pattern_has_bindings),
        HirPattern::Wildcard | HirPattern::Literal(_) => false,
    }
}

/// Whether `expr` is, syntactically, nothing more than a bare name for an
/// *already-bound* local, as opposed to an expression that computes or
/// reads a value of its own. Deliberately does *not* look through a
/// `Block`'s tail even when that tail is itself a bare name: a `Block`
/// always credits its own escaping value correctly on its own
/// (`FnBuilder::lower_block` calls [`FnBuilder::lower_escaping_value`] internally,
/// which already applies this exact same reasoning) — treating the whole
/// `Block` as "still a trivial alias" too would double the retain that
/// already happened inside it. See [`FnBuilder::prepare_new_binding`] for
/// why this distinction is what decides whether binding `expr`'s value to
/// a second name needs a fresh `Retain`.
pub(super) fn is_trivial_local_alias(expr: &MonoExpr) -> bool {
    matches!(expr.kind, MonoExprKind::Local(_))
}
