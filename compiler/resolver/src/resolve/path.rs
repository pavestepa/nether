use super::*;

impl Resolver<'_> {
    pub(super) fn resolve_value_path(&mut self, path: &Path) {
        let first = &path.segments[0];

        if let Some(local_id) = self.scopes.lookup(&first.name) {
            self.path_res.insert(
                path.id,
                PathResolution {
                    base: Resolution::Local(local_id),
                    consumed: 1,
                },
            );
            return;
        }

        if self.is_generic_param(&first.name) {
            let base = if self.is_const_generic_param(&first.name) {
                Resolution::ConstParam
            } else {
                Resolution::GenericParam
            };
            self.path_res
                .insert(path.id, PathResolution { base, consumed: 1 });
            return;
        }

        let Some(id) = self.defs.lookup_in(path.span.file, &first.name) else {
            // A bare variant name (`Some`, `None`, ...) promoted into
            // scope by `use module.Enum.Variant;` (`stdlib/mod.nr`'s own
            // `use option.Option.Some;`, for instance) — a variant has no
            // `DefId` of its own, so it can't be found via `lookup_in`
            // above; resolves directly to the same `Resolution::EnumVariant`
            // kind the qualified `Option.Some` form already produces.
            if path.segments.len() == 1 {
                if let Some((enum_id, idx)) =
                    self.defs.lookup_variant_in(path.span.file, &first.name)
                {
                    self.path_res.insert(
                        path.id,
                        PathResolution {
                            base: Resolution::EnumVariant(enum_id, idx),
                            consumed: 1,
                        },
                    );
                    return;
                }
            }
            self.unresolved_value(first);
            self.path_res.insert(
                path.id,
                PathResolution {
                    base: Resolution::Error,
                    consumed: path.segments.len(),
                },
            );
            return;
        };

        let def = self.defs.get(id);
        if path.segments.len() == 1 {
            self.path_res.insert(
                path.id,
                PathResolution {
                    base: Resolution::Def(id),
                    consumed: 1,
                },
            );
            return;
        }

        let second = &path.segments[1];
        match def.kind {
            DefKind::Enum => {
                if let Some(idx) = variant_index(def, &second.name) {
                    self.path_res.insert(
                        path.id,
                        PathResolution {
                            base: Resolution::EnumVariant(id, idx),
                            consumed: 2,
                        },
                    );
                } else if let Some(idx) = method_index(def, &second.name) {
                    self.path_res.insert(
                        path.id,
                        PathResolution {
                            base: Resolution::StaticMember(id, idx),
                            consumed: 2,
                        },
                    );
                } else if let Some(idx) = constant_index(def, &second.name) {
                    self.path_res.insert(
                        path.id,
                        PathResolution {
                            base: Resolution::StaticConst(id, idx),
                            consumed: 2,
                        },
                    );
                } else {
                    self.error(
                        second.span,
                        format!(
                            "enum `{}` has no variant or method named `{}`",
                            first.name, second.name
                        ),
                    );
                    self.path_res.insert(
                        path.id,
                        PathResolution {
                            base: Resolution::Error,
                            consumed: 2,
                        },
                    );
                }
            }
            DefKind::Type => {
                if let Some(idx) = method_index(def, &second.name) {
                    self.path_res.insert(
                        path.id,
                        PathResolution {
                            base: Resolution::StaticMember(id, idx),
                            consumed: 2,
                        },
                    );
                } else if let Some(idx) = constant_index(def, &second.name) {
                    self.path_res.insert(
                        path.id,
                        PathResolution {
                            base: Resolution::StaticConst(id, idx),
                            consumed: 2,
                        },
                    );
                } else {
                    self.error(
                        second.span,
                        format!(
                            "type `{}` has no method named `{}`",
                            first.name, second.name
                        ),
                    );
                    self.path_res.insert(
                        path.id,
                        PathResolution {
                            base: Resolution::Error,
                            consumed: 2,
                        },
                    );
                }
            }
            DefKind::Trait | DefKind::Fn | DefKind::Primitive | DefKind::TypeAlias => {
                self.error(
                    second.span,
                    format!("`{}` has no member named `{}`", first.name, second.name),
                );
                self.path_res.insert(
                    path.id,
                    PathResolution {
                        base: Resolution::Error,
                        consumed: 2,
                    },
                );
            }
            DefKind::Imported => {
                // Compatibility mode for callers that resolve a single
                // parsed file without asking the driver to load imports.
                // The imported declaration is opaque in that API.
                self.path_res.insert(
                    path.id,
                    PathResolution {
                        base: Resolution::Def(id),
                        consumed: 1,
                    },
                );
            }
        }
    }

    pub(super) fn unresolved_value(&mut self, name: &nether_ast::Ident) {
        let mut candidates = self.scopes.visible_names();
        candidates.extend(
            self.defs
                .visible_in(name.span.file)
                .into_iter()
                .map(|id| self.defs.get(id).name.clone()),
        );
        candidates.sort();
        candidates.dedup();

        let mut diagnostic =
            Diagnostic::error(format!("cannot find `{}` in this scope", name.name))
                .with_label(name.span, "unknown name");
        if let Some(candidate) = closest_name(&name.name, &candidates) {
            diagnostic = diagnostic.with_suggestion(
                name.span,
                candidate.to_string(),
                format!("did you mean `{candidate}`?"),
            );
        }
        self.diagnostics.push(diagnostic);
    }
}
