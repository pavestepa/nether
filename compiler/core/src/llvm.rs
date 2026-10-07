//! Baseline LLVM 18 IR for the checked Copy-value subset.
//! All potentially failing operations branch before producing LLVM poison.
use crate::hir::*;
use nether_frontend::source::Diagnostic;
use nether_semantics::{AggregateLayout, EnumLayout, Layout, OverflowChecks};
use std::collections::BTreeSet;
mod cfg;

pub fn emit(program: &Program, checks: OverflowChecks) -> Result<String, Diagnostic> {
    emit_mir(&crate::mir::lower(program), checks)
}

pub fn emit_mir(
    program: &crate::mir::Program,
    checks: OverflowChecks,
) -> Result<String, Diagnostic> {
    emit_mir_with_sources(program, checks, &[])
}

pub fn emit_with_sources(
    program: &Program,
    checks: OverflowChecks,
    sources: &[nether_frontend::source::Source],
) -> Result<String, Diagnostic> {
    emit_mir_with_sources(&crate::mir::lower(program), checks, sources)
}

pub fn emit_mir_with_sources(
    program: &crate::mir::Program,
    checks: OverflowChecks,
    sources: &[nether_frontend::source::Source],
) -> Result<String, Diagnostic> {
    let mut globals = Vec::new();
    let mut declarations = BTreeSet::from([
        "declare void @__nether_v1_report_panic(ptr)".to_owned(),
        "declare void @__nether_v1_abort_double_panic(ptr) noreturn".to_owned(),
    ]);
    let mut functions = Vec::new();
    for (id, function) in program.functions.iter().enumerate() {
        if let Some(error) = crate::mir::borrow_check::check(function).first() {
            return Err(Diagnostic::new(
                "E0381",
                error.message,
                function.blocks[error.block].span,
            ));
        }
        layout(&function.result)
            .map_err(|message| Diagnostic::new("E0400", message, function.span))?;
        for local in &function.locals {
            layout(&local.ty).map_err(|message| Diagnostic::new("E0400", message, local.span))?;
        }
        let mut emitter = Emitter {
            location: None,
            checks,
            unwind: None,
            next: 0,
            allocas: Vec::new(),
            body: Vec::new(),
            terminated: false,
            loops: Vec::new(),
            globals: &mut globals,
            declarations: &mut declarations,
        };
        for (index, local) in function.locals.iter().enumerate() {
            emitter.allocas.push(format!(
                "  %l{index} = alloca {}, align {}",
                if local.view.is_some() {
                    "ptr".into()
                } else {
                    ty(&local.ty)
                },
                if local.view.is_some() {
                    8
                } else {
                    layout(&local.ty).unwrap().align()
                }
            ));
        }
        for (index, local) in function.parameters.iter().enumerate() {
            if function.locals[*local].view.is_some() {
                emitter.line(format!("store ptr %a{index}, ptr %l{local}"));
                continue;
            }
            let value = emitter.value(format!(
                "load {}, ptr %a{index}",
                ty(&function.locals[*local].ty)
            ));
            emitter.line(format!(
                "store {} {value}, ptr %l{local}",
                ty(&function.locals[*local].ty)
            ));
        }
        if let Some((block, local)) = crate::mir::dataflow::initialization_errors(function).first()
        {
            return Err(Diagnostic::new(
                "E0401",
                format!("MIR reads uninitialized local {local}"),
                function.blocks[*block].span,
            ));
        }
        emitter.jump(&format!("bb{}", function.entry));
        for (id, block) in function.blocks.iter().enumerate() {
            emitter.location = sources.get(block.span.source.0).and_then(|source| {
                let (line, column) = source.location(block.span.start)?;
                Some((
                    source.name.clone(),
                    u32::try_from(line).ok()?,
                    u32::try_from(column).ok()?,
                ))
            });
            emitter.label(&format!("bb{id}"));
            emitter.mir_terminator(&block.terminator, function, block.span)?;
        }
        let args = (0..function.parameters.len())
            .map(|i| format!(", ptr %a{i}"))
            .collect::<String>();
        functions.push(format!(
            "define i1 @_N1_f{id}(ptr %panic, ptr %result{args}) {{\nentry:\n{}\n{}\n}}\n",
            emitter.allocas.join("\n"),
            emitter.body.join("\n")
        ));
    }
    let mut output=String::from("target triple = \"x86_64-unknown-linux-gnu\"\ntarget datalayout = \"e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128\"\n%Panic = type { i32, i32, ptr, i32, i32, i64, [192 x i8] }\n");
    output.push_str(&globals.join("\n"));
    output.push('\n');
    output.push_str(&declarations.into_iter().collect::<Vec<_>>().join("\n"));
    output.push('\n');
    output.push_str(&functions.join("\n"));
    if let Some(main) = program.main {
        let result = ty(&program.functions[main].result);
        let returned: String = if program.functions[main].result == Type::Unit {
            "  ret i32 0".into()
        } else {
            "  %exit = load i32, ptr %result\n  ret i32 %exit".into()
        };
        output.push_str(&format!("\ndefine i32 @main() {{\nentry:\n  %panic = alloca %Panic, align 8\n  store %Panic zeroinitializer, ptr %panic\n  %result = alloca {result}\n  %ok = call i1 @_N1_f{main}(ptr %panic, ptr %result)\n  br i1 %ok, label %success, label %failure\nsuccess:\n{returned}\nfailure:\n  call void @__nether_v1_report_panic(ptr %panic)\n  ret i32 101\n}}\n"));
    }
    Ok(output)
}

struct Emitter<'a> {
    location: Option<(String, u32, u32)>,
    checks: OverflowChecks,
    unwind: Option<String>,
    next: usize,
    allocas: Vec<String>,
    body: Vec<String>,
    terminated: bool,
    loops: Vec<(String, String)>,
    globals: &'a mut Vec<String>,
    declarations: &'a mut BTreeSet<String>,
}
impl Emitter<'_> {
    fn fresh(&mut self) -> String {
        let name = format!("v{}", self.next);
        self.next += 1;
        name
    }
    fn line(&mut self, line: String) {
        if !self.terminated {
            self.body.push(format!("  {line}"));
        }
    }
    fn value(&mut self, line: String) -> String {
        let name = format!("%{}", self.fresh());
        self.line(format!("{name} = {line}"));
        name
    }
    fn label(&mut self, name: &str) {
        self.body.push(format!("{name}:"));
        self.terminated = false;
    }
    fn jump(&mut self, name: &str) {
        self.line(format!("br label %{name}"));
        self.terminated = true;
    }
    fn branch(&mut self, value: &str, yes: &str, no: &str) {
        self.line(format!("br i1 {value}, label %{yes}, label %{no}"));
        self.terminated = true;
    }
    fn temporary(&mut self, kind: &Type) -> String {
        let name = format!("%{}", self.fresh());
        self.allocas.push(format!(
            "  {name} = alloca {}, align {}",
            ty(kind),
            layout(kind).unwrap().align()
        ));
        name
    }
    fn guard(&mut self, bad: &str, kind: u32) {
        if self.terminated {
            return;
        }
        let fail = self.fresh();
        let pass = self.fresh();
        self.branch(bad, &fail, &pass);
        self.label(&fail);
        let active = self.value("load i32, ptr %panic".into());
        let nested = self.value(format!("icmp ne i32 {active}, 0"));
        let abort = self.fresh();
        let origin = self.fresh();
        self.branch(&nested, &abort, &origin);
        self.label(&abort);
        self.line("call void @__nether_v1_abort_double_panic(ptr %panic)".into());
        self.line("unreachable".into());
        self.terminated = true;
        self.label(&origin);
        self.line("store i32 1, ptr %panic".into());
        let code = self.value("getelementptr %Panic, ptr %panic, i32 0, i32 1".into());
        self.line(format!("store i32 {kind}, ptr {code}"));
        if let Some((file, line, column)) = self.location.clone() {
            let name = format!("@panic_file{}", self.globals.len());
            let bytes = file
                .bytes()
                .chain(std::iter::once(0))
                .map(|b| format!("\\{b:02X}"))
                .collect::<String>();
            self.globals.push(format!(
                "{name} = private unnamed_addr constant [{} x i8] c\"{bytes}\"",
                file.len() + 1
            ));
            let address = self.value("getelementptr %Panic, ptr %panic, i32 0, i32 2".into());
            self.line(format!("store ptr {name}, ptr {address}"));
            for (index, number) in [(3, line), (4, column)] {
                let address = self.value(format!(
                    "getelementptr %Panic, ptr %panic, i32 0, i32 {index}"
                ));
                self.line(format!("store i32 {number}, ptr {address}"));
            }
        }
        if let Some(target) = self.unwind.clone() {
            self.jump(&target);
        } else {
            self.line("ret i1 false".into());
            self.terminated = true;
        }
        self.label(&pass);
    }
    fn block(&mut self, block: &Block) -> Result<String, Diagnostic> {
        let mut result = "zeroinitializer".to_owned();
        for statement in &block.statements {
            if self.terminated {
                break;
            }
            match statement {
                Statement::Initialize(id, value) => {
                    let computed = self.expr(value)?;
                    self.line(format!("store {} {computed}, ptr %l{id}", ty(&value.ty)));
                    result = "zeroinitializer".into();
                }
                Statement::Evaluate(value) => {
                    result = self.expr(value)?;
                }
                Statement::Return(value) => {
                    if let Some(value) = value {
                        let computed = self.expr(value)?;
                        if value.ty != Type::Unit {
                            self.line(format!("store {} {computed}, ptr %result", ty(&value.ty)));
                        }
                    }
                    self.line("ret i1 true".into());
                    self.terminated = true;
                }
                Statement::Break => {
                    let target = self.loops.last().unwrap().1.clone();
                    self.jump(&target);
                }
                Statement::Continue => {
                    let target = self.loops.last().unwrap().0.clone();
                    self.jump(&target);
                }
            }
        }
        Ok(result)
    }

    fn expr(&mut self, expr: &Expr) -> Result<String, Diagnostic> {
        if self.terminated {
            return Ok("zeroinitializer".into());
        }
        let result = match &expr.kind {
            ExprKind::ArgumentView { .. } => {
                return Err(Diagnostic::new(
                    "E0400",
                    "view argument used outside a call",
                    expr.span,
                ))
            }
            ExprKind::View(_) => {
                let address = self.place(expr)?;
                self.value(format!("load {}, ptr {address}", ty(&expr.ty)))
            }
            ExprKind::Borrow { local, place } => {
                let address = self.place(place)?;
                self.line(format!("store ptr {address}, ptr %l{local}"));
                "zeroinitializer".into()
            }
            ExprKind::Block(block) => self.block(block)?,
            ExprKind::Enum { variant, fields } => {
                let address = self.temporary(&expr.ty);
                self.line(format!(
                    "store {} zeroinitializer, ptr {address}",
                    ty(&expr.ty)
                ));
                self.line(format!("store i32 {variant}, ptr {address}"));
                let offset = enum_layout(&expr.ty).unwrap().payload_offset;
                let payload = self.value(format!("getelementptr i8, ptr {address}, i64 {offset}"));
                let payload_type = Type::Tuple(fields.iter().map(|f| f.ty.clone()).collect());
                for (index, field) in fields.iter().enumerate() {
                    let value = self.expr(field)?;
                    let field_address = self.value(format!(
                        "getelementptr {}, ptr {payload}, i32 0, i32 {index}",
                        ty(&payload_type)
                    ));
                    self.line(format!(
                        "store {} {value}, ptr {field_address}",
                        ty(&field.ty)
                    ));
                }
                self.value(format!("load {}, ptr {address}", ty(&expr.ty)))
            }
            ExprKind::Tag(value) => {
                let address = self.place(value)?;
                self.value(format!("load i32, ptr {address}"))
            }
            ExprKind::Payload(value, _) => {
                let address = self.place(value)?;
                let offset = enum_layout(&value.ty).unwrap().payload_offset;
                let payload = self.value(format!("getelementptr i8, ptr {address}, i64 {offset}"));
                self.value(format!("load {}, ptr {payload}", ty(&expr.ty)))
            }
            ExprKind::Match(arms) => {
                let end = self.fresh();
                let slot = self.temporary(&expr.ty);
                let mut reaches = false;
                for arm in arms {
                    let body = self.fresh();
                    let next = self.fresh();
                    let condition = self.expr(&arm.condition)?;
                    self.branch(&condition, &body, &next);
                    self.label(&body);
                    self.block(&Block {
                        storage: vec![],
                        statements: arm.guard_bindings.clone(),
                        result: Type::Unit,
                        diverges: false,
                        span: expr.span,
                    })?;
                    if let Some(guard) = &arm.guard {
                        let condition = self.expr(guard)?;
                        let accepted = self.fresh();
                        self.branch(&condition, &accepted, &next);
                        self.label(&accepted);
                    }
                    self.block(&Block {
                        storage: vec![],
                        statements: arm.bindings.clone(),
                        result: Type::Unit,
                        diverges: false,
                        span: expr.span,
                    })?;
                    let value = self.block(&arm.body)?;
                    reaches |= !self.terminated;
                    if expr.ty != Type::Unit {
                        self.line(format!("store {} {value}, ptr {slot}", ty(&expr.ty)));
                    }
                    self.jump(&end);
                    self.label(&next);
                }
                // Only well-typed exhaustive matches reach this lowering.
                self.line("unreachable".into());
                self.terminated = true;
                if reaches {
                    self.label(&end);
                    self.value(format!("load {}, ptr {slot}", ty(&expr.ty)))
                } else {
                    "zeroinitializer".into()
                }
            }

            ExprKind::Value(value) => match value {
                Value::Enum { variant, fields } => {
                    let Type::Enum { variants, .. } = &expr.ty else {
                        unreachable!()
                    };
                    let fields = fields
                        .iter()
                        .zip(&variants[*variant].fields)
                        .map(|(value, ty)| Expr {
                            kind: ExprKind::Value(value.clone()),
                            ty: ty.clone(),
                            span: expr.span,
                        })
                        .collect();
                    self.expr(&Expr {
                        kind: ExprKind::Enum {
                            variant: *variant,
                            fields,
                        },
                        ty: expr.ty.clone(),
                        span: expr.span,
                    })?
                }
                Value::Unit => "zeroinitializer".into(),
                Value::Bool(value) => value.to_string(),
                Value::Char(value) => u32::from(*value).to_string(),
                Value::Integer(value) => value
                    .signed_value()
                    .map_or_else(|| value.bits().to_string(), |v| v.to_string()),
                Value::F32(value) => format!("0x{:016X}", f64::from(*value).to_bits()),
                Value::F64(value) => format!("0x{:016X}", value.to_bits()),
                Value::Str(value) => {
                    let id = self.globals.len();
                    let bytes = value
                        .as_bytes()
                        .iter()
                        .map(|b| format!("\\{b:02X}"))
                        .collect::<String>();
                    self.globals.push(format!(
                        "@.str{id} = private constant [{} x i8] c\"{bytes}\"",
                        value.len()
                    ));
                    format!("{{ ptr @.str{id}, i64 {} }}", value.len())
                }
                Value::Aggregate(values) => {
                    let fields = match &expr.ty {
                        Type::Array(element, _) => vec![(**element).clone(); values.len()],
                        Type::Tuple(fields) => fields.clone(),
                        Type::Record { fields, .. } => {
                            fields.iter().map(|f| f.ty.clone()).collect()
                        }
                        _ => {
                            return Err(Diagnostic::new(
                                "E0400",
                                "invalid constant aggregate type",
                                expr.span,
                            ))
                        }
                    };
                    let mut aggregate = "zeroinitializer".to_owned();
                    for (index, (value, field)) in values.iter().zip(fields).enumerate() {
                        let value = self.expr(&Expr {
                            kind: ExprKind::Value(value.clone()),
                            ty: field.clone(),
                            span: expr.span,
                        })?;
                        aggregate = self.value(format!(
                            "insertvalue {} {aggregate}, {} {value}, {index}",
                            ty(&expr.ty),
                            ty(&field)
                        ));
                    }
                    aggregate
                }
            },
            ExprKind::Local(id) => self.value(format!("load {}, ptr %l{id}", ty(&expr.ty))),
            ExprKind::Tuple(values) | ExprKind::Array(values) => {
                let mut aggregate = "zeroinitializer".into();
                for (index, value) in values.iter().enumerate() {
                    let value_ir = self.expr(value)?;
                    aggregate = self.value(format!(
                        "insertvalue {} {aggregate}, {} {value_ir}, {index}",
                        ty(&expr.ty),
                        ty(&value.ty)
                    ));
                }
                aggregate
            }
            ExprKind::Unary(operator, value) => {
                let operand = self.expr(value)?;
                match *operator {
                    "!" => self.value(format!(
                        "xor {} {operand}, {}",
                        ty(&value.ty),
                        if value.ty == Type::Bool { "true" } else { "-1" }
                    )),
                    "-" if matches!(value.ty, Type::F32 | Type::F64) => {
                        self.value(format!("fneg {} {operand}", ty(&value.ty)))
                    }
                    "-" => self.binary("-", "0", &operand, &value.ty, &value.ty),
                    _ => unreachable!(),
                }
            }
            ExprKind::Binary(operator, left, right) => {
                let a = self.expr(left)?;
                if self.terminated {
                    return Ok("zeroinitializer".into());
                }
                if ["&&", "||"].contains(operator) {
                    let slot = self.temporary(&Type::Bool);
                    self.line(format!("store i1 {a}, ptr {slot}"));
                    let rhs = self.fresh();
                    let end = self.fresh();
                    if *operator == "&&" {
                        self.branch(&a, &rhs, &end);
                    } else {
                        self.branch(&a, &end, &rhs);
                    }
                    self.label(&rhs);
                    let b = self.expr(right)?;
                    self.line(format!("store i1 {b}, ptr {slot}"));
                    self.jump(&end);
                    self.label(&end);
                    self.value(format!("load i1, ptr {slot}"))
                } else {
                    let b = self.expr(right)?;
                    self.binary(operator, &a, &b, &left.ty, &right.ty)
                }
            }
            ExprKind::Cast(value) => {
                let operand = self.expr(value)?;
                self.cast(&operand, &value.ty, &expr.ty)
            }
            ExprKind::Call(id, arguments) => {
                let mut args = Vec::new();
                for argument in arguments {
                    if let ExprKind::ArgumentView { place, .. } = &argument.kind {
                        let slot = self.place(place)?;
                        args.push(format!(", ptr {slot}"));
                        continue;
                    }
                    let value = self.expr(argument)?;
                    let slot = self.temporary(&argument.ty);
                    self.line(format!("store {} {value}, ptr {slot}", ty(&argument.ty)));
                    args.push(format!(", ptr {slot}"));
                }
                if self.terminated {
                    return Ok("zeroinitializer".into());
                }
                let result = self.temporary(&expr.ty);
                let ok = self.value(format!(
                    "call i1 @_N1_f{id}(ptr %panic, ptr {result}{})",
                    args.join("")
                ));
                let pass = self.fresh();
                let fail = self.fresh();
                self.branch(&ok, &pass, &fail);
                self.label(&fail);
                if let Some(target) = self.unwind.clone() {
                    self.jump(&target);
                } else {
                    self.line("ret i1 false".into());
                    self.terminated = true;
                }
                self.label(&pass);
                if expr.ty == Type::Unit {
                    "zeroinitializer".into()
                } else {
                    self.value(format!("load {}, ptr {result}", ty(&expr.ty)))
                }
            }
            ExprKind::Assign {
                place,
                operator,
                value,
            } => {
                let address = self.place(place)?;
                let old = if *operator != "=" {
                    Some(self.value(format!("load {}, ptr {address}", ty(&place.ty))))
                } else {
                    None
                };
                let computed = self.expr(value)?;
                let computed = if let Some(old) = old {
                    self.binary(
                        operator.trim_end_matches('='),
                        &old,
                        &computed,
                        &place.ty,
                        &value.ty,
                    )
                } else {
                    computed
                };
                self.line(format!("store {} {computed}, ptr {address}", ty(&place.ty)));
                "zeroinitializer".into()
            }
            ExprKind::Index(_, _) | ExprKind::Field(_, _) => {
                let address = self.place(expr)?;
                self.value(format!("load {}, ptr {address}", ty(&expr.ty)))
            }
            ExprKind::If { condition, yes, no } => {
                let condition = self.expr(condition)?;
                if self.terminated {
                    return Ok("zeroinitializer".into());
                }
                let yes_label = self.fresh();
                let no_label = self.fresh();
                let end = self.fresh();
                let slot = self.temporary(&expr.ty);
                self.branch(&condition, &yes_label, &no_label);
                self.label(&yes_label);
                let value = self.block(yes)?;
                let yes_reaches = !self.terminated;
                if expr.ty != Type::Unit {
                    self.line(format!("store {} {value}, ptr {slot}", ty(&expr.ty)));
                }
                self.jump(&end);
                self.label(&no_label);
                let value = if let Some(no) = no {
                    self.block(no)?
                } else {
                    "zeroinitializer".into()
                };
                let no_reaches = !self.terminated;
                if expr.ty != Type::Unit {
                    self.line(format!("store {} {value}, ptr {slot}", ty(&expr.ty)));
                }
                self.jump(&end);
                if yes_reaches || no_reaches {
                    self.label(&end);
                    if expr.ty == Type::Unit {
                        "zeroinitializer".into()
                    } else {
                        self.value(format!("load {}, ptr {slot}", ty(&expr.ty)))
                    }
                } else {
                    self.terminated = true;
                    "zeroinitializer".into()
                }
            }
            ExprKind::While { condition, body } => {
                let test = self.fresh();
                let loop_body = self.fresh();
                let end = self.fresh();
                self.jump(&test);
                self.label(&test);
                let condition = self.expr(condition)?;
                if self.terminated {
                    return Ok("zeroinitializer".into());
                }
                self.branch(&condition, &loop_body, &end);
                self.label(&loop_body);
                self.loops.push((test.clone(), end.clone()));
                self.block(body)?;
                self.loops.pop();
                self.jump(&test);
                self.label(&end);
                "zeroinitializer".into()
            }
        };
        Ok(result)
    }

    fn place(&mut self, expr: &Expr) -> Result<String, Diagnostic> {
        Ok(match &expr.kind {
            ExprKind::Local(id) => format!("%l{id}"),
            ExprKind::View(id) => self.value(format!("load ptr, ptr %l{id}")),
            ExprKind::Payload(base, _) => {
                let address = self.place(base)?;
                let offset = enum_layout(&base.ty).unwrap().payload_offset;
                self.value(format!("getelementptr i8, ptr {address}, i64 {offset}"))
            }
            ExprKind::Index(base, index) => {
                let address = self.place(base)?;
                let index = self.expr(index)?;
                let Type::Array(_, length) = base.ty else {
                    unreachable!()
                };
                let bad = self.value(format!("icmp uge i64 {index}, {length}"));
                self.guard(&bad, 2);
                self.value(format!(
                    "getelementptr {}, ptr {address}, i64 0, i64 {index}",
                    ty(&base.ty)
                ))
            }
            ExprKind::Field(base, index) => {
                let address = self.place(base)?;
                self.value(format!(
                    "getelementptr {}, ptr {address}, i32 0, i32 {index}",
                    ty(&base.ty)
                ))
            }
            _ => {
                let value = self.expr(expr)?;
                let address = self.temporary(&expr.ty);
                self.line(format!("store {} {value}, ptr {address}", ty(&expr.ty)));
                address
            }
        })
    }

    fn binary(&mut self, operator: &str, a: &str, b: &str, left: &Type, right: &Type) -> String {
        if self.terminated {
            return "zeroinitializer".into();
        }
        let kind = ty(left);
        let float = matches!(left, Type::F32 | Type::F64);
        let signed = matches!(left,Type::Integer(i) if i.is_signed());
        if ["==", "!=", "<", ">", "<=", ">="].contains(&operator) {
            let predicate = match (float, signed, operator) {
                (true, _, "==") => "oeq",
                (true, _, "!=") => "une",
                (true, _, "<") => "olt",
                (true, _, ">") => "ogt",
                (true, _, "<=") => "ole",
                (true, _, _) => "oge",
                (false, _, "==") => "eq",
                (false, _, "!=") => "ne",
                (false, true, "<") => "slt",
                (false, true, ">") => "sgt",
                (false, true, "<=") => "sle",
                (false, true, _) => "sge",
                (false, false, "<") => "ult",
                (false, false, ">") => "ugt",
                (false, false, "<=") => "ule",
                _ => "uge",
            };
            return self.value(format!(
                "{} {predicate} {kind} {a}, {b}",
                if float { "fcmp" } else { "icmp" }
            ));
        }
        if float {
            let instruction = match operator {
                "+" => "fadd",
                "-" => "fsub",
                "*" => "fmul",
                "/" => "fdiv",
                "%" => "frem",
                _ => unreachable!(),
            };
            return self.value(format!("{instruction} {kind} {a}, {b}"));
        }
        let Type::Integer(integer) = left else {
            unreachable!()
        };
        if ["+", "-", "*"].contains(&operator) && self.checks == OverflowChecks::Checked {
            let op = match operator {
                "+" => "add",
                "-" => "sub",
                _ => "mul",
            };
            let name = format!(
                "llvm.{}{op}.with.overflow.{kind}",
                if signed { "s" } else { "u" }
            );
            self.declarations
                .insert(format!("declare {{ {kind}, i1 }} @{name}({kind}, {kind})"));
            let pair = self.value(format!(
                "call {{ {kind}, i1 }} @{name}({kind} {a}, {kind} {b})"
            ));
            let bad = self.value(format!("extractvalue {{ {kind}, i1 }} {pair}, 1"));
            self.guard(&bad, 3);
            return self.value(format!("extractvalue {{ {kind}, i1 }} {pair}, 0"));
        }
        if ["/", "%"].contains(&operator) {
            let zero = self.value(format!("icmp eq {kind} {b}, 0"));
            self.guard(&zero, 4);
            if signed {
                let minimum = 1u128 << (integer.bits() - 1);
                let is_min = self.value(format!("icmp eq {kind} {a}, {minimum}"));
                let minus_one = self.value(format!("icmp eq {kind} {b}, -1"));
                let bad = self.value(format!("and i1 {is_min}, {minus_one}"));
                self.guard(&bad, 3);
            }
        }
        let b = if ["<<", ">>"].contains(&operator) {
            let Type::Integer(count) = right else {
                unreachable!()
            };
            let count_kind = ty(right);
            if self.checks == OverflowChecks::Checked {
                let bad = self.value(format!("icmp uge {count_kind} {b}, {}", integer.bits()));
                self.guard(&bad, 5);
            }
            let masked = self.value(format!("and {count_kind} {b}, {}", integer.bits() - 1));
            match count.bits().cmp(&integer.bits()) {
                std::cmp::Ordering::Greater => {
                    self.value(format!("trunc {count_kind} {masked} to {kind}"))
                }
                std::cmp::Ordering::Less => {
                    self.value(format!("zext {count_kind} {masked} to {kind}"))
                }
                std::cmp::Ordering::Equal => masked,
            }
        } else {
            b.into()
        };
        let instruction = match operator {
            "+" => "add",
            "-" => "sub",
            "*" => "mul",
            "/" if signed => "sdiv",
            "/" => "udiv",
            "%" if signed => "srem",
            "%" => "urem",
            "&" => "and",
            "|" => "or",
            "^" => "xor",
            "<<" => "shl",
            ">>" if signed => "ashr",
            ">>" => "lshr",
            _ => unreachable!(),
        };
        self.value(format!("{instruction} {kind} {a}, {b}"))
    }

    fn cast(&mut self, value: &str, source: &Type, dest: &Type) -> String {
        if self.terminated {
            return "zeroinitializer".into();
        }
        let from = ty(source);
        let to = ty(dest);
        if from == to {
            return value.into();
        }
        let source_float = matches!(source, Type::F32 | Type::F64);
        let dest_float = matches!(dest, Type::F32 | Type::F64);
        if source_float && !dest_float {
            let signed = matches!(dest,Type::Integer(i) if i.is_signed());
            let name = format!(
                "llvm.fpto{}i.sat.{to}.{}",
                if signed { "s" } else { "u" },
                if *source == Type::F32 { "f32" } else { "f64" }
            );
            self.declarations
                .insert(format!("declare {to} @{name}({from})"));
            return self.value(format!("call {to} @{name}({from} {value})"));
        }
        let instruction = if source_float && dest_float {
            if *source == Type::F32 {
                "fpext"
            } else {
                "fptrunc"
            }
        } else if dest_float {
            if matches!(source,Type::Integer(i) if i.is_signed()) {
                "sitofp"
            } else {
                "uitofp"
            }
        } else if bits(source) > bits(dest) {
            "trunc"
        } else if matches!(source,Type::Integer(i) if i.is_signed()) {
            "sext"
        } else {
            "zext"
        };
        self.value(format!("{instruction} {from} {value} to {to}"))
    }
}

fn bits(ty: &Type) -> u32 {
    match ty {
        Type::Bool => 1,
        Type::Char => 32,
        Type::Integer(i) => i.bits(),
        _ => unreachable!(),
    }
}
fn ty(kind: &Type) -> String {
    match kind {
        Type::Enum { .. } => {
            let layout = enum_layout(kind).unwrap().layout;
            format!(
                "{{ [0 x i{}], [{} x i8] }}",
                layout.align() * 8,
                layout.size()
            )
        }
        Type::Unit => "{}".into(),
        Type::Bool => "i1".into(),
        Type::Char => "i32".into(),
        Type::Integer(i) => format!("i{}", i.bits()),
        Type::F32 => "float".into(),
        Type::F64 => "double".into(),
        Type::Str => "{ ptr, i64 }".into(),
        Type::Tuple(fields) => format!(
            "{{ {} }}",
            fields.iter().map(ty).collect::<Vec<_>>().join(", ")
        ),
        Type::Record { fields, .. } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|f| ty(&f.ty))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Type::Array(element, n) => format!("[{n} x {}]", ty(element)),
    }
}
pub fn layout(kind: &Type) -> Result<Layout, String> {
    Ok(match kind {
        Type::Enum { .. } => enum_layout(kind)?.layout,
        Type::Unit => Layout::UNIT,
        Type::Bool => Layout::BOOL,
        Type::Char => Layout::CHAR,
        Type::Integer(i) => Layout::integer(*i),
        Type::F32 => Layout::F32,
        Type::F64 => Layout::F64,
        Type::Str => Layout::STR,
        Type::Array(element, n) => layout(element)?
            .array(*n)
            .map_err(|e| format!("invalid array layout: {e:?}"))?,
        Type::Record { fields, .. } => {
            AggregateLayout::new(
                &fields
                    .iter()
                    .map(|f| layout(&f.ty))
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .map_err(|e| format!("invalid struct layout: {e:?}"))?
            .layout
        }
        Type::Tuple(fields) => {
            AggregateLayout::new(&fields.iter().map(layout).collect::<Result<Vec<_>, _>>()?)
                .map_err(|e| format!("invalid tuple layout: {e:?}"))?
                .layout
        }
    })
}

fn enum_layout(ty: &Type) -> Result<EnumLayout, String> {
    let Type::Enum { variants, .. } = ty else {
        return Err("enum layout requires enum".into());
    };
    let payloads = variants
        .iter()
        .map(|v| layout(&Type::Tuple(v.fields.clone())))
        .collect::<Result<Vec<_>, _>>()?;
    EnumLayout::new(&payloads).map_err(|e| format!("invalid enum layout: {e:?}"))
}
