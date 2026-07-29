use nether_llvm::{Codegen, IntPredicate};

#[test]
fn target_and_optimization_configuration_is_validated() {
    let host = Codegen::host_triple();
    assert!(Codegen::with_target(&host, 3).is_ok());
    let invalid_level = match Codegen::with_target(&host, 4) {
        Ok(_) => panic!("O4 must be rejected"),
        Err(error) => error,
    };
    assert!(invalid_level.contains("expected O0..O3"));
    assert!(Codegen::with_target(
        "not-a-real-llvm-target",
        0
    )
    .is_err());
}

#[test]
fn builds_verifies_and_emits_a_simple_function() {
    let codegen = Codegen::new();
    let m = codegen.module("smoke");

    let i32_ty = m.int_type(32);
    let fn_ty = m.fn_type(&[i32_ty, i32_ty], Some(i32_ty));
    let add_fn = m.declare_function("add", fn_ty);
    let entry = m.append_block(add_fn, "entry");
    m.position_at_end(entry);
    let a = m.param(add_fn, 0);
    let b = m.param(add_fn, 1);
    let sum = m.int_add(a, b, "sum");
    m.ret(Some(sum));

    m.verify().expect("module should verify");
    let ir = m.print_to_string();
    assert!(ir.contains("define i32 @add"), "expected the IR to contain the defined function:\n{ir}");

    let out = std::env::temp_dir().join("nether_llvm_test_add.o");
    m.emit_object(&out).expect("emit_object should succeed");
    assert!(out.exists());
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn builds_a_branching_function_with_a_call() {
    let codegen = Codegen::new();
    let m = codegen.module("branch_smoke");

    let i32_ty = m.int_type(32);
    let bool_ty = m.bool_type();

    // fn is_positive(x: i32) -> i32 { if x > 0 { 1 } else { 0 } }
    let fn_ty = m.fn_type(&[i32_ty], Some(i32_ty));
    let f = m.declare_function("is_positive", fn_ty);
    let entry = m.append_block(f, "entry");
    let then_block = m.append_block(f, "then");
    let else_block = m.append_block(f, "else");

    m.position_at_end(entry);
    let x = m.param(f, 0);
    let zero = m.const_int(i32_ty, 0, false);
    let cond = m.int_compare(IntPredicate::SGT, x, zero, "cond");
    m.cond_br(cond, then_block, else_block);

    m.position_at_end(then_block);
    m.ret(Some(m.const_int(i32_ty, 1, false)));

    m.position_at_end(else_block);
    m.ret(Some(m.const_int(i32_ty, 0, false)));

    // A second function that calls the first — exercises `declare_function`
    // finding the already-defined function rather than redeclaring it.
    let caller_ty = m.fn_type(&[], Some(i32_ty));
    let caller = m.declare_function("caller", caller_ty);
    let caller_entry = m.append_block(caller, "entry");
    m.position_at_end(caller_entry);
    let five = m.const_int(i32_ty, 5, false);
    let result = m.call(f, &[five], "call_result").expect("is_positive returns a value");
    m.ret(Some(result));

    let _ = bool_ty;
    m.verify().expect("module should verify");
}

#[test]
fn struct_gep_load_store_roundtrip() {
    let codegen = Codegen::new();
    let m = codegen.module("struct_smoke");

    let i32_ty = m.int_type(32);
    let struct_ty = m.struct_type(&[i32_ty, i32_ty]);
    let struct_basic_ty: nether_llvm::Ty = struct_ty.into();

    let fn_ty = m.fn_type(&[], Some(i32_ty));
    let f = m.declare_function("make_point_sum", fn_ty);
    let entry = m.append_block(f, "entry");
    m.position_at_end(entry);

    let slot = m.alloca(struct_basic_ty, "point");
    let field0 = m.struct_gep(struct_ty, slot, 0, "x_ptr");
    m.store(field0, m.const_int(i32_ty, 3, false));
    let field1 = m.struct_gep(struct_ty, slot, 1, "y_ptr");
    m.store(field1, m.const_int(i32_ty, 4, false));

    let x = m.load(i32_ty, field0, "x");
    let y = m.load(i32_ty, field1, "y");
    let sum = m.int_add(x, y, "sum");
    m.ret(Some(sum));

    m.verify().expect("module should verify");
}
