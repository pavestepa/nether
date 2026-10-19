use nether_core::{
    check::check,
    mir::{self, effects},
};
use nether_frontend::{parser::parse, source::SourceId};

fn effect(source: &str, name: &str) -> effects::Summary {
    let parsed = parse(SourceId(0), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let hir = check(&parsed.module.unwrap()).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    let mir = mir::lower(&hir);
    let summaries = effects::infer(&mir);
    let id = mir
        .functions
        .iter()
        .position(|f| {
            f.name.starts_with(name)
                && f.parameters
                    .first()
                    .is_some_and(|p| f.locals[*p].view.is_some())
        })
        .unwrap();
    summaries[id].clone()
}

#[test]
fn finalizing_target_distinguishes_fields_aliases_and_rebinding() {
    for (body, replace, fields) in [
        ("this.value=2", false, true),
        ("*this=Cell {value:2}", true, false),
        ("let ref var alias=this;*alias=Cell {value:2}", true, false),
        ("let ref var alias=this;alias.value=2", false, true),
        (
            "var other=Cell {value:3};let ref var alias=this;alias=other;alias.value=2",
            false,
            false,
        ),
        (
            "let ref var alias=this;let ref var nested=alias;*nested=Cell {value:2}",
            true,
            false,
        ),
    ] {
        let source = format!("struct Cell {{value:i32; fn change(var this):() {{{body}}}}} fn main():i32 {{var x=Cell {{value:1}};x.change();return x.value}}");
        let summary = effect(&source, "Cell.change");
        assert_eq!(
            summary.parameters[0],
            effects::TargetEffects {
                replace,
                write_fields: fields
            },
            "{body}"
        );
        assert_eq!(effects::permits_finalizing(&summary, 0), !replace);
    }
}

#[test]
fn effects_propagate_through_mutually_recursive_calls() {
    for (write, replace) in [("*this=Cell {value:2}", true), ("this.value=2", false)] {
        let source = format!("struct Cell {{value:i32; fn first(var this,n:i32):() {{if n==0 {{return}};this.second(n-1)}} fn second(var this,n:i32):() {{if n==0 {{{write};return}};this.first(n-1)}}}} fn main():i32 {{var x=Cell {{value:1}};x.first(3);return x.value}}");
        let summary = effect(&source, "Cell.first");
        assert_eq!(summary.parameters[0].replace, replace);
        assert_eq!(summary.parameters[0].write_fields, !replace);
        assert_eq!(effects::permits_finalizing(&summary, 0), !replace);
    }
}

#[test]
fn replacing_a_field_via_a_call_does_not_replace_the_parent() {
    let source = "struct Cell {value:i32; fn reset(var this):() {*this=Cell {value:2}}} struct Pair {cell:Cell; fn change(var this):() {this.cell.reset()}} fn main():i32 {var x=Pair {cell:Cell {value:1}};x.change();return x.cell.value}";
    let summary = effect(source, "Pair.change");
    assert_eq!(
        summary.parameters[0],
        effects::TargetEffects {
            replace: false,
            write_fields: true
        }
    );
    assert!(effects::permits_finalizing(&summary, 0));
}

#[test]
fn recursive_returned_aliases_carry_finalizing_effects() {
    for (write, replace) in [("*alias=Cell {value:2}", true), ("alias.value=2", false)] {
        let source=format!("struct Cell {{value:i32;fn first(var this,n:i32):Cell {{if n==0 {{return this}};return second(n-1)}};fn second(var this,n:i32):Cell {{return first(n)}};fn change(var this):() {{let ref var alias=first(2);{write}}}}} fn main():i32 {{var x=Cell {{value:1}};x.change();return x.value}}");
        let summary = effect(&source, "Cell.change");
        assert_eq!(
            summary.parameters[0],
            effects::TargetEffects {
                replace,
                write_fields: !replace
            }
        );
        let source = source.replace("fn change(var this):()", "destructor");
        let parsed = parse(SourceId(0), &source.replace("x.change();", ""));
        let result = check(&parsed.module.unwrap());
        if replace {
            assert!(result.unwrap_err().iter().any(|e| e.code == "E0391"));
        } else {
            result.unwrap();
        }
    }
}
