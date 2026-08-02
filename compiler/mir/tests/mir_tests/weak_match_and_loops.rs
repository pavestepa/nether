use super::*;

#[test]
fn constructing_a_weak_field_from_a_heap_value_uses_weak_retain_not_retain() {
    // `Parent { kid: c }` stores `c` into a `weak Child` field — spec
    // §13/`arc-model.md` §3.5: this must never touch `c`'s own strong
    // reference count, only bump a separate weak count on a fresh
    // `weak`-typed copy (`nether_mir::build::FnBuilder::prepare_weak_binding`).
    let functions = build(
        r#"
type Child { name: String }
type Parent { kid: weak Child }
fn main() {
    let c = Child { name: "Rex" };
    let p = Parent { kid: c };
}
"#,
    );
    let main = find_fn(&functions, "main");
    let shape = retain_release_shape(main);
    // One WeakRetain for the field's own weak copy of `c`; then, in
    // reverse declaration order (§3.2), `p`'s own scope-exit release,
    // then `c`'s — neither is a weak release, since `c` and `p` are each
    // independently heap-kind locals, unrelated to the weak field inside
    // `p`. No plain `retain` anywhere: `c`'s own strong count is never
    // touched by being woven into a weak field.
    assert_eq!(shape, vec!["weak_retain", "release", "release"]);
}

#[test]
fn assigning_into_a_weak_field_uses_weak_release_and_weak_retain() {
    let functions = build(
        r#"
type Child { name: String }
type Parent { kid: weak Child }
fn set_kid(mut p: Parent, c: Child) {
    p.kid = c;
}
fn main() {
    let mut p = Parent { kid: Child { name: "Rex" } };
    let c = Child { name: "Buddy" };
    set_kid(mut p, c);
}
"#,
    );
    let set_kid = find_fn(&functions, "set_kid");
    let shape = retain_release_shape(set_kid);
    // The leading generic `Retain` applies to a `weak Child` local and is
    // therefore emitted as weak-retain by codegen. It owns the projected
    // snapshot while the store occurs; two WeakReleases then cancel that
    // incidental snapshot credit and the field's original weak credit.
    // The explicit WeakRetain owns the replacement field value. `p` is
    // now a `mut` (borrowed) parameter — mutating a heap-typed field
    // through it requires `mut` under the language's Rust-like mutation
    // rules — so, like `mut self` elsewhere in this file, the caller
    // retains ownership and the callee neither retains nor releases it;
    // only `c` (an ordinary owned heap param) gets the trailing release.
    assert_eq!(
        shape,
        vec![
            "retain",
            "weak_retain",
            "weak_release",
            "weak_release",
            "release",
        ],
    );
    let weak_snapshot_retain = all_instrs(set_kid).into_iter().find_map(|instr| {
        let Instr::Retain(local) = instr else {
            return None;
        };
        matches!(
            set_kid.local_decl(*local).ty,
            nether_typecheck::Type::Weak(_)
        )
        .then_some(())
    });
    assert!(weak_snapshot_retain.is_some());
}

#[test]
fn match_on_enum_with_heap_payload_binds_via_variant_field_retain() {
    let functions = build(
        r#"
type Dog { name: String }
enum Wrapper { Boxed(Dog), Empty }
fn unwrap(w: Wrapper): String {
    match w {
        Boxed(d) => d.name,
        Empty => "none",
    }
}
fn main() {
    let w = Wrapper.Boxed(Dog { name: "Rex" });
    let s = unwrap(w);
    println(s);
}
"#,
    );
    let unwrap_fn = find_fn(&functions, "unwrap");
    // `d` is bound via a `VariantField` read (aliasing, per `insert_arc`)
    // and then immediately used as the arm's escaping result via a bare
    // field access — this should produce a small, finite, *balanced*
    // sequence rather than panicking or looping forever building the
    // pattern-test chain.
    let shape = retain_release_shape(unwrap_fn);
    assert!(
        !shape.is_empty(),
        "expected at least the VariantField bind-time retain"
    );
    let w = unwrap_fn.params[0];
    assert_eq!(
        all_instrs(unwrap_fn)
            .iter()
            .filter(|instr| matches!(instr, Instr::Release(local) if *local == w))
            .count(),
        1,
        "the original enum parameter must be deeply dropped once at function exit",
    );
    // A flattened CFG contains one release of the match's owned
    // scrutinee copy on each mutually-exclusive arm, so raw global
    // retain/release counts are intentionally not equal.
    assert!(shape.iter().filter(|kind| **kind == "release").count() >= 3);
}

#[test]
fn while_loop_body_release_is_inside_the_loop_not_after_it() {
    let functions = build(
        r#"
type Dog { name: String }
fn main() {
    let mut i = 0;
    while i < 3 {
        let d = Dog { name: "Rex" };
        println(d.name);
        i = i + 1;
    }
}
"#,
    );
    let main = find_fn(&functions, "main");
    // The `let d = ...` inside the loop body must be released once per
    // iteration, i.e. inside the *body* block, not hoisted out after the
    // loop — find the body block (the one whose Goto target is the loop
    // header, per `lower_while`) and confirm it contains a Release.
    let has_release_in_some_non_final_block = main.blocks.iter().any(|b| {
        matches!(b.terminator, Terminator::Goto(_))
            && b.instrs.iter().any(|i| matches!(i, Instr::Release(_)))
    });
    assert!(
        has_release_in_some_non_final_block,
        "expected the loop body's own block to contain a Release for `d`, executed every iteration"
    );
}

#[test]
fn mut_parameter_of_stack_type_gets_no_retain_or_release() {
    let functions = build(
        r#"
fn bump(mut x: i32) {
    x = x + 1;
}
fn main() {
    let mut n = 1;
    bump(mut n);
}
"#,
    );
    let bump = find_fn(&functions, "bump");
    assert!(
        retain_release_shape(bump).is_empty(),
        "a stack-kind mut parameter should never participate in ARC (arc-model.md §3.6)"
    );
}

#[test]
fn reassigning_an_existing_heap_variable_releases_the_old_value() {
    let functions = build(
        r#"
type Dog { name: String }
fn main() {
    let mut d = Dog { name: "Rex" };
    d = Dog { name: "Buddy" };
}
"#,
    );
    let main = find_fn(&functions, "main");
    let shape = retain_release_shape(main);
    // The reassignment's own old-value release (one release — a plain
    // reassignment reads the existing local directly, no incidental
    // retain to cancel, unlike a field store), then the final scope-exit
    // release of `d` itself.
    assert_eq!(shape, vec!["release", "release"]);
}
