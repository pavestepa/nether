use super::*;

#[test]
fn async_calls_produce_tasks_and_await_extracts_the_output() {
    assert_ok(
        r#"
async fn fetch() i32 { return 7; }
async fn main() {
    await timer.sleep(1);
    let pending = task.spawn(fetch());
    let value i32 = await pending;
}
"#,
    );
}

#[test]
fn await_requires_an_async_body_and_a_task_operand() {
    assert_err(
        r#"
async fn fetch() i32 { return 7; }
fn main() { let value = await fetch(); }
"#,
        "`await` is only allowed inside an `async fn`",
    );
    assert_err(
        r#"
async fn main() { let value = await 7; }
"#,
        "expected a task",
    );
}

#[test]
fn task_spawn_requires_an_async_body() {
    // `task.spawn`'s real scheduling (`nether_rt_task_spawn`) only ever
    // runs from inside a Tokio runtime that `nether_rt_task_block_on`
    // set up — which only exists once an `async fn` (ultimately `async
    // fn main`) is on the call stack. A fully synchronous program could
    // reach `task.spawn` with no runtime active at all, so this is a
    // compile-time restriction, not just an async-context nicety —
    // mirrors `await`'s own existing rule.
    assert_err(
        r#"
async fn fetch() i32 { return 7; }
fn main() { let pending = task.spawn(fetch()); }
"#,
        "`task.spawn` is only allowed inside an `async fn`",
    );
}

#[test]
fn task_spawn_rejects_capturing_a_borrow_of_a_local_owned_by_the_spawning_function() {
    // Stage 6's spawned-borrow diagnostic (language-spec §19) applies to
    // `task.spawn` too, not just `thread.spawn` — a detached task's
    // lifetime is independent of the spawning function's own stack
    // frame, exactly the same hazard a real OS thread has. Before Stage
    // 6 this had zero enforcement at all: an async fn's reference-typed
    // parameters were ordinary `BorrowOrigin::Parameter`s, whose "the
    // referent necessarily outlives this invocation" assumption only
    // holds for an *ordinary synchronous* call.
    assert_err(
        r#"
struct Dog { name String }
async fn fetch(d: &Dog) String { return d.name; }
async fn main() {
    let dog: Dog = :Dog { name = "Rex" };
    let r: &Dog = dog;
    let pending = task.spawn(fetch(r));
}
"#,
        "cannot spawn a task capturing a borrow of a local owned by this function",
    );
}
