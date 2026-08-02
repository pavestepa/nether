use super::*;

impl ModuleCx<'_> {
    /// Returns the verifier diagnostic for malformed generated LLVM.
    pub fn verify(&self) -> Result<(), String> {
        self.module.verify().map_err(|e| e.to_string())
    }

    pub fn print_to_string(&self) -> String {
        self.module.print_to_string().to_string()
    }

    /// Runs LLVM's new pass manager pipeline (`"default<O{level}>"`) over
    /// the module in place.
    pub fn optimize(&self, level: u8) {
        let machine = self.target_machine();
        self.module
            .run_passes(
                &format!("default<O{level}>"),
                &machine,
                PassBuilderOptions::create(),
            )
            .expect("run_passes");
    }

    /// # Errors
    /// Propagates any I/O error writing the object file.
    pub fn emit_object(&self, out: &Path) -> std::io::Result<()> {
        if self.opt_level > 0 {
            self.optimize(self.opt_level);
        }
        let machine = self.target_machine();
        machine
            .write_to_file(&self.module, FileType::Object, out)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    fn target_machine(&self) -> TargetMachine {
        create_target_machine(&self.target_triple, self.opt_level)
            .expect("target was validated by Codegen::with_target")
    }
}
