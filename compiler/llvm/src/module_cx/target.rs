use super::*;

pub(super) fn create_target_machine(
    target_triple: &str,
    opt_level: u8,
) -> Result<TargetMachine, String> {
    let triple = TargetTriple::create(target_triple);
    let target = Target::from_triple(&triple).map_err(|error| error.to_string())?;
    let optimization = match opt_level {
        0 => inkwell::OptimizationLevel::None,
        1 => inkwell::OptimizationLevel::Less,
        2 => inkwell::OptimizationLevel::Default,
        _ => inkwell::OptimizationLevel::Aggressive,
    };
    target
        .create_target_machine(
            &triple,
            "generic",
            "",
            optimization,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| format!("cannot create target machine for `{target_triple}`"))
}

pub(super) fn param_ty_to_fn<'ctx>(ret: Ty<'ctx>, params: &[ParamTy<'ctx>]) -> FnTy<'ctx> {
    match ret {
        Ty::ArrayType(t) => t.fn_type(params, false),
        Ty::FloatType(t) => t.fn_type(params, false),
        Ty::IntType(t) => t.fn_type(params, false),
        Ty::PointerType(t) => t.fn_type(params, false),
        Ty::StructType(t) => t.fn_type(params, false),
        Ty::VectorType(t) => t.fn_type(params, false),
    }
}
