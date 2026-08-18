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
