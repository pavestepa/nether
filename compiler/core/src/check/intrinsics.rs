use super::*;
use nether_frontend::intrinsics::Intrinsic;
impl Checker<'_> {
    pub(super) fn intrinsic(
        &mut self,
        id: Intrinsic,
        types: &[a::TypeArgument],
        args: &[a::Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<h::Expr> {
        let [a::TypeArgument::Type(source)] = types else {
            return Err(error(
                span,
                "E0313",
                "intrinsic requires one explicit type argument",
            ));
        };
        let ty = resolve_type_with(source, self.constants, self.nominals)?;
        if matches!(id, Intrinsic::Size | Intrinsic::Align) {
            if !args.is_empty() {
                return Err(error(span, "E0313", "layout query takes no values"));
            }
            let layout = crate::layout::layout(&ty).map_err(|e| error(span, "E0318", e))?;
            let n = if id == Intrinsic::Size {
                layout.size()
            } else {
                layout.align()
            };
            let result = Type::Integer(I::Usize);
            if let Some(expected) = expected {
                same(expected, &result, span)?;
            }
            return Ok(h::Expr {
                kind: h::ExprKind::Value(Value::Integer(Integer::from_bits(I::Usize, n as u128))),
                ty: result,
                span,
            });
        }
        if id == Intrinsic::CheckIndex {
            if args.len() != 2 {
                return Err(error(
                    span,
                    "E0313",
                    "index check requires index and length",
                ));
            }
            let result = Type::Integer(I::Usize);
            let index = self.expr(&args[0], Some(&result), true)?;
            let length = self.expr(&args[1], Some(&result), true)?;
            if let Some(expected) = expected {
                same(expected, &result, span)?;
            }
            return Ok(h::Expr {
                kind: h::ExprKind::Binary("checked_index", Box::new(index), Box::new(length)),
                ty: result,
                span,
            });
        }
        self.require_raw_unsafe(span)?;
        if matches!(id, Intrinsic::View | Intrinsic::ViewMut) {
            if args.len() != 2 {
                return Err(error(
                    span,
                    "E0313",
                    "storage view requires storage and index",
                ));
            }
            if !matches!(ty, Type::Class { copy: false, .. })
                && !ty.raw_copy_value()
                && !self.mono.borrow().auditing
            {
                return Err(error(
                    span,
                    "E0900",
                    "storage views currently require a non-Copy class or a plain Copy value",
                ));
            }
            let storage = self.expr(&args[0], None, true)?;
            if !views::place(&storage) {
                return Err(error(
                    span,
                    "E0382",
                    "storage view requires a live storage place",
                ));
            }
            if id == Intrinsic::ViewMut {
                self.mutable_place(&storage)?;
            }
            let index = self.expr(&args[1], Some(&Type::Integer(I::Usize)), true)?;
            if let Some(expected) = expected {
                same(expected, &ty, span)?;
            }
            return Ok(h::Expr {
                kind: h::ExprKind::Unary(
                    if id == Intrinsic::View {
                        "storage_view"
                    } else {
                        "storage_view_mut"
                    },
                    Box::new(h::Expr {
                        ty: Type::Tuple(vec![storage.ty.clone(), index.ty.clone()]),
                        kind: h::ExprKind::Tuple(vec![storage, index]),
                        span,
                    }),
                ),
                ty,
                span,
            });
        }
        if !ty.raw_storage_value() && !self.mono.borrow().auditing {
            return Err(error(
                span,
                "E0900",
                "owned RawStorage requires a supported owned representation without borrowed fields",
            ));
        }
        let pointer = Type::Pointer {
            mutable: true,
            pointee: Box::new(ty.clone()),
        };
        if id == Intrinsic::Detach {
            if args.len() != 1 {
                return Err(error(span, "E0313", "into_raw_parts requires one storage"));
            }
            let value = self.expr(&args[0], None, true)?;
            let Type::Record { fields, .. } = &value.ty else {
                return Err(error(span, "E0503", "expected RawStorage"));
            };
            if fields.len() != 2
                || fields[0].name != "data"
                || !fields[0].private
                || fields[0].ty != pointer
                || fields[1].name != "cap"
                || !fields[1].private
                || fields[1].ty != Type::Integer(I::Usize)
            {
                return Err(error(
                    span,
                    "E0503",
                    "expected owned RawStorage representation",
                ));
            }
            let result = Type::Tuple(vec![pointer, Type::Integer(I::Usize)]);
            if let Some(expected) = expected {
                same(expected, &result, span)?
            }
            return Ok(h::Expr {
                kind: h::ExprKind::Unary("storage_detach", Box::new(value)),
                ty: result,
                span,
            });
        }
        if matches!(id, Intrinsic::DropInPlace | Intrinsic::ReadOwned) {
            let result = if id == Intrinsic::ReadOwned {
                ty.clone()
            } else {
                Type::Unit
            };
            if args.len() != 1 {
                return Err(error(
                    span,
                    "E0313",
                    "raw owning operation requires one pointer",
                ));
            }
            let value = self.expr(&args[0], Some(&pointer), true)?;
            self.same_storage_mode(&pointer, &value.ty, span)?;
            if let Some(expected) = expected {
                same(expected, &result, span)?
            }
            return Ok(h::Expr {
                kind: h::ExprKind::Unary(
                    if id == Intrinsic::ReadOwned {
                        "raw_take"
                    } else {
                        "raw_drop"
                    },
                    Box::new(h::Expr {
                        ty: Type::Tuple(vec![pointer, Type::Integer(I::Usize)]),
                        kind: h::ExprKind::Tuple(vec![
                            value,
                            h::Expr {
                                kind: h::ExprKind::Value(Value::Integer(Integer::from_bits(
                                    I::Usize,
                                    0,
                                ))),
                                ty: Type::Integer(I::Usize),
                                span,
                            },
                        ]),
                        span,
                    }),
                ),
                ty: result,
                span,
            });
        }
        let usize_ty = Type::Integer(I::Usize);
        let (opcode, parameters, result) = match id {
            Intrinsic::SlotAddress | Intrinsic::SlotAddressMut => (
                if id == Intrinsic::SlotAddress {
                    "storage_address"
                } else {
                    "storage_address_mut"
                },
                vec![pointer, usize_ty.clone(), usize_ty],
                Type::Pointer {
                    mutable: id == Intrinsic::SlotAddressMut,
                    pointee: Box::new(ty),
                },
            ),
            Intrinsic::MoveSlots => (
                "storage_move_slots",
                vec![
                    pointer.clone(),
                    usize_ty.clone(),
                    usize_ty.clone(),
                    pointer,
                    usize_ty.clone(),
                    usize_ty.clone(),
                    usize_ty,
                ],
                Type::Unit,
            ),
            Intrinsic::CheckRange | Intrinsic::DropRange => (
                if id == Intrinsic::DropRange {
                    "storage_drop_range"
                } else {
                    "storage_check_range"
                },
                vec![pointer, usize_ty.clone(), usize_ty.clone(), usize_ty],
                Type::Unit,
            ),
            Intrinsic::Allocate => ("storage_allocate", vec![usize_ty], pointer),
            Intrinsic::Initialize | Intrinsic::WriteOwned => (
                if id == Intrinsic::WriteOwned {
                    "raw_initialize"
                } else {
                    "storage_initialize"
                },
                vec![pointer, usize_ty, ty],
                Type::Unit,
            ),
            Intrinsic::Replace => ("storage_replace", vec![pointer, usize_ty, ty.clone()], ty),
            Intrinsic::Take => ("storage_take", vec![pointer, usize_ty], ty),
            Intrinsic::DropSlot => ("storage_drop", vec![pointer, usize_ty], Type::Unit),
            Intrinsic::Adopt => ("storage_adopt", vec![pointer, usize_ty], Type::Unit),
            Intrinsic::Release => ("storage_release", vec![pointer, usize_ty], Type::Unit),
            _ => unreachable!(),
        };
        if args.len() != parameters.len() {
            return Err(error(span, "E0313", "intrinsic arity mismatch"));
        }
        let mut values = args
            .iter()
            .zip(&parameters)
            .map(|(v, t)| {
                let value = self.expr(v, Some(t), true)?;
                self.same_storage_mode(t, &value.ty, v.span)?;
                if matches!(
                    id,
                    Intrinsic::Initialize | Intrinsic::Replace | Intrinsic::WriteOwned
                ) && value.ty.contains_views()
                    && !self.mono.borrow().auditing
                {
                    return Err(error(
                        v.span,
                        "E0900",
                        "owned storage cannot erase borrowed payload dependencies",
                    ));
                }
                Ok(value)
            })
            .collect::<Result<Vec<_>>>()?;
        let operand = if id == Intrinsic::Allocate {
            values.pop().unwrap()
        } else {
            h::Expr {
                kind: h::ExprKind::Tuple(values),
                ty: Type::Tuple(parameters),
                span,
            }
        };
        if let Some(expected) = expected {
            same(expected, &result, span)?;
        }
        Ok(h::Expr {
            kind: h::ExprKind::Unary(opcode, Box::new(operand)),
            ty: result,
            span,
        })
    }
}
