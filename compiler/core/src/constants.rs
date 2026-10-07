use crate::{
    check,
    hir::{Type, Value},
};
use nether_frontend::{
    ast as a,
    source::{Diagnostic, Span},
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type Constants = BTreeMap<String, (Value, Type)>;
type Result<T> = std::result::Result<T, Diagnostic>;

pub(crate) fn evaluate(module: &a::Module) -> Result<Constants> {
    let mut declarations = BTreeMap::new();
    for item in &module.items {
        if let a::ItemKind::Const { name, ty, value } = &item.kind {
            if declarations.insert(name.clone(), (ty, value)).is_some() {
                return Err(error(item.span, "E0301", "duplicate constant declaration"));
            }
        }
    }
    let mut context = Context {
        declarations,
        values: BTreeMap::new(),
        active: BTreeSet::new(),
    };
    for name in context.declarations.keys().cloned().collect::<Vec<_>>() {
        context.evaluate(&name)?;
    }
    Ok(context.values)
}
struct Context<'a> {
    declarations: BTreeMap<String, (&'a a::Type, &'a a::Expr)>,
    values: Constants,
    active: BTreeSet<String>,
}
impl Context<'_> {
    fn evaluate(&mut self, name: &str) -> Result<()> {
        if self.values.contains_key(name) {
            return Ok(());
        }
        let (ty, value) = self.declarations[name];
        if !self.active.insert(name.into()) {
            return Err(error(value.span, "E0331", "cycle in constant evaluation"));
        }
        if self.active.len() > 32 {
            return Err(error(
                value.span,
                "E0334",
                "constant dependency depth limit exceeded",
            ));
        }
        let mut dependencies = Vec::new();
        names(value, &mut dependencies)?;
        type_names(ty, &mut dependencies)?;
        for (dependency, span) in dependencies {
            if !self.declarations.contains_key(&dependency) {
                return Err(error(
                    span,
                    "E0310",
                    format!("unknown constant '{dependency}'"),
                ));
            }
            self.evaluate(&dependency)?;
        }
        let resolved = check::resolve_type_in(ty, &self.values)?;
        let result = check::constant_value(value, &resolved, &self.values)?;
        self.values.insert(name.into(), (result, resolved));
        self.active.remove(name);
        Ok(())
    }
}
fn type_names(ty: &a::Type, out: &mut Vec<(String, Span)>) -> Result<()> {
    match &ty.kind {
        a::TypeKind::Array { element, length } => {
            type_names(element, out)?;
            names(length, out)?;
        }
        a::TypeKind::Tuple(fields) => {
            for field in fields {
                type_names(field, out)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn block_names(block: &a::Block, out: &mut Vec<(String, Span)>) -> Result<()> {
    for statement in &block.statements {
        let a::StatementKind::Expression(value) = &statement.kind else {
            return Err(error(
                statement.span,
                "E0330",
                "constant branch must contain pure expressions",
            ));
        };
        names(value, out)?;
    }
    Ok(())
}
fn names(value: &a::Expr, out: &mut Vec<(String, Span)>) -> Result<()> {
    match &value.kind {
        a::ExprKind::Literal(_) => {}
        a::ExprKind::Name(name) => out.push((name.clone(), value.span)),
        a::ExprKind::Group(value) | a::ExprKind::Unary { value, .. } => names(value, out)?,
        a::ExprKind::Cast { value, ty } => {
            names(value, out)?;
            type_names(ty, out)?;
        }
        a::ExprKind::Binary { left, right, .. } => {
            names(left, out)?;
            names(right, out)?;
        }
        a::ExprKind::Tuple(values) | a::ExprKind::Array(values) => {
            for value in values {
                names(value, out)?;
            }
        }
        a::ExprKind::Index { value, index } => {
            names(value, out)?;
            names(index, out)?;
        }
        a::ExprKind::Member { value, .. } => names(value, out)?,
        a::ExprKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            names(condition, out)?;
            block_names(then_block, out)?;
            if let Some(branch) = else_branch {
                names(branch, out)?;
            }
        }
        a::ExprKind::Branch(block) => block_names(block, out)?,
        _ => {
            return Err(error(
                value.span,
                "E0330",
                "operation is not allowed in a constant expression",
            ))
        }
    }
    Ok(())
}
fn error(span: Span, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message, span)
}
