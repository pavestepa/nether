//! Генератор случайных (типизированных) программ для проверки свойств вывода.

pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum T {
    Unit,
    Int,
    User,
}

struct Sig {
    name: String,
    params: Vec<(T, bool)>,
    ret: T,
}

#[derive(Clone)]
struct Var {
    name: String,
    ty: T,
    #[allow(dead_code)]
    var: bool,
}

struct G<'a> {
    rng: Rng,
    sigs: &'a [Sig],
    counter: usize,
}

impl G<'_> {
    fn fresh(&mut self) -> String {
        self.counter += 1;
        format!("v{}", self.counter)
    }

    fn pick(&mut self, env: &[Var], ty: T) -> Option<String> {
        let c: Vec<&Var> = env.iter().filter(|v| v.ty == ty).collect();
        (!c.is_empty()).then(|| c[self.rng.below(c.len())].name.clone())
    }

    fn arg(&mut self, env: &[Var], ty: T) -> Option<String> {
        match ty {
            T::Int => Some(self.pick(env, T::Int).unwrap_or_else(|| "1".into())),
            T::User => {
                if self.rng.below(5) == 0 {
                    Some("new User(1)".into())
                } else {
                    self.pick(env, T::User)
                }
            }
            T::Unit => None,
        }
    }

    fn call(&mut self, env: &[Var]) -> Option<(String, T)> {
        let s = &self.sigs[self.rng.below(self.sigs.len())];
        let (name, ret, ptys) = (s.name.clone(), s.ret, s.params.clone());
        let mut args = Vec::new();
        for (t, _) in ptys {
            args.push(self.arg(env, t)?);
        }
        Some((format!("{name}({})", args.join(", ")), ret))
    }

    fn block(&mut self, depth: usize, env: &mut Vec<Var>, n: usize, out: &mut String) {
        for _ in 0..n {
            let k = self.rng.below(10);
            let kw = if self.rng.below(2) == 0 { "let" } else { "var" };
            match k {
                0 => {
                    let v = self.fresh();
                    out.push_str(&format!("    {kw} {v} = new User({})\n", self.rng.below(9)));
                    env.push(Var {
                        name: v,
                        ty: T::User,
                        var: kw == "var",
                    });
                }
                1 | 2 => {
                    if let Some(a) = self.pick(env, T::User) {
                        let v = self.fresh();
                        out.push_str(&format!("    {kw} {v} = {a}\n"));
                        env.push(Var {
                            name: v,
                            ty: T::User,
                            var: kw == "var",
                        });
                    }
                }
                3 => {
                    if let Some(a) = self.pick(env, T::User) {
                        out.push_str(&format!("    {a}.age += 1\n"));
                    }
                }
                4 | 5 => {
                    if let Some((c, _)) = self.call(env) {
                        out.push_str(&format!("    {c}\n"));
                    }
                }
                6 => {
                    if let Some((c, ty)) = self.call(env) {
                        if ty != T::Unit {
                            let v = self.fresh();
                            out.push_str(&format!("    {kw} {v} = {c}\n"));
                            env.push(Var {
                                name: v,
                                ty,
                                var: kw == "var",
                            });
                        }
                    }
                }
                7 => {
                    if let Some(a) = self.pick(env, T::User) {
                        let v = self.fresh();
                        out.push_str(&format!("    let {v} = new Box({a})\n"));
                    }
                }
                8 if depth < 2 => {
                    let c = self.pick(env, T::Int).unwrap_or_else(|| "1".into());
                    out.push_str(&format!("    if {c} < 3 {{\n"));
                    let mut e1 = env.clone();
                    let n1 = 1 + self.rng.below(3);
                    self.block(depth + 1, &mut e1, n1, out);
                    out.push_str("    } else {\n");
                    let mut e2 = env.clone();
                    let n2 = self.rng.below(3);
                    self.block(depth + 1, &mut e2, n2, out);
                    out.push_str("    }\n");
                }
                9 if depth < 2 => {
                    let c = self.pick(env, T::Int).unwrap_or_else(|| "1".into());
                    out.push_str(&format!("    while {c} < 3 {{\n"));
                    let mut e1 = env.clone();
                    let n1 = 1 + self.rng.below(3);
                    self.block(depth + 1, &mut e1, n1, out);
                    out.push_str("    }\n");
                }
                _ => {}
            }
        }
    }
}

/// Сгенерировать программу из нескольких взаимно вызывающих функций.
pub fn gen_program(seed: u64) -> String {
    // Чётные seed — плотные функции (много ошибок), нечётные — короткие
    // (больше допустимых программ).
    gen_program_sized(seed, if seed % 2 == 0 { 6 } else { 2 })
}

pub fn gen_program_sized(seed: u64, max_stmts: usize) -> String {
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    for _ in 0..3 {
        rng.next_u64();
    }
    let nf = 2 + rng.below(5);
    let mut sigs = Vec::new();
    for i in 0..nf {
        let np = 1 + rng.below(3);
        let params = (0..np)
            .map(|_| match rng.below(4) {
                0 => (T::Int, false),
                1 => (T::User, true),
                _ => (T::User, false),
            })
            .collect();
        let ret = match rng.below(5) {
            0 | 1 => T::Unit,
            2 => T::Int,
            _ => T::User,
        };
        sigs.push(Sig {
            name: format!("f{i}"),
            params,
            ret,
        });
    }
    let mut g = G {
        rng,
        sigs: &sigs,
        counter: 0,
    };
    let mut out = String::from("class User { age: i32 }\nclass Box { value: User }\n");
    for s in &sigs {
        let ps: Vec<String> = s
            .params
            .iter()
            .enumerate()
            .map(|(i, (t, v))| {
                let ty = if *t == T::Int { "i32" } else { "User" };
                format!("{}p{i}: {ty}", if *v { "var " } else { "" })
            })
            .collect();
        let rty = match s.ret {
            T::Unit => "()",
            T::Int => "i32",
            T::User => "User",
        };
        out.push_str(&format!("fn {}({}): {rty} {{\n", s.name, ps.join(", ")));
        let mut env: Vec<Var> = s
            .params
            .iter()
            .enumerate()
            .map(|(i, (t, v))| Var {
                name: format!("p{i}"),
                ty: *t,
                var: *v,
            })
            .collect();
        let n = 1 + g.rng.below(max_stmts);
        g.block(0, &mut env, n, &mut out);
        match s.ret {
            T::Unit => {}
            T::Int => out.push_str("    return 1\n"),
            T::User => {
                let r = match g.rng.below(4) {
                    0 => "new User(1)".to_string(),
                    _ => g
                        .pick(&env, T::User)
                        .unwrap_or_else(|| "new User(1)".into()),
                };
                out.push_str(&format!("    return {r}\n"));
            }
        }
        out.push_str("}\n");
    }
    out
}
