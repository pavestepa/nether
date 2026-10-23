//! Контракты функций (решётка) — результат вывода.

use std::collections::BTreeSet;

/// Права доступа. Порядок `W < R`: объединение путей даёт более слабое право.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Acc {
    W,
    R,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RetKind {
    /// Ещё нет информации (начало fixed point).
    Bottom,
    /// Результат Copy-типа или unit.
    None,
    /// Новый объект: O принадлежит вызывающему.
    Fresh,
    /// Alias на входные параметры.
    Borrowed,
    /// O входного параметра передан в результат.
    Transferred,
    /// Пути возвращают несовместимые виды результата.
    Conflict,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RetC {
    pub kind: RetKind,
    pub params: BTreeSet<usize>,
    pub acc: Option<Acc>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParamC {
    pub name: String,
    pub var: bool,
    pub consuming: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Contract {
    pub params: Vec<ParamC>,
    pub ret: RetC,
}

impl RetC {
    pub fn bottom() -> Self {
        RetC {
            kind: RetKind::Bottom,
            params: BTreeSet::new(),
            acc: None,
        }
    }

    /// Объединение путей одного тела: owned и borrowed не смешиваются (NMM §11.4).
    pub fn combine(&mut self, o: &RetC) {
        let kind = match (self.kind, o.kind) {
            (RetKind::Bottom, k) | (k, RetKind::Bottom) => k,
            (a, b) if a == b => a,
            _ => RetKind::Conflict,
        };
        self.kind = kind;
        self.params.extend(o.params.iter().copied());
        self.acc = match (self.acc, o.acc) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, None) => a,
            (None, b) => b,
        };
    }

    /// Верхняя грань между итерациями fixed point (Borrowed → Transferred
    /// допускается как монотонное уточнение). Возвращает, изменилось ли значение.
    pub fn join(&mut self, o: &RetC) -> bool {
        use RetKind as K;
        let old = self.clone();
        let kind = match (self.kind, o.kind) {
            (K::Bottom, k) | (k, K::Bottom) => k,
            (a, b) if a == b => a,
            (K::Borrowed, K::Transferred) | (K::Transferred, K::Borrowed) => K::Transferred,
            _ => K::Conflict,
        };
        self.kind = kind;
        self.params.extend(o.params.iter().copied());
        self.acc = match (self.acc, o.acc) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, None) => a,
            (None, b) => b,
        };
        *self != old
    }
}

impl Contract {
    pub fn join(&mut self, derived_consuming: &[bool], ret: &RetC) -> bool {
        let mut changed = false;
        for (p, d) in self.params.iter_mut().zip(derived_consuming) {
            if *d && !p.consuming {
                p.consuming = true;
                changed = true;
            }
        }
        changed | self.ret.join(ret)
    }

    pub fn render(&self, name: &str) -> String {
        let ps: Vec<String> = self
            .params
            .iter()
            .map(|p| {
                format!(
                    "{}: {}{}",
                    p.name,
                    if p.var { "W" } else { "R" },
                    if p.consuming { "+O" } else { "" }
                )
            })
            .collect();
        let set = |s: &BTreeSet<usize>| {
            s.iter()
                .map(|i| self.params[*i].name.clone())
                .collect::<Vec<_>>()
                .join(",")
        };
        let acc = match self.ret.acc {
            Some(Acc::W) => "@W",
            Some(Acc::R) => "@R",
            None => "",
        };
        let r = match self.ret.kind {
            RetKind::Bottom => "?".to_string(),
            RetKind::None => "-".to_string(),
            RetKind::Fresh => "Fresh".to_string(),
            RetKind::Borrowed => format!("Borrowed{{{}}}{acc}", set(&self.ret.params)),
            RetKind::Transferred => format!("Transferred{{{}}}{acc}", set(&self.ret.params)),
            RetKind::Conflict => "CONFLICT".to_string(),
        };
        format!("{name}({}) -> {r}", ps.join(", "))
    }
}
