use super::*;

fn await_terminators(f: &MirFunction) -> Vec<&Terminator> {
    f.blocks
        .iter()
        .map(|b| &b.terminator)
        .filter(|t| matches!(t, Terminator::Await { .. }))
        .collect()
}

#[test]
fn the_await_check_block_is_empty_so_resuming_never_recomputes_the_task() {
    // Regression test for a real design bug caught before it ever reached
    // codegen: an earlier version of this pass turned the *same* block
    // that computes `task` into the `Terminator::Await` block directly.
    // Since a pending suspend must re-enter that exact block on every
    // later poll, that shape would re-run the instructions that compute
    // `task` (here, the `fetch()` call) on every single re-poll —
    // silently discarding the original, still-pending task and starting
    // a fresh one each time instead of re-checking it. The check block
    // must contain none of its own instructions, so re-entering it only
    // ever re-runs the poll check.
    let functions = build(
        r#"
async fn fetch() String { return "ready"; }
async fn main() {
    let text = await fetch();
    println(text);
}
"#,
    );
    let main = find_fn(&functions, "main");
    let check_block = main
        .blocks
        .iter()
        .find(|b| matches!(b.terminator, Terminator::Await { .. }))
        .expect("expected a block ending in Terminator::Await");
    assert!(
        check_block.instrs.is_empty(),
        "the await-check block must have no instructions of its own, or resuming into it would redo them: {:?}",
        check_block.instrs
    );
    // Whatever block computes `fetch()`'s task must reach the check block
    // through a plain `Goto`, not directly own the `Await` terminator
    // itself.
    let computes_task = main.blocks.iter().find(|b| {
        matches!(&b.terminator, Terminator::Goto(target) if *target == check_block.id)
    });
    assert!(
        computes_task.is_some(),
        "expected a predecessor block reaching the check block via a plain Goto"
    );
}

#[test]
fn a_single_await_splits_its_block_and_relocates_the_relocated_release() {
    let functions = build(
        r#"
async fn fetch() String { return "ready"; }
async fn main() {
    let text = await fetch();
    println(text);
}
"#,
    );
    let main = find_fn(&functions, "main");

    let awaits = await_terminators(main);
    assert_eq!(awaits.len(), 1, "expected exactly one Terminator::Await");
    let Terminator::Await {
        output_local,
        resume,
        ..
    } = awaits[0]
    else {
        unreachable!()
    };

    // No `Rvalue::Await` should survive the split anywhere in the
    // function — it's fully replaced by the terminator.
    assert!(
        !all_instrs(main)
            .iter()
            .any(|i| matches!(i, Instr::Assign(_, Rvalue::Await(_)))),
        "Rvalue::Await must not remain after split_await_points"
    );

    // The resume block must exist and must not itself contain the await
    // assignment — the temporary/task's release that `insert_arc`/
    // `build_mir` placed right after the await must have moved there with
    // the rest of the tail, not stayed behind on the check side.
    let resume_block = main.block(*resume);
    assert!(
        resume_block
            .instrs
            .iter()
            .any(|i| matches!(i, Instr::Release(_))),
        "the awaited task's temporary release must land in the resume block"
    );
    assert_eq!(
        resume_block.instrs.iter().filter(|i| matches!(i, Instr::Assign(place, _) if place.local == *output_local)).count(),
        0,
        "the resume block must not re-assign the output local — only the terminator does"
    );
}

#[test]
fn sequential_awaits_in_one_source_block_each_get_their_own_suspend_point() {
    let functions = build(
        r#"
async fn fetch() String { return "ready"; }
async fn answer() i32 { return 42; }
async fn main() {
    let text = await fetch();
    let value = await answer();
    println(`${text}:${value}`);
}
"#,
    );
    let main = find_fn(&functions, "main");
    assert_eq!(
        await_terminators(main).len(),
        2,
        "two sequential awaits in one original block must produce two suspend points"
    );
}

#[test]
fn await_inside_a_while_loop_body_splits_correctly() {
    let functions = build(
        r#"
async fn tick() i32 { return 1; }
async fn main() {
    let mut n = 0;
    while n < 3 {
        let v = await tick();
        n = n + v;
    }
}
"#,
    );
    let main = find_fn(&functions, "main");
    assert_eq!(
        await_terminators(main).len(),
        1,
        "one await inside a loop body is still exactly one suspend point"
    );
}

#[test]
fn await_inside_both_if_arms_gets_distinct_suspend_points() {
    let functions = build(
        r#"
async fn a() i32 { return 1; }
async fn b() i32 { return 2; }
async fn main() {
    let flag = true;
    let v = if flag { await a() } else { await b() };
    println(`${v}`);
}
"#,
    );
    let main = find_fn(&functions, "main");
    let awaits = await_terminators(main);
    assert_eq!(
        awaits.len(),
        2,
        "an await in each if/else arm must produce two independent suspend points"
    );
    let resumes: Vec<BlockId> = awaits
        .iter()
        .map(|t| match t {
            Terminator::Await { resume, .. } => *resume,
            _ => unreachable!(),
        })
        .collect();
    assert_ne!(
        resumes[0], resumes[1],
        "the two arms' suspend points must resume into distinct blocks"
    );
}

#[test]
fn sync_functions_are_never_touched_by_the_split() {
    let functions = build(
        r#"
fn main() {
    let x = 1 + 2;
    println(`${x}`);
}
"#,
    );
    let main = find_fn(&functions, "main");
    assert!(await_terminators(main).is_empty());
}
