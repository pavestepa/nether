//! Structured, infallible scalar entries. Keep the Result ABI wrapper for code
//! pointers and fallback MIR callers; calls within this subset use plain values.
use super::*;
use hir::{Block as HBlock, Expr, ExprKind as E, Statement as S};
use std::collections::BTreeSet;

fn scalar(ty: &Type) -> bool {
    matches!(ty, Type::Unit | Type::Bool | Type::Integer(_))
}

pub(super) fn candidates(p: &hir::Program, checks: OverflowChecks) -> BTreeSet<usize> {
    if checks != OverflowChecks::Wrapping {
        return BTreeSet::new();
    }
    let mut active: BTreeSet<_> = p
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            scalar(&f.result) && f.locals.iter().all(|l| scalar(&l.ty) && l.view.is_none())
        })
        .map(|(id, _)| id)
        .collect();
    loop {
        let next = active
            .iter()
            .copied()
            .filter(|id| block(&p.functions[*id].body, &active).is_some())
            .collect();
        if next == active {
            return active;
        }
        active = next;
    }
}

fn binary(op: &str, a: &str, b: &str, ty: &Type, rhs: Option<&Expr>) -> Option<String> {
    if matches!(ty, Type::Integer(_)) {
        let method = match op {
            "+" => Some("wrapping_add"),
            "-" => Some("wrapping_sub"),
            "*" => Some("wrapping_mul"),
            "<<" => Some("wrapping_shl"),
            ">>" => Some("wrapping_shr"),
            _ => None,
        };
        if let Some(method) = method {
            let rhs = if matches!(op, "<<" | ">>") {
                format!("({b}) as u32")
            } else {
                b.into()
            };
            return Some(format!("({a}).{method}({rhs})"));
        }
        if matches!(op, "/" | "%") {
            // A positive constant cannot be zero or signed MIN / -1. All
            // dynamic divisors retain MIR's checked division and panic edge.
            let E::Value(Value::Integer(n)) = &rhs?.kind else {
                return None;
            };
            if n.signed_value().map_or(n.bits() == 0, |n| n <= 0) {
                return None;
            }
        }
    }
    Some(format!("(({a}) {op} ({b}))"))
}
fn expr(e: &Expr, active: &BTreeSet<usize>) -> Option<String> {
    if !scalar(&e.ty) {
        return None;
    }
    Some(match &e.kind {
        E::Value(Value::Integer(n)) => format!(
            "{}{}",
            n.signed_value()
                .map_or_else(|| n.bits().to_string(), |n| n.to_string()),
            match &e.ty {
                Type::Integer(t) => format!("{t:?}").to_lowercase(),
                _ => return None,
            }
        ),
        E::Value(Value::Bool(v)) => v.to_string(),
        E::Value(Value::Unit) => "()".into(),
        E::Local(id) => format!("l{id}"),
        E::Binary(op, a, b) => binary(op, &expr(a, active)?, &expr(b, active)?, &a.ty, Some(b))?,
        E::Unary("-", v) => format!("({}).wrapping_neg()", expr(v, active)?),
        E::Unary(op, v) if matches!(*op, "!" | "~") => format!("!({})", expr(v, active)?),
        E::Cast(v) => format!("({} as {})", expr(v, active)?, scalar_name(&e.ty)?),
        E::Call(id, args) if active.contains(id) => format!(
            "fast{id}({})",
            args.iter()
                .map(|a| expr(a, active))
                .collect::<Option<Vec<_>>>()?
                .join(",")
        ),
        E::Assign {
            place,
            operator,
            value,
        } => {
            let E::Local(id) = place.kind else {
                return None;
            };
            let v = expr(value, active)?;
            let result = if *operator == "=" {
                v
            } else {
                binary(
                    operator.strip_suffix('=')?,
                    &format!("l{id}"),
                    &v,
                    &place.ty,
                    Some(value),
                )?
            };
            // HIR assignment returns unit.
            if e.ty != Type::Unit {
                return None;
            }
            format!("{{ l{id} = {result}; }}")
        }
        E::If { condition, yes, no } if e.ty == Type::Unit => format!(
            "if {} {{ {} }} else {{ {} }}",
            expr(condition, active)?,
            block(yes, active)?,
            match no {
                Some(b) => block(b, active)?,
                None => String::new(),
            }
        ),
        E::While { condition, body } => format!(
            "while {} {{ {} }}",
            expr(condition, active)?,
            block(body, active)?
        ),
        E::Block(b) if e.ty == Type::Unit => format!("{{ {} }}", block(b, active)?),
        _ => return None,
    })
}
fn scalar_name(ty: &Type) -> Option<String> {
    Some(match ty {
        Type::Unit => "()".into(),
        Type::Bool => "bool".into(),
        Type::Integer(t) => format!("{t:?}").to_lowercase(),
        _ => return None,
    })
}
fn block(b: &HBlock, active: &BTreeSet<usize>) -> Option<String> {
    let mut out = String::new();
    for s in &b.statements {
        out.push_str(&match s {
            S::Initialize(id, e) => format!("l{id} = {};\n", expr(e, active)?),
            S::Evaluate(e) => format!("{};\n", expr(e, active)?),
            S::Return(e) => format!(
                "return {};\n",
                match e {
                    Some(e) => expr(e, active)?,
                    None => "()".into(),
                }
            ),
            S::Break => "break;\n".into(),
            S::Continue => "continue;\n".into(),
        });
    }
    Some(out)
}

pub(super) fn emit(p: &hir::Program, active: &BTreeSet<usize>) -> String {
    let mut out = String::new();
    for id in active {
        let f = &p.functions[*id];
        let params = f
            .parameters
            .iter()
            .enumerate()
            .map(|(i, l)| format!("a{i}:{}", scalar_name(&f.locals[*l].ty).unwrap()))
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&format!(
            "// infallible structured entry\nunsafe fn fast{id}({params})->{} {{\n",
            scalar_name(&f.result).unwrap()
        ));
        for (l, local) in f.locals.iter().enumerate() {
            let init = if let Some(i) = f.parameters.iter().position(|p| *p == l) {
                format!("a{i}")
            } else {
                match local.ty {
                    Type::Unit => "()",
                    Type::Bool => "false",
                    _ => "0",
                }
                .into()
            };
            out.push_str(&format!(
                "let mut l{l}:{}={init};\n",
                scalar_name(&local.ty).unwrap()
            ));
        }
        out.push_str(&block(&f.body, active).unwrap());
        if f.result != Type::Unit {
            out.push_str("unreachable!();\n");
        }
        out.push_str("}\n");
    }
    out
}
