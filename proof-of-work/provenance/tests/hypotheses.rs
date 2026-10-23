use provenance::experiments::*;
use provenance::infer::*;

#[test]
fn e1_labeled_cases_match_expectations() {
    let (n, bad) = check_cases();
    assert!(n >= 30, "too few cases: {n}");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn e2_naive_single_pass_is_unsound() {
    for name in ["neg_16_transitive_consume.nr", "neg_17_recursive_borrow.nr"] {
        let c = load_cases(&cases_dir())
            .into_iter()
            .find(|c| c.file == name)
            .unwrap();
        let b = provenance::compile(&c.src).unwrap();
        assert!(
            !infer_joint(&b, &source_order(b.len())).diags.is_empty(),
            "{name}: joint must reject"
        );
        assert!(
            infer_naive(&b).diags.is_empty(),
            "{name}: naive is expected to accept (unsound)"
        );
    }
}

#[test]
fn e3_random_programs_are_stable_and_order_independent() {
    let f = fuzz(1500);
    assert_eq!(
        f.generator_rejects, 0,
        "generator must produce well-typed programs"
    );
    assert!(f.programs > 1000);
    assert_eq!(f.unstable, 0);
    assert_eq!(f.order_mismatch, 0);
    assert!(
        f.accepted > 100 && f.rejected > 100,
        "workload must exercise both outcomes"
    );
}

#[test]
fn e4_chain_scales_linearly_in_any_declaration_order() {
    for rev in [false, true] {
        let mut prev = None;
        for k in [20usize, 40, 80] {
            let b = provenance::compile(&chain_program(k, rev)).unwrap();
            let o = infer_joint(&b, &source_order(b.len()));
            assert!(str_codes(&o).contains("NMM_INVALIDATION"));
            let per = o.analyses as f64 / b.len() as f64;
            assert!(per < 4.0, "k={k} rev={rev}: {per} analyses per function");
            if let Some(p) = prev {
                assert!(per < p * 1.5 + 0.5, "growth is not linear");
            }
            prev = Some(per);
        }
    }
}

#[test]
fn e5_ablations_show_result_provenance_is_necessary() {
    let (total, missed_access) = ablation(1);
    assert!(total >= 20);
    for f in ["neg_03_return_launder.nr", "neg_20_return_launder_swap.nr"] {
        assert!(
            missed_access.iter().any(|m| m == f),
            "{f} must be accepted without result access: {missed_access:?}"
        );
    }
    let (_, missed_origin) = ablation(2);
    for f in ["neg_17_recursive_borrow.nr", "neg_21_result_origin.nr"] {
        assert!(
            missed_origin.iter().any(|m| m == f),
            "{f} must be accepted without result origins: {missed_origin:?}"
        );
    }
    // Потеря access не даёт ложных отказов; потеря origins ломает и допустимую
    // рекурсию (результат вида Borrowed смешивается с Fresh).
    assert!(ablation_false_rejects(1).is_empty());
    assert_eq!(
        ablation_false_rejects(2),
        vec!["pos_07_recursion.nr".to_string()]
    );
}
