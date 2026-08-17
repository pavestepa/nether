use super::*;

impl Lowerer<'_> {
    pub(super) fn lower_struct_lit(
        &mut self,
        path: &Path,
        fields: &[(Ident, Expr)],
        result_ty: Type,
    ) -> HirExpr {
        let Some(res) = self.resolved.path_res.get(&path.id).cloned() else {
            return HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Error,
            };
        };
        let Resolution::Def(id) = res.base else {
            return HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Error,
            };
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
                None => HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Error,
                },
            })
            .collect();
        HirExpr {
            kind: HirExprKind::Construct {
                ty: id,
                fields: ordered,
            },
            ty: result_ty,
        }
    }

    pub(super) fn lower_pattern(&mut self, pattern: &Pattern) -> HirPattern {
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
                    HirPattern::Variant {
                        enum_id,
                        variant: idx,
                        payload: Vec::new(),
                    }
                } else {
                    HirPattern::Wildcard
                }
            }
            Pattern::Literal(lit, _) => HirPattern::Literal(lit.clone()),
            Pattern::Tuple(elems, _) => {
                HirPattern::Tuple(elems.iter().map(|p| self.lower_pattern(p)).collect())
            }
            Pattern::Variant { path, payload, .. } => {
                if let Some(Resolution::EnumVariant(enum_id, idx)) =
                    self.resolved.path_res.get(&path.id).map(|r| r.base)
                {
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

    pub(super) fn lower_match_arm(&mut self, arm: &MatchArm) -> HirMatchArm {
        HirMatchArm {
            pattern: self.lower_pattern(&arm.pattern),
            body: self.lower_expr(&arm.body),
        }
    }

    pub(super) fn lower_template(&mut self, parts: &[TemplatePart], result_ty: Type) -> HirExpr {
        let mut pieces = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                TemplatePart::Literal(s) => {
                    pieces.push(HirExpr {
                        kind: HirExprKind::Literal(Literal::Str(s.clone())),
                        ty: Type::String,
                    });
                }
                TemplatePart::Expr(e) => {
                    let hir = self.lower_expr(e);
                    pieces.push(self.convert_to_string(hir));
                }
            }
        }
        HirExpr {
            kind: HirExprKind::Concat(pieces),
            ty: result_ty,
        }
    }

    pub(super) fn convert_to_string(&self, expr: HirExpr) -> HirExpr {
        match &expr.ty {
            Type::String => expr,
            Type::Primitive(_) => HirExpr {
                ty: Type::String,
                kind: HirExprKind::ToString(Box::new(expr)),
            },
            Type::Struct(owner, _) | Type::TupleStruct(owner, _) | Type::Enum(owner, _) => {
                // `into_string` is always `Into<String>`'s trait
                // method — never specialized (traits are rejected on
                // a concrete-specialization `impl` block), so `.generic`
                // alone is authoritative.
                // `Into<String>` is always an ARC-domain (`self`) method in
                // every current usage — owned-domain `Into<String>` isn't
                // part of the language yet.
                match self
                    .methods
                    .get(&(*owner, Symbol::new("into_string"), ReceiverDomain::Arc))
                    .and_then(|set| set.generic)
                {
                    Some(fn_id) => HirExpr {
                        ty: Type::String,
                        kind: HirExprKind::CallStatic {
                            fn_id,
                            generic_args: Vec::new(),
                            args: vec![expr],
                        },
                    },
                    None => HirExpr {
                        ty: Type::Error,
                        kind: HirExprKind::Unit,
                    },
                }
            }
            Type::Generic(name) => match self.generics.get(name).cloned().flatten() {
                Some(bound) => HirExpr {
                    ty: Type::String,
                    kind: HirExprKind::CallGenericMethod {
                        receiver: Box::new(expr),
                        bound_trait: bound.trait_id,
                        method_name: Symbol::new("into_string"),
                        is_static: false,
                        // `Into<String>` is always ARC-domain today, same
                        // reasoning as the concrete-owner case above.
                        domain: ReceiverDomain::Arc,
                        generic_args: Vec::new(),
                        args: Vec::new(),
                    },
                },
                None => HirExpr {
                    ty: Type::Error,
                    kind: HirExprKind::Unit,
                },
            },
            _ => HirExpr {
                ty: Type::Error,
                kind: HirExprKind::Unit,
            },
        }
    }
}
