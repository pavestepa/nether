use nether_core::{check::check, hir, interpret, mir};
use nether_frontend::{
    parser::parse,
    source::{SourceId, Span},
};
use nether_semantics::OverflowChecks;
fn compile(source: &str) -> hir::Program {
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    check(&parsed.module.unwrap()).unwrap()
}
#[test]
fn cfg_execution_preserves_evaluation_order_and_control_flow() {
    for source in [
  include_str!("../../../examples/arithmetic/main.nr"),
  "fn main():i32 {var a=1;return a+if true {a=5;a} else {0}}",
  "fn main():i32 {var a={1,2};var i:usize=0;a[if true {i+=1;i} else {0}]+=if true {a[1]=10;3} else {0};return a[1]}",
  "fn main():i32 {var a={1};return a[if true {a[0]=3;0} else {0}]}",
  "fn main():i32 {var x=0;while x<10 {x+=1;if x==2 {continue};if x==5 {break}};return x}",
  "fn main():i32 {return if false && 1/0==0 {1} else {7}}",
  "fn main():i32 {return if true {return 4} else {return 8}}",
  "fn main():i32 {var a={1};a[2]=1/0;return 0}",
  "#[Copy] enum E {A(i32),B} fn main():i32 {return match E.B {E.A(v)=>v,E.B=>5}}",
  "#[Copy] struct S {a:i32;b:i32} fn main():i32 {var S {a,b}=S {a:1,b:2};a+=3;return a+b}",
  "fn main():i32 {let a:i8=127;return (a+1) as i32}",
 ]{
  let hir=compile(source);let cfg=mir::lower(&hir);
  for function in &cfg.functions {
   assert!(mir::dataflow::initialization_errors(function).is_empty(),"{source}: {:?}",mir::dataflow::initialization_errors(function));
   let _=mir::dataflow::liveness(function);
  }
  for mode in [OverflowChecks::Checked,OverflowChecks::Wrapping] {
   let reference=interpret::execute(&hir,hir.main.unwrap(),vec![],mode,100000).map_err(|t|t.kind);
   let result=mir::interpret::execute(&cfg,cfg.main.unwrap(),vec![],mode,100000).map_err(|t|t.kind);
   assert_eq!(result,reference,"{source}");
  }
 }
}
#[test]
fn liveness_keeps_loop_and_panic_uses_and_init_intersects_branches() {
    use mir::{Block, Function, Operand as V, Operation as O, Place, Terminator as T};
    let span = Span {
        source: SourceId(0),
        start: 0,
        end: 1,
    };
    let mut function = Function {
        name: "test".into(),
        parameters: vec![0],
        locals: (0..2)
            .map(|id| hir::Local {
                view: None,
                name: id.to_string(),
                ty: hir::Type::Bool,
                mutable: true,
                span,
            })
            .collect(),
        result: hir::Type::Bool,
        entry: 0,
        span,
        blocks: vec![
            Block {
                span,
                terminator: T::Branch {
                    condition: V::Local(0),
                    yes: 1,
                    no: 3,
                },
            },
            Block {
                span,
                terminator: T::Evaluate {
                    operation: O::Use(V::Constant(hir::Value::Bool(true))),
                    destination: Place::local(1),
                    next: 2,
                    unwind: 4,
                },
            },
            Block {
                span,
                terminator: T::Goto(0),
            },
            Block {
                span,
                terminator: T::Return(V::Local(1)),
            },
            Block {
                span,
                terminator: T::Return(V::Local(1)),
            },
        ],
    };
    let live = mir::dataflow::liveness(&function);
    assert!(
        live.live_in[1].contains(&1),
        "failed write must preserve old value for panic edge"
    );
    assert!(
        live.live_in[2].contains(&0),
        "condition remains live across loop back edge"
    );
    assert_eq!(
        mir::dataflow::initialization_errors(&function),
        vec![(3, 1), (4, 1)]
    );
    function.parameters.push(1);
    assert!(mir::dataflow::initialization_errors(&function).is_empty());
}
#[test]
fn explicit_moves_invalidate_only_the_selected_path_and_backend_rejects_later_reads() {
    use mir::{Block, Function, Operand as V, Operation as O, Place, Projection, Terminator as T};
    use nether_semantics::{Integer, IntegerType};
    let span = Span {
        source: SourceId(0),
        start: 0,
        end: 1,
    };
    let integer = hir::Type::Integer(IntegerType::I32);
    let pair = hir::Type::Tuple(vec![integer.clone(), integer.clone()]);
    let number = |n| hir::Value::Integer(Integer::from_bits(IntegerType::I32, n));
    let field = |n| Place {
        local: 0,
        projections: vec![Projection::Field(n)],
    };
    let function = Function {
        name: "partial".into(),
        parameters: vec![0],
        locals: [pair.clone(), integer.clone(), integer]
            .into_iter()
            .enumerate()
            .map(|(id, ty)| hir::Local {
                view: None,
                name: id.to_string(),
                ty,
                mutable: true,
                span,
            })
            .collect(),
        result: pair,
        entry: 0,
        span,
        blocks: vec![
            Block {
                span,
                terminator: T::Evaluate {
                    operation: O::Move(field(0)),
                    destination: Place::local(1),
                    next: 1,
                    unwind: 4,
                },
            },
            Block {
                span,
                terminator: T::Evaluate {
                    operation: O::Copy(field(1)),
                    destination: Place::local(2),
                    next: 2,
                    unwind: 4,
                },
            },
            Block {
                span,
                terminator: T::Evaluate {
                    operation: O::Use(V::Constant(number(9))),
                    destination: field(0),
                    next: 3,
                    unwind: 4,
                },
            },
            Block {
                span,
                terminator: T::Return(V::Local(0)),
            },
            Block {
                span,
                terminator: T::ResumePanic,
            },
        ],
    };
    let mut program = mir::Program {
        functions: vec![function],
        main: None,
    };
    let args = vec![hir::Value::Aggregate(vec![number(1), number(2)])];
    assert!(mir::dataflow::initialization_errors(&program.functions[0]).is_empty());
    assert_eq!(
        mir::interpret::execute(&program, 0, args.clone(), OverflowChecks::Checked, 1000).unwrap(),
        hir::Value::Aggregate(vec![number(9), number(2)])
    );
    assert!(nether_core::llvm::emit_mir(&program, OverflowChecks::Checked).is_ok());
    program.functions[0].blocks[2].terminator = T::Goto(3);
    assert_eq!(
        mir::dataflow::initialization_errors(&program.functions[0]),
        vec![(3, 0)]
    );
    assert_eq!(
        mir::interpret::execute(&program, 0, args, OverflowChecks::Checked, 1000)
            .unwrap_err()
            .kind,
        interpret::TrapKind::InvalidIr
    );
    assert_eq!(
        nether_core::llvm::emit_mir(&program, OverflowChecks::Checked)
            .unwrap_err()
            .code,
        "E0401"
    );
}
