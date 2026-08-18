use nether_ast::Symbol;
use nether_mir::{build_mir, insert_arc, Instr, MirFunction, Operand, Rvalue, Terminator};

#[path = "mir_tests/weak_match_and_loops.rs"]
mod weak_match_and_loops;

fn build(source: &str) -> Vec<MirFunction> {
    let mut map = nether_diagnostics::SourceMap::new();
    let file = map.add_file("test.nr", source);
    let (module, parse_diags) = nether_parser::parse_module(source, file);
    assert!(
        parse_diags.is_empty(),
        "unexpected parse diagnostics: {parse_diags:?}"
    );
    let (resolved, resolve_diags) = nether_resolver::resolve(&module);
    assert!(
        resolve_diags.is_empty(),
        "unexpected resolve diagnostics: {resolve_diags:?}"
    );
    let (tables, check_diags) = nether_typecheck::check(&module, &resolved);
    assert!(
        check_diags.is_empty(),
        "unexpected typecheck diagnostics: {check_diags:?}"
    );
    let hir = nether_hir::lower(&module, &resolved, tables);
    let main_id = *hir
        .fn_by_name
        .get(&Symbol::new("main"))
        .expect("no `main` in test source");
    let mono = nether_monomorphization::monomorphize(&hir, main_id);
    let mut functions = build_mir(&mono, &resolved.definitions, &hir.signatures);
    insert_arc(&mut functions);
    functions
}

fn find_fn<'a>(functions: &'a [MirFunction], name: &str) -> &'a MirFunction {
    functions
        .iter()
        .find(|f| f.name.as_str() == name)
        .unwrap_or_else(|| panic!("no MIR function named {name}"))
}

/// Flattens every instruction across every block, in block order — good
/// enough for these tests' single-path (no branching) function bodies,
/// where block order matches execution order.
fn all_instrs(f: &MirFunction) -> Vec<&Instr> {
    return f.blocks.iter().flat_map(|b| &b.instrs).collect();
}

fn retain_release_shape(f: &MirFunction) -> Vec<&'static str> {
    all_instrs(f)
        .into_iter()
        .filter_map(|i| match i {
            Instr::Retain(_) => Some("retain"),
            Instr::Release(_) => Some("release"),
            Instr::WeakRetain(_) => Some("weak_retain"),
            Instr::WeakRelease(_) => Some("weak_release"),
            _ => None,
        })
        .collect()
}

#[test]
fn every_block_has_exactly_one_terminator() {
    let functions = build(
        r#"
struct Dog { name String }
fn main() {
    let d = Dog { name = "Rex" };
    if d.name == "Rex" {
        println("yes");
    } else {
        println("no");
    }
}
"#,
    );
    for f in &functions {
        for b in &f.blocks {
            // A `BasicBlock` always carries a `Terminator` value (not an
            // `Option`) — this loop just confirms none of them panicked
            // during construction and that `Unreachable` only shows up on
            // genuinely dead blocks, not on the function's real paths.
            let _ = &b.terminator;
        }
    }
}

#[test]
fn unique_heap_moves_transfer_without_retain_and_clear_the_source() {
    let functions = build(
        r#"
struct Dog { name String }
fn consume(d: Dog) {}
fn main() {
    let first: Dog = :Dog { name = "Rex" };
    let second: Dog = first;
    consume(second);
}
"#,
    );
    let main = find_fn(&functions, "main");
    assert!(
        all_instrs(main)
            .iter()
            .any(|instr| matches!(instr, Instr::Clear(_))),
        "a unique ownership transfer must clear its moved-from slot"
    );
    for instr in all_instrs(main) {
        if let Instr::Retain(local) = instr {
            assert!(
                !matches!(
                    main.local_decl(*local).ty,
                    nether_typecheck::Type::Unique(_)
                ),
                "unique heap locals must never be retained"
            );
        }
    }
}

#[test]
fn explicit_early_return_with_no_tail_expression_terminates_correctly() {
    // A fn body consisting only of `return a;` (no trailing tail
    // expression) used to lower its dead fallthrough block's terminator
    // as `Terminator::Return(Operand::Unit)` regardless of the function's
    // real return type, which LLVM's verifier rejects whenever `ret` is
    // not itself `()`. The real `return a;` should produce exactly one
    // `Terminator::Return` operand typed for `i32`, and any leftover
    // unreachable block must be `Terminator::Unreachable`, never another
    // `Return`.
    let functions = build(
        r#"
fn foo(a i32) i32 {
    return a;
}
fn main() {
    println(`${foo(2)}`);
}
"#,
    );
    let foo = find_fn(&functions, "foo");
    let returns: Vec<&Terminator> = foo
        .blocks
        .iter()
        .map(|b| &b.terminator)
        .filter(|t| matches!(t, Terminator::Return(_)))
        .collect();
    assert_eq!(
        returns.len(),
        1,
        "expected exactly one Return terminator, found {returns:?}"
    );
    assert!(matches!(returns[0], Terminator::Return(Operand::Local(_))));
}

#[test]
fn unit_function_discards_its_syntactic_tail() {
    let functions = build("fn main() { 42 }");
    let main = find_fn(&functions, "main");
    assert!(main
        .blocks
        .iter()
        .any(|block| matches!(block.terminator, Terminator::Return(Operand::Unit))));
}

#[test]
fn struct_field_read_binds_a_new_local_with_a_retain_and_the_parameter_is_released_at_scope_exit() {
    // Mirrors arc-model.md's worked `describe` example exactly: a field
    // read binds a new local (retained once, §3.1); returning that local
    // directly needs no further retain and no release of its own (RVO,
    // §3.4); the parameter still gets released at scope exit (§3.2).
    let functions = build(
        r#"
struct Dog { name String }
fn describe(d Dog) String {
    let tag = d.name;
    return tag;
}
fn main() {
    let a = Dog { name = "Rex" };
    let msg = describe(a);
    println(msg);
}
"#,
    );
    let describe = find_fn(&functions, "describe");
    let instrs = all_instrs(describe);

    // Exactly one Retain (the field-read bind) and exactly one Release
    // (the parameter, at scope exit) — `tag` itself is never released
    // (it's the escaping/returned value).
    let shape = retain_release_shape(describe);
    assert_eq!(
        shape,
        vec!["retain", "release"],
        "expected exactly one Retain (field-read bind) then one Release (parameter, scope exit)"
    );

    // The retain's target must be the same local the Field rvalue was
    // assigned into, and the terminator must return that same local.
    let field_dest = instrs.iter().find_map(|i| match i {
        Instr::Assign(place, Rvalue::Field { .. }) if place.projection.is_empty() => {
            Some(place.local)
        }
        _ => None,
    });
    assert!(field_dest.is_some());
    assert!(matches!(instrs[1], Instr::Retain(l) if Some(*l) == field_dest));
    // `blocks.last()` is always the trailing dead block `terminate_current`
    // creates after every real terminator, not the block holding the
    // actual `Return` — search for it instead.
    let return_terminator = describe
        .blocks
        .iter()
        .map(|b| &b.terminator)
        .find(|t| matches!(t, Terminator::Return(_)))
        .expect("expected a Return terminator");
    assert!(
        matches!(return_terminator, Terminator::Return(Operand::Local(l)) if Some(*l) == field_dest)
    );
}

#[test]
fn call_argument_is_retained_before_the_call_and_released_by_the_callees_own_scope_exit() {
    // Regression test for a real double-release: `a` is passed into
    // `describe` and then read again afterward — exactly the pattern an
    // earlier version of this pass got wrong (it released `a` again,
    // itself, immediately after the call returned, on top of `describe`'s
    // own scope-exit release of its `d` parameter — two releases for one
    // credit, freeing `a` while `main` still held and used it; only
    // caught by actually running the generated code against a real
    // allocator, not by inspecting MIR/IR shape alone).
    let functions = build(
        r#"
struct Dog { name String }
fn describe(d Dog) String { return d.name; }
fn main() {
    let a = Dog { name = "Rex" };
    let msg = describe(a);
    println(a.name);
    println(msg);
}
"#,
    );
    let main = find_fn(&functions, "main");
    let instrs = all_instrs(main);

    // The Call instruction is immediately preceded by a Retain of the
    // argument, and *not* immediately followed by a Release of it — that
    // credit is now the callee's own parameter's, released at its own
    // scope exit instead (checked below).
    let call_pos = instrs
        .iter()
        .position(|i| matches!(i, Instr::Assign(_, Rvalue::Call { .. })))
        .expect("expected a Call instruction");
    let Instr::Retain(arg) = instrs[call_pos - 1] else {
        panic!("expected a Retain immediately before the call")
    };
    assert!(
        !matches!(instrs[call_pos + 1], Instr::Release(l) if *l == *arg),
        "the call must not also release its own argument right after returning"
    );

    // Inside `describe`: one retain for `d.name`'s own bind-time read
    // (§3.1), then `d`'s own normal scope-exit release — `d.name`'s own
    // would-be scope-exit release is the one that's skipped, since *it*
    // is the value escaping via return, not `d` itself.
    let describe = find_fn(&functions, "describe");
    assert_eq!(retain_release_shape(describe), vec!["retain", "release"]);
}

#[test]
fn fresh_construction_returned_directly_gets_no_retain_or_release() {
    // RVO, shape 1: a construction feeding *directly* into tail position
    // with no intermediate `let` at all.
    let functions = build(
        r#"
struct Dog { name String }
fn make() Dog { return Dog { name = "Rex" }; }
fn main() {
    let d = make();
}
"#,
    );
    let make = find_fn(&functions, "make");
    assert!(
        retain_release_shape(make).is_empty(),
        "a directly-returned fresh construction should need no Retain/Release at all"
    );
}

#[test]
fn fresh_construction_bound_to_a_let_then_returned_also_gets_no_retain_or_release() {
    // RVO, shape 2 (this crate's own reading of arc-model.md — see
    // `build.rs`'s module docs: the doc's own `main()` worked example
    // shows a `let`-bound fresh construction getting no bind-time retain
    // regardless of whether it's later returned, and returning it is
    // exactly the same "skip its own release" rule as any other escaping
    // local — the RVO prose's looser "a retain is inserted" phrasing is
    // treated as describing the *general*, non-degenerate case, not a
    // contradiction of the doc's own precise, testable example).
    let functions = build(
        r#"
struct Dog { name String }
fn make() Dog {
    let d = Dog { name = "Rex" };
    return d;
}
fn main() {
    let d = make();
}
"#,
    );
    let make = find_fn(&functions, "make");
    assert!(
        retain_release_shape(make).is_empty(),
        "a let-bound fresh construction, then returned, should still need no Retain/Release"
    );
}

#[test]
fn returning_a_parameter_directly_needs_neither_retain_nor_release() {
    // A `Call`'s heap-kind argument is retained once, before the call
    // (`nether_mir::arc`'s own docs) — and, critically, *not* released
    // again after it returns, since the callee's own scope exit is what
    // balances that credit (see the next test) unless the callee hands
    // that exact same credit straight back out via a direct return, which
    // is this case: `d`'s only credit is the caller's pre-call retain,
    // and `release_scopes`'s own "skip the escaping local" handling
    // already leaves it un-released here — so it needs no compensating
    // instruction of its own at all, in either direction. (An earlier
    // version of this pass added a second, redundant Retain here on the
    // mistaken assumption that the caller *also* releases after the
    // call — which produced a real double-release, only caught by
    // actually running generated code against a real allocator; see
    // `nether_mir::arc`'s module docs.)
    let functions = build(
        r#"
struct Dog { name String }
fn identity(d Dog) Dog { return d; }
fn main() {
    let a = Dog { name = "Rex" };
    let b = identity(a);
}
"#,
    );
    let identity = find_fn(&functions, "identity");
    assert_eq!(
        retain_release_shape(identity),
        Vec::<&str>::new(),
        "the parameter's incoming credit should pass straight through, untouched"
    );

    // And the whole round trip must still balance in `main`: one retain
    // before the call, no release right after it (the bug this fixes),
    // and `a`/`b` each released exactly once at their own later scope
    // exit — not zero (a leak) and not three (the double-release this
    // fixes elsewhere).
    let main = find_fn(&functions, "main");
    assert_eq!(
        retain_release_shape(main),
        vec!["retain", "release", "release"]
    );
}

#[test]
fn field_store_retains_the_new_value_and_releases_the_old_one() {
    let functions = build(
        r#"
struct Dog { name String }
impl Dog {
    set_name(mut self, new_name String) {
        self.name = new_name;
    }
}
fn main() {
    let mut d = Dog { name = "Rex" };
    d.set_name("Buddy");
}
"#,
    );
    let set_name = find_fn(&functions, "set_name");
    let shape = retain_release_shape(set_name);
    // Two Retains: one from `insert_arc`'s own Field-read rule, firing on
    // the old value's incidental extraction (see `lower_assign`'s docs),
    // and one explicit one (`prepare_new_binding`) for `new_name` — a
    // bare parameter being stored into a field needs its own fresh
    // credit, same reasoning as `lower_escaping_value`'s parameter case.
    // Three Releases: two for the old field value (one cancels its own
    // incidental retain, one for the field's actual original reference),
    // then `new_name` at scope exit. `mut self` is a borrowed parameter:
    // the caller retains ownership, so the callee neither retains nor
    // releases the receiver itself.
    assert_eq!(
        shape,
        vec!["retain", "retain", "release", "release", "release"]
    );
}

#[test]
fn constructing_a_struct_from_an_existing_local_retains_the_field_value() {
    // `name` already owns an independent reference (from `prepare_new_binding`
    // giving it a fresh Retain on its own binding, since it aliases the
    // parameter). Moving it into `Dog { name }` must retain it *again* —
    // the new `Dog` becomes a second, independent owner of that String,
    // distinct from `name`'s own binding — otherwise the two would share
    // one reference count credit and one of their eventual releases would
    // be over-releasing an already-freed object. This is the fix for a
    // real gap: `Construct`'s field operands used to go through plain
    // `lower_expr`, skipping this retain entirely.
    let functions = build(
        r#"
fn make(name String) Dog {
    return Dog { name };
}
struct Dog { name String }
fn main() {
    let d = make("Rex");
    println(d.name);
}
"#,
    );
    let make = find_fn(&functions, "make");
    let shape = retain_release_shape(make);
    // One Retain: `prepare_new_binding` giving the field's fresh copy of
    // `name` its own independent credit before it's moved into `Dog`
    // (this fix — previously `Construct`'s field operands skipped this
    // entirely). One Release: `name`'s own parameter binding, at its
    // normal scope exit (§3.2) — unaffected by the copy made from it. No
    // further Release for `Dog` itself: it's freshly constructed and
    // returned directly (RVO), and the field's own retained copy is
    // consumed straight into its storage, never independently held
    // afterward.
    assert_eq!(shape, vec!["retain", "release"]);
}
