use super::*;
use crate::mir::loans::Access;
/// Explicit views of local inline values. Borrowing is represented in HIR/MIR;
/// no copy of the source aggregate is inserted.
impl Checker<'_> {
    pub(super) fn view_binding(
        &mut self,
        pattern: &a::Pattern,
        source: &a::Expr,
        annotation: Option<&a::Type>,
    ) -> Result<h::Statement> {
        let expected = annotation
            .map(|t| resolve_type_with(t, self.constants, self.nominals))
            .transpose()?;
        let value = self.expr(source, expected.as_ref(), true)?;
        if !place(&value) {
            return Err(error(
                source.span,
                "E0382",
                "explicit ref requires an existing local place",
            ));
        }
        if !only_refs(pattern) {
            return Err(error(
                pattern.span,
                "E0900",
                "mixed consuming/ref patterns require ownership extraction lowering",
            ));
        }
        let access = if mutable_ref(pattern) {
            Access::Mutable
        } else {
            Access::Readonly
        };
        if access == Access::Mutable {
            self.mutable_place(&value)?;
        }
        let id = self.locals.len();
        self.locals.push(h::Local {
            name: "<pattern view>".into(),
            ty: value.ty.clone(),
            mutable: false,
            view: Some(access),
            span: source.span,
        });
        let reference = h::Expr {
            kind: h::ExprKind::View(id),
            ty: value.ty.clone(),
            span: source.span,
        };
        let mut bindings = vec![h::Statement::Evaluate(h::Expr {
            kind: h::ExprKind::Borrow {
                local: id,
                place: Box::new(value),
            },
            ty: Type::Unit,
            span: source.span,
        })];
        self.scopes.push(BTreeMap::new());
        let (_, irrefutable, _) = self.pattern(pattern, &reference, &mut bindings)?;
        if !irrefutable {
            return Err(error(
                pattern.span,
                "E0344",
                "binding requires an irrefutable pattern",
            ));
        }
        let names = self.scopes.pop().unwrap();
        self.scopes.last_mut().unwrap().extend(names);
        Ok(h::Statement::Evaluate(h::Expr {
            kind: h::ExprKind::Block(h::Block {
                storage: vec![],
                statements: bindings,
                result: Type::Unit,
                diverges: false,
                span: pattern.span,
            }),
            ty: Type::Unit,
            span: pattern.span,
        }))
    }
}
pub(super) fn place(value: &h::Expr) -> bool {
    match &value.kind {
        h::ExprKind::Local(_) | h::ExprKind::View(_) => true,
        h::ExprKind::Index(base, _)
        | h::ExprKind::Field(base, _)
        | h::ExprKind::Payload(base, _) => place(base),
        _ => false,
    }
}
pub(super) fn view_root(value: &h::Expr) -> bool {
    match &value.kind {
        h::ExprKind::View(_) => true,
        h::ExprKind::Index(base, _)
        | h::ExprKind::Field(base, _)
        | h::ExprKind::Payload(base, _) => view_root(base),
        _ => false,
    }
}
fn children(pattern: &a::Pattern) -> Vec<&a::Pattern> {
    match &pattern.kind {
        a::PatternKind::Tuple(v) | a::PatternKind::Array(v) => v.iter().collect(),
        a::PatternKind::Variant { fields, .. } => match fields {
            a::PatternFields::Tuple(v) => v.iter().collect(),
            a::PatternFields::Named(v, _) => v.iter().map(|(_, p)| p).collect(),
            _ => vec![],
        },
        _ => vec![],
    }
}
pub(super) fn has_ref(pattern: &a::Pattern) -> bool {
    matches!(pattern.kind, a::PatternKind::Binding { by_ref: true, .. })
        || children(pattern).iter().any(|p| has_ref(p))
}
fn mutable_ref(pattern: &a::Pattern) -> bool {
    matches!(
        pattern.kind,
        a::PatternKind::Binding {
            by_ref: true,
            mutable: true,
            ..
        }
    ) || children(pattern).iter().any(|p| mutable_ref(p))
}
fn only_refs(pattern: &a::Pattern) -> bool {
    !matches!(pattern.kind, a::PatternKind::Binding { by_ref: false, .. })
        && children(pattern).iter().all(|p| only_refs(p))
}

pub(super) fn inferred_pattern(pattern: &a::Pattern, mutable: bool) -> a::Pattern {
    let mut result = pattern.clone();
    match &mut result.kind {
        a::PatternKind::Binding {
            by_ref,
            mutable: access,
            ..
        } => {
            *by_ref = true;
            *access |= mutable;
        }
        a::PatternKind::Tuple(fields) | a::PatternKind::Array(fields) => {
            for field in fields {
                *field = inferred_pattern(field, mutable);
            }
        }
        a::PatternKind::Variant { fields, .. } => match fields {
            a::PatternFields::Tuple(fields) => {
                for field in fields {
                    *field = inferred_pattern(field, mutable);
                }
            }
            a::PatternFields::Named(fields, _) => {
                for (_, field) in fields {
                    *field = inferred_pattern(field, mutable);
                }
            }
            _ => (),
        },
        _ => (),
    }
    result
}

impl Checker<'_> {
    pub(super) fn freeze_place(
        &mut self,
        mut value: h::Expr,
        prelude: &mut Vec<h::Statement>,
    ) -> h::Expr {
        match &mut value.kind {
            h::ExprKind::Index(base, index) => {
                **base = self.freeze_place((**base).clone(), prelude);
                let id = self.locals.len();
                self.locals.push(h::Local {
                    name: "<pattern index>".into(),
                    ty: index.ty.clone(),
                    mutable: false,
                    view: None,
                    span: index.span,
                });
                prelude.push(h::Statement::Initialize(id, (**index).clone()));
                index.kind = h::ExprKind::Local(id);
            }
            h::ExprKind::Field(base, _) | h::ExprKind::Payload(base, _) => {
                **base = self.freeze_place((**base).clone(), prelude)
            }
            _ => (),
        }
        value
    }
}
