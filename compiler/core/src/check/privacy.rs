//! Access is checked in the lexical declaring type, independent of entry modes,
//! concrete generic arguments, receiver aliases, and the caller's source module.
use super::*;
impl Checker<'_> {
    pub(super) fn check_function_access(&self, name: &str, span: Span) -> Result<()> {
        let Some(declaration) = self.private_functions.get(name) else {
            return Ok(());
        };
        let allowed = if let Some((owner, _)) = name.rsplit_once('.') {
            self.member_owner.as_deref() == Some(owner)
        } else {
            declaration.source == span.source
        };
        if allowed {
            return Ok(());
        }
        Err(error(
            span,
            "E0352",
            "private function or method is not accessible here",
        )
        .with_note(*declaration, "private declaration is here"))
    }
    pub(super) fn check_field_access(
        &self,
        ty: &Type,
        field: &h::RecordField,
        span: Span,
    ) -> Result<()> {
        if !field.private {
            return Ok(());
        }
        let (Type::Record { name, .. } | Type::Class { name, .. }) = ty else {
            unreachable!()
        };
        let owner = self.nominals.origin(name);
        if self.member_owner.as_ref() == Some(&owner) {
            return Ok(());
        }
        let diagnostic = error(
            span,
            "E0352",
            "private field is accessible only inside its declaring type",
        );
        Err(
            if let Some(declaration) = self.nominals.field_declaration(&owner, &field.name) {
                diagnostic.with_note(declaration, "private field is declared here")
            } else {
                diagnostic
            },
        )
    }
}
