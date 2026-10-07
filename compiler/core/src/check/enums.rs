use super::*;
impl Checker<'_> {
    pub(super) fn enum_pattern(
        &self,
        ty: &Type,
        path: &[String],
        span: Span,
    ) -> Result<Option<(Type, usize, h::Variant)>> {
        let Type::Enum { name, variants, .. } = ty else {
            return Ok(None);
        };
        if path.len() != 2 || self.nominals.origin(name) != path[0] && *name != path[0] {
            return Ok(None);
        }
        let index = variants
            .iter()
            .position(|v| v.name == path[1])
            .ok_or_else(|| error(span, "E0360", "unknown enum variant"))?;
        Ok(Some((ty.clone(), index, variants[index].clone())))
    }
    fn enum_variant(
        &self,
        path: &[String],
        arguments: &[a::TypeArgument],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Option<(Type, usize, h::Variant)>> {
        if path.len() != 2 || !self.nominals.is_enum(&path[0]) {
            return Ok(None);
        }
        let ty = if arguments.is_empty() {
            if let Some(Type::Enum { name, .. }) = expected {
                if self.nominals.origin(name) == path[0] {
                    expected.unwrap().clone()
                } else {
                    self.nominals
                        .resolve(&path[0], arguments, self.constants, span)?
                }
            } else {
                self.nominals
                    .resolve(&path[0], arguments, self.constants, span)?
            }
        } else {
            self.nominals
                .resolve(&path[0], arguments, self.constants, span)?
        };
        self.enum_pattern(&ty, path, span)
    }
    pub(super) fn enum_unit(
        &self,
        base: &a::Expr,
        name: &str,
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Option<h::Expr>> {
        let Some((base, arguments)) = type_expression(base) else {
            return Ok(None);
        };
        if self.lookup(base).is_some() {
            return Ok(None);
        }
        let Some((ty, variant, definition)) =
            self.enum_variant(&[base.to_owned(), name.into()], arguments, expected, span)?
        else {
            return Ok(None);
        };
        if definition.shape != h::VariantShape::Unit {
            return Err(error(
                span,
                "E0360",
                "variant requires payload construction",
            ));
        }
        if let Some(expected) = expected {
            same(expected, &ty, span)?;
        }
        Ok(Some(h::Expr {
            kind: h::ExprKind::Enum {
                variant,
                fields: vec![],
            },
            ty,
            span,
        }))
    }
    pub(super) fn enum_call(
        &mut self,
        callee: &a::Expr,
        args: &[a::Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Option<h::Expr>> {
        let a::ExprKind::Member { value, name } = &callee.kind else {
            return Ok(None);
        };
        let Some((base, arguments)) = type_expression(value) else {
            return Ok(None);
        };
        if self.lookup(base).is_some() {
            return Ok(None);
        }
        let Some((ty, variant, definition)) =
            self.enum_variant(&[base.to_owned(), name.clone()], arguments, expected, span)?
        else {
            return Ok(None);
        };
        if definition.shape != h::VariantShape::Tuple || args.len() != definition.fields.len() {
            return Err(error(
                span,
                "E0360",
                "variant payload shape or argument count mismatch",
            ));
        }
        if let Some(expected) = expected {
            same(expected, &ty, span)?;
        }
        let mut fields = Vec::new();
        for (arg, ty) in args.iter().zip(&definition.fields) {
            fields.push(self.expr(arg, Some(ty), true)?);
        }
        Ok(Some(h::Expr {
            kind: h::ExprKind::Enum { variant, fields },
            ty,
            span,
        }))
    }
    pub(super) fn enum_named(
        &mut self,
        source_ty: &a::Type,
        args: &[(String, a::Expr)],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<Option<h::Expr>> {
        let a::TypeKind::Named { path, arguments } = &source_ty.kind else {
            return Ok(None);
        };
        let Some((ty, variant, definition)) = self.enum_variant(path, arguments, expected, span)?
        else {
            return Ok(None);
        };
        let h::VariantShape::Named(names) = definition.shape else {
            return Err(error(span, "E0360", "variant does not have named fields"));
        };
        if let Some(expected) = expected {
            same(expected, &ty, span)?;
        }
        let mut fields = vec![None; names.len()];
        let mut statements = Vec::new();
        for (name, arg) in args {
            let index = names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| error(arg.span, "E0360", "unknown variant field"))?;
            if fields[index].is_some() {
                return Err(error(arg.span, "E0360", "duplicate variant field"));
            }
            let value = self.expr(arg, Some(&definition.fields[index]), true)?;
            let id = self.locals.len();
            self.locals.push(h::Local {
                view: None,
                name: format!("<variant {name}>"),
                ty: value.ty.clone(),
                mutable: false,
                span: arg.span,
            });
            fields[index] = Some(h::Expr {
                kind: h::ExprKind::Local(id),
                ty: value.ty.clone(),
                span: arg.span,
            });
            statements.push(h::Statement::Initialize(id, value));
        }
        if fields.iter().any(Option::is_none) {
            return Err(error(
                span,
                "E0360",
                "every variant field must be initialized",
            ));
        }
        statements.push(h::Statement::Evaluate(h::Expr {
            kind: h::ExprKind::Enum {
                variant,
                fields: fields.into_iter().map(Option::unwrap).collect(),
            },
            ty: ty.clone(),
            span,
        }));
        Ok(Some(h::Expr {
            kind: h::ExprKind::Block(h::Block {
                storage: vec![],
                statements,
                result: ty.clone(),
                diverges: false,
                span,
            }),
            ty,
            span,
        }))
    }
}

fn type_expression(expr: &a::Expr) -> Option<(&str, &[a::TypeArgument])> {
    match &expr.kind {
        a::ExprKind::Name(name) => Some((name, &[])),
        a::ExprKind::Specialize { value, arguments } => {
            let a::ExprKind::Name(name) = &value.kind else {
                return None;
            };
            Some((name, arguments))
        }
        _ => None,
    }
}
