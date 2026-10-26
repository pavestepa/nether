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
        self.require_raw_unsafe(span)?;
        if !ty.raw_scalar() && ty != Type::Unit && !self.mono.borrow().auditing {
            return Err(error(
                span,
                "E0900",
                "owned RawStorage currently supports scalar and unit elements only",
            ));
        }
        let pointer = Type::Pointer {
            mutable: true,
            pointee: Box::new(ty.clone()),
        };
        let usize_ty = Type::Integer(I::Usize);
        let (opcode, parameters, result) = match id {
            Intrinsic::Allocate => ("storage_allocate", vec![usize_ty], pointer),
            Intrinsic::Initialize => (
                "storage_initialize",
                vec![pointer, usize_ty, ty],
                Type::Unit,
            ),
            Intrinsic::Replace => ("storage_replace", vec![pointer, usize_ty, ty.clone()], ty),
            Intrinsic::Take => ("storage_take", vec![pointer, usize_ty], ty),
            Intrinsic::DropSlot => ("storage_drop", vec![pointer, usize_ty], Type::Unit),
            Intrinsic::Release => ("storage_release", vec![pointer, usize_ty], Type::Unit),
            _ => unreachable!(),
        };
        if args.len() != parameters.len() {
            return Err(error(span, "E0313", "intrinsic arity mismatch"));
        }
        let mut values = args
            .iter()
            .zip(&parameters)
            .map(|(v, t)| self.expr(v, Some(t), true))
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
