use super::*;

#[test]
fn extern_declaration_produces_a_trivial_is_extern_mir_function() {
    let functions = build(
        r#"
extern "C" {
    fn abs(n i32) i32;
}
fn main() {
    let value = unsafe { abs(-3) };
    println(`${value}`);
}
"#,
    );
    let abs = find_fn(&functions, "abs");
    assert!(
        abs.is_extern,
        "an extern declaration's MirFunction must be flagged is_extern"
    );
    assert!(
        all_instrs(abs).is_empty(),
        "an extern declaration has no Nether-side body — its MIR must carry no real instructions: {:?}",
        all_instrs(abs)
    );
}

#[test]
fn unsafe_block_leaves_no_trace_in_lowered_mir() {
    // `unsafe { ... }` is a pure compile-time permission gate (language-spec
    // §17) — it must desugar to nothing at all by the time MIR sees it, so
    // an otherwise-identical function with/without the wrapper produces
    // structurally identical MIR.
    let with_unsafe = build(
        r#"
fn main() {
    let value = unsafe { 1 + 2 };
    println(`${value}`);
}
"#,
    );
    let without_unsafe = build(
        r#"
fn main() {
    let value = 1 + 2;
    println(`${value}`);
}
"#,
    );
    let main_with = find_fn(&with_unsafe, "main");
    let main_without = find_fn(&without_unsafe, "main");
    let shape_with: Vec<String> = all_instrs(main_with).iter().map(|i| format!("{i:?}")).collect();
    let shape_without: Vec<String> = all_instrs(main_without)
        .iter()
        .map(|i| format!("{i:?}"))
        .collect();
    assert_eq!(shape_with, shape_without);
    assert_eq!(main_with.blocks.len(), main_without.blocks.len());
}
