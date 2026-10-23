//! Анализ одной функции: потоковые состояния O/R/W, происхождение (origins),
//! loans по liveness и вывод производного контракта.
//!
//! Модель (см. `../../provenance-inference.md`):
//! * `Origin` — абстрактная цель: параметр или место создания объекта;
//! * у каждого non-Copy локала есть вид (`Owner`, `Alias`, корень параметра),
//!   множество origins, право доступа `acc` и режим loan;
//! * заимствование конфликтует с живым (по liveness) пересекающимся alias,
//!   если не является его предком в цепочке reborrow.

use crate::contract::{Acc, Contract, RetC, RetKind};
use crate::mir::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

thread_local! {
    static ABLATION: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// Бит 1 — контракт результата не несёт права доступа; бит 2 — не несёт origins.
/// Используется только экспериментами-абляциями.
pub fn with_ablation<T>(bits: u8, f: impl FnOnce() -> T) -> T {
    ABLATION.with(|a| a.set(bits));
    let r = f();
    ABLATION.with(|a| a.set(0));
    r
}

fn ablated(bit: u8) -> bool {
    ABLATION.with(|a| a.get() & bit != 0)
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub code: &'static str,
    pub func: String,
    pub line: usize,
    pub msg: String,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Origin {
    Param(usize),
    Alloc(usize),
}
type OSet = BTreeSet<Origin>;

#[derive(Clone, PartialEq, Eq, Debug)]
enum Kind {
    Copy,
    Owner,
    Alias,
    ParamRoot(usize),
    /// Разные виды на разных путях: owner-token не определён однозначно.
    Mixed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Loc {
    kind: Kind,
    origins: OSet,
    acc: Acc,
    loan: Option<Acc>,
    parents: BTreeSet<L>,
    moved: bool,
    /// Значение пришло из вызова, контракт которого ещё не известен.
    unk: bool,
}

type State = Vec<Option<Loc>>;

fn merge_loc(a: &Loc, b: &Loc) -> Loc {
    Loc {
        kind: if a.kind == b.kind {
            a.kind.clone()
        } else {
            Kind::Mixed
        },
        origins: a.origins.union(&b.origins).copied().collect(),
        acc: a.acc.max(b.acc),
        loan: match (a.loan, b.loan) {
            (Some(x), Some(y)) => Some(x.min(y)), // W < R: берём W как более сильный loan
            (x, None) | (None, x) => x,
        },
        parents: a.parents.union(&b.parents).copied().collect(),
        moved: a.moved || b.moved,
        unk: a.unk || b.unk,
    }
}

fn merge_state(into: &mut State, from: &State) -> bool {
    let mut changed = false;
    for (i, f) in from.iter().enumerate() {
        let merged = match (&into[i], f) {
            (Some(a), Some(b)) => Some(merge_loc(a, b)),
            (None, Some(b)) => Some(b.clone()),
            (a, None) => a.clone(),
        };
        if merged != into[i] {
            into[i] = merged;
            changed = true;
        }
    }
    changed
}

/// Производный контракт, собранный по телу функции.
#[derive(Clone, Debug)]
pub struct Derived {
    pub consuming: Vec<bool>,
    pub ret: RetC,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum M {
    R,
    W,
    Consume,
}

type Live = Vec<Vec<BTreeSet<L>>>;

fn uses(i: &Ins) -> Vec<L> {
    match i {
        Ins::Def { rv, .. } => match rv {
            Rv::Const => vec![],
            Rv::Bin(a, b) => vec![*a, *b],
            Rv::Use(a) => vec![*a],
            Rv::Field { base, .. } => vec![*base],
            Rv::New { args, .. } | Rv::Call { args, .. } => args.clone(),
        },
        Ins::FieldWrite { base, src, .. } => vec![*base, *src],
    }
}

fn term_uses(t: &Term) -> Vec<L> {
    match t {
        Term::Goto(_) => vec![],
        Term::Branch { cond, .. } => vec![*cond],
        Term::Ret { val, .. } => val.iter().copied().collect(),
    }
}

fn succs(t: &Term) -> Vec<usize> {
    match t {
        Term::Goto(b) => vec![*b],
        Term::Branch { t, f, .. } => vec![*t, *f],
        Term::Ret { .. } => vec![],
    }
}

/// Обратный анализ живости; `live[b][i]` — множество живых после инструкции `i`.
fn liveness(body: &Body) -> Live {
    let n = body.blocks.len();
    let mut live_in: Vec<BTreeSet<L>> = vec![BTreeSet::new(); n];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..n).rev() {
            let blk = &body.blocks[b];
            let mut cur: BTreeSet<L> = succs(&blk.term)
                .iter()
                .flat_map(|s| live_in[*s].iter().copied())
                .collect();
            cur.extend(term_uses(&blk.term));
            for ins in blk.ins.iter().rev() {
                if let Ins::Def { dst, .. } = ins {
                    cur.remove(dst);
                }
                cur.extend(uses(ins));
            }
            if cur != live_in[b] {
                live_in[b] = cur;
                changed = true;
            }
        }
    }
    let mut out = Vec::new();
    for blk in &body.blocks {
        let mut cur: BTreeSet<L> = succs(&blk.term)
            .iter()
            .flat_map(|s| live_in[*s].iter().copied())
            .collect();
        cur.extend(term_uses(&blk.term));
        let mut v = vec![BTreeSet::new(); blk.ins.len()];
        for (i, ins) in blk.ins.iter().enumerate().rev() {
            v[i] = cur.clone();
            if let Ins::Def { dst, .. } = ins {
                cur.remove(dst);
            }
            cur.extend(uses(ins));
        }
        out.push(v);
    }
    out
}

enum Part {
    /// Возврат корня параметра: вид (Borrowed/Transferred) зависит от итогового
    /// признака consuming, поэтому определяется после обхода всего тела.
    Root(usize),
    Done(RetC),
}

struct A<'a> {
    parts: Vec<Part>,
    body: &'a Body,
    contracts: &'a BTreeMap<String, Contract>,
    live: Live,
    /// Финальный проход: собираем диагностики и производный контракт.
    fin: bool,
    diags: Vec<Diag>,
    derived: Derived,
}

fn overlap(a: &OSet, b: &OSet) -> bool {
    a.iter().any(|x| b.contains(x))
}

fn ancestors(st: &State, x: L) -> BTreeSet<L> {
    let mut seen = BTreeSet::new();
    let mut q = VecDeque::from([x]);
    while let Some(c) = q.pop_front() {
        if let Some(Some(l)) = st.get(c) {
            for p in &l.parents {
                if seen.insert(*p) {
                    q.push_back(*p);
                }
            }
        }
    }
    seen
}

impl A<'_> {
    fn err(&mut self, code: &'static str, line: usize, msg: String) {
        if self.fin {
            self.diags.push(Diag {
                code,
                func: self.body.name.clone(),
                line,
                msg,
            });
        }
    }
    fn copy(&self, l: L) -> bool {
        self.body.locals[l].ty.is_copy()
    }
    fn name(&self, l: L) -> String {
        self.body.locals[l].name.clone()
    }

    fn check_live(&mut self, st: &State, x: L, line: usize) -> bool {
        match &st[x] {
            None => false,
            Some(l) if l.moved => {
                let n = self.name(x);
                self.err(
                    "NMM_USE_AFTER_TRANSFER",
                    line,
                    format!("'{n}' used after its ownership was transferred"),
                );
                false
            }
            Some(_) => true,
        }
    }

    /// Доступ к `x` в режиме `mode` относительно живых пересекающихся loans.
    fn access(&mut self, st: &State, live: &BTreeSet<L>, x: L, mode: M, line: usize) {
        if self.copy(x) || !self.check_live(st, x, line) {
            return;
        }
        let lx = st[x].as_ref().unwrap();
        if mode == M::W && lx.acc != Acc::W {
            let n = self.name(x);
            self.err(
                "NMM_READONLY_TO_WRITE",
                line,
                format!("write access through readonly path '{n}'"),
            );
        }
        let anc = ancestors(st, x);
        let mut found: Vec<(&'static str, String)> = Vec::new();
        for &y in live {
            if y == x || self.copy(y) || anc.contains(&y) {
                continue;
            }
            let Some(ly) = st[y].as_ref() else { continue };
            if ly.moved || !overlap(&lx.origins, &ly.origins) {
                continue;
            }
            let Some(lm) = ly.loan else { continue };
            let (xn, yn) = (self.name(x), self.name(y));
            if mode == M::Consume {
                found.push((
                    "NMM_INVALIDATION",
                    format!("'{xn}' is transferred while view '{yn}' is still used"),
                ));
            } else if mode == M::W || lm == Acc::W {
                let desc = ancestors(st, y).contains(&x);
                let code = if desc && lm == Acc::W {
                    "NMM_SUSPENDED_ACCESS"
                } else {
                    "NMM_BORROW_CONFLICT"
                };
                found.push((
                    code,
                    format!("access to '{xn}' conflicts with live borrow '{yn}'"),
                ));
            }
        }
        for (c, m) in found {
            self.err(c, line, m);
        }
    }

    /// Потребляющая позиция: нужен owner-token.
    fn sink(&mut self, st: &mut State, live: &BTreeSet<L>, x: L, line: usize) {
        if self.copy(x) {
            return;
        }
        // Признак consuming не зависит от того, нашлись ли в этой точке другие
        // ошибки: иначе вывод контракта немонотонен и зависит от порядка.
        if let Some(Kind::ParamRoot(i)) = st[x].as_ref().map(|l| l.kind.clone()) {
            if self.fin && !self.body.params[i].var {
                self.derived.consuming[i] = true;
            }
        }
        if !self.check_live(st, x, line) {
            return;
        }
        match st[x].as_ref().unwrap().kind.clone() {
            Kind::Owner => {}
            Kind::ParamRoot(i) => {
                if self.body.params[i].var {
                    let n = self.name(x);
                    self.err(
                        "NMM_VAR_CONSUME",
                        line,
                        format!("`var` parameter '{n}' cannot be consumed"),
                    );
                    return;
                }
            }
            _ => {
                let n = self.name(x);
                self.err(
                    "NMM_NO_OWNER_TOKEN",
                    line,
                    format!("'{n}' is a borrowed path and cannot be passed to an owning position"),
                );
                return;
            }
        }
        self.access(st, live, x, M::Consume, line);
        st[x].as_mut().unwrap().moved = true;
    }

    fn alias_of(&mut self, st: &State, live: &BTreeSet<L>, dst: L, srcs: &[L], line: usize) -> Loc {
        let (dmut, dtemp) = (self.body.locals[dst].mutable, self.body.locals[dst].temp);
        let mut origins = OSet::new();
        let mut acc = Acc::W;
        let mut unk = false;
        for &s in srcs {
            if let Some(l) = &st[s] {
                origins.extend(l.origins.iter().copied());
                acc = acc.max(l.acc);
                unk |= l.unk;
            }
        }
        let want_w = dmut && !dtemp;
        if want_w && acc != Acc::W {
            let n = self.name(dst);
            self.err(
                "NMM_READONLY_TO_WRITE",
                line,
                format!("'{n}' needs write access but its source is readonly"),
            );
        }
        let eff = if want_w && acc == Acc::W {
            Acc::W
        } else if dtemp {
            acc
        } else {
            Acc::R
        };
        let _ = live;
        Loc {
            kind: Kind::Alias,
            origins,
            acc: eff,
            loan: Some(if eff == Acc::W { Acc::W } else { Acc::R }),
            parents: srcs.iter().copied().collect(),
            moved: false,
            unk,
        }
    }

    fn owner_loc(&self, dst: L, origins: OSet, acc_limit: Acc, unk: bool) -> Loc {
        let l = &self.body.locals[dst];
        let wants = l.mutable; // var и временные
        let acc = if wants && acc_limit == Acc::W {
            Acc::W
        } else {
            Acc::R
        };
        Loc {
            kind: Kind::Owner,
            origins,
            acc,
            loan: None,
            parents: BTreeSet::new(),
            moved: false,
            unk,
        }
    }

    fn copy_loc() -> Loc {
        Loc {
            kind: Kind::Copy,
            origins: OSet::new(),
            acc: Acc::W,
            loan: None,
            parents: BTreeSet::new(),
            moved: false,
            unk: false,
        }
    }

    fn step(&mut self, st: &mut State, bi: usize, ii: usize) {
        let ins = self.body.blocks[bi].ins[ii].clone();
        let live = self.live[bi][ii].clone();
        match ins {
            Ins::FieldWrite {
                base,
                field_ty,
                src,
                line,
            } => {
                self.access(st, &live, base, M::W, line);
                if !field_ty.is_copy() {
                    self.sink(st, &live, src, line);
                }
            }
            Ins::Def {
                dst,
                rv,
                rebind,
                line,
            } => {
                // Переопределяемый локал не конфликтует со своим прежним значением.
                let mut live = live;
                live.remove(&dst);
                let live = live;
                if rebind && !self.body.locals[dst].mutable {
                    let n = self.name(dst);
                    self.err(
                        "NMM_READONLY_TO_WRITE",
                        line,
                        format!("assignment to readonly binding '{n}'"),
                    );
                }
                let was_owner = matches!(st[dst].as_ref().map(|l| &l.kind), Some(Kind::Owner));
                let new = match rv {
                    Rv::Const => Self::copy_loc(),
                    Rv::Bin(..) => Self::copy_loc(),
                    Rv::Use(a) => {
                        if self.copy(a) {
                            Self::copy_loc()
                        } else {
                            self.alias_from(st, &live, dst, a, line)
                        }
                    }
                    Rv::Field { base, field_ty } => {
                        if field_ty.is_copy() {
                            self.access(st, &live, base, M::R, line);
                            Self::copy_loc()
                        } else {
                            self.alias_from(st, &live, dst, base, line)
                        }
                    }
                    Rv::New { args, site } => {
                        for a in args {
                            if !self.copy(a) {
                                self.sink(st, &live, a, line);
                            }
                        }
                        self.owner_loc(dst, [Origin::Alloc(site)].into(), Acc::W, false)
                    }
                    Rv::Call { f, args, site } => self.call(st, &live, dst, &f, &args, site, line),
                };
                if rebind && was_owner && new.kind != Kind::Owner && new.kind != Kind::Copy {
                    let n = self.name(dst);
                    self.err(
                        "NMM_NO_OWNER_TOKEN",
                        line,
                        format!("owning binding '{n}' cannot be replaced by a borrowed value"),
                    );
                }
                st[dst] = Some(new);
            }
        }
    }

    fn alias_from(&mut self, st: &mut State, live: &BTreeSet<L>, dst: L, a: L, line: usize) -> Loc {
        if st[a].is_none() {
            return Loc {
                kind: Kind::Alias,
                origins: OSet::new(),
                acc: Acc::R,
                loan: None,
                parents: BTreeSet::new(),
                moved: false,
                unk: true,
            };
        }
        self.check_live(st, a, line);
        let want_w = self.body.locals[dst].mutable && !self.body.locals[dst].temp;
        let la_acc = st[a].as_ref().unwrap().acc;
        self.access(
            st,
            live,
            a,
            if want_w && la_acc == Acc::W {
                M::W
            } else {
                M::R
            },
            line,
        );
        self.alias_of(st, live, dst, &[a], line)
    }

    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        st: &mut State,
        live: &BTreeSet<L>,
        dst: L,
        f: &str,
        args: &[L],
        site: usize,
        line: usize,
    ) -> Loc {
        let c = self.contracts[f].clone();
        let mut modes: Vec<Option<M>> = Vec::new();
        for (i, &a) in args.iter().enumerate() {
            if self.copy(a) {
                modes.push(None);
                continue;
            }
            let p = &c.params[i];
            if p.consuming {
                self.sink(st, live, a, line);
                modes.push(Some(M::Consume));
            } else if p.var {
                self.access(st, live, a, M::W, line);
                modes.push(Some(M::W));
            } else {
                self.access(st, live, a, M::R, line);
                modes.push(Some(M::R));
            }
        }
        for i in 0..args.len() {
            for j in i + 1..args.len() {
                let (Some(mi), Some(mj)) = (modes[i], modes[j]) else {
                    continue;
                };
                let (Some(li), Some(lj)) = (st[args[i]].as_ref(), st[args[j]].as_ref()) else {
                    continue;
                };
                if overlap(&li.origins, &lj.origins) && (mi != M::R || mj != M::R) {
                    let (a, b) = (self.name(args[i]), self.name(args[j]));
                    self.err(
                        "NMM_BORROW_CONFLICT",
                        line,
                        format!("arguments '{a}' and '{b}' of {f} overlap and one is mutable"),
                    );
                }
            }
        }
        let origins_of = |st: &State, set: &BTreeSet<usize>| -> (OSet, Acc, Vec<L>, bool) {
            let (mut o, mut acc, mut ps, mut unk) = (OSet::new(), Acc::W, Vec::new(), false);
            for &i in set {
                if let Some(l) = &st[args[i]] {
                    o.extend(l.origins.iter().copied());
                    acc = acc.max(l.acc);
                    unk |= l.unk;
                    ps.push(args[i]);
                }
            }
            (o, acc, ps, unk)
        };
        match c.ret.kind {
            RetKind::None | RetKind::Conflict => Self::copy_loc(),
            RetKind::Bottom => self.owner_loc(dst, OSet::new(), Acc::W, true),
            RetKind::Fresh => self.owner_loc(dst, [Origin::Alloc(site)].into(), Acc::W, false),
            RetKind::Borrowed if ablated(2) => {
                self.owner_loc(dst, [Origin::Alloc(site)].into(), Acc::W, false)
            }
            RetKind::Borrowed | RetKind::Transferred => {
                let (o, acc, ps, unk) = origins_of(st, &c.ret.params);
                // право результата не сильнее права контракта и права аргументов
                let acc = if ablated(1) {
                    Acc::W
                } else {
                    acc.max(c.ret.acc.unwrap_or(Acc::R))
                };
                let d = &self.body.locals[dst];
                let (want_w, temp) = (d.mutable && !d.temp, d.temp);
                if want_w && acc != Acc::W {
                    let n = self.name(dst);
                    self.err(
                        "NMM_READONLY_TO_WRITE",
                        line,
                        format!("result of {f} is readonly, '{n}' needs write access"),
                    );
                }
                let eff = if want_w && acc == Acc::W {
                    Acc::W
                } else if temp {
                    acc
                } else {
                    Acc::R
                };
                if c.ret.kind == RetKind::Borrowed {
                    Loc {
                        kind: Kind::Alias,
                        origins: o,
                        acc: eff,
                        loan: Some(eff),
                        parents: ps.into_iter().collect(),
                        moved: false,
                        unk,
                    }
                } else {
                    Loc {
                        kind: Kind::Owner,
                        origins: o,
                        acc: eff,
                        loan: None,
                        parents: BTreeSet::new(),
                        moved: false,
                        unk,
                    }
                }
            }
        }
    }

    fn ret(&mut self, st: &mut State, val: Option<L>, line: usize) {
        let Some(x) = val else { return };
        if self.copy(x) {
            return;
        }
        // Ошибка использования после передачи уже сообщена; вклад в контракт
        // от неё не зависит (иначе вывод перестаёт быть монотонным).
        self.check_live(st, x, line);
        let Some(l) = st[x].clone() else { return };
        let params: BTreeSet<usize> = l
            .origins
            .iter()
            .filter_map(|o| {
                if let Origin::Param(i) = o {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect();
        let has_alloc = l.origins.iter().any(|o| matches!(o, Origin::Alloc(_)));
        let contrib = match l.kind {
            Kind::Owner if l.unk => None,
            Kind::Owner => {
                let kind = match (params.is_empty(), has_alloc) {
                    (true, _) => RetKind::Fresh,
                    (false, false) => RetKind::Transferred,
                    (false, true) => RetKind::Conflict,
                };
                Some(RetC {
                    kind,
                    params,
                    acc: None,
                })
            }
            Kind::ParamRoot(i) => {
                if self.fin {
                    self.parts.push(Part::Root(i));
                }
                None
            }
            Kind::Alias => {
                if l.unk {
                    None
                } else if has_alloc {
                    let n = self.name(x);
                    self.err(
                        "NMM_ESCAPE",
                        line,
                        format!(
                            "view '{n}' refers to a local object that does not outlive the call"
                        ),
                    );
                    None
                } else if params.is_empty() {
                    None
                } else {
                    Some(RetC {
                        kind: RetKind::Borrowed,
                        params,
                        acc: Some(l.acc),
                    })
                }
            }
            Kind::Mixed => {
                let n = self.name(x);
                self.err(
                    "NMM_NO_OWNER_TOKEN",
                    line,
                    format!("owner of '{n}' is not definite on all paths"),
                );
                None
            }
            Kind::Copy => None,
        };
        if let (true, Some(c)) = (self.fin, contrib) {
            self.parts.push(Part::Done(c));
        }
    }
}

fn initial_state(body: &Body) -> State {
    let mut st: State = vec![None; body.locals.len()];
    for (i, p) in body.params.iter().enumerate() {
        st[i] = Some(if p.ty.is_copy() {
            A::copy_loc()
        } else {
            Loc {
                kind: Kind::ParamRoot(i),
                origins: [Origin::Param(i)].into(),
                acc: if p.var { Acc::W } else { Acc::R },
                loan: None,
                parents: BTreeSet::new(),
                moved: false,
                unk: false,
            }
        });
    }
    st
}

pub fn initial_contract(body: &Body) -> Contract {
    Contract {
        params: body
            .params
            .iter()
            .map(|p| crate::contract::ParamC {
                name: p.name.clone(),
                var: p.var,
                consuming: false,
            })
            .collect(),
        ret: if matches!(body.ret, crate::syntax::Ty::Class(_)) {
            RetC::bottom()
        } else {
            RetC {
                kind: RetKind::None,
                params: BTreeSet::new(),
                acc: None,
            }
        },
    }
}

/// Проанализировать функцию при заданных контрактах вызываемых.
pub fn analyze(body: &Body, contracts: &BTreeMap<String, Contract>) -> (Derived, Vec<Diag>) {
    let own = &contracts[&body.name];
    let mut a = A {
        parts: Vec::new(),
        body,
        contracts,
        live: liveness(body),
        fin: false,
        diags: Vec::new(),
        derived: Derived {
            consuming: vec![false; body.params.len()],
            ret: RetC::bottom(),
        },
    };
    if !matches!(body.ret, crate::syntax::Ty::Class(_)) {
        a.derived.ret = RetC {
            kind: RetKind::None,
            params: BTreeSet::new(),
            acc: None,
        };
    }
    // 1. Прямой поток до неподвижной точки (без диагностик).
    let n = body.blocks.len();
    let mut ins: Vec<Option<State>> = vec![None; n];
    ins[0] = Some(initial_state(body));
    let mut work: VecDeque<usize> = VecDeque::from([0]);
    while let Some(b) = work.pop_front() {
        let mut st = ins[b].clone().unwrap();
        for i in 0..body.blocks[b].ins.len() {
            a.step(&mut st, b, i);
        }
        for s in succs(&body.blocks[b].term) {
            let changed = match &mut ins[s] {
                Some(cur) => merge_state(cur, &st),
                slot @ None => {
                    *slot = Some(st.clone());
                    true
                }
            };
            if changed {
                work.push_back(s);
            }
        }
    }
    // 2. Финальный проход по стабилизированным состояниям.
    a.fin = true;
    for (b, entry) in ins.iter().enumerate() {
        let Some(mut st) = entry.clone() else {
            continue;
        };
        for i in 0..body.blocks[b].ins.len() {
            a.step(&mut st, b, i);
        }
        if let Term::Ret { val, line } = body.blocks[b].term {
            a.ret(&mut st, val, line);
        }
    }
    // 3. Вид результата: пути внутри одного тела должны быть совместимы.
    let parts = std::mem::take(&mut a.parts);
    for p in parts {
        let c = match p {
            Part::Done(c) => c,
            Part::Root(i) => {
                let consuming = own.params[i].consuming || a.derived.consuming[i];
                RetC {
                    kind: if consuming {
                        RetKind::Transferred
                    } else {
                        RetKind::Borrowed
                    },
                    params: [i].into(),
                    acc: Some(if body.params[i].var { Acc::W } else { Acc::R }),
                }
            }
        };
        a.derived.ret.combine(&c);
    }
    (a.derived, a.diags)
}
