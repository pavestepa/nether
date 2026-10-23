//! Эксперименты над гипотезами H1–H6 (описаны в provenance-inference.md).

use crate::fuzz::{gen_program, Rng};
use crate::infer::*;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

pub struct Case {
    pub file: String,
    pub src: String,
    /// Ожидаемые коды (пусто = программа допустима).
    pub expect: BTreeSet<String>,
    pub contracts: Vec<String>,
    pub naive: Option<BTreeSet<String>>,
}

fn codes_of(s: &str) -> BTreeSet<String> {
    s.split_whitespace()
        .filter(|c| *c != "ok")
        .map(String::from)
        .collect()
}

pub fn load_cases(dir: &Path) -> Vec<Case> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "nr"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let src = std::fs::read_to_string(&p).unwrap();
            let mut c = Case {
                file: p.file_name().unwrap().to_string_lossy().into(),
                src: src.clone(),
                expect: BTreeSet::new(),
                contracts: Vec::new(),
                naive: None,
            };
            for l in src.lines() {
                if let Some(r) = l.strip_prefix("// EXPECT:") {
                    c.expect = codes_of(r);
                } else if let Some(r) = l.strip_prefix("// CONTRACT:") {
                    c.contracts.push(r.trim().into());
                } else if let Some(r) = l.strip_prefix("// NAIVE:") {
                    c.naive = Some(codes_of(r));
                }
            }
            c
        })
        .collect()
}

pub fn cases_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("cases")
}

pub fn str_codes(o: &Outcome) -> BTreeSet<String> {
    codes(o).into_iter().map(String::from).collect()
}

/// H2/H3: результат на размеченных кейсах. Возвращает список нарушений.
pub fn check_cases() -> (usize, Vec<String>) {
    let mut bad = Vec::new();
    let cases = load_cases(&cases_dir());
    for c in &cases {
        let bodies = match crate::compile(&c.src) {
            Ok(b) => b,
            Err(e) => {
                bad.push(format!("{}: compile error: {e}", c.file));
                continue;
            }
        };
        let o = infer_joint(&bodies, &source_order(bodies.len()));
        if str_codes(&o) != c.expect {
            bad.push(format!(
                "{}: expected {:?}, got {:?}",
                c.file,
                c.expect,
                str_codes(&o)
            ));
        }
        let rendered: Vec<String> = bodies
            .iter()
            .map(|b| o.contracts[&b.name].render(&b.name))
            .collect();
        for want in &c.contracts {
            if !rendered.contains(want) {
                bad.push(format!(
                    "{}: contract `{want}` not found in {rendered:?}",
                    c.file
                ));
            }
        }
        if let Some(n) = &c.naive {
            let no = infer_naive(&bodies);
            if &str_codes(&no) != n {
                bad.push(format!(
                    "{}: naive expected {n:?}, got {:?}",
                    c.file,
                    str_codes(&no)
                ));
            }
        }
    }
    (cases.len(), bad)
}

fn sig(o: &Outcome) -> (Vec<String>, Vec<String>) {
    let mut c: Vec<String> = o.contracts.iter().map(|(n, c)| c.render(n)).collect();
    c.sort();
    let mut d: Vec<String> = o
        .diags
        .iter()
        .map(|d| format!("{}:{}", d.func, d.code))
        .collect();
    d.sort();
    (c, d)
}

pub struct FuzzStats {
    pub programs: usize,
    pub generator_rejects: usize,
    pub order_variants: usize,
    pub order_mismatch: usize,
    pub unstable: usize,
    pub accepted: usize,
    pub rejected: usize,
    pub naive_unsound_accept: usize,
    pub naive_extra_reject: usize,
    pub max_analyses_ratio: f64,
    pub total_analyses: usize,
    pub total_functions: usize,
}

pub fn fuzz(n: u64) -> FuzzStats {
    let mut s = FuzzStats {
        programs: 0,
        generator_rejects: 0,
        order_variants: 0,
        order_mismatch: 0,
        unstable: 0,
        accepted: 0,
        rejected: 0,
        naive_unsound_accept: 0,
        naive_extra_reject: 0,
        max_analyses_ratio: 0.0,
        total_analyses: 0,
        total_functions: 0,
    };
    for seed in 0..n {
        let src = gen_program(seed);
        let Ok(bodies) = crate::compile(&src) else {
            s.generator_rejects += 1;
            continue;
        };
        s.programs += 1;
        let base = infer_joint(&bodies, &source_order(bodies.len()));
        if !is_stable(&bodies, &base) {
            s.unstable += 1;
        }
        let mut rng = Rng(seed ^ 0xABCDEF);
        for _ in 0..3 {
            let mut ord = source_order(bodies.len());
            rng.shuffle(&mut ord);
            let o = infer_joint(&bodies, &ord);
            s.order_variants += 1;
            if sig(&o) != sig(&base) {
                s.order_mismatch += 1;
            }
        }
        if base.diags.is_empty() {
            s.accepted += 1;
        } else {
            s.rejected += 1;
        }
        let nv = infer_naive(&bodies);
        let (jc, nc) = (str_codes(&base), str_codes(&nv));
        if !jc.is_empty() && nc.is_empty() {
            s.naive_unsound_accept += 1;
        }
        if jc.is_empty() && !nc.is_empty() {
            s.naive_extra_reject += 1;
        }
        s.total_analyses += base.analyses;
        s.total_functions += bodies.len();
        s.max_analyses_ratio = s
            .max_analyses_ratio
            .max(base.analyses as f64 / bodies.len() as f64);
    }
    s
}

/// Цепочка взаимных вызовов длины k: потребление выводится только с конца.
pub fn chain_program(k: usize, reverse: bool) -> String {
    let mut fs = Vec::new();
    for i in 0..k {
        let body = if i + 1 < k {
            format!("    if n < 1 {{ return }}\n    c{}(u, n - 1)\n", i + 1)
        } else {
            "    let b = new Box(u)\n".to_string()
        };
        fs.push(format!("fn c{i}(u: User, n: i32): () {{\n{body}}}\n"));
    }
    if reverse {
        fs.reverse();
    }
    let mut s = String::from("class User { age: i32 }\nclass Box { value: User }\nfn inspect(u: User): i32 { return u.age }\n");
    s.push_str(&fs.concat());
    s.push_str("fn caller(): i32 {\n    var o = new User(1)\n    let v = o\n    c0(o, 1)\n    return inspect(v)\n}\n");
    s
}

/// Абляции: какие размеченные негативные кейсы перестают отклоняться, если
/// из контракта убрать право доступа результата (бит 1) или его origins (бит 2).
pub fn ablation(bits: u8) -> (usize, Vec<String>) {
    let mut missed = Vec::new();
    let mut total = 0;
    for c in load_cases(&cases_dir()) {
        if c.expect.is_empty() {
            continue;
        }
        total += 1;
        let b = crate::compile(&c.src).unwrap();
        let o = crate::analysis::with_ablation(bits, || infer_joint(&b, &source_order(b.len())));
        if str_codes(&o) != c.expect {
            missed.push(c.file.clone());
        }
    }
    (total, missed)
}

/// Абляция при положительных кейсах: ложные отказы не должны появляться.
pub fn ablation_false_rejects(bits: u8) -> Vec<String> {
    load_cases(&cases_dir())
        .into_iter()
        .filter(|c| c.expect.is_empty())
        .filter(|c| {
            let b = crate::compile(&c.src).unwrap();
            !crate::analysis::with_ablation(bits, || infer_joint(&b, &source_order(b.len())))
                .diags
                .is_empty()
        })
        .map(|c| c.file)
        .collect()
}

pub fn report() -> String {
    let mut r = String::new();
    let (n, bad) = check_cases();
    writeln!(
        r,
        "== E1. Размеченные кейсы (H2, H3): {} кейсов, нарушений: {}",
        n,
        bad.len()
    )
    .unwrap();
    for b in &bad {
        writeln!(r, "   FAIL {b}").unwrap();
    }
    writeln!(
        r,
        "\n== E2. Совместный вывод против наивного однопроходного (H4)"
    )
    .unwrap();
    writeln!(r, "{:<34} {:<26} {:<26}", "кейс", "joint", "naive").unwrap();
    for c in load_cases(&cases_dir()) {
        if c.naive.is_some() {
            let b = crate::compile(&c.src).unwrap();
            let j = infer_joint(&b, &source_order(b.len()));
            let nv = infer_naive(&b);
            writeln!(
                r,
                "{:<34} {:<26} {:<26}",
                c.file,
                format!("{:?}", str_codes(&j)),
                format!("{:?}", str_codes(&nv))
            )
            .unwrap();
        }
    }
    writeln!(r, "\n== E5. Абляции контракта результата (H2)").unwrap();
    for (bits, name) in [
        (1u8, "без права доступа в результате"),
        (2, "без origins в результате"),
    ] {
        let (total, missed) = ablation(bits);
        writeln!(
            r,
            "{name}: пропущено {} из {} негативных кейсов: {:?}; ложных отказов: {:?}",
            missed.len(),
            total,
            missed,
            ablation_false_rejects(bits)
        )
        .unwrap();
    }
    writeln!(r, "\n== E3. Случайные программы (H1, H5, H6)").unwrap();
    let t = std::time::Instant::now();
    let f = fuzz(3000);
    writeln!(
        r,
        "программ: {} (отброшено генератором: {})",
        f.programs, f.generator_rejects
    )
    .unwrap();
    writeln!(r, "допущено / отклонено: {} / {}", f.accepted, f.rejected).unwrap();
    writeln!(
        r,
        "перестановки порядка: {}, расхождений: {}",
        f.order_variants, f.order_mismatch
    )
    .unwrap();
    writeln!(
        r,
        "нестабильных результатов (повторный анализ меняет контракт/диагностику): {}",
        f.unstable
    )
    .unwrap();
    writeln!(
        r,
        "наивный пропустил небезопасное: {}; наивный отклонил лишнее: {}",
        f.naive_unsound_accept, f.naive_extra_reject
    )
    .unwrap();
    writeln!(
        r,
        "анализов тел на функцию: среднее {:.2}, максимум {:.2}",
        f.total_analyses as f64 / f.total_functions as f64,
        f.max_analyses_ratio
    )
    .unwrap();
    writeln!(r, "время: {:?}", t.elapsed()).unwrap();
    writeln!(r, "\n== E4. Масштабирование цепочки взаимных вызовов (H6)").unwrap();
    writeln!(
        r,
        "{:>6} {:>10} {:>12} {:>12} {:>10}",
        "k", "порядок", "анализов", "анализов/k", "время"
    )
    .unwrap();
    for k in [10usize, 20, 40, 80, 160, 320] {
        for rev in [false, true] {
            let src = chain_program(k, rev);
            let b = crate::compile(&src).unwrap();
            let t = std::time::Instant::now();
            let o = infer_joint(&b, &source_order(b.len()));
            let dt = t.elapsed();
            assert!(str_codes(&o).contains("NMM_INVALIDATION"));
            writeln!(
                r,
                "{:>6} {:>10} {:>12} {:>12.2} {:>10.1?}",
                k,
                if rev {
                    "обратный"
                } else {
                    "прямой"
                },
                o.analyses,
                o.analyses as f64 / b.len() as f64,
                dt
            )
            .unwrap();
        }
    }
    r
}
