//! Типизация и lowering в CFG с явными временными значениями (ANF).

use crate::syntax::*;
use std::collections::HashMap;

pub type L = usize;

#[derive(Clone, Debug)]
pub struct Local {
    pub name: String,
    pub ty: Ty,
    /// Объявлен через `var` (временные значения — тоже изменяемы).
    pub mutable: bool,
    pub temp: bool,
}

#[derive(Clone, Debug)]
pub enum Rv {
    Const,
    Bin(L, L),
    Use(L),
    Field {
        base: L,
        field_ty: Ty,
    },
    New {
        args: Vec<L>,
        site: usize,
    },
    Call {
        f: String,
        args: Vec<L>,
        site: usize,
    },
}

#[derive(Clone, Debug)]
pub enum Ins {
    Def {
        dst: L,
        rv: Rv,
        rebind: bool,
        line: usize,
    },
    FieldWrite {
        base: L,
        field_ty: Ty,
        src: L,
        line: usize,
    },
}

#[derive(Clone, Debug)]
pub enum Term {
    Goto(usize),
    Branch { cond: L, t: usize, f: usize },
    Ret { val: Option<L>, line: usize },
}

#[derive(Clone, Debug)]
pub struct Block {
    pub ins: Vec<Ins>,
    pub term: Term,
}

#[derive(Clone, Debug)]
pub struct Body {
    pub name: String,
    pub params: Vec<ParamDef>,
    pub ret: Ty,
    /// Локальные 0..params.len() — параметры, по порядку.
    pub locals: Vec<Local>,
    pub blocks: Vec<Block>,
    pub line: usize,
}

struct Sig {
    params: Vec<Ty>,
    ret: Ty,
}

struct Lower<'a> {
    classes: &'a HashMap<String, Vec<(String, Ty)>>,
    sigs: &'a HashMap<String, Sig>,
    ret_ty: Ty,
    locals: Vec<Local>,
    blocks: Vec<(Vec<Ins>, Option<Term>)>,
    scopes: Vec<HashMap<String, L>>,
    cur: Option<usize>,
    site: usize,
    nparams: usize,
}

type R<T> = Result<T, String>;

impl Lower<'_> {
    fn new_local(&mut self, name: &str, ty: Ty, mutable: bool, temp: bool) -> L {
        self.locals.push(Local {
            name: name.into(),
            ty,
            mutable,
            temp,
        });
        self.locals.len() - 1
    }
    fn lookup(&self, n: &str, line: usize) -> R<L> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(n).copied())
            .ok_or_else(|| format!("{line}: unknown variable '{n}'"))
    }
    fn block(&mut self) -> usize {
        self.blocks.push((Vec::new(), None));
        self.blocks.len() - 1
    }
    fn emit(&mut self, i: Ins) {
        if let Some(b) = self.cur {
            self.blocks[b].0.push(i);
        }
    }
    fn end(&mut self, t: Term) {
        if let Some(b) = self.cur.take() {
            self.blocks[b].1 = Some(t);
        }
    }
    fn field_ty(&self, base: L, field: &str, line: usize) -> R<Ty> {
        let Ty::Class(c) = &self.locals[base].ty else {
            return Err(format!(
                "{line}: '{}' is not a class value",
                self.locals[base].name
            ));
        };
        self.classes[c]
            .iter()
            .find(|f| f.0 == field)
            .map(|f| f.1.clone())
            .ok_or_else(|| format!("{line}: class {c} has no field '{field}'"))
    }

    /// Выражение → правая часть присваивания с операндами-локалами.
    fn rv(&mut self, e: &Expr, line: usize) -> R<(Rv, Ty)> {
        match e {
            Expr::Int(_) => Ok((Rv::Const, Ty::Int)),
            Expr::Var(n) => {
                let l = self.lookup(n, line)?;
                Ok((Rv::Use(l), self.locals[l].ty.clone()))
            }
            Expr::Field(b, f) => {
                let base = self.lookup(b, line)?;
                let field_ty = self.field_ty(base, f, line)?;
                Ok((
                    Rv::Field {
                        base,
                        field_ty: field_ty.clone(),
                    },
                    field_ty,
                ))
            }
            Expr::Bin(a, _, b) => {
                let (a, ta) = self.operand(a, line)?;
                let (b, tb) = self.operand(b, line)?;
                if ta != Ty::Int || tb != Ty::Int {
                    return Err(format!("{line}: arithmetic on non-i32"));
                }
                Ok((Rv::Bin(a, b), Ty::Int))
            }
            Expr::New(c, args) => {
                let fields = self
                    .classes
                    .get(c)
                    .ok_or_else(|| format!("{line}: unknown class {c}"))?
                    .clone();
                if fields.len() != args.len() {
                    return Err(format!("{line}: {c} expects {} arguments", fields.len()));
                }
                let mut ops = Vec::new();
                for (a, (_, fty)) in args.iter().zip(&fields) {
                    let (l, t) = self.operand(a, line)?;
                    if &t != fty {
                        return Err(format!("{line}: argument type mismatch in new {c}"));
                    }
                    ops.push(l);
                }
                self.site += 1;
                Ok((
                    Rv::New {
                        args: ops,
                        site: self.site,
                    },
                    Ty::Class(c.clone()),
                ))
            }
            Expr::Call(f, args) => {
                let sig = self
                    .sigs
                    .get(f)
                    .ok_or_else(|| format!("{line}: unknown function {f}"))?;
                if sig.params.len() != args.len() {
                    return Err(format!(
                        "{line}: {f} expects {} arguments",
                        sig.params.len()
                    ));
                }
                let (ptys, ret) = (sig.params.clone(), sig.ret.clone());
                let mut ops = Vec::new();
                for (a, pt) in args.iter().zip(&ptys) {
                    let (l, t) = self.operand(a, line)?;
                    if &t != pt {
                        return Err(format!("{line}: argument type mismatch in call to {f}"));
                    }
                    ops.push(l);
                }
                self.site += 1;
                Ok((
                    Rv::Call {
                        f: f.clone(),
                        args: ops,
                        site: self.site,
                    },
                    ret,
                ))
            }
        }
    }

    /// Выражение → локал (переменные берутся как есть, прочее — во временное).
    fn operand(&mut self, e: &Expr, line: usize) -> R<(L, Ty)> {
        if let Expr::Var(n) = e {
            let l = self.lookup(n, line)?;
            return Ok((l, self.locals[l].ty.clone()));
        }
        let (rv, ty) = self.rv(e, line)?;
        let t = self.new_local("_t", ty.clone(), true, true);
        self.emit(Ins::Def {
            dst: t,
            rv,
            rebind: false,
            line,
        });
        Ok((t, ty))
    }

    fn stmts(&mut self, ss: &[Stmt]) -> R<()> {
        self.scopes.push(HashMap::new());
        for s in ss {
            self.stmt(s)?;
        }
        self.scopes.pop();
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        if self.cur.is_none() {
            return Ok(()); // недостижимый код после return
        }
        match s {
            Stmt::Let {
                name,
                var,
                init,
                line,
            } => {
                let (rv, ty) = self.rv(init, *line)?;
                if ty == Ty::Unit {
                    return Err(format!("{line}: cannot bind a unit value"));
                }
                let dst = self.new_local(name, ty, *var, false);
                self.emit(Ins::Def {
                    dst,
                    rv,
                    rebind: false,
                    line: *line,
                });
                self.scopes.last_mut().unwrap().insert(name.clone(), dst);
            }
            Stmt::Assign { name, rhs, line } => {
                let dst = self.lookup(name, *line)?;
                if dst < self.nparams {
                    return Err(format!("{line}: cannot assign to parameter '{name}'"));
                }
                let (rv, ty) = self.rv(rhs, *line)?;
                if ty != self.locals[dst].ty {
                    return Err(format!("{line}: type mismatch in assignment"));
                }
                self.emit(Ins::Def {
                    dst,
                    rv,
                    rebind: true,
                    line: *line,
                });
            }
            Stmt::FieldAssign {
                base,
                field,
                rhs,
                line,
            } => {
                let b = self.lookup(base, *line)?;
                let fty = self.field_ty(b, field, *line)?;
                let (src, t) = self.operand(rhs, *line)?;
                if t != fty {
                    return Err(format!("{line}: type mismatch in field assignment"));
                }
                self.emit(Ins::FieldWrite {
                    base: b,
                    field_ty: fty,
                    src,
                    line: *line,
                });
            }
            Stmt::Return { value, line } => {
                let val = match value {
                    Some(e) => {
                        let (l, t) = self.operand(e, *line)?;
                        if t != self.ret_ty {
                            return Err(format!("{line}: return type mismatch"));
                        }
                        Some(l)
                    }
                    None if self.ret_ty == Ty::Unit => None,
                    None => return Err(format!("{line}: missing return value")),
                };
                self.end(Term::Ret { val, line: *line });
            }
            Stmt::If {
                cond,
                then,
                els,
                line,
            } => {
                let (c, t) = self.operand(cond, *line)?;
                if t != Ty::Int {
                    return Err(format!("{line}: condition must be i32"));
                }
                let (bt, bf, join) = (self.block(), self.block(), self.block());
                self.end(Term::Branch {
                    cond: c,
                    t: bt,
                    f: bf,
                });
                let mut reachable = false;
                for (b, body) in [(bt, then), (bf, els)] {
                    self.cur = Some(b);
                    self.stmts(body)?;
                    reachable |= self.cur.is_some();
                    self.end(Term::Goto(join));
                }
                self.cur = if reachable { Some(join) } else { None };
            }
            Stmt::While { cond, body, line } => {
                let (head, bb, exit) = (self.block(), self.block(), self.block());
                self.end(Term::Goto(head));
                self.cur = Some(head);
                let (c, t) = self.operand(cond, *line)?;
                if t != Ty::Int {
                    return Err(format!("{line}: condition must be i32"));
                }
                self.end(Term::Branch {
                    cond: c,
                    t: bb,
                    f: exit,
                });
                self.cur = Some(bb);
                self.stmts(body)?;
                self.end(Term::Goto(head));
                self.cur = Some(exit);
            }
            Stmt::Expr { expr, line } => {
                self.operand(expr, *line)?;
            }
        }
        Ok(())
    }
}

pub fn lower(p: &Program) -> R<Vec<Body>> {
    let classes: HashMap<_, _> = p
        .classes
        .iter()
        .map(|c| (c.name.clone(), c.fields.clone()))
        .collect();
    let mut sigs = HashMap::new();
    for f in &p.funcs {
        let sig = Sig {
            params: f.params.iter().map(|x| x.ty.clone()).collect(),
            ret: f.ret.clone(),
        };
        if sigs.insert(f.name.clone(), sig).is_some() {
            return Err(format!("{}: duplicate function {}", f.line, f.name));
        }
    }
    let mut out = Vec::new();
    for f in &p.funcs {
        let mut lw = Lower {
            classes: &classes,
            sigs: &sigs,
            ret_ty: f.ret.clone(),
            locals: Vec::new(),
            blocks: Vec::new(),
            scopes: vec![HashMap::new()],
            cur: None,
            site: 0,
            nparams: f.params.len(),
        };
        for prm in &f.params {
            let l = lw.new_local(&prm.name, prm.ty.clone(), prm.var, false);
            lw.scopes[0].insert(prm.name.clone(), l);
        }
        lw.cur = Some(lw.block());
        lw.stmts(&f.body)?;
        if lw.cur.is_some() {
            if f.ret != Ty::Unit {
                return Err(format!(
                    "{}: function {} may finish without returning",
                    f.line, f.name
                ));
            }
            lw.end(Term::Ret {
                val: None,
                line: f.line,
            });
        }
        let blocks = lw
            .blocks
            .into_iter()
            .map(|(ins, t)| Block {
                ins,
                term: t.unwrap_or(Term::Ret {
                    val: None,
                    line: f.line,
                }),
            })
            .collect();
        out.push(Body {
            name: f.name.clone(),
            params: f.params.clone(),
            ret: f.ret.clone(),
            locals: lw.locals,
            blocks,
            line: f.line,
        });
    }
    Ok(out)
}
