//! Proof of work: совместный вывод контрактов функций и access provenance.
//!
//! Конвейер: текст → [`syntax`] (AST) → [`mir`] (CFG, типы) →
//! [`analysis`] (потоковый анализ одной функции) → [`infer`] (fixed point
//! по контрактам всех функций) → диагностики NMM.

pub mod analysis;
pub mod contract;
pub mod experiments;
pub mod fuzz;
pub mod infer;
pub mod mir;
pub mod syntax;

use mir::Body;

/// Разобрать и привести программу к CFG.
pub fn compile(src: &str) -> Result<Vec<Body>, String> {
    let prog = syntax::parse(src)?;
    mir::lower(&prog)
}
