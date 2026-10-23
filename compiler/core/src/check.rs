use crate::hir::{self as h, Type, Value};
use nether_frontend::{
    ast as a,
    source::{Diagnostic, Span},
};
use nether_semantics::{Integer, IntegerType as I};
use std::collections::BTreeMap;
mod classes;
mod classes_init;
mod constructors;
mod consume;
mod entries;
mod enums;
mod function_values;
mod future;
mod generics;
mod inheritance;
mod interfaces;
mod lifecycle;
mod methods;
mod patterns;
mod privacy;
mod records;
mod storage;
mod uses;
mod views;

type Result<T> = std::result::Result<T, Diagnostic>;
/// Parameters inferred as consuming, by declaration name (without entry suffix).
type Consuming = std::rc::Rc<std::cell::RefCell<BTreeMap<String, Vec<bool>>>>;
#[derive(Clone)]
struct Signature {
    modes: Vec<Option<crate::mir::loans::Access>>,
    id: usize,
    parameters: Vec<Type>,
    result: Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReturnInfo {
    complete: bool,
    parameters: Vec<Type>,
    modes: Vec<Option<crate::mir::loans::Access>>,
    ty: Type,
    view: Option<crate::mir::returns::Contract>,
    storage: crate::mir::returns::StorageContract,
    writes: crate::mir::storage_writes::Contract,
}
type ReturnContracts = BTreeMap<String, ReturnInfo>;
enum Pass {
    Ready(h::Program),
    Retry(ReturnContracts),
}
pub fn check(module: &a::Module) -> std::result::Result<h::Program, Vec<Diagnostic>> {
    inheritance::validate(module).map_err(|e| vec![e])?;
    let mut contracts = ReturnContracts::new();
    let consuming = Consuming::default();
    for _ in 0..64 {
        match check_pass(module, &contracts, &consuming)? {
            Pass::Ready(program) => return Ok(program),
            Pass::Retry(next) => contracts = next,
        }
    }
    Err(vec![error(
        module.items.first().map(|i| i.span).unwrap_or(Span {
            source: nether_frontend::source::SourceId(0),
            start: 0,
            end: 0,
        }),
        "E0395",
        "result contracts did not converge",
    )])
}
fn check_pass(
    module: &a::Module,
    contracts: &ReturnContracts,
    consuming: &Consuming,
) -> std::result::Result<Pass, Vec<Diagnostic>> {
    let consuming_before = consuming.borrow().clone();
    let class_module = classes::prepare(module).map_err(|e| vec![e])?;
    let lifecycle_module = lifecycle::prepare(&class_module).map_err(|e| vec![e])?;
    let prepared = methods::collect(&lifecycle_module);
    let module = &prepared;
    let private_functions: BTreeMap<_, _> = module
        .items
        .iter()
        .filter_map(|item| match &item.kind {
            a::ItemKind::Function(function) if item.private => {
                Some((function.name.clone(), function.span))
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
                | a::ItemKind::Aggregate { .. }
                | a::ItemKind::Enum { .. }
                | a::ItemKind::Interface { .. }
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
                modes: methods::parameter_modes(function, &parameters),
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
        result_contracts: contracts.clone(),
        next_id: declarations.len(),
        consuming: consuming.clone(),
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
    let mut functions = BTreeMap::new();
    loop {
        let before = mono.borrow().next_id;
        let mut deferred = Vec::new();
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
                Ok(function) => {
                    functions.insert(signature.id, function);
                }
                Err(e) if e.code == function_values::PENDING_CALLABLE => {
                    deferred.push((signature, declaration, e));
                }
                Err(e) => errors.push(e),
            }
            work.extend(mono.borrow_mut().pending.drain(..));
        }
        if deferred.is_empty() {
            break;
        }
        // A returned pointer can name a generic entry which its producer has
        // not instantiated yet in this contract pass. Check queued producers
        // first, then retry callers; require progress to avoid an infinite retry.
        if mono.borrow().next_id == before {
            errors.extend(deferred.into_iter().map(|(_, _, e)| e));
            break;
        }
        work.extend(deferred.into_iter().map(|(s, d, _)| (s, d)));
    }
    // Newly inferred consuming parameters change entry modes: errors of this
    // pass may be artifacts of the provisional borrowed entries.
    if *consuming.borrow() != consuming_before {
        return Ok(Pass::Retry(contracts.clone()));
    }
    let functions = functions.into_values().collect::<Vec<_>>();
    if !mono.borrow().result_requests.is_empty() {
        let mut next = contracts.clone();
        let mono_ref = mono.borrow();
        for (key, ty) in &mono_ref.result_requests {
            let signature = signatures
                .values()
                .chain(mono_ref.instances.values())
                .chain(mono_ref.entries.values())
                .find(|s| {
                    mono_ref
                        .bodies
                        .get(&s.id)
                        .is_some_and(|d| entries::result_key(d, s) == *key)
                });
            let Some(signature) = signature else { continue };
            let info = next.entry(key.clone()).or_insert_with(|| ReturnInfo {
                complete: false,
                parameters: signature.parameters.clone(),
                modes: signature.modes.clone(),
                ty: signature.result.clone(),
                view: None,
                storage: Vec::new(),
                writes: Vec::new(),
            });
            info.ty = storage::union(&info.ty, ty, mono_ref.bodies[&signature.id].span)
                .map_err(|e| vec![e])?;
            info.complete = false;
        }
        if &next != contracts {
            return Ok(Pass::Retry(next));
        }
    }
    if errors.is_empty() {
        let mut audit_work: std::collections::VecDeque<_> =
            mono.borrow_mut().audits.drain(..).collect();
        let audit_mono = std::cell::RefCell::new(generics::Monomorphization {
            templates: mono.borrow().templates.clone(),
            result_contracts: contracts.clone(),
            consuming: consuming.clone(),
            bodies: mono.borrow().bodies.clone(),
            next_id: mono.borrow().next_id,
            auditing: true,
            ..Default::default()
        });
        loop {
            let before = audit_mono.borrow().next_id;
            let mut deferred = Vec::new();
            while let Some((signature, declaration)) = audit_work.pop_front() {
                match check_declaration(
                    &declaration,
                    &signature,
                    &signatures,
                    &constants,
                    &nominals,
                    &audit_mono,
                    &private_functions,
                ) {
                    Ok(_) => {}
                    Err(diagnostic) if diagnostic.code == function_values::PENDING_CALLABLE => {
                        deferred.push((signature, declaration, diagnostic));
                    }
                    Err(diagnostic) => {
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
                }
                audit_work.extend(audit_mono.borrow_mut().pending.drain(..));
            }
            if !errors.is_empty() || deferred.is_empty() {
                break;
            }
            if audit_mono.borrow().next_id == before {
                errors.extend(deferred.into_iter().map(|(_, _, e)| e));
                break;
            }
            audit_work.extend(deferred.into_iter().map(|(s, d, _)| (s, d)));
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
            for function in &functions {
                for local in &function.locals {
                    nominals.register_storage(&local.ty)
                }
                nominals.register_storage(&function.result);
            }
            let mut program = h::Program {
                classes: nominals.classes(),
                functions,
                main,
            };
            let mut typed = contracts.clone();
            for (id, function) in program.functions.iter_mut().enumerate() {
                let signature = Signature {
                    id,
                    modes: function
                        .parameters
                        .iter()
                        .map(|p| function.locals[*p].view)
                        .collect(),
                    parameters: function
                        .parameters
                        .iter()
                        .map(|p| function.locals[*p].ty.clone())
                        .collect(),
                    result: function.result.clone(),
                };
                let key = mono.borrow().entry_keys[&id].clone();
                let mut info = contracts
                    .get(&key)
                    .filter(|info| info.ty == function.result)
                    .cloned()
                    .unwrap_or_else(|| ReturnInfo {
                        complete: false,
                        parameters: signature.parameters.clone(),
                        modes: signature.modes.clone(),
                        ty: function.result.clone(),
                        view: None,
                        storage: Vec::new(),
                        writes: Vec::new(),
                    });
                info.parameters = signature.parameters.clone();
                info.modes = signature.modes.clone();
                function.storage_writes = info.writes.clone();
                typed.insert(key, info);
            }
            if typed.keys().any(|key| !contracts.contains_key(key))
                || typed.iter().any(|(key, info)| {
                    contracts.get(key).is_some_and(|old| {
                        old.ty != info.ty
                            || old.parameters != info.parameters
                            || old.modes != info.modes
                    })
                })
            {
                return Ok(Pass::Retry(typed));
            }
            let mir = crate::mir::lower(&program);
            let inferred = crate::mir::returns::infer(&mir).map_err(|e| {
                let mut diagnostic = error(program.functions[e.function].span, "E0395", e.message);
                if let Some(origin) = e.origin {
                    diagnostic = diagnostic.with_note(
                        origin,
                        "this owner ends its lifetime when the function exits",
                    );
                }
                vec![diagnostic]
            })?;
            let storage = crate::mir::returns::infer_storage(&mir, &inferred).map_err(|e| {
                let mut diagnostic = error(program.functions[e.function].span, "E0395", e.message);
                if let Some(origin) = e.origin {
                    diagnostic = diagnostic.with_note(
                        origin,
                        "this owner ends its lifetime when the function exits",
                    );
                }
                vec![diagnostic]
            })?;
            let writes = crate::mir::storage_writes::infer(&mir).map_err(|e| {
                let mut diagnostic = error(program.functions[e.function].span, "E0396", e.message);
                if let Some(origin) = e.origin {
                    diagnostic = diagnostic
                        .with_note(origin, "this owner cannot outlive the caller's storage");
                }
                vec![diagnostic]
            })?;
            let mut next = contracts.clone();
            for (id, ((function, contract), storage)) in program
                .functions
                .iter_mut()
                .zip(inferred)
                .zip(storage)
                .enumerate()
            {
                let signature = Signature {
                    id,
                    modes: function
                        .parameters
                        .iter()
                        .map(|p| function.locals[*p].view)
                        .collect(),
                    parameters: function
                        .parameters
                        .iter()
                        .map(|p| function.locals[*p].ty.clone())
                        .collect(),
                    result: function.result.clone(),
                };
                next.insert(
                    mono.borrow().entry_keys[&id].clone(),
                    ReturnInfo {
                        complete: true,
                        parameters: signature.parameters.clone(),
                        modes: signature.modes.clone(),
                        ty: function.result.clone(),
                        view: contract.clone(),
                        storage,
                        writes: writes[id].clone(),
                    },
                );
                function.result_view = contract;
                function.storage_writes = writes[id].clone();
            }
            if &next != contracts {
                return Ok(Pass::Retry(next));
            }
            let mir = crate::mir::lower(&program);
            classes_init::validate(&mir).map_err(|e| vec![e])?;
            let mut errors: Vec<_> = mir
                .functions
                .iter()
                .flat_map(|f| {
                    crate::mir::borrow_check::check(f)
                        .into_iter()
                        .map(|e| crate::mir::borrow_check::diagnostic(f, e, "E0381"))
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
                lifecycle::validate(&program).map_err(|e| vec![e])?;
                Ok(Pass::Ready(program))
            } else {
                Err(errors)
            }
        }
    } else {
        Err(errors)
    }
}

#[derive(Clone)]
struct Checker<'a> {
    member_owner: Option<String>,
    storage_hints: storage::Modes,
    storage_requests: storage::Modes,
    storage_roots: std::collections::BTreeSet<usize>,
    storage_sources: BTreeMap<usize, Vec<h::Expr>>,
    storage_parameters: std::collections::BTreeSet<usize>,
    signatures: &'a BTreeMap<String, Signature>,
    mono: &'a std::cell::RefCell<generics::Monomorphization>,
    constants: &'a BTreeMap<String, (Value, Type)>,
    nominals: &'a records::Nominals,
    private_functions: &'a BTreeMap<String, Span>,
    scopes: Vec<BTreeMap<String, usize>>,
    locals: Vec<h::Local>,
    result: Type,
    actual_result: Option<Type>,
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
                    let hint = self.storage_hints.get(&storage::key(pattern.span)).cloned();
                    if let (Some(annotation), Some(hint)) = (&ty, &hint) {
                        same(annotation, hint, pattern.span)?;
                    }
                    let value = self.expr(value, hint.as_ref().or(ty.as_ref()), true)?;
                    if let Some(hint) = &hint {
                        let joined = storage::union(hint, &value.ty, pattern.span)?;
                        if joined != *hint {
                            self.storage_requests
                                .insert(storage::key(pattern.span), joined);
                        } else if value.ty != *hint && !self.request_storage_mode(&value, hint)? {
                            self.same_storage_mode(hint, &value.ty, value.span)?;
                        }
                    }
                    // NMM §7.1: a binding of an existing non-Copy place is an alias
                    // (R, or W for `var`); it never becomes a move by absence of
                    // later uses. Ownership transfers only in owning contexts.
                    if !value.ty.copyable() && views::place(&value) {
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
                    self.storage_sources.insert(id, vec![value.clone()]);
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
                    if matches!(pattern.kind, a::PatternKind::Binding { by_ref: false, .. }) {
                        self.storage_roots.extend(names.values().copied());
                    }
                    self.scopes.last_mut().unwrap().extend(names);
                    diverges |= value_diverges;
                    h::Statement::Evaluate(h::Expr {
                        kind: h::ExprKind::Block(h::Block {
                            construction: vec![],
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
                        if let Some(expected) = expected {
                            self.request_storage_mode(&checked, expected)?;
                        }
                        result = checked.ty.clone();
                    }
                    diverges |= expression_diverges(&checked);
                    h::Statement::Evaluate(checked)
                }
                a::StatementKind::Return(value) => {
                    let expected = self.actual_result.as_ref().unwrap_or(&self.result).clone();
                    let value = value
                        .as_ref()
                        .map(|v| self.expr(v, Some(&expected), true))
                        .transpose()?;
                    if let Some(value) = &value {
                        if !expression_diverges(value) {
                            same(&self.result, &value.ty, value.span)?;
                            self.request_storage_mode(value, &expected)?;
                            if !self.actual_result.as_ref().is_some_and(|previous| {
                                previous.contains_views()
                                    && !value.ty.contains_views()
                                    && !self.mono.borrow().pending_results.is_empty()
                            }) {
                                self.actual_result =
                                    Some(if let Some(previous) = &self.actual_result {
                                        storage::union(previous, &value.ty, value.span)?
                                    } else {
                                        value.ty.clone()
                                    });
                            }
                        }
                    }
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
            construction: vec![],
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
        let expression_hint = self.storage_hints.get(&storage::key(span)).cloned();
        let expected = expression_hint.as_ref().or(expected);
        let (kind, ty) = match &source.kind {
            a::ExprKind::Match { value, arms } => {
                return self.match_expression(value, arms, expected, value_context, span);
            }
            a::ExprKind::Branch(block) => {
                let block = self.block(block, expected, value_context)?;
                let ty = block.result.clone();
                (h::ExprKind::Block(block), ty)
            }
            a::ExprKind::New { ty, arguments } => {
                return self.new_class(ty, arguments, expected, span)
            }
            a::ExprKind::Group(value) => return self.expr(value, expected, value_context),
            a::ExprKind::Literal(value) => literal(value, expected, false, span)?,
            a::ExprKind::Name(name) => {
                if self.lookup(name).is_none() {
                    if let Some(receiver) = self.lookup("this") {
                        if self
                            .nominals
                            .fields(&self.locals[receiver].ty)
                            .is_some_and(|fields| fields.iter().any(|f| f.name == *name))
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
                    if let Some(value) = self.function_value(name, &[], expected, span)? {
                        if let Some(expected) = expected {
                            same(expected, &value.ty, span)?;
                        }
                        return Ok(value);
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
                if matches!(left.ty, Type::Class { .. }) && ["==", "!="].contains(operator) {
                    if let Some(expected) = expected {
                        same(expected, &Type::Bool, span)?;
                    }
                    let identity = h::Expr {
                        kind: h::ExprKind::Identity(Box::new(left), Box::new(right)),
                        ty: Type::Bool,
                        span,
                    };
                    return Ok(if *operator == "==" {
                        identity
                    } else {
                        h::Expr {
                            kind: h::ExprKind::Unary("!", Box::new(identity)),
                            ty: Type::Bool,
                            span,
                        }
                    });
                }
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
                if matches!(&callee.kind, a::ExprKind::Name(name) if name=="same_identity" && self.lookup(name).is_none() && !self.signatures.contains_key(name) && !self.mono.borrow().templates.contains_key(name))
                {
                    if !types.is_empty() || arguments.len() != 2 {
                        return Err(error(
                            span,
                            "E0313",
                            "same_identity requires two class operands",
                        ));
                    }
                    let left = self.expr(&arguments[0], None, true)?;
                    let right = self.expr(&arguments[1], Some(&left.ty), true)?;
                    if !matches!(left.ty, Type::Class { .. }) {
                        return Err(error(
                            span,
                            "E0311",
                            "same_identity requires compatible class operands",
                        ));
                    }
                    if let Some(expected) = expected {
                        same(expected, &Type::Bool, span)?;
                    }
                    return Ok(h::Expr {
                        kind: h::ExprKind::Identity(Box::new(left), Box::new(right)),
                        ty: Type::Bool,
                        span,
                    });
                }
                if types.is_empty() {
                    if let Some(value) = self.enum_call(callee, arguments, expected, span)? {
                        return Ok(value);
                    }
                }
                if let Some(value) = self.instance_call(callee, types, arguments, expected, span)? {
                    return self.contextual_result(value, expected);
                }
                if let Some(value) = self.indirect_call(callee, types, arguments, span)? {
                    if let Some(expected) = expected {
                        same(expected, &value.ty, span)?;
                    }
                    return self.contextual_result(value, expected);
                }
                let (name, mut generic_arguments) = match &callee.kind {
                    a::ExprKind::Name(name) => (name.clone(), Vec::new()),
                    _ => methods::resolved_static_call(callee, self.nominals).ok_or_else(|| {
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
                self.check_function_access(name, span)?;
                if self.lookup(name).is_some() {
                    return Err(error(callee.span, "E0313", "local binding shadows this function; callable values require function-pointer checking"));
                }
                if self.mono.borrow().templates.contains_key(name) {
                    let value = self.generic_call(name, types, arguments, expected, span)?;
                    return self.contextual_result(value, expected);
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
                let value = self.call_entry(signature, arguments, checked, span)?;
                return self.contextual_result(value, expected);
            }
            a::ExprKind::Specialize { value, arguments } => {
                if let Some(pointer) =
                    self.static_function_value(value, arguments, expected, span)?
                {
                    return Ok(pointer);
                }
                let a::ExprKind::Name(name) = &value.kind else {
                    return Err(error(
                        span,
                        "E0900",
                        "specialized function values require a named function",
                    ));
                };
                if self.lookup(name).is_some() {
                    return Err(error(
                        span,
                        "E0390",
                        "stored function values cannot be specialized",
                    ));
                }
                return self
                    .function_value(name, arguments, expected, span)?
                    .ok_or_else(|| error(span, "E0310", "unknown function"));
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
                        checked.push(self.storage_value(value, hints.and_then(|v| v.get(index)))?);
                    }
                    let ty = Type::Tuple(checked.iter().map(|e| e.ty.clone()).collect());
                    (h::ExprKind::Tuple(checked), ty)
                }
            }
            a::ExprKind::Array(values) => return self.storage_array(values, expected, span),
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
                if let Some(pointer) = self.static_function_value(source, &[], expected, span)? {
                    return Ok(pointer);
                }
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
                    Type::Record { .. } | Type::Class { .. } => {
                        let fields = self.nominals.fields(&value.ty).unwrap();
                        let index = fields
                            .iter()
                            .position(|field| field.name == *name)
                            .ok_or_else(|| error(span, "E0351", "unknown struct field"))?;
                        let field = &fields[index];
                        self.check_field_access(&value.ty, field, span)?;
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
                if let h::ExprKind::StoredView(slot) = &place.kind {
                    if *operator != "=" {
                        return Err(error(
                            span,
                            "E0396",
                            "stored readonly view only supports rebinding",
                        ));
                    }
                    self.mutable_place(slot)?;
                    let value = self.storage_value(value, Some(&slot.ty))?;
                    self.same_storage_mode(&slot.ty, &value.ty, value.span)?;
                    return Ok(h::Expr {
                        kind: h::ExprKind::Assign {
                            place: slot.clone(),
                            operator,
                            value: Box::new(value),
                        },
                        ty: Type::Unit,
                        span,
                    });
                }
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
                    if explicit(source) && matches!(place.ty, Type::Class { .. }) {
                        return Err(error(
                            span,
                            "E0394",
                            "class view cannot replace its owning slot",
                        ));
                    }
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
                        self.same_storage_mode(&place.ty, &value.ty, value.span)?;
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
                        self.storage_sources
                            .entry(local)
                            .or_default()
                            .push(value.clone());
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
                let value = if *operator == "="
                    && self.infers_storage_place(&place)
                    && matches!(
                        place.kind,
                        h::ExprKind::Field(..) | h::ExprKind::Index(..) | h::ExprKind::Payload(..)
                    ) {
                    self.storage_assignment(value, &place)?
                } else {
                    self.expr(value, if shift { None } else { Some(&place.ty) }, true)?
                };
                if !shift && !self.request_storage_mode(&place, &value.ty)? {
                    self.same_storage_mode(&place.ty, &value.ty, value.span)?;
                }
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
                if let Some(no) = &no {
                    if !yes.diverges && !no.diverges {
                        let joined = storage::union(&yes.result, &no.result, span)?;
                        if !self.request_expression_mode(span, &yes.result, &joined)?
                            && !self.storage_requests.contains_key(&storage::key(span))
                        {
                            storage::same_mode(&joined, &yes.result, span)?;
                            storage::same_mode(&joined, &no.result, span)?;
                        }
                    }
                }
                let ty =
                    if value_context && !yes.diverges && no.as_ref().is_some_and(|b| !b.diverges) {
                        storage::union(&yes.result, &no.as_ref().unwrap().result, span)?
                    } else if !value_context {
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
        };
        let checked = storage::read(h::Expr { kind, ty, span });
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
            construction: vec![],
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
        if self.pending_result_place(place) {
            return Ok(());
        }
        match &place.kind {
            h::ExprKind::Local(id) if self.locals[*id].mutable => Ok(()),
            h::ExprKind::View(id)
                if self.locals[*id].view == Some(crate::mir::loans::Access::Mutable) =>
            {
                Ok(())
            }
            h::ExprKind::BorrowedCall { contract, .. }
                if contract.access == crate::mir::loans::Access::Mutable =>
            {
                Ok(())
            }
            h::ExprKind::Index(base, _)
            | h::ExprKind::Field(base, _)
            | h::ExprKind::Payload(base, _) => self.mutable_place(base),
            _ => {
                let mut diagnostic =
                    error(place.span, "E0317", "assignment requires a mutable place");
                if let h::ExprKind::View(id) = place.kind {
                    diagnostic = diagnostic.with_note(
                        self.locals[id].span,
                        "this view only grants readonly access",
                    );
                } else if matches!(place.kind, h::ExprKind::BorrowedCall { .. }) {
                    diagnostic =
                        diagnostic.with_note(place.span, "this call returns a readonly view");
                }
                Err(diagnostic)
            }
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
        a::TypeKind::Function { parameters, result } => {
            let parameters = parameters
                .iter()
                .map(|(mutable, ty)| Ok((*mutable, resolve_type_with(ty, constants, nominals)?)))
                .collect::<Result<Vec<_>>>()?;
            let result = resolve_type_with(result, constants, nominals)?;
            Type::Function {
                parameters,
                result: Box::new(result),
                contract: None,
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
    crate::layout::layout(&resolved).map_err(|message| error(ty.span, "E0318", message))?;
    Ok(resolved)
}

pub(crate) fn constant_value(
    value: &a::Expr,
    ty: &Type,
    constants: &crate::constants::Constants,
) -> Result<Value> {
    let signatures = BTreeMap::new();
    let mut checker = Checker {
        member_owner: None,
        storage_hints: Default::default(),
        storage_requests: Default::default(),
        storage_roots: Default::default(),
        storage_sources: Default::default(),
        storage_parameters: Default::default(),
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
        actual_result: None,
        loops: 0,
    };
    let expression = checker.expr(value, Some(ty), true)?;
    let function = h::Function {
        storage_writes: Vec::new(),
        result_view: None,
        name: "constant".into(),
        parameters: Vec::new(),
        locals: checker.locals,
        result: ty.clone(),
        body: h::Block {
            construction: vec![],
            storage: vec![],
            statements: vec![h::Statement::Return(Some(expression))],
            result: Type::Unit,
            diverges: true,
            span: value.span,
        },
        span: value.span,
    };
    let program = h::Program {
        classes: Default::default(),
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
    if expected.logical() == actual.logical() {
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
    private_functions: &BTreeMap<String, Span>,
) -> Result<h::Function> {
    let key = entries::result_key(declaration, signature);
    mono.borrow_mut()
        .entry_keys
        .insert(signature.id, key.clone());
    let initial = mono.borrow().clone();
    let mut hints = storage::Modes::new();
    if let Some(info) = initial.result_contracts.get(&key) {
        for (index, (parameter, ty)) in declaration
            .parameters
            .iter()
            .zip(&info.parameters)
            .enumerate()
        {
            hints.insert(
                storage::key(parameter.span),
                if info.modes[index].is_some() && signature.modes[index].is_none() {
                    Type::StoredView(Box::new(ty.clone()))
                } else {
                    ty.clone()
                },
            );
        }
    }
    for _ in 0..64 {
        let (result, requests) = check_declaration_modes(
            declaration,
            signature,
            signatures,
            constants,
            nominals,
            mono,
            private_functions,
            hints.clone(),
        );
        if requests.is_empty() {
            return result;
        }
        for (key, ty) in requests {
            hints.insert(key, ty);
        }
        // Provisional calls/destructor entries must not survive a representation
        // retry. They may have a different ABI from the stable local modes.
        let requests = mono.borrow().result_requests.clone();
        *mono.borrow_mut() = initial.clone();
        mono.borrow_mut().result_requests.extend(requests);
    }
    Err(error(
        declaration.span,
        "E0396",
        "local storage mode inference did not converge",
    ))
}

#[allow(clippy::too_many_arguments)]
fn check_declaration_modes(
    declaration: &a::Function,
    signature: &Signature,
    signatures: &BTreeMap<String, Signature>,
    constants: &crate::constants::Constants,
    nominals: &records::Nominals,
    mono: &std::cell::RefCell<generics::Monomorphization>,
    private_functions: &BTreeMap<String, Span>,
    hints: storage::Modes,
) -> (Result<h::Function>, storage::Modes) {
    let mut checker = Checker {
        member_owner: declaration
            .name
            .rsplit_once('.')
            .map(|(owner, _)| owner.to_owned()),
        storage_hints: hints,
        storage_requests: Default::default(),
        storage_roots: Default::default(),
        storage_sources: Default::default(),
        storage_parameters: Default::default(),
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
        result: mono
            .borrow()
            .result_contracts
            .get(&entries::result_key(declaration, signature))
            .map_or_else(|| signature.result.clone(), |i| i.ty.clone()),
        actual_result: None,
        loops: 0,
    };
    let result = (|| {
        let mut parameters = Vec::new();
        let mut prelude = Vec::new();
        // A plain parameter name is a view local bound to the parameter itself.
        let mut parameter_names = BTreeMap::new();
        for (parameter_index, (parameter, ty)) in declaration
            .parameters
            .iter()
            .zip(&signature.parameters)
            .enumerate()
        {
            let ty = checker
                .storage_hints
                .get(&storage::key(parameter.span))
                .unwrap_or(ty)
                .clone();
            let (ty, mode) = match ty {
                Type::StoredView(target) => (
                    *target,
                    Some(if parameter.mutable {
                        crate::mir::loans::Access::Mutable
                    } else {
                        crate::mir::loans::Access::Readonly
                    }),
                ),
                ty => (ty, signature.modes[parameter_index]),
            };
            let id = checker.locals.len();
            checker.locals.push(h::Local {
                view: mode,
                name: "<parameter>".into(),
                ty: ty.clone(),
                mutable: false,
                span: parameter.span,
            });
            parameters.push(id);
            checker.storage_roots.insert(id);
            checker.storage_parameters.insert(id);
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
            if checker.locals.len() == first + 1 {
                parameter_names.insert(first, id);
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
        for ty in checker
            .locals
            .iter()
            .map(|l| l.ty.clone())
            .collect::<Vec<_>>()
        {
            checker.schedule_destructors(&ty, declaration.span)?;
        }
        checker.schedule_destructors(&signature.result, declaration.span)?;
        let result = checker
            .actual_result
            .clone()
            .unwrap_or_else(|| signature.result.clone());
        let function = h::Function {
            storage_writes: Vec::new(),
            result_view: None,
            name: declaration.name.clone(),
            parameters,
            locals: std::mem::take(&mut checker.locals),
            result,
            body,
            span: declaration.span,
        };
        consume::record(&function, declaration, mono, &parameter_names)?;
        Ok(function)
    })();
    (result, checker.storage_requests)
}
