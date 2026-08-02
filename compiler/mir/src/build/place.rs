use super::*;

impl FnBuilder<'_> {
    pub(super) fn lower_place(&mut self, expr: &MonoExpr) -> Place {
        match &expr.kind {
            MonoExprKind::Local(id) => Place::local(self.local_for(*id)),
            MonoExprKind::Field { base, index } => {
                let mut place = self.lower_place(base);
                place.projection.push(Projection::Field(*index));
                place
            }
            MonoExprKind::Index { base, index } => {
                let mut place = self.lower_place(base);
                let index_op = self.lower_expr(index);
                place.projection.push(Projection::Index(index_op));
                place
            }
            other => panic!("mir: {other:?} is not an assignable place (typecheck should have rejected this target)"),
        }
    }

    /// Reads a place's current value as an `Operand` — the mirror image
    /// of [`Self::lower_place`], used only to read the *outgoing* value
    /// out of a field/index store before overwriting it (see the
    /// `Assign` case in [`Self::lower_expr`]).
    pub(super) fn read_place(&mut self, place: &Place, ty: Type) -> Operand {
        if place.projection.is_empty() {
            return Operand::Local(place.local);
        }
        let mut current = Operand::Local(place.local);
        let mut current_ty = self.locals[place.local.index()].ty.clone();
        let mut intermediates = Vec::new();
        for (position, projection) in place.projection.iter().enumerate() {
            let projected_ty = self
                .projected_type(&current_ty, projection)
                .unwrap_or_else(|| {
                    if position + 1 == place.projection.len() {
                        ty.clone()
                    } else {
                        Type::Error
                    }
                });
            let rvalue = match projection {
                Projection::Field(index) => Rvalue::Field {
                    base: current.clone(),
                    index: *index,
                },
                Projection::Index(index) => Rvalue::Index {
                    base: current.clone(),
                    index: index.clone(),
                },
                Projection::VariantField { variant, index } => Rvalue::VariantField {
                    base: current.clone(),
                    variant: *variant,
                    index: *index,
                },
            };
            let next = self.materialize(rvalue, projected_ty.clone());
            if position > 0 {
                if let Operand::Local(previous) = current {
                    if self.sigs.has_managed_content(&current_ty, self.defs) {
                        intermediates.push(previous);
                    }
                }
            }
            current = Operand::Local(next);
            current_ty = projected_ty;
        }
        for intermediate in intermediates {
            self.push_instr(Instr::Release(intermediate));
        }
        current
    }

    pub(super) fn projected_type(&self, base: &Type, projection: &Projection) -> Option<Type> {
        match projection {
            Projection::Field(index) => match base {
                Type::Tuple(items) => items.get(*index as usize).cloned(),
                _ => self.sigs.type_fields(base)?.get(*index as usize).cloned(),
            },
            Projection::VariantField { variant, index } => self
                .sigs
                .enum_payload(base, *variant)?
                .get(*index as usize)
                .cloned(),
            Projection::Index(_) => match base {
                Type::Array(elem) => Some((**elem).clone()),
                _ => None,
            },
        }
    }
}
