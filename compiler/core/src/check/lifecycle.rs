//! Destructor declarations become private borrowed entries with checked effects.
use super::*;
use crate::mir::{drop_plan::Destructors, effects};

pub(super) fn prepare(module: &a::Module) -> Result<a::Module> {
    let mut module = module.clone();
    for item in &mut module.items {
        let a::ItemKind::Aggregate { members, .. } = &mut item.kind else {
            continue;
        };
        let mut seen = false;
        for member in members {
            let a::Member::Destructor(body) = member else {
                continue;
            };
            if seen {
                return Err(error(body.span, "E0390", "duplicate destructor"));
            }
            seen = true;
            if item.attributes.contains(&a::Attribute::Copy) {
                return Err(error(
                    body.span,
                    "E0390",
                    "Copy type cannot declare a destructor",
                ));
            }
            let span = body.span;
            *member = a::Member::Method {
                dispatch: a::MethodDispatch::Ordinary,
                private: true,
                function: a::Function {
                    name: "$drop".into(),
                    generics: vec![],
                    parameters: vec![a::Parameter {
                        pattern: a::Pattern {
                            kind: a::PatternKind::Binding {
                                name: "this".into(),
                                by_ref: false,
                                mutable: true,
                            },
                            span,
                        },
                        mutable: true,
                        ty: None,
                        receiver: true,
                        span,
                    }],
                    result: a::Type {
                        kind: a::TypeKind::Tuple(vec![]),
                        span,
                    },
                    body: Some(body.clone()),
                    c_abi: false,
                    unsafe_: false,
                    span,
                },
            };
        }
    }
    Ok(module)
}
pub(super) fn is_destructor(name: &str) -> bool {
    name.rsplit_once(".$drop").is_some_and(|(_, suffix)| {
        suffix.is_empty() || suffix.starts_with("$g") || suffix.starts_with("$entry")
    })
}
pub(super) fn validate(program: &h::Program) -> Result<Destructors> {
    if !program.functions.iter().any(|f| is_destructor(&f.name)) {
        return Ok(Destructors::default());
    }
    let mir = crate::mir::lower(program);
    let summaries = effects::infer(&mir);
    let mut registry = Destructors::default();
    for (id, function) in program.functions.iter().enumerate() {
        if !is_destructor(&function.name) {
            continue;
        }
        let receiver = function.parameters[0];
        if !effects::permits_finalizing(&summaries[id], 0) {
            return Err(error(function.span,"E0391","destructor cannot replace its finalizing target, including through aliases or calls"));
        }
        registry
            .insert(&function.locals[receiver].ty, id)
            .map_err(|message| error(function.span, "E0390", message))?;
    }
    // A user destructor may read all fields. Extracting an owned subfield would
    // leave its body with an invalid target. Whole moves remain permitted.
    for function in &mir.functions {
        for block in &function.blocks {
            let crate::mir::Terminator::Evaluate { operation, .. } = &block.terminator else {
                continue;
            };
            for place in crate::mir::dataflow::consumed(function, operation) {
                let mut ty = function.locals[place.local].ty.clone();
                for projection in &place.projections {
                    if registry.entry(&ty).is_some()
                        && !matches!(projection, crate::mir::Projection::Dereference)
                    {
                        return Err(error(
                            block.span,
                            "E0392",
                            "cannot move an owned field out of a type with a destructor",
                        ));
                    }
                    ty = match (projection, &ty) {
                        (crate::mir::Projection::Dereference, _) => ty.clone(),
                        (crate::mir::Projection::Field(i), Type::Class { .. }) => {
                            program.classes[&ty.class_key()].fields[*i].ty.clone()
                        }
                        (crate::mir::Projection::Field(i), Type::Record { fields, .. }) => {
                            fields[*i].ty.clone()
                        }
                        (crate::mir::Projection::Field(i), Type::Tuple(fields)) => {
                            fields[*i].clone()
                        }
                        (crate::mir::Projection::Variant(i), Type::Enum { variants, .. }) => {
                            Type::Tuple(variants[*i].fields.clone())
                        }
                        (
                            crate::mir::Projection::Index(_)
                            | crate::mir::Projection::ConstantIndex(_),
                            Type::Array(element, _),
                        ) => (**element).clone(),
                        _ => {
                            return Err(error(block.span, "E0392", "invalid destructor move path"))
                        }
                    };
                }
            }
        }
    }
    Ok(registry)
}

impl Checker<'_> {
    pub(super) fn schedule_destructors(&mut self, ty: &Type, span: Span) -> Result<()> {
        if self.mono.borrow().auditing {
            return Ok(());
        }
        match ty {
            Type::Class { name, .. } | Type::Record { name, .. } => {
                if matches!(ty, Type::Class { .. })
                    && !self
                        .mono
                        .borrow_mut()
                        .class_destructor_payloads
                        .insert(ty.class_key())
                {
                    return Ok(());
                }
                let fields = self.nominals.fields(ty).unwrap();
                let candidate = format!("{}.$drop", self.nominals.origin(name));
                let generic = self.mono.borrow().templates.contains_key(&candidate);
                let has_destructor = generic || self.signatures.contains_key(&candidate);
                if has_destructor
                    && self
                        .mono
                        .borrow_mut()
                        .destructor_types
                        .insert(format!("{ty:?}"))
                {
                    let types = self
                        .nominals
                        .arguments(name)
                        .into_iter()
                        .map(|a| match a {
                            records::Argument::Type(t) => {
                                a::TypeArgument::Type(generics::substitute::ast_type(&t, span))
                            }
                            records::Argument::Const(n) => a::TypeArgument::Const(a::Expr {
                                kind: a::ExprKind::Literal(a::Literal::Integer(n.to_string())),
                                span,
                            }),
                        })
                        .collect::<Vec<_>>();
                    let original = self.locals.len();
                    self.scopes.push(BTreeMap::new());
                    self.local("<destructor receiver>".into(), ty.clone(), true, span)?;
                    let source = a::Expr {
                        kind: a::ExprKind::Name("<destructor receiver>".into()),
                        span,
                    };
                    let result = if generic {
                        self.generic_call(&candidate, &types, &[source], Some(&Type::Unit), span)
                    } else {
                        let signature = self.signatures[&candidate].clone();
                        let value = self.expr(&source, Some(&signature.parameters[0]), true)?;
                        self.call_entry(signature, &[source], vec![value], span)
                    };
                    self.locals.truncate(original);
                    self.scopes.pop();
                    result?;
                }
                for field in fields {
                    self.schedule_destructors(&field.ty, span)?;
                }
            }
            Type::Tuple(fields) => {
                for ty in fields {
                    self.schedule_destructors(ty, span)?;
                }
            }
            Type::Array(ty, _) => self.schedule_destructors(ty, span)?,
            Type::Enum { variants, .. } => {
                for ty in variants.iter().flat_map(|v| &v.fields) {
                    self.schedule_destructors(ty, span)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nether_frontend::{parser::parse, source::SourceId};
    fn analyzed(source: &str) -> std::result::Result<h::Program, Vec<Diagnostic>> {
        let parsed = parse(SourceId(0), source);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        super::super::check(&parsed.module.unwrap())
    }
    #[test]
    fn typed_destructors_allow_fields_but_reject_transitive_whole_replacement() {
        for body in [
            "value=2",
            "let ref var alias=this;alias.value=3",
            "this.update()",
        ] {
            let source=format!("struct Resource {{value:i32; fn update(var this):() {{value+=1}}; destructor {{{body}}}}} fn main():i32 {{let x=Resource {{value:1}};return x.value}}");
            let p = analyzed(&source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
            let registry = validate(&p).unwrap();
            let destructor = p.functions.iter().find(|f| is_destructor(&f.name)).unwrap();
            assert!(registry
                .entry(&destructor.locals[destructor.parameters[0]].ty)
                .is_some());
        }
        for body in [
            "*this=Resource {value:2}",
            "let ref var alias=this;*alias=Resource {value:2}",
            "this.replace()",
        ] {
            let source=format!("struct Resource {{value:i32; fn replace(var this):() {{*this=Resource {{value:2}}}}; destructor {{{body}}}}}");
            assert!(
                analyzed(&source)
                    .unwrap_err()
                    .iter()
                    .any(|e| e.code == "E0391"),
                "{source}"
            );
        }
    }
    #[test]
    fn copy_duplicate_and_partial_move_are_rejected() {
        for source in [
            "#[Copy] struct R {destructor {}}",
            "struct R {destructor {};destructor {}}",
        ] {
            assert!(analyzed(source)
                .unwrap_err()
                .iter()
                .any(|e| e.code == "E0390"));
        }
        let source="struct Cell {value:i32} struct R {cell:Cell;destructor {}} struct H {c:Cell} fn main():i32 {let r=R {cell:Cell {value:1}};let h=H {c:r.cell};return h.c.value}";
        assert!(analyzed(source)
            .unwrap_err()
            .iter()
            .any(|e| e.code == "E0392"));
    }
    fn execute_cleanup(
        source: &str,
    ) -> (
        std::result::Result<Value, crate::interpret::Trap>,
        Vec<i128>,
    ) {
        let program = analyzed(source).unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        let mir = crate::mir::lower(&program);
        let (result, events) = crate::mir::interpret::execute_traced(
            &mir,
            mir.main.unwrap(),
            vec![],
            nether_semantics::OverflowChecks::Checked,
            100_000,
        );
        let (hir_result, hir_events) = crate::interpret::execute_traced(
            &program,
            program.main.unwrap(),
            vec![],
            nether_semantics::OverflowChecks::Checked,
            100_000,
        );
        assert_eq!(hir_result, result, "HIR/MIR result: {source}");
        assert_eq!(hir_events, events, "HIR/MIR cleanup: {source}");
        let events = events
            .into_iter()
            .map(|event| {
                let Value::Aggregate(fields) = event.value else {
                    panic!()
                };
                let Value::Integer(value) = &fields[0] else {
                    panic!()
                };
                value.signed_value().unwrap()
            })
            .collect();
        (result, events)
    }
    #[test]
    fn mir_cleanup_executes_source_bodies_in_ownership_order() {
        let resource = "struct R {value:i32;fn replace(var this):() {*this=R {value:3}};destructor {value+=10}} ";
        for (body,expected) in [
            ("fn main():i32 {let a=R {value:1};let b=R {value:2};return 0}",vec![2,1]),
            ("fn main():i32 {var a=R {value:1};let b=R {value:2};a=R {value:3};return 0}",vec![1,3,2]),
            ("fn identity(x:R):R {return x} fn main():i32 {let a=R {value:1};let b=identity(a);return b.value}",vec![1]),
            ("fn make():R {return R {value:1}} fn main():i32 {let a=make();return a.value}",vec![1]),
            ("fn main():i32 {let xs={R {value:1},R {value:2}};return 0}",vec![1,2]),
            ("struct Pair {a:R;b:R} fn main():i32 {let pair=Pair {a:R {value:1},b:R {value:2}};return 0}",vec![1,2]),
            ("fn main():i32 {var a=R {value:1};if true {let b=R {value:2};a=R {value:3}};return 0}",vec![1,2,3]),
            ("fn main():i32 {var a=R {value:1};let b=R {value:2};a.replace();return a.value}",vec![1,3,2]),
            ("fn main():i32 {R {value:9};let x=R {value:2};return 0}",vec![9,2]),
            ("fn main():i32 {var n=0;while n<3 {let x=R {value:n};n+=1;if n==2 {continue};if n==3 {break}};return n}",vec![0,1,2]),
            ("struct Pair {a:R;b:R} fn main():i32 {let p=Pair {a:R {value:1},b:R {value:2}};let x=p.a;return x.value}",vec![1,2]),
            ("enum E {A(R,R),B} fn main():i32 {let x=E.A(R {value:1},R {value:2});return 0}",vec![1,2]),
            ("fn main():i32 {return R {value:7}.value}",vec![7]),
        ] {
            let source=format!("{resource}{body}");
            let (result,events)=execute_cleanup(&source);
            assert!(result.is_ok(),"{source}: {result:?}");
            assert_eq!(events,expected,"{source}");
        }
    }
    #[test]
    fn mir_failed_aggregate_construction_cleans_completed_fields_in_order() {
        let types = "struct R {value:i32;destructor {}} struct Parent {a:R;b:R;c:i32;destructor {let fail=1/0}} ";
        for expression in [
            "Parent {a:R {value:1},b:R {value:2},c:1/0}",
            "Parent {b:R {value:2},a:R {value:1},c:1/0}",
            "(R {value:1},R {value:2},1/0)",
        ] {
            let (result, events) = execute_cleanup(&format!(
                "{types} fn main():i32 {{let outer=R {{value:9}};let x={expression};return 0}}"
            ));
            assert!(matches!(
                result.unwrap_err().kind,
                crate::interpret::TrapKind::Arithmetic(_)
            ));
            assert_eq!(events, vec![1, 2, 9]);
        }
        let (result, events) = execute_cleanup("struct R {value:i32;destructor {}} fn fail():R {let x=1/0;return R {value:3}} fn main():i32 {let x={R {value:1},R {value:2},fail()};return 0}");
        assert!(result.is_err());
        assert_eq!(events, vec![1, 2]);
    }
    #[test]
    fn resource_call_arguments_clean_in_reverse_on_failure_before_entry() {
        let source="struct R {value:i32;destructor {}} fn take(a:R,b:R,n:i32):i32 {let x=a;let y=b;return n} fn main():i32 {return take(R {value:1},R {value:2},1/0)}";
        let (result, events) = execute_cleanup(source);
        assert!(result.is_err());
        assert_eq!(events, vec![2, 1]);
    }
    #[test]
    fn mir_unwind_cleans_return_staging_and_stops_on_double_panic() {
        let types="struct R {value:i32;destructor {}} struct Fail {value:i32;destructor {let failed=1/value}} ";
        let (result,events)=execute_cleanup(&format!("{types} fn make():R {{let fail=Fail {{value:0}};return R {{value:7}}}} fn main():i32 {{let result=make();return result.value}}"));
        assert!(result.is_err());
        assert_eq!(events, vec![0, 7]);
        let (result,events)=execute_cleanup(&format!("{types} fn main():i32 {{let live=R {{value:3}};let a=Fail {{value:0}};let b=Fail {{value:0}};return 0}}"));
        assert_eq!(
            result.unwrap_err().kind,
            crate::interpret::TrapKind::DoublePanic
        );
        assert_eq!(events, vec![0, 0]);
    }
    #[test]
    fn generic_destructors_are_specialized_and_audited() {
        let source="struct R<T:Copy> {value:T;destructor {let copy=value}} fn main():i32 {let a=R<i32> {value:1};let b=R<i64> {value:2};return 0}";
        let (result, events) = execute_cleanup(source);
        assert!(result.is_ok());
        assert_eq!(events, vec![2, 1]);
        let invalid="struct R<T> {value:T;destructor {value+=1}} fn main():i32 {let a=R<i32> {value:1};return 0}";
        assert!(analyzed(invalid)
            .unwrap_err()
            .iter()
            .any(|e| e.code == "E0373"));
    }
    #[test]
    fn generated_rust_executes_resource_cleanup() {
        use std::{fs, process::Command};
        let cases = [
            "struct R {value:i32;destructor {}} fn id<T>(x:T):T {return x} fn apply<T>(f:(T)=>T,x:T):T {return f(x)} fn main():i32 {let f:(R)=>R=id;let t=R {value:9};let r=apply(f,t);return r.value}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn relay(x:C):C {return x} fn main():i32 {let f:(C)=>C=relay;let a=new C(7);let b=f(a);return b.value}",
            "struct R {value:i32;destructor {}} fn fail(a:R,b:R):R {let n=1/0;return a} fn main():i32 {let f=fail;let t1=R {value:1};let t2=R {value:2};let a=f(t1,t2);return a.value}",
            "struct R {value:i32;destructor {}} fn take(a:R,b:R):i32 {return a.value+b.value} fn bad():R {let n=1/0;return R {value:8}} fn main():i32 {let f=take;return f(R {value:1},bad())}",

            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn id(x:C):C {return x} fn relay(x:C):C {return id(x)} fn main():i32 {let a=new C(7);let b=relay(a);return a.value+b.value}",
            "struct R {value:i32;destructor {}} struct P {a:R;b:R;fn get(this):R {return a}} fn main():i32 {let p=P {a:R {value:1},b:R {value:2}};let r=p.get();return p.b.value+r.value}",
            "struct R {value:i32;destructor {}} fn id(var x:R):R {return x} fn main():i32 {var a=R {value:1};let other=R {value:9};let ref var v=id(a);*v=R {value:2};return a.value}",
            "struct R {value:i32;destructor {}} struct F {value:i32;destructor {let fail=1/value}} fn get(x:R):R {let f=F {value:0};return x} fn main():i32 {let a=R {value:7};let b=get(a);return a.value+b.value}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn pick(flag:bool,x:C,y:C):C {return if flag {x} else {y}} fn main():i32 {let a=new C(3);let b=new C(7);let c=pick(false,a,b);return a.value+b.value+c.value}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn main():i32 {let a=new C(1);let b=new C(1);let ref v=a;if a!=b && a==v && same_identity(v,a) {return 7};return 0}",
            "#[Copy] class C {value:i32;constructor(n:i32) {value=n}} fn main():i32 {let identity_probe=new C(1);let ref v=identity_probe;if identity_probe==v && same_identity(identity_probe,v) {return 3};return 0}",
            "#[Copy] class C {value:i32;constructor(n:i32) {value=n}} fn main():i32 {let a=new C(1);let b=a;if a==b {return 1};return 0}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn main():i32 {return if same_identity(new C(1),new C(1)) {1} else {0}}",
            "#[Copy] class C {value:i32;constructor(n:i32) {value=n}} #[Copy] struct S {c:C} fn main():i32 {let a=S {c:new C(1)};var b=a;b.c.value=7;return a.c.value*10+b.c.value}",

            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn relay(x:C):C {return x} fn main():i32 {let a=new C(7);let b=relay(a);return b.value}",
            "class Empty {} fn main():i32 {let e=new Empty();return 0}",
            "class C {value:i32;constructor(flag:bool) {match flag {true=>{value=1},false=>{value=2}}}} fn main():i32 {let c=new C(false);return c.value}",
            "class C {value:i32;constructor(n:i32) {let ref var alias=this;alias.value=n}} class Pair {left:C;right:C;constructor() {right=new C(2);left=new C(1)}} fn main():i32 {var p=new Pair();p.left=new C(3);return p.left.value+p.right.value}",
            "class C {value:i32;constructor(n:i32) {value=n}} class Pair {left:C;right:C;constructor() {left=new C(1);let fail=1/0;right=new C(2)}} fn main():i32 {let p=new Pair();return 0}",
            "class Cell<T> {value:T;constructor(v:T) {value=v}} fn read<T:Copy>(x:Cell<T>):T {return x.value} class Array<const N:usize> {values:{i32;N};constructor(v:{i32;N}) {values=v}} fn main():i32 {let c=new Cell<i32>(3);let a=new Array<2>({4,5});return read(c)+a.values[1]}",

            "#[Copy] class C {value:i32;constructor(n:i32) {value=n}} fn main():i32 {let a=new C(1);var b=a;b.value=7;return a.value*10+b.value}",
            "#[Copy] class C {value:i32;constructor(n:i32) {value=n}} #[Copy] class Pair {left:C;right:C;constructor(a:C,b:C) {left=a;right=b}} fn main():i32 {let a=new Pair(new C(1),new C(2));var b=a;b.left.value=7;return a.left.value*10+b.left.value}",
            "#[Copy] class C {value:i32;constructor(n:i32) {value=n}} #[Copy] enum E {Some(C),None} fn main():i32 {let a=E.Some(new C(1));var b=a;match b {E.Some(ref var x)=>{x.value=7},E.None=>{}};return match a {E.Some(x)=>x.value,E.None=>0}}",
            "#[Copy] class C {value:i32;constructor(n:i32) {value=n}} fn id(x:C):C {return x} fn main():i32 {let a={new C(1),new C(2)};var b=a;b[0].value=7;let f:(C)=>C=id;var c=f(b[0]);c.value=9;return a[0].value*10+b[0].value+c.value}",

            "class C {value:i32;constructor(n:i32) {value=n};fn bump(var this):() {value+=1};destructor {value+=10}} fn main():i32 {var x=new C(3);x.bump();return x.value}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn make(n:i32):C {return new C(n)} fn id(x:C):C {return x} fn main():i32 {var x=make(1);let y=id(x);let n=y.value;x=make(3);return x.value+n}",
            "struct R {value:i32;destructor {}} class C {value:i32;a:R;b:R;constructor() {value=9;b=R {value:2};a=R {value:1};let fail=1/0};destructor {}} fn main():i32 {let outer=R {value:8};let x=new C();return 0}",
            "struct R {value:i32;destructor {}} class C {value:i32;a:R;b:R;constructor() {value=9;a=R {value:1};b=R {value:2}};destructor {a.value+=10}} fn main():i32 {let x=new C();return 0}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn main():i32 {var a=new C(1);let b=new C(2);a=new C(3);return a.value+b.value}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {let fail=1/value}} fn main():i32 {var a=new C(0);a=new C(2);return 0}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {let fail=1/value}} fn main():i32 {let a=new C(0);let b=new C(0);return 0}",
            "class C<T:Copy> {value:T;constructor(n:T) {value=n};fn read(this):T {return value};destructor {let x=value}} fn main():i32 {let a=new C<i32>(4);let b=new C<i64>(5);return a.read()+b.read() as i32}",
            "class Node {value:i32;next:Link;constructor(n:i32,tail:Link) {value=n;next=tail};destructor {}} enum Link {End,More(Node)} fn main():i32 {let a=new Node(1,Link.End);let b=new Node(2,Link.More(a));return b.value}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn main():i32 {var a=new C(1);var b=new C(2);let ref var v=a;v.value=4;v=b;v.value=5;return a.value+b.value}",
            "class C {value:i32;constructor(n:i32) {value=n};destructor {}} fn main():i32 {let x={new C(1),new C(2)};return x[0].value+x[1].value}",
            "struct R {value:i32;destructor {}} class C {value:i32;a:R;constructor(n:i32,x:R) {value=n;a=x};destructor {}} fn main():i32 {let x=new C(9,R {value:1});return x.a.value}",

            "struct R {value:i32;destructor {}} enum E {A(R,R),B} fn main():i32 {let e=E.A(R {value:1},R {value:2});match e {E.A(a,b)=>{let x=a;let y=b},E.B=>{}};return 0}",
            "struct R {value:i32;destructor {}} enum E {A(R,R),B} fn main():i32 {let e=E.A(R {value:1},R {value:2});match e {E.A(a,b) if false=>{},E.A(a,b)=>{let x=a;let y=b},E.B=>{}};return 0}",
            "struct R {value:i32;destructor {}} struct P {value:i32;a:R;b:R;destructor {a.value+=10}} fn main():i32 {let p=P {value:9,a:R {value:1},b:R {value:2}};return 0}",
            "struct R {value:i32;destructor {}} struct P {value:i32;a:R;b:R;destructor {let fail=1/0}} fn main():i32 {let p=P {value:9,a:R {value:1},b:R {value:2}};return 0}",
            "struct R {value:i32;destructor {let fail=1/value}} fn main():i32 {var a=R {value:0};a=R {value:2};return 0}",
            "struct R {value:i32;destructor {}} struct P {a:R;b:R} fn main():i32 {var p=P {a:R {value:1},b:R {value:2}};let other=R {value:9};let ref var a=p.a;*a=R {value:3};return 0}",
            "struct R {value:i32;destructor {}} struct F {value:i32;destructor {let fail=1/value}} fn main():i32 {let live=R {value:3};let a=F {value:0};let b=F {value:0};return 0}",
            "struct R {value:i32;destructor {}} fn main():i32 {let values={R {value:1},R {value:2}};let i:usize=3;let x=values[i];return 0}",
            "struct R<T:Copy> {value:T;destructor {let copy=value}} fn main():i32 {let a=R<i32> {value:1};let b=R<i64> {value:2};return 0}",
            "struct R {value:i32;destructor {value+=10}} fn main():i32 {var a=R {value:1};let b=R {value:2};a=R {value:3};return 0}",
            "struct R {value:i32;fn replace(var this):() {*this=R {value:3}};destructor {}} fn main():i32 {var a=R {value:1};let b=R {value:2};a.replace();return 0}",
            "struct R {value:i32;destructor {}} struct P {a:R;b:R;c:i32} fn main():i32 {let p=P {b:R {value:2},a:R {value:1},c:1/0};return 0}",
            "struct R {value:i32;destructor {}} fn main():i32 {var n=0;while n<3 {let x=R {value:n};n+=1;if n==2 {continue};if n==3 {break}};return 0}",
            "struct R {value:i32;destructor {}} fn id(x:R):R {return x} fn main():i32 {let t=R {value:7};let x=id(t);return 0}",
            "struct R {value:i32;destructor {}} enum E {A(R,R),B} fn main():i32 {let x=E.A(R {value:1},R {value:2});let y={R {value:3},R {value:4}};return 0}",
            "struct R {value:i32;destructor {}} struct F {value:i32;destructor {let failure=1/value}} fn make():R {let fail=F {value:0};return R {value:7}} fn main():i32 {let x=make();return 0}",
            "struct R {value:i32;destructor {}} struct P {a:R;b:R} fn main():i32 {let p=P {a:R {value:1},b:R {value:2}};let x=p.a;return 0}",
        ];
        let mut rust = String::new();
        let mut expected = Vec::new();
        for (i, source) in cases.iter().enumerate() {
            let p = analyzed(source).unwrap();
            let (result, events) = execute_cleanup(source);
            expected.push((result.map_err(|e| e.kind), events));
            let emitted = crate::rust::emit(&p, nether_semantics::OverflowChecks::Checked).unwrap();
            let emitted = emitted.replace("fn main()", "pub fn run()");
            let emitted = emitted
                .lines()
                .map(|line| {
                    if line == "// class-allocation" {
                        "eprintln!(\"ALLOC:{:p}\",heap);"
                    } else if line == "// class-deallocation" {
                        "eprintln!(\"FREE:{:p}\",heap);"
                    } else if line.starts_with("// class-destructor-entry ") {
                        "eprintln!(\"DROP:{}\",(*(*heap).data.as_ptr()).f0);"
                    } else if line.starts_with("// destructor-entry ") {
                        "eprintln!(\"DROP:{}\",(*p).f0);"
                    } else {
                        line
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            rust.push_str(&format!("mod case{i} {{\n{emitted}\n}}\n"));
        }
        rust.push_str(
            "fn main() {match std::env::args().nth(1).unwrap().parse::<usize>().unwrap() {\n",
        );
        for i in 0..cases.len() {
            rust.push_str(&format!("{i}=>case{i}::run(),\n"));
        }
        rust.push_str("_=>panic!(),}}\n");
        let directory =
            std::env::temp_dir().join(format!("nether-resource-native-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("cases.rs");
        fs::write(&source, rust).unwrap();
        for optimization in ["0", "3"] {
            let binary = directory.join(format!("cases-{optimization}"));
            let output =
                Command::new(std::env::var("NETHER_RUSTC").unwrap_or_else(|_| "rustc".into()))
                    .arg("--edition=2021")
                    .args(["-C", &format!("opt-level={optimization}")])
                    .arg(&source)
                    .arg("-o")
                    .arg(&binary)
                    .output()
                    .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            for (i, (expected_result, events)) in expected.iter().enumerate() {
                let output = Command::new(&binary).arg(i.to_string()).output().unwrap();
                if let Ok(value) = expected_result {
                    let code = match value {
                        Value::Integer(n) => n.bits() as i32 & 255,
                        _ => 0,
                    };
                    assert_eq!(
                        output.status.code(),
                        Some(code),
                        "case {i}: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                if let Err(kind) = expected_result {
                    if *kind == crate::interpret::TrapKind::DoublePanic {
                        assert!(output.status.code().is_none(), "double panic must abort");
                    } else {
                        assert_eq!(output.status.code(), Some(101));
                    }
                }
                let mut allocations = 0;
                let mut live = std::collections::BTreeSet::new();
                for line in String::from_utf8_lossy(&output.stderr).lines() {
                    if let Some(address) = line.strip_prefix("ALLOC:") {
                        allocations += 1;
                        assert!(
                            live.insert(address.to_owned()),
                            "allocation identity reused while live: case {i}"
                        );
                    } else if let Some(address) = line.strip_prefix("FREE:") {
                        assert!(
                            live.remove(address),
                            "double free or unknown identity: case {i}"
                        );
                    }
                }
                if cases[i].contains("let identity_probe=") {
                    assert_eq!(
                        allocations, 1,
                        "identity comparison must not copy its operands"
                    );
                }
                if cases[i].contains("fn relay(") {
                    assert_eq!(allocations, 1, "move must preserve allocation identity");
                }
                if cases[i].contains("let a=new C(1);var b=a;") {
                    assert_eq!(allocations, 2, "Copy must create exactly one new identity");
                }
                if !matches!(
                    expected_result,
                    Err(crate::interpret::TrapKind::DoublePanic)
                ) {
                    assert!(
                        live.is_empty(),
                        "leaked class allocation: case {i}: {live:?}"
                    );
                }
                let trace = String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .filter_map(|line| line.strip_prefix("DROP:"))
                    .map(|n| n.parse::<i128>().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(&trace, events, "case {i}, O{optimization}");
            }
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
