use crate::hir::{self as h, Type, Value};
use nether_frontend::{
    ast as a,
    source::{Diagnostic, Span},
};
use nether_semantics::{Integer, IntegerType as I};
use std::collections::BTreeMap;
mod entries;
mod enums;
mod future;
mod generics;
mod methods;
mod patterns;
mod records;
mod uses;
mod views;

type Result<T> = std::result::Result<T, Diagnostic>;
#[derive(Clone)]
struct Signature {
    modes: Vec<Option<crate::mir::loans::Access>>,
    id: usize,
    parameters: Vec<Type>,
    result: Type,
}

pub fn check(module: &a::Module) -> std::result::Result<h::Program, Vec<Diagnostic>> {
    let prepared = methods::collect(module);
    let module = &prepared;
    let private_functions: BTreeMap<_, _> = module
        .items
        .iter()
        .filter_map(|item| match &item.kind {
            a::ItemKind::Function(function) if item.private => {
                Some((function.name.clone(), function.span.source))
            }
            _ => None,
        })
        .collect();
    let constants = crate::constants::evaluate(module).map_err(|e| vec![e])?;
    let nominals = records::collect(module, &constants).map_err(|e| vec![e])?;
    let mut signatures = BTreeMap::new();
    let mut declarations = Vec::new();
    let mut errors = Vec::new();
    let mut templates = BTreeMap::new();
    for item in &module.items {
        if matches!(
            item.kind,
            a::ItemKind::Const { .. }
                | a::ItemKind::Aggregate { class: false, .. }
                | a::ItemKind::Enum { .. }
        ) {
            continue;
        }
        let a::ItemKind::Function(function) = &item.kind else {
            errors.push(error(
                item.span,
                "E0900",
                "this declaration is not yet implemented in the value checker",
            ));
            continue;
        };
        if constants.contains_key(&function.name) {
            errors.push(error(
                function.span,
                "E0301",
                "function conflicts with a constant declaration",
            ));
            continue;
        }
        if !function.generics.is_empty() && !function.c_abi {
            if function.name == "main" {
                errors.push(error(
                    item.span,
                    "E0303",
                    "main cannot have generic parameters",
                ));
            }

            if templates
                .insert(function.name.clone(), function.clone())
                .is_some()
                || signatures.contains_key(&function.name)
            {
                errors.push(error(item.span, "E0301", "duplicate function declaration"));
            }
            continue;
        }
        if function.c_abi {
            errors.push(error(
                item.span,
                "E0900",
                "foreign function checking is not implemented yet",
            ));
            continue;
        }
        let signature = (|| {
            let mut parameters = Vec::new();
            for param in &function.parameters {
                let ty = param
                    .ty
                    .as_ref()
                    .ok_or_else(|| error(param.span, "E0300", "unexpected receiver"))?;
                parameters.push(resolve_type_with(ty, &constants, &nominals)?);
            }
            Ok(Signature {
                modes: methods::receiver_modes(function),
                id: declarations.len(),
                parameters,
                result: resolve_type_with(&function.result, &constants, &nominals)?,
            })
        })();
        match signature {
            Ok(signature) => {
                if templates.contains_key(&function.name) {
                    errors.push(error(item.span, "E0301", "duplicate function declaration"));
                }
                if signatures
                    .insert(function.name.clone(), signature)
                    .is_some()
                {
                    errors.push(error(
                        function.span,
                        "E0301",
                        "duplicate function declaration",
                    ));
                }
                declarations.push(function.clone());
            }
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mono = std::cell::RefCell::new(generics::Monomorphization {
        templates,
        next_id: declarations.len(),
        ..Default::default()
    });
    let mut work: std::collections::VecDeque<_> = declarations
        .into_iter()
        .map(|function| (signatures[&function.name].clone(), function))
        .collect();
    for (signature, declaration) in &work {
        mono.borrow_mut()
            .bodies
            .insert(signature.id, declaration.clone());
    }
    let mut functions = Vec::new();
    while let Some((signature, declaration)) = work.pop_front() {
        let checked = check_declaration(
            &declaration,
            &signature,
            &signatures,
            &constants,
            &nominals,
            &mono,
            &private_functions,
        );
        match checked {
            Ok(function) => functions.push(function),
            Err(e) => errors.push(e),
        }
        work.extend(mono.borrow_mut().pending.drain(..));
    }
    if errors.is_empty() {
        let mut audit_work: std::collections::VecDeque<_> =
            mono.borrow_mut().audits.drain(..).collect();
        let audit_mono = std::cell::RefCell::new(generics::Monomorphization {
            templates: mono.borrow().templates.clone(),
            bodies: mono.borrow().bodies.clone(),
            next_id: mono.borrow().next_id,
            auditing: true,
            ..Default::default()
        });
        while let Some((signature, declaration)) = audit_work.pop_front() {
            if let Err(diagnostic) = check_declaration(
                &declaration,
                &signature,
                &signatures,
                &constants,
                &nominals,
                &audit_mono,
                &private_functions,
            ) {
                errors.push(error(
                    diagnostic.span,
                    "E0373",
                    format!(
                        "generic body is not valid under its declared constraints: {}",
                        diagnostic.message
                    ),
                ));
                break;
            }
            audit_work.extend(audit_mono.borrow_mut().pending.drain(..));
        }
    }
    let main = signatures.get("main").map(|s| s.id);
    if let Some(signature) = signatures.get("main") {
        let declaration = module
            .items
            .iter()
            .find(|item| matches!(&item.kind,a::ItemKind::Function(f) if f.name=="main"))
            .unwrap();
        if declaration.private
            || !signature.parameters.is_empty()
            || !matches!(signature.result, Type::Unit | Type::Integer(I::I32))
        {
            errors.push(error(
                declaration.span,
                "E0303",
                "main must be public, have no parameters and return () or i32",
            ));
        }
    }
    if errors.is_empty() {
        {
            let program = h::Program { functions, main };
            let mir = crate::mir::lower(&program);
            let mut errors: Vec<_> = mir
                .functions
                .iter()
                .flat_map(|f| {
                    crate::mir::borrow_check::check(f)
                        .into_iter()
                        .map(|e| error(f.blocks[e.block].span, "E0381", e.message))
                })
                .collect();
            for function in &mir.functions {
                for (block, place) in crate::mir::dataflow::place_initialization_errors(function) {
                    errors.push(error(
                        function.blocks[block].span,
                        "E0384",
                        format!(
                            "use of moved or uninitialized place '{}'",
                            function.locals[place.local].name
                        ),
                    ));
                }
            }
            if errors.is_empty() {
                Ok(program)
            } else {
                Err(errors)
            }
        }
    } else {
        Err(errors)
    }
}

struct Checker<'a> {
    signatures: &'a BTreeMap<String, Signature>,
    mono: &'a std::cell::RefCell<generics::Monomorphization>,
    constants: &'a BTreeMap<String, (Value, Type)>,
    nominals: &'a records::Nominals,
    private_functions: &'a BTreeMap<String, nether_frontend::source::SourceId>,
    scopes: Vec<BTreeMap<String, usize>>,
    locals: Vec<h::Local>,
    result: Type,
    loops: usize,
    value_uses: BTreeMap<String, Vec<Span>>,
    pattern_views: bool,
    continuations: Vec<(a::Block, usize)>,
}

impl Checker<'_> {
    fn local(&mut self, name: String, ty: Type, mutable: bool, span: Span) -> Result<usize> {
        let scope = self.scopes.last_mut().unwrap();
        // Same-scope shadowing follows declaration order; each binding has a new ID.
        let id = self.locals.len();
        scope.insert(name.clone(), id);
        self.locals.push(h::Local {
            view: None,
            name,
            ty,
            mutable,
            span,
        });
        Ok(id)
    }
    fn lookup(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn block(
        &mut self,
        source: &a::Block,
        expected: Option<&Type>,
        value_context: bool,
    ) -> Result<h::Block> {
        let storage_start = self.locals.len();
        self.continuations.push((source.clone(), 0));
        self.scopes.push(BTreeMap::new());
        let mut statements = Vec::new();
        let mut result = Type::Unit;
        let mut diverges = false;
        for (index, statement) in source.statements.iter().enumerate() {
            self.continuations.last_mut().unwrap().1 = index;
            let last = index + 1 == source.statements.len();
            let checked = match &statement.kind {
                a::StatementKind::Binding {
                    mutable,
                    pattern,
                    ty,
                    value,
                } => {
                    if views::has_ref(pattern) {
                        statements.push(self.view_binding(pattern, value, ty.as_ref())?);
                        continue;
                    }
                    if !matches!(
                        pattern.kind,
                        a::PatternKind::Binding { by_ref: false, .. } | a::PatternKind::Wildcard
                    ) && uses::binds(pattern)
                        && self.needs_pattern_view(value)
                    {
                        statements.push(self.view_binding(
                            &views::inferred_pattern(pattern, *mutable),
                            value,
                            ty.as_ref(),
                        )?);
                        continue;
                    }
                    let ty = ty
                        .as_ref()
                        .map(|ty| resolve_type_with(ty, self.constants, self.nominals))
                        .transpose()?;
                    let value = self.expr(value, ty.as_ref(), true)?;
                    if !value.ty.copyable()
                        && self.ownership_view_needed(match &statement.kind {
                            a::StatementKind::Binding { value, .. } => value,
                            _ => unreachable!(),
                        })
                    {
                        let a::StatementKind::Binding {
                            value: source,
                            ty: annotation,
                            ..
                        } = &statement.kind
                        else {
                            unreachable!()
                        };
                        statements.push(self.view_binding(
                            &views::inferred_pattern(pattern, *mutable),
                            source,
                            annotation.as_ref(),
                        )?);
                        continue;
                    }
                    let value_diverges = expression_diverges(&value);
                    let id = self.locals.len();
                    self.locals.push(h::Local {
                        view: None,
                        name: "<binding value>".into(),
                        ty: value.ty.clone(),
                        mutable: false,
                        span: pattern.span,
                    });
                    let reference = h::Expr {
                        kind: h::ExprKind::Local(id),
                        ty: value.ty.clone(),
                        span: pattern.span,
                    };
                    let mut bindings = vec![h::Statement::Initialize(id, value)];
                    self.scopes.push(BTreeMap::new());
                    let first = self.locals.len();
                    let (_, irrefutable, _) = self.pattern(pattern, &reference, &mut bindings)?;
                    if !irrefutable {
                        return Err(error(
                            pattern.span,
                            "E0344",
                            "binding requires an irrefutable pattern",
                        ));
                    }
                    if *mutable {
                        for local in &mut self.locals[first..] {
                            local.mutable = true;
                        }
                    }
                    let names = self.scopes.pop().unwrap();
                    self.scopes.last_mut().unwrap().extend(names);
                    diverges |= value_diverges;
                    h::Statement::Evaluate(h::Expr {
                        kind: h::ExprKind::Block(h::Block {
                            storage: vec![],
                            statements: bindings,
                            result: Type::Unit,
                            diverges: value_diverges,
                            span: pattern.span,
                        }),
                        ty: Type::Unit,
                        span: pattern.span,
                    })
                }
                a::StatementKind::Expression(value) => {
                    let checked = self.expr(
                        value,
                        if last && value_context {
                            expected
                        } else {
                            None
                        },
                        last && value_context,
                    )?;
                    if !diverges && last && value_context {
                        result = checked.ty.clone();
                    }
                    diverges |= expression_diverges(&checked);
                    h::Statement::Evaluate(checked)
                }
                a::StatementKind::Return(value) => {
                    let expected = self.result.clone();
                    let value = value
                        .as_ref()
                        .map(|v| self.expr(v, Some(&expected), true))
                        .transpose()?;
                    if value.is_none() && expected != Type::Unit {
                        return Err(error(
                            statement.span,
                            "E0304",
                            "return without value requires unit result",
                        ));
                    }
                    diverges = true;
                    h::Statement::Return(value)
                }
                a::StatementKind::Break | a::StatementKind::Continue => {
                    if self.loops == 0 {
                        return Err(error(
                            statement.span,
                            "E0305",
                            "loop control outside a loop",
                        ));
                    }
                    diverges = true;
                    if matches!(statement.kind, a::StatementKind::Break) {
                        h::Statement::Break
                    } else {
                        h::Statement::Continue
                    }
                }
                a::StatementKind::Unsafe(_) => {
                    return Err(error(
                        statement.span,
                        "E0900",
                        "unsafe checking is not implemented yet",
                    ))
                }
            };
            statements.push(checked);
        }
        self.scopes.pop();
        self.continuations.pop();
        if value_context && !diverges {
            if let Some(expected) = expected {
                same(expected, &result, source.span)?;
            }
        }
        Ok(h::Block {
            storage: (storage_start..self.locals.len()).collect(),
            statements,
            result,
            diverges,
            span: source.span,
        })
    }

    fn expr(
        &mut self,
        source: &a::Expr,
        expected: Option<&Type>,
        value_context: bool,
    ) -> Result<h::Expr> {
        let span = source.span;
        let (kind, ty) = match &source.kind {
            a::ExprKind::Match { value, arms } => {
                return self.match_expression(value, arms, expected, value_context, span);
            }
            a::ExprKind::Branch(block) => {
                let block = self.block(block, expected, value_context)?;
                let ty = block.result.clone();
                (h::ExprKind::Block(block), ty)
            }
            a::ExprKind::Group(value) => return self.expr(value, expected, value_context),
            a::ExprKind::Literal(value) => literal(value, expected, false, span)?,
            a::ExprKind::Name(name) => {
                if self.lookup(name).is_none() {
                    if let Some(receiver) = self.lookup("this") {
                        if matches!(&self.locals[receiver].ty,Type::Record {fields,..} if fields.iter().any(|f|f.name==*name))
                        {
                            let member = a::Expr {
                                kind: a::ExprKind::Member {
                                    value: Box::new(a::Expr {
                                        kind: a::ExprKind::Name("this".into()),
                                        span,
                                    }),
                                    name: name.clone(),
                                },
                                span,
                            };
                            return self.expr(&member, expected, value_context);
                        }
                    }
                    if let Some((value, ty)) = self.constants.get(name) {
                        if let Some(expected) = expected {
                            same(expected, ty, span)?;
                        }
                        return Ok(h::Expr {
                            kind: h::ExprKind::Value(value.clone()),
                            ty: ty.clone(),
                            span,
                        });
                    }
                }
                let id = self
                    .lookup(name)
                    .ok_or_else(|| error(span, "E0310", format!("unknown value '{name}'")))?;
                (
                    if self.locals[id].view.is_some() {
                        h::ExprKind::View(id)
                    } else {
                        h::ExprKind::Local(id)
                    },
                    self.locals[id].ty.clone(),
                )
            }
            a::ExprKind::Unary { operator, value } => {
                if *operator == "-" {
                    if let a::ExprKind::Literal(a::Literal::Integer(_)) = &value.kind {
                        let a::ExprKind::Literal(literal_value) = &value.kind else {
                            unreachable!()
                        };
                        let (kind, ty) = literal(literal_value, expected, true, span)?;
                        return Ok(h::Expr { kind, ty, span });
                    }
                }
                fn view_name(value: &a::Expr) -> bool {
                    match &value.kind {
                        a::ExprKind::Name(_) => true,
                        a::ExprKind::Group(v) => view_name(v),
                        _ => false,
                    }
                }
                let named_view = view_name(value);
                let value = self.expr(value, expected, true)?;
                let ty = value.ty.clone();
                if *operator == "*" && named_view && matches!(value.kind, h::ExprKind::View(_)) {
                    return Ok(value);
                }
                let valid = match *operator {
                    "-" => {
                        matches!(ty,Type::Integer(i) if i.is_signed())
                            || matches!(ty, Type::F32 | Type::F64)
                    }
                    "!" => matches!(ty, Type::Bool | Type::Integer(_)),
                    _ => false,
                };
                if !valid {
                    return Err(error(
                        span,
                        "E0311",
                        "operator is not available for this type",
                    ));
                }
                (h::ExprKind::Unary(operator, Box::new(value)), ty)
            }
            a::ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                let comparison = ["==", "!=", "<", ">", "<=", ">="].contains(operator);
                let logical = ["&&", "||"].contains(operator);
                let shift = ["<<", ">>"].contains(operator);
                let hint = if logical {
                    Some(&Type::Bool)
                } else if comparison {
                    None
                } else {
                    expected
                };
                let inferred = if hint.is_none() && !shift {
                    self.known_type(right)
                } else {
                    None
                };
                let hint = hint.or(inferred.as_ref());
                let left = self.expr(left, hint, true)?;
                let right = self.expr(right, if shift { None } else { Some(&left.ty) }, true)?;
                let valid = if logical {
                    left.ty == Type::Bool
                } else if shift {
                    matches!(left.ty, Type::Integer(_)) && matches!(right.ty, Type::Integer(_))
                } else if ["&", "|", "^"].contains(operator) {
                    matches!(left.ty, Type::Integer(_))
                } else if comparison {
                    left.ty.numeric() || matches!(left.ty, Type::Bool | Type::Char)
                } else {
                    left.ty.numeric()
                };
                if !valid {
                    return Err(error(
                        span,
                        "E0311",
                        "binary operator is not available for these types",
                    ));
                }
                let ty = if comparison || logical {
                    Type::Bool
                } else {
                    left.ty.clone()
                };
                (
                    h::ExprKind::Binary(operator, Box::new(left), Box::new(right)),
                    ty,
                )
            }
            a::ExprKind::Cast { value, ty } => {
                let target = resolve_type_with(ty, self.constants, self.nominals)?;
                let value = self.expr(value, None, true)?;
                let valid = value.ty.numeric() && target.numeric()
                    || matches!(value.ty, Type::Bool | Type::Char)
                        && matches!(target, Type::Integer(_))
                    || value.ty == Type::Integer(I::U8) && target == Type::Char;
                if !valid {
                    return Err(error(span, "E0312", "invalid cast"));
                }
                (h::ExprKind::Cast(Box::new(value)), target)
            }
            a::ExprKind::Call {
                callee,
                types,
                arguments,
            } => {
                if types.is_empty() {
                    if let Some(value) = self.enum_call(callee, arguments, expected, span)? {
                        return Ok(value);
                    }
                }
                if let Some(value) = self.instance_call(callee, types, arguments, expected, span)? {
                    return Ok(value);
                }
                let (name, mut generic_arguments) = match &callee.kind {
                    a::ExprKind::Name(name) => (name.clone(), Vec::new()),
                    _ => methods::static_call(callee).ok_or_else(|| {
                        error(
                            span,
                            "E0900",
                            "instance/function pointer calls require further checking",
                        )
                    })?,
                };
                if let a::ExprKind::Member { value, .. } = &callee.kind {
                    if let a::ExprKind::Name(owner) = &value.kind {
                        if self.lookup(owner).is_some() {
                            return Err(error(
                                span,
                                "E0900",
                                "instance calls require receiver checking",
                            ));
                        }
                    }
                }
                generic_arguments.extend(types.iter().cloned());
                let types = &generic_arguments;
                let name = &name;
                if self
                    .private_functions
                    .get(name)
                    .is_some_and(|source| *source != span.source)
                {
                    return Err(error(
                        span,
                        "E0352",
                        "private function is not accessible from this module",
                    ));
                }
                if self.lookup(name).is_some() {
                    return Err(error(callee.span, "E0313", "local binding shadows this function; callable values require function-pointer checking"));
                }
                if self.mono.borrow().templates.contains_key(name) {
                    return self.generic_call(name, types, arguments, expected, span);
                }
                if !types.is_empty() {
                    return Err(error(span, "E0370", "function has no generic parameters"));
                }
                let signature = self.signatures.get(name).cloned().ok_or_else(|| {
                    error(callee.span, "E0310", format!("unknown function '{name}'"))
                })?;
                if signature.parameters.len() != arguments.len() {
                    return Err(error(span, "E0313", "wrong number of arguments"));
                }
                let mut checked = Vec::new();
                for (arg, ty) in arguments.iter().zip(&signature.parameters) {
                    checked.push(self.expr(arg, Some(ty), true)?);
                }
                return self.call_entry(signature, arguments, checked, span);
            }
            a::ExprKind::Construct { ty, fields } => {
                return self.record(ty, fields, expected, span)
            }
            a::ExprKind::Tuple(values) => {
                if values.is_empty() {
                    (h::ExprKind::Value(Value::Unit), Type::Unit)
                } else {
                    let hints = if let Some(Type::Tuple(types)) = expected {
                        Some(types)
                    } else {
                        None
                    };
                    let mut checked = Vec::new();
                    for (index, value) in values.iter().enumerate() {
                        checked.push(self.expr(value, hints.and_then(|v| v.get(index)), true)?);
                    }
                    let ty = Type::Tuple(checked.iter().map(|e| e.ty.clone()).collect());
                    (h::ExprKind::Tuple(checked), ty)
                }
            }
            a::ExprKind::Array(values) => {
                let mut element = if let Some(Type::Array(element, _)) = expected {
                    Some((**element).clone())
                } else {
                    None
                };
                let mut checked = Vec::new();
                for value in values {
                    let value = self.expr(value, element.as_ref(), true)?;
                    element = Some(value.ty.clone());
                    checked.push(value);
                }
                let element = element
                    .ok_or_else(|| error(span, "E0314", "empty array requires an element type"))?;
                (
                    h::ExprKind::Array(checked),
                    Type::Array(Box::new(element), values.len() as u64),
                )
            }
            a::ExprKind::Index { value, index } => {
                let value = self.expr(value, None, true)?;
                let index = self.expr(index, Some(&Type::Integer(I::Usize)), true)?;
                let Type::Array(element, _) = &value.ty else {
                    return Err(error(span, "E0315", "indexing requires a fixed array"));
                };
                let ty = (**element).clone();
                (h::ExprKind::Index(Box::new(value), Box::new(index)), ty)
            }
            a::ExprKind::Member { value, name } => {
                if let Some(value) = self.enum_unit(value, name, expected, span)? {
                    return Ok(value);
                }
                let value = self.expr(value, None, true)?;
                let (index, ty) = match &value.ty {
                    Type::Tuple(fields) => {
                        let index = name.parse::<usize>().map_err(|_| {
                            error(span, "E0315", "tuple field must be an integer index")
                        })?;
                        let ty = fields
                            .get(index)
                            .cloned()
                            .ok_or_else(|| error(span, "E0315", "tuple field out of bounds"))?;
                        (index, ty)
                    }
                    Type::Record { fields, .. } => {
                        let index = fields
                            .iter()
                            .position(|field| field.name == *name)
                            .ok_or_else(|| error(span, "E0351", "unknown struct field"))?;
                        let field = &fields[index];
                        if field.private && field.source != span.source {
                            return Err(error(
                                span,
                                "E0352",
                                "private field is not accessible from this module",
                            ));
                        }
                        (index, field.ty.clone())
                    }
                    _ => return Err(error(span, "E0900", "member access requires an aggregate")),
                };
                (h::ExprKind::Field(Box::new(value), index), ty)
            }
            a::ExprKind::Assign {
                operator,
                place,
                value,
            } => {
                let place = self.expr(place, None, true)?;
                if let h::ExprKind::View(local) = place.kind {
                    // Only explicit dereference writes the whole target.
                    fn explicit(v: &a::Expr) -> bool {
                        match &v.kind {
                            a::ExprKind::Unary { operator: "*", .. } => true,
                            a::ExprKind::Group(v) => explicit(v),
                            _ => false,
                        }
                    }
                    let a::ExprKind::Assign { place: source, .. } = &source.kind else {
                        unreachable!()
                    };
                    if !explicit(source) {
                        if !self.locals[local].mutable {
                            return Err(error(
                                span,
                                "E0382",
                                "rebinding requires a mutable view binding",
                            ));
                        }
                        if *operator != "=" {
                            return Err(error(
                                span,
                                "E0382",
                                "use *view for compound assignment to the target",
                            ));
                        }
                        let value = self.expr(value, Some(&place.ty), true)?;
                        if !views::place(&value) {
                            return Err(error(
                                span,
                                "E0382",
                                "view rebinding requires an existing place",
                            ));
                        }
                        if self.locals[local].view == Some(crate::mir::loans::Access::Mutable) {
                            self.mutable_place(&value)?;
                        }
                        return Ok(h::Expr {
                            kind: h::ExprKind::Borrow {
                                local,
                                place: Box::new(value),
                            },
                            ty: Type::Unit,
                            span,
                        });
                    }
                }
                self.mutable_place(&place)?;
                let shift = ["<<=", ">>="].contains(operator);
                let value = self.expr(value, if shift { None } else { Some(&place.ty) }, true)?;
                if *operator != "=" {
                    let integer_only = ["&=", "|=", "^=", "<<=", ">>="].contains(operator);
                    if !place.ty.numeric()
                        || integer_only && !matches!(place.ty, Type::Integer(_))
                        || shift && !matches!(value.ty, Type::Integer(_))
                    {
                        return Err(error(span, "E0311", "invalid compound assignment"));
                    }
                }
                (
                    h::ExprKind::Assign {
                        place: Box::new(place),
                        operator,
                        value: Box::new(value),
                    },
                    Type::Unit,
                )
            }
            a::ExprKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                let condition = Box::new(self.expr(condition, Some(&Type::Bool), true)?);
                let yes = self.block(then_block, expected, value_context)?;
                let branch_hint = if yes.diverges {
                    expected
                } else if value_context {
                    Some(&yes.result)
                } else {
                    None
                };
                let no = else_branch
                    .as_ref()
                    .map(|branch| self.branch(branch, branch_hint, value_context))
                    .transpose()?;
                let ty = if !value_context {
                    Type::Unit
                } else if yes.diverges {
                    no.as_ref().map_or(Type::Unit, |b| b.result.clone())
                } else {
                    yes.result.clone()
                };
                if no.is_none() && value_context && ty != Type::Unit {
                    return Err(error(span, "E0316", "value-producing if requires else"));
                }
                (h::ExprKind::If { condition, yes, no }, ty)
            }
            a::ExprKind::While { condition, body } => {
                let condition = Box::new(self.expr(condition, Some(&Type::Bool), true)?);
                self.loops += 1;
                let checked = self.block(body, None, false);
                self.loops -= 1;
                let body = checked?;
                (h::ExprKind::While { condition, body }, Type::Unit)
            }
            _ => {
                return Err(error(
                    span,
                    "E0900",
                    "this expression requires a later checker stage; it is not executed",
                ))
            }
        };
        let checked = h::Expr { kind, ty, span };
        if !expression_diverges(&checked) {
            if let Some(expected) = expected {
                same(expected, &checked.ty, span)?;
            }
        }
        Ok(checked)
    }

    fn branch(
        &mut self,
        branch: &a::Expr,
        expected: Option<&Type>,
        value_context: bool,
    ) -> Result<h::Block> {
        if let a::ExprKind::Branch(block) = &branch.kind {
            return self.block(block, expected, value_context);
        }
        let expression = self.expr(branch, expected, value_context)?;
        let result = expression.ty.clone();
        let diverges = expression_diverges(&expression);
        Ok(h::Block {
            storage: vec![],
            statements: vec![h::Statement::Evaluate(expression)],
            result,
            diverges,
            span: branch.span,
        })
    }

    fn known_type(&self, value: &a::Expr) -> Option<Type> {
        match &value.kind {
            a::ExprKind::Name(name) => self
                .lookup(name)
                .map(|id| self.locals[id].ty.clone())
                .or_else(|| self.constants.get(name).map(|(_, ty)| ty.clone())),
            a::ExprKind::Group(value) | a::ExprKind::Unary { value, .. } => self.known_type(value),
            a::ExprKind::Cast { ty, .. } => {
                resolve_type_with(ty, self.constants, self.nominals).ok()
            }
            a::ExprKind::Call { callee, .. } => {
                if let a::ExprKind::Name(name) = &callee.kind {
                    self.signatures.get(name).map(|s| s.result.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn mutable_place(&self, place: &h::Expr) -> Result<()> {
        match &place.kind {
            h::ExprKind::Local(id) if self.locals[*id].mutable => Ok(()),
            h::ExprKind::View(id)
                if self.locals[*id].view == Some(crate::mir::loans::Access::Mutable) =>
            {
                Ok(())
            }
            h::ExprKind::Index(base, _)
            | h::ExprKind::Field(base, _)
            | h::ExprKind::Payload(base, _) => self.mutable_place(base),
            _ => Err(error(
                place.span,
                "E0317",
                "assignment requires a mutable place",
            )),
        }
    }
}

pub fn resolve_type(ty: &a::Type) -> Result<Type> {
    resolve_type_in(ty, &BTreeMap::new())
}

pub(crate) fn resolve_type_in(
    ty: &a::Type,
    constants: &crate::constants::Constants,
) -> Result<Type> {
    resolve_type_with(ty, constants, &records::Nominals::default())
}

fn resolve_type_with(
    ty: &a::Type,
    constants: &crate::constants::Constants,
    nominals: &records::Nominals,
) -> Result<Type> {
    let resolved = match &ty.kind {
        a::TypeKind::Named { path, arguments } if path.len() == 1 => {
            if !arguments.is_empty() {
                return nominals.resolve(&path[0], arguments, constants, ty.span);
            }
            match path[0].as_str() {
                "bool" => Type::Bool,
                "char" => Type::Char,
                "str" => Type::Str,
                "f32" => Type::F32,
                "f64" => Type::F64,
                "i8" => Type::Integer(I::I8),
                "i16" => Type::Integer(I::I16),
                "i32" => Type::Integer(I::I32),
                "i64" => Type::Integer(I::I64),
                "i128" => Type::Integer(I::I128),
                "isize" => Type::Integer(I::Isize),
                "u8" => Type::Integer(I::U8),
                "u16" => Type::Integer(I::U16),
                "u32" => Type::Integer(I::U32),
                "u64" => Type::Integer(I::U64),
                "u128" => Type::Integer(I::U128),
                "usize" => Type::Integer(I::Usize),
                name if nominals.contains(name) => {
                    nominals.resolve(name, arguments, constants, ty.span)?
                }
                _ => {
                    return Err(error(
                        ty.span,
                        "E0900",
                        "named type resolution is not implemented yet",
                    ))
                }
            }
        }
        a::TypeKind::Tuple(elements) => {
            if elements.is_empty() {
                Type::Unit
            } else {
                Type::Tuple(
                    elements
                        .iter()
                        .map(|ty| resolve_type_with(ty, constants, nominals))
                        .collect::<Result<_>>()?,
                )
            }
        }
        a::TypeKind::Array { element, length } => {
            let Value::Integer(length) =
                constant_value(length, &Type::Integer(I::Usize), constants)?
            else {
                unreachable!()
            };
            let size = u64::try_from(length.bits())
                .map_err(|_| error(ty.span, "E0318", "array length does not fit usize"))?;
            Type::Array(
                Box::new(resolve_type_with(element, constants, nominals)?),
                size,
            )
        }
        _ => {
            return Err(error(
                ty.span,
                "E0900",
                "this type requires a later checker stage",
            ))
        }
    };
    crate::llvm::layout(&resolved).map_err(|message| error(ty.span, "E0318", message))?;
    Ok(resolved)
}

pub(crate) fn constant_value(
    value: &a::Expr,
    ty: &Type,
    constants: &crate::constants::Constants,
) -> Result<Value> {
    let signatures = BTreeMap::new();
    let mut checker = Checker {
        signatures: &signatures,
        mono: &std::cell::RefCell::new(generics::Monomorphization::default()),
        constants,
        pattern_views: false,
        continuations: vec![],
        value_uses: BTreeMap::new(),
        nominals: &records::Nominals::default(),
        private_functions: &BTreeMap::new(),
        scopes: vec![BTreeMap::new()],
        locals: Vec::new(),
        result: ty.clone(),
        loops: 0,
    };
    let expression = checker.expr(value, Some(ty), true)?;
    let function = h::Function {
        name: "constant".into(),
        parameters: Vec::new(),
        locals: checker.locals,
        result: ty.clone(),
        body: h::Block {
            storage: vec![],
            statements: vec![h::Statement::Return(Some(expression))],
            result: Type::Unit,
            diverges: true,
            span: value.span,
        },
        span: value.span,
    };
    let program = h::Program {
        functions: vec![function],
        main: None,
    };
    crate::interpret::execute(
        &program,
        0,
        Vec::new(),
        nether_semantics::OverflowChecks::Checked,
        100_000,
    )
    .map_err(|trap| {
        error(
            trap.span,
            "E0332",
            format!("constant evaluation failed: {:?}", trap.kind),
        )
    })
}

fn literal(
    value: &a::Literal,
    expected: Option<&Type>,
    negative: bool,
    span: Span,
) -> Result<(h::ExprKind, Type)> {
    let (value, ty) = match value {
        a::Literal::Integer(text) => {
            let ty = match expected {
                Some(Type::Integer(ty)) => *ty,
                None => I::I32,
                _ => {
                    return Err(error(
                        span,
                        "E0319",
                        "integer literal requires an integer context",
                    ))
                }
            };
            let value = Integer::literal(ty, magnitude(text, span)?, negative)
                .map_err(|_| error(span, "E0319", "integer literal out of range"))?;
            (Value::Integer(value), Type::Integer(ty))
        }
        a::Literal::Float(text) => {
            let text = text.replace('_', "");
            if expected == Some(&Type::F32) {
                (
                    Value::F32(
                        text.parse()
                            .map_err(|_| error(span, "E0319", "invalid f32 literal"))?,
                    ),
                    Type::F32,
                )
            } else {
                (
                    Value::F64(
                        text.parse()
                            .map_err(|_| error(span, "E0319", "invalid f64 literal"))?,
                    ),
                    Type::F64,
                )
            }
        }
        a::Literal::Bool(value) => (Value::Bool(*value), Type::Bool),
        a::Literal::Char(value) => (Value::Char(*value), Type::Char),
        a::Literal::String(value) => (Value::Str(value.clone()), Type::Str),
    };
    if let Some(expected) = expected {
        same(expected, &ty, span)?;
    }
    Ok((h::ExprKind::Value(value), ty))
}
fn magnitude(text: &str, span: Span) -> Result<u128> {
    let text = text.replace('_', "");
    let (digits, base) = if let Some(s) = text.strip_prefix("0x") {
        (s, 16)
    } else if let Some(s) = text.strip_prefix("0o") {
        (s, 8)
    } else if let Some(s) = text.strip_prefix("0b") {
        (s, 2)
    } else {
        (text.as_str(), 10)
    };
    u128::from_str_radix(digits, base)
        .map_err(|_| error(span, "E0319", "integer literal exceeds 128 bits"))
}
fn expression_diverges(expr: &h::Expr) -> bool {
    match &expr.kind {
        h::ExprKind::Block(block) => block.diverges,
        h::ExprKind::Match(arms) => arms.iter().all(|arm| arm.body.diverges),
        h::ExprKind::If {
            yes, no: Some(no), ..
        } => yes.diverges && no.diverges,
        _ => false,
    }
}
fn same(expected: &Type, actual: &Type, span: Span) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(error(
            span,
            "E0320",
            format!("expected {expected:?}, found {actual:?}"),
        ))
    }
}
fn error(span: Span, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message, span)
}

fn check_declaration(
    declaration: &a::Function,
    signature: &Signature,
    signatures: &BTreeMap<String, Signature>,
    constants: &crate::constants::Constants,
    nominals: &records::Nominals,
    mono: &std::cell::RefCell<generics::Monomorphization>,
    private_functions: &BTreeMap<String, nether_frontend::source::SourceId>,
) -> Result<h::Function> {
    let mut checker = Checker {
        signatures,
        mono,
        constants,
        nominals,
        private_functions,
        pattern_views: false,
        continuations: vec![],
        value_uses: declaration
            .body
            .as_ref()
            .map(uses::names)
            .unwrap_or_default(),
        scopes: vec![BTreeMap::new()],
        locals: Vec::new(),
        result: signature.result.clone(),
        loops: 0,
    };
    (|| {
        let mut parameters = Vec::new();
        let mut prelude = Vec::new();
        for (parameter_index, (parameter, ty)) in declaration
            .parameters
            .iter()
            .zip(&signature.parameters)
            .enumerate()
        {
            let mode = signature.modes[parameter_index];
            let id = checker.locals.len();
            checker.locals.push(h::Local {
                view: mode,
                name: "<parameter>".into(),
                ty: ty.clone(),
                mutable: false,
                span: parameter.span,
            });
            parameters.push(id);
            let value = h::Expr {
                kind: if mode.is_some() {
                    h::ExprKind::View(id)
                } else {
                    h::ExprKind::Local(id)
                },
                ty: ty.clone(),
                span: parameter.span,
            };
            let first = checker.locals.len();
            let (_, irrefutable, _) = checker.pattern(
                &if mode.is_some() {
                    views::inferred_pattern(&parameter.pattern, parameter.mutable)
                } else {
                    parameter.pattern.clone()
                },
                &value,
                &mut prelude,
            )?;
            if !irrefutable {
                return Err(error(
                    parameter.span,
                    "E0344",
                    "parameter requires an irrefutable pattern",
                ));
            }
            if parameter.mutable {
                for local in &mut checker.locals[first..] {
                    local.mutable = true;
                }
            }
        }
        let source_body = declaration
            .body
            .as_ref()
            .ok_or_else(|| error(declaration.span, "E0300", "function body required"))?;
        let mut body = checker.block(source_body, None, false)?;
        body.storage = (0..checker.locals.len()).collect();
        prelude.append(&mut body.statements);
        body.statements = prelude;
        if signature.result != Type::Unit && !body.diverges {
            return Err(error(
                declaration.span,
                "E0302",
                "non-unit function must explicitly return on every path",
            ));
        }
        Ok(h::Function {
            name: declaration.name.clone(),
            parameters,
            locals: checker.locals,
            result: signature.result.clone(),
            body,
            span: declaration.span,
        })
    })()
}
