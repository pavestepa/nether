//! Совместный вывод контрактов (fixed point по графу вызовов) и «наивный»
//! однопроходный режим для сравнения.

use crate::analysis::{analyze, initial_contract, Derived, Diag};
use crate::contract::{Contract, RetKind};
use crate::mir::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub struct Outcome {
    pub contracts: BTreeMap<String, Contract>,
    pub diags: Vec<Diag>,
    /// Сколько раз анализировалось тело функции.
    pub analyses: usize,
}

fn callees(b: &Body) -> BTreeSet<String> {
    b.blocks
        .iter()
        .flat_map(|bl| bl.ins.iter())
        .filter_map(|i| match i {
            Ins::Def {
                rv: Rv::Call { f, .. },
                ..
            } => Some(f.clone()),
            _ => None,
        })
        .collect()
}

fn apply(c: &mut Contract, d: &Derived) -> bool {
    c.join(&d.consuming, &d.ret)
}

fn conflict_diag(b: &Body, c: &Contract) -> Option<Diag> {
    (c.ret.kind == RetKind::Conflict).then(|| Diag {
        code: "NMM_RESULT_CONTRACT",
        func: b.name.clone(),
        line: b.line,
        msg: "return paths produce incompatible result kinds (owned vs borrowed)".into(),
    })
}

/// Совместный вывод: worklist до неподвижной точки. `order` — порядок
/// первоначальной обработки (для проверки независимости от порядка).
pub fn infer_joint(bodies: &[Body], order: &[usize]) -> Outcome {
    let mut contracts: BTreeMap<String, Contract> = bodies
        .iter()
        .map(|b| (b.name.clone(), initial_contract(b)))
        .collect();
    let idx: BTreeMap<&str, usize> = bodies
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name.as_str(), i))
        .collect();
    let mut callers: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); bodies.len()];
    for (i, b) in bodies.iter().enumerate() {
        for c in callees(b) {
            callers[idx[c.as_str()]].insert(i);
        }
    }
    let mut queue: VecDeque<usize> = order.iter().copied().collect();
    let mut queued: BTreeSet<usize> = queue.iter().copied().collect();
    let mut diags: Vec<Vec<Diag>> = vec![Vec::new(); bodies.len()];
    let mut analyses = 0;
    while let Some(i) = queue.pop_front() {
        queued.remove(&i);
        analyses += 1;
        assert!(analyses < 1_000_000, "fixed point does not terminate");
        let (d, ds) = analyze(&bodies[i], &contracts);
        diags[i] = ds;
        let changed = apply(contracts.get_mut(&bodies[i].name).unwrap(), &d);
        if changed {
            for &c in &callers[i] {
                if queued.insert(c) {
                    queue.push_back(c);
                }
            }
        }
    }
    let mut all: Vec<Diag> = diags.into_iter().flatten().collect();
    all.extend(
        bodies
            .iter()
            .filter_map(|b| conflict_diag(b, &contracts[&b.name])),
    );
    Outcome {
        contracts,
        diags: all,
        analyses,
    }
}

pub fn source_order(n: usize) -> Vec<usize> {
    (0..n).collect()
}

/// Наивный режим: один проход в порядке исходника; ещё не проанализированные
/// и рекурсивные вызываемые считаются «пустым» контрактом, без итераций.
pub fn infer_naive(bodies: &[Body]) -> Outcome {
    let mut contracts: BTreeMap<String, Contract> = bodies
        .iter()
        .map(|b| (b.name.clone(), initial_contract(b)))
        .collect();
    let mut all = Vec::new();
    for b in bodies {
        let (d, ds) = analyze(b, &contracts);
        all.extend(ds);
        apply(contracts.get_mut(&b.name).unwrap(), &d);
    }
    all.extend(
        bodies
            .iter()
            .filter_map(|b| conflict_diag(b, &contracts[&b.name])),
    );
    Outcome {
        contracts,
        diags: all,
        analyses: bodies.len(),
    }
}

/// Проверка устойчивости: повторный анализ с итоговыми контрактами ничего не
/// меняет ни в контрактах, ни в диагностиках.
pub fn is_stable(bodies: &[Body], o: &Outcome) -> bool {
    let mut again: Vec<String> = Vec::new();
    for b in bodies {
        let (d, ds) = analyze(b, &o.contracts);
        let mut c = o.contracts[&b.name].clone();
        if apply(&mut c, &d) {
            return false;
        }
        again.extend(ds.iter().map(key));
    }
    again.extend(
        bodies
            .iter()
            .filter_map(|b| conflict_diag(b, &o.contracts[&b.name]))
            .map(|d| key(&d)),
    );
    again.sort();
    let mut first: Vec<String> = o.diags.iter().map(key).collect();
    first.sort();
    first == again
}

pub fn key(d: &Diag) -> String {
    format!("{}:{}:{}", d.func, d.line, d.code)
}

pub fn codes(o: &Outcome) -> BTreeSet<&'static str> {
    o.diags.iter().map(|d| d.code).collect()
}
