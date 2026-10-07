use super::*;
use crate::mir::{self as m, Operand, Operation, Projection, Terminator};
use nether_frontend::source::Span;
impl Emitter<'_> {
    pub(super) fn mir_terminator(
        &mut self,
        term: &Terminator,
        function: &m::Function,
        span: Span,
    ) -> Result<(), Diagnostic> {
        self.unwind = None;
        match term {
            Terminator::Evaluate {
                operation,
                destination,
                next,
                unwind,
            } => {
                self.unwind = Some(format!("bb{unwind}"));
                if let Operation::Borrow(source) | Operation::CapturePlace(source) = operation {
                    let address = self.place(&place_expr(source, function, span))?;
                    self.line(format!("store ptr {address}, ptr %l{}", destination.local));
                    self.jump(&format!("bb{next}"));
                    return Ok(());
                }
                let destination = place_expr(destination, function, span);
                let operand =
                    |value: &Operand| operand_expr(value, function, &destination.ty, span);
                let kind = match operation {
                    Operation::Borrow(_) | Operation::CapturePlace(_) => unreachable!(),
                    Operation::Copy(place) | Operation::Move(place) => {
                        place_expr(place, function, span).kind
                    }
                    Operation::Use(value) => operand(value).kind,
                    Operation::Unary(op, value) => ExprKind::Unary(op, Box::new(operand(value))),
                    Operation::Binary(op, left, right) => {
                        // Comparisons have bool result; operand locals carry their actual types.
                        let left = operand(left);
                        let right = operand_expr(right, function, &left.ty, span);
                        ExprKind::Binary(op, Box::new(left), Box::new(right))
                    }
                    Operation::Cast(value, _) => ExprKind::Cast(Box::new(operand(value))),
                    Operation::Aggregate(values) => {
                        let fields = match &destination.ty {
                            Type::Tuple(fields) => fields.clone(),
                            Type::Record { fields, .. } => {
                                fields.iter().map(|f| f.ty.clone()).collect()
                            }
                            Type::Array(element, _) => vec![(**element).clone(); values.len()],
                            _ => vec![],
                        };
                        ExprKind::Tuple(
                            values
                                .iter()
                                .zip(fields)
                                .map(|(value, ty)| operand_expr(value, function, &ty, span))
                                .collect(),
                        )
                    }
                    Operation::Enum(variant, values) => {
                        let Type::Enum { variants, .. } = &destination.ty else {
                            unreachable!()
                        };
                        ExprKind::Enum {
                            variant: *variant,
                            fields: values
                                .iter()
                                .zip(&variants[*variant].fields)
                                .map(|(value, ty)| operand_expr(value, function, ty, span))
                                .collect(),
                        }
                    }
                    Operation::TagPlace(place) => {
                        ExprKind::Tag(Box::new(place_expr(place, function, span)))
                    }
                    Operation::Tag(value) => ExprKind::Tag(Box::new(operand(value))),
                    Operation::Call(id, args) => {
                        ExprKind::Call(*id, args.iter().map(operand).collect())
                    }
                };
                let value = self.expr(&Expr {
                    kind,
                    ty: destination.ty.clone(),
                    span,
                })?;
                let address = self.place(&destination)?;
                self.line(format!(
                    "store {} {value}, ptr {address}",
                    ty(&destination.ty)
                ));
                self.jump(&format!("bb{next}"));
            }
            Terminator::CheckPlace {
                place,
                next,
                unwind,
            } => {
                self.unwind = Some(format!("bb{unwind}"));
                self.place(&place_expr(place, function, span))?;
                self.jump(&format!("bb{next}"));
            }
            Terminator::Goto(next) | Terminator::EndStorage { next, .. } => {
                self.jump(&format!("bb{next}"))
            }
            Terminator::Branch { condition, yes, no } => {
                let condition = self.expr(&operand_expr(condition, function, &Type::Bool, span))?;
                self.branch(&condition, &format!("bb{yes}"), &format!("bb{no}"));
            }
            Terminator::Return(value) => {
                if function.result != Type::Unit {
                    let value =
                        self.expr(&operand_expr(value, function, &function.result, span))?;
                    self.line(format!(
                        "store {} {value}, ptr %result",
                        ty(&function.result)
                    ));
                }
                self.line("ret i1 true".into());
                self.terminated = true;
            }
            Terminator::ResumePanic => {
                self.line("ret i1 false".into());
                self.terminated = true;
            }
            Terminator::Unreachable => {
                self.line("unreachable".into());
                self.terminated = true;
            }
        }
        Ok(())
    }
}
fn operand_expr(value: &Operand, function: &m::Function, hint: &Type, span: Span) -> Expr {
    match value {
        Operand::Address(place) => {
            let place = place_expr(place, function, span);
            Expr {
                ty: place.ty.clone(),
                kind: ExprKind::ArgumentView {
                    place: Box::new(place),
                    access: crate::mir::loans::Access::Readonly,
                },
                span,
            }
        }
        Operand::Local(id) => Expr {
            kind: ExprKind::Local(*id),
            ty: function.locals[*id].ty.clone(),
            span,
        },
        Operand::Constant(value) => {
            let ty = match value {
                Value::Unit => Type::Unit,
                Value::Bool(_) => Type::Bool,
                Value::Char(_) => Type::Char,
                Value::Integer(i) => Type::Integer(i.ty()),
                Value::F32(_) => Type::F32,
                Value::F64(_) => Type::F64,
                Value::Str(_) => Type::Str,
                _ => hint.clone(),
            };
            Expr {
                kind: ExprKind::Value(value.clone()),
                ty,
                span,
            }
        }
    }
}
fn place_expr(place: &m::Place, function: &m::Function, span: Span) -> Expr {
    let mut value = Expr {
        kind: ExprKind::Local(place.local),
        ty: function.locals[place.local].ty.clone(),
        span,
    };
    for projection in &place.projections {
        let (kind, ty) = match projection {
            Projection::Dereference => (ExprKind::View(place.local), value.ty.clone()),
            Projection::Field(index) => {
                let ty = match &value.ty {
                    Type::Tuple(fields) => fields[*index].clone(),
                    Type::Record { fields, .. } => fields[*index].ty.clone(),
                    _ => unreachable!(),
                };
                (ExprKind::Field(Box::new(value), *index), ty)
            }
            Projection::ConstantIndex(index) => {
                let Type::Array(element, _) = &value.ty else {
                    unreachable!()
                };
                let ty = (**element).clone();
                let index = Expr {
                    kind: ExprKind::Value(Value::Integer(nether_semantics::Integer::from_bits(
                        nether_semantics::IntegerType::Usize,
                        *index as u128,
                    ))),
                    ty: Type::Integer(nether_semantics::IntegerType::Usize),
                    span,
                };
                (ExprKind::Index(Box::new(value), Box::new(index)), ty)
            }
            Projection::Index(id) => {
                let Type::Array(element, _) = &value.ty else {
                    unreachable!()
                };
                let ty = (**element).clone();
                (
                    ExprKind::Index(
                        Box::new(value),
                        Box::new(operand_expr(
                            &Operand::Local(*id),
                            function,
                            &Type::Integer(nether_semantics::IntegerType::Usize),
                            span,
                        )),
                    ),
                    ty,
                )
            }
            Projection::Variant(index) => {
                let Type::Enum { variants, .. } = &value.ty else {
                    unreachable!()
                };
                let ty = Type::Tuple(variants[*index].fields.clone());
                (ExprKind::Payload(Box::new(value), *index), ty)
            }
        };
        value = Expr { kind, ty, span };
    }
    value
}
