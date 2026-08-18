use super::*;

impl Resolver<'_> {
    pub(super) fn resolve_block_in_current_scope(&mut self, block: &Block) {
        // Caller already pushed a scope (function bodies push one scope
        // that also holds `self`/params); nested `{}` uses `resolve_block`.
        for stmt in &block.stmts {
            self.resolve_stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            self.resolve_expr(tail);
        }
    }

    pub(super) fn resolve_block(&mut self, block: &Block) {
        self.scopes.push();
        self.resolve_block_in_current_scope(block);
        self.scopes.pop();
    }

    pub(super) fn resolve_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(let_stmt) => {
                // Resolve the value *before* introducing the binding, so
                // `let x = x;` resolves the right-hand `x` to any outer
                // binding, matching Rust's shadowing semantics.
                self.resolve_expr(&let_stmt.value);
                if let Some(ty) = &let_stmt.ty {
                    self.resolve_type_expr(ty);
                }
                self.bind_local(let_stmt.id, let_stmt.name.name.clone());
            }
            Stmt::Expr(expr) => self.resolve_expr(expr),
        }
    }

    pub(super) fn resolve_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Literal(_) | ExprKind::Continue => {}
            ExprKind::Path(path) => self.resolve_value_path(path),
            ExprKind::Tuple(elems) | ExprKind::Array(elems) => {
                for e in elems {
                    self.resolve_expr(e);
                }
            }
            ExprKind::StringTemplate(parts) => {
                for part in parts {
                    if let nether_ast::TemplatePart::Expr(e) = part {
                        self.resolve_expr(e);
                    }
                }
            }
            ExprKind::Unary { expr, .. } | ExprKind::MutArg(expr) => self.resolve_expr(expr),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.resolve_expr(lhs);
                self.resolve_expr(rhs);
            }
            ExprKind::Assign { target, value } => {
                self.resolve_expr(target);
                self.resolve_expr(value);
            }
            ExprKind::Call {
                callee,
                generic_args,
                args,
            } => {
                self.resolve_expr(callee);
                for ty in generic_args {
                    self.resolve_type_expr(ty);
                }
                for a in args {
                    self.resolve_expr(a);
                }
            }
            ExprKind::MethodCall {
                receiver,
                generic_args,
                args,
                ..
            } => {
                self.resolve_expr(receiver);
                for ty in generic_args {
                    self.resolve_type_expr(ty);
                }
                for a in args {
                    self.resolve_expr(a);
                }
            }
            ExprKind::Field { base, .. } => self.resolve_expr(base),
            ExprKind::Index { base, index } => {
                self.resolve_expr(base);
                self.resolve_expr(index);
            }
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.resolve_expr(cond);
                self.resolve_block(then_branch);
                if let Some(e) = else_branch {
                    self.resolve_expr(e);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.resolve_expr(scrutinee);
                for arm in arms {
                    self.scopes.push();
                    self.resolve_pattern(&arm.pattern);
                    self.resolve_expr(&arm.body);
                    self.scopes.pop();
                }
            }
            ExprKind::Block(block) => self.resolve_block(block),
            ExprKind::While { cond, body } => {
                self.resolve_expr(cond);
                self.resolve_block(body);
            }
            ExprKind::ForIn {
                pattern,
                iter,
                body,
            } => {
                self.resolve_expr(iter);
                self.scopes.push();
                self.resolve_pattern(pattern);
                self.resolve_block_in_current_scope(body);
                self.scopes.pop();
            }
            ExprKind::Loop { body } => self.resolve_block(body),
            ExprKind::Break(value) | ExprKind::Return(value) => {
                if let Some(v) = value {
                    self.resolve_expr(v);
                }
            }
            ExprKind::Closure { params, body, .. } => {
                self.scopes.push();
                for param in params {
                    self.resolve_type_expr(&param.ty);
                    self.bind_param(param);
                }
                self.resolve_expr(body);
                self.scopes.pop();
            }
            ExprKind::StructLit {
                path,
                fields,
                owned: _,
            } => {
                self.resolve_struct_lit_path(path);
                for (_, value) in fields {
                    self.resolve_expr(value);
                }
            }
        }
    }

    pub(super) fn resolve_struct_lit_path(&mut self, path: &Path) {
        // A struct literal's path names a *type*, never a local — resolve
        // it the same way a type-position path is resolved.
        self.resolve_type_path(path);
    }

    pub(super) fn resolve_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard(_) | Pattern::Literal(_, _) => {}
            Pattern::Binding(id, ident) => self.resolve_binding_or_unit_variant(*id, ident),
            Pattern::Tuple(elems, _) => {
                for e in elems {
                    self.resolve_pattern(e);
                }
            }
            Pattern::Variant { path, payload, .. } => {
                self.resolve_variant_pattern_path(path);
                for p in payload {
                    self.resolve_pattern(p);
                }
            }
        }
    }

    /// A bare identifier in pattern position (`Same => ...`) is
    /// syntactically ambiguous between introducing a fresh binding and
    /// matching an existing unit variant by name — `nether_parser` can't
    /// tell without name information, so it always produces
    /// [`Pattern::Binding`], and disambiguation happens here, the same way
    /// Rust's own resolver treats a bare path pattern that happens to name
    /// a unit variant/const as that item rather than a new binding.
    pub(super) fn resolve_binding_or_unit_variant(
        &mut self,
        id: NodeId,
        ident: &nether_ast::Ident,
    ) {
        match find_unique_variant(self.defs, ident.span.file, &ident.name) {
            Ok((enum_id, idx)) => {
                self.path_res.insert(
                    id,
                    PathResolution {
                        base: Resolution::EnumVariant(enum_id, idx),
                        consumed: 1,
                    },
                );
            }
            Err(0) => self.bind_local(id, ident.name.clone()),
            Err(_) => {
                self.error(
                    ident.span,
                    format!(
                        "`{}` is ambiguous: more than one enum defines a variant with this name",
                        ident.name
                    ),
                );
            }
        }
    }

    pub(super) fn resolve_variant_pattern_path(&mut self, path: &Path) {
        if path.segments.len() >= 2 {
            let variant_name = path.segments.last().unwrap();
            let enum_name = &path.segments[path.segments.len() - 2];
            match self.defs.lookup_in(path.span.file, &enum_name.name) {
                Some(id) if self.defs.get(id).kind == DefKind::Enum => {
                    match variant_index(self.defs.get(id), &variant_name.name) {
                        Some(idx) => {
                            self.path_res.insert(
                                path.id,
                                PathResolution {
                                    base: Resolution::EnumVariant(id, idx),
                                    consumed: path.segments.len(),
                                },
                            );
                        }
                        None => {
                            self.error(
                                variant_name.span,
                                format!(
                                    "enum `{}` has no variant named `{}`",
                                    enum_name.name, variant_name.name
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
                }
                _ => {
                    self.error(
                        enum_name.span,
                        format!("cannot find enum `{}` in this scope", enum_name.name),
                    );
                    self.path_res.insert(
                        path.id,
                        PathResolution {
                            base: Resolution::Error,
                            consumed: path.segments.len(),
                        },
                    );
                }
            }
            return;
        }

        // A bare `Custom(x)` pattern with no enum-name qualifier: search
        // every known enum for a unique variant with this name.
        let name = &path.segments[0];
        match find_unique_variant(self.defs, name.span.file, &name.name) {
            Ok((enum_id, idx)) => {
                self.path_res.insert(
                    path.id,
                    PathResolution {
                        base: Resolution::EnumVariant(enum_id, idx),
                        consumed: 1,
                    },
                );
            }
            Err(0) => {
                self.error(
                    name.span,
                    format!("no enum variant named `{}` found", name.name),
                );
                self.path_res.insert(
                    path.id,
                    PathResolution {
                        base: Resolution::Error,
                        consumed: 1,
                    },
                );
            }
            Err(_) => {
                self.error(
                    name.span,
                    format!(
                        "`{}` is ambiguous: more than one enum defines a variant with this name",
                        name.name
                    ),
                );
                self.path_res.insert(
                    path.id,
                    PathResolution {
                        base: Resolution::Error,
                        consumed: 1,
                    },
                );
            }
        }
    }
}
