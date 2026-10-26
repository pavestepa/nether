//! Static readonly aggregate components. Descriptor storage is never ownership
//! of its target, even if the enclosing aggregate is moved or copied.
use super::*;
pub(super) fn read(value: h::Expr) -> h::Expr {
    if let Type::StoredView(target) = &value.ty {
        h::Expr {
            ty: (**target).clone(),
            span: value.span,
            kind: h::ExprKind::StoredView(Box::new(value)),
        }
    } else {
        value
    }
}
fn callable_mode(ty: &Type) -> bool {
    match ty {
        Type::Function {
            contract: Some(_), ..
        } => true,
        Type::Record { fields, .. }
        | Type::Class {
            storage: Some(fields),
            ..
        } => fields.iter().any(|f| callable_mode(&f.ty)),
        Type::Tuple(fields) => fields.iter().any(callable_mode),
        Type::Array(t, _) | Type::StoredView(t) => callable_mode(t),
        Type::Enum { variants, .. } => variants.iter().flat_map(|v| &v.fields).any(callable_mode),
        _ => false,
    }
}
pub(super) fn same_mode(expected: &Type, actual: &Type, span: Span) -> Result<()> {
    if let (
        Type::Function {
            unsafe_: a_unsafe,
            parameters: a,
            result: ar,
            contract: Some(ac),
        },
        Type::Function {
            unsafe_: b_unsafe,
            parameters: b,
            result: br,
            contract: Some(bc),
        },
    ) = (expected, actual)
    {
        let view = match (&ac.view, &bc.view) {
            (Some(a), Some(b)) => a.access == b.access && b.origins.is_subset(&a.origins),
            (None, None) => true,
            _ => false,
        };
        if a_unsafe == b_unsafe && a == b && ar == br && ac.modes == bc.modes && view {
            return Ok(());
        }
    }
    if expected != actual
        && (expected.contains_views() || actual.contains_views() || callable_mode(expected))
    {
        Err(error(
            span,
            "E0396",
            "incompatible static storage or callable ABI modes",
        ))
    } else {
        same(expected, actual, span)
    }
}
impl Checker<'_> {
    pub(super) fn same_storage_mode(
        &self,
        expected: &Type,
        actual: &Type,
        span: Span,
    ) -> Result<()> {
        // A caller can be checked before its callee's representation is known.
        // Recheck physical compatibility once result inference has converged.
        if !self.mono.borrow().pending_results.is_empty() {
            same(expected, actual, span)
        } else {
            same_mode(expected, actual, span)
        }
    }

    pub(super) fn request_parameter_view(&mut self, value: &h::Expr) {
        if views::view_root(value) {
            return;
        }
        for (root, _) in place_paths(self, value, &mut Vec::new()) {
            if self.storage_parameters.contains(&root) && self.locals[root].view.is_none() {
                self.storage_requests.insert(
                    key(self.locals[root].span),
                    Type::StoredView(Box::new(self.locals[root].ty.clone())),
                );
            }
        }
    }
    pub(super) fn storage_assignment(
        &mut self,
        source: &a::Expr,
        place: &h::Expr,
    ) -> Result<h::Expr> {
        fn class_field(v: &h::Expr) -> bool {
            match &v.kind {
                h::ExprKind::Field(base, _) => {
                    matches!(base.ty, Type::Class { .. }) || class_field(base)
                }
                h::ExprKind::Index(base, _) | h::ExprKind::Payload(base, _) => class_field(base),
                _ => false,
            }
        }
        if !class_field(place) {
            return self.storage_value(source, Some(&place.ty));
        }
        let value = self.expr(source, Some(&place.ty), true)?;
        if matches!(place.ty, Type::StoredView(_)) {
            self.request_parameter_view(&value);
        }
        if !value.ty.copyable()
            && views::place(&value)
            && (views::view_root(&value) || matches!(place.ty, Type::StoredView(_)))
        {
            Ok(h::Expr {
                ty: Type::StoredView(Box::new(value.ty.clone())),
                span: value.span,
                kind: h::ExprKind::StoreView(Box::new(value)),
            })
        } else {
            Ok(value)
        }
    }
    pub(super) fn storage_value(
        &mut self,
        source: &a::Expr,
        expected: Option<&Type>,
    ) -> Result<h::Expr> {
        let value = self.expr(source, expected, true)?;
        if let Some(expected) = expected {
            let expected = match expected {
                Type::StoredView(target) => &**target,
                ty => ty,
            };
            self.request_storage_mode(&value, expected)?;
        }
        if matches!(expected, Some(Type::StoredView(_))) {
            self.request_parameter_view(&value);
        }
        if !value.ty.copyable()
            && views::place(&value)
            && (views::view_root(&value)
                || matches!(expected, Some(Type::StoredView(_)))
                || self.ownership_view_needed(source))
        {
            Ok(h::Expr {
                ty: Type::StoredView(Box::new(value.ty.clone())),
                span: value.span,
                kind: h::ExprKind::StoreView(Box::new(value)),
            })
        } else {
            Ok(value)
        }
    }
}

pub(super) type Modes = BTreeMap<(usize, usize, usize), Type>;
pub(super) fn key(span: Span) -> (usize, usize, usize) {
    (span.source.0, span.start, span.end)
}

// Readonly descriptors dominate owning components in a static representation.
// This chooses a type, never a runtime tag or an implicit owner allocation.
pub(super) fn union(a: &Type, b: &Type, span: Span) -> Result<Type> {
    same(a, b, span)?;
    if a == b {
        return Ok(a.clone());
    }
    Ok(match (a, b) {
        (
            Type::Function { contract: None, .. },
            b @ Type::Function {
                contract: Some(_), ..
            },
        ) => b.clone(),
        (
            a @ Type::Function {
                contract: Some(_), ..
            },
            Type::Function { contract: None, .. },
        ) => a.clone(),
        (
            Type::Function {
                unsafe_: a_unsafe,
                parameters: a,
                result: ar,
                contract: Some(ac),
            },
            Type::Function {
                unsafe_: b_unsafe,
                parameters: b,
                result: br,
                contract: Some(bc),
            },
        ) => {
            debug_assert_eq!(a_unsafe, b_unsafe);
            let mut c = (**ac).clone();
            for source in &bc.sources {
                if !c.sources.contains(source) {
                    c.sources.push(source.clone())
                }
            }
            c.sources.sort();
            for (mode, other) in c.modes.iter_mut().zip(&bc.modes) {
                if mode.is_none() {
                    *mode = *other
                }
            }
            match (&mut c.view, &bc.view) {
                (Some(a), Some(b)) => {
                    if a.access != b.access {
                        return Err(error(
                            span,
                            "E0396",
                            "incompatible function result capabilities",
                        ));
                    }
                    a.origins.extend(b.origins.clone());
                }
                (None, Some(b)) => c.view = Some(b.clone()),
                _ => {}
            }
            for item in &bc.storage {
                if let Some(old) = c.storage.iter_mut().find(|old| old.path == item.path) {
                    old.origins.extend(item.origins.clone())
                } else {
                    c.storage.push(item.clone())
                }
            }
            for item in &bc.writes {
                if let Some(old) = c.writes.iter_mut().find(|old| old.target == item.target) {
                    old.origins.extend(item.origins.clone())
                } else {
                    c.writes.push(item.clone())
                }
            }
            Type::Function {
                unsafe_: *a_unsafe,
                parameters: a
                    .iter()
                    .zip(b)
                    .map(|((m, a), (_, b))| Ok((*m, union(a, b, span)?)))
                    .collect::<Result<_>>()?,
                result: Box::new(union(ar, br, span)?),
                contract: Some(Box::new(c)),
            }
        }
        (Type::StoredView(a), Type::StoredView(b)) => {
            Type::StoredView(Box::new(union(a, b, span)?))
        }
        (Type::StoredView(a), b) | (b, Type::StoredView(a)) => {
            Type::StoredView(Box::new(union(a, b, span)?))
        }
        (
            Type::Class { storage: None, .. },
            right @ Type::Class {
                storage: Some(_), ..
            },
        ) => right.clone(),
        (
            left @ Type::Class {
                storage: Some(_), ..
            },
            Type::Class { storage: None, .. },
        ) => left.clone(),
        (
            left @ Type::Class {
                storage: Some(a), ..
            },
            Type::Class {
                storage: Some(b), ..
            },
        ) => {
            let mut result = left.clone();
            let Type::Class {
                storage: Some(fields),
                ..
            } = &mut result
            else {
                unreachable!()
            };
            for ((field, a), b) in fields.iter_mut().zip(a).zip(b) {
                field.ty = union(&a.ty, &b.ty, span)?;
            }
            result
        }
        (left @ Type::Record { fields: a, .. }, Type::Record { fields: b, .. }) => {
            let mut ty = a
                .iter()
                .zip(b)
                .map(|(a, b)| {
                    let mut field = a.clone();
                    field.ty = union(&a.ty, &b.ty, span)?;
                    Ok(field)
                })
                .collect::<Result<Vec<_>>>()?;
            let mut result = left.clone();
            let Type::Record { fields, .. } = &mut result else {
                unreachable!()
            };
            std::mem::swap(fields, &mut ty);
            result
        }
        (Type::Tuple(a), Type::Tuple(b)) => Type::Tuple(
            a.iter()
                .zip(b)
                .map(|(a, b)| union(a, b, span))
                .collect::<Result<_>>()?,
        ),
        (Type::Array(a, n), Type::Array(b, _)) => Type::Array(Box::new(union(a, b, span)?), *n),
        (left @ Type::Enum { variants: a, .. }, Type::Enum { variants: b, .. }) => {
            let mut result = left.clone();
            let Type::Enum { variants, .. } = &mut result else {
                unreachable!()
            };
            for ((variant, a), b) in variants.iter_mut().zip(a).zip(b) {
                variant.fields = a
                    .fields
                    .iter()
                    .zip(&b.fields)
                    .map(|(a, b)| union(a, b, span))
                    .collect::<Result<_>>()?;
            }
            result
        }
        _ => {
            return Err(error(
                span,
                "E0396",
                "incompatible static storage representations",
            ))
        }
    })
}

#[derive(Clone)]
enum Step {
    Field(usize),
    Index,
    Variant(usize),
}
fn place_paths(
    checker: &Checker<'_>,
    value: &h::Expr,
    seen: &mut Vec<usize>,
) -> Vec<(usize, Vec<Step>)> {
    match &value.kind {
        h::ExprKind::Local(local) | h::ExprKind::View(local) => {
            if checker.storage_roots.contains(local) {
                return vec![(*local, Vec::new())];
            }
            if seen.contains(local) {
                return Vec::new();
            }
            seen.push(*local);
            let paths = checker
                .storage_sources
                .get(local)
                .into_iter()
                .flatten()
                .flat_map(|source| place_paths(checker, source, seen))
                .collect();
            seen.pop();
            paths
        }
        h::ExprKind::Field(base, i) | h::ExprKind::Payload(base, i) => {
            let step = if matches!(value.kind, h::ExprKind::Field(..)) {
                Step::Field(*i)
            } else {
                Step::Variant(*i)
            };
            place_paths(checker, base, seen)
                .into_iter()
                .map(|(root, mut path)| {
                    path.push(step.clone());
                    (root, path)
                })
                .collect()
        }
        h::ExprKind::Index(base, _) => place_paths(checker, base, seen)
            .into_iter()
            .map(|(root, mut path)| {
                path.push(Step::Index);
                (root, path)
            })
            .collect(),
        _ => Vec::new(),
    }
}
fn promote(
    ty: &mut Type,
    path: &[Step],
    requested: &Type,
    span: Span,
    nominals: &records::Nominals,
) -> Result<bool> {
    let Some((head, tail)) = path.split_first() else {
        *ty = union(ty, requested, span)?;
        return Ok(true);
    };
    if let Type::Class { .. } = ty {
        let Some(mut fields) = nominals.fields(ty) else {
            return Ok(false);
        };
        let Step::Field(i) = head else {
            return Ok(false);
        };
        if !promote(&mut fields[*i].ty, tail, requested, span, nominals)? {
            return Ok(false);
        }
        let Type::Class { storage, .. } = ty else {
            unreachable!()
        };
        *storage = Some(fields);
        return Ok(true);
    }
    match (head, ty) {
        (Step::Field(i), Type::Record { fields, .. }) => {
            promote(&mut fields[*i].ty, tail, requested, span, nominals)
        }
        (Step::Field(i), Type::Tuple(fields)) => {
            promote(&mut fields[*i], tail, requested, span, nominals)
        }
        (Step::Index, Type::Array(element, _)) => promote(element, tail, requested, span, nominals),
        (Step::Variant(i), Type::Enum { variants, .. }) => {
            let Some((Step::Field(field), tail)) = tail.split_first() else {
                return Ok(false);
            };
            promote(
                &mut variants[*i].fields[*field],
                tail,
                requested,
                span,
                nominals,
            )
        }
        _ => Ok(false),
    }
}
impl Checker<'_> {
    pub(super) fn infers_storage_place(&self, place: &h::Expr) -> bool {
        let paths = place_paths(self, place, &mut Vec::new());
        !paths.is_empty()
            && paths.iter().all(|(root, path)| {
                promote(
                    &mut self.locals[*root].ty.clone(),
                    path,
                    &place.ty,
                    place.span,
                    self.nominals,
                )
                .unwrap_or(false)
            })
    }

    pub(super) fn request_storage_mode(
        &mut self,
        place: &h::Expr,
        requested: &Type,
    ) -> Result<bool> {
        if place.ty == *requested {
            return Ok(false);
        }
        let paths = place_paths(self, place, &mut Vec::new());
        if paths.is_empty() {
            if self.request_result_mode(place, requested)? {
                return Ok(true);
            }
            if matches!(
                place.kind,
                h::ExprKind::Block(_)
                    | h::ExprKind::If { .. }
                    | h::ExprKind::Tuple(_)
                    | h::ExprKind::Array(_)
                    | h::ExprKind::Match(_)
                    | h::ExprKind::New { .. }
            ) {
                return self.request_expression_mode(place.span, &place.ty, requested);
            }
            return Ok(false);
        }
        let mut changed = false;
        for (root, path) in paths {
            let key = key(self.locals[root].span);
            let mut target = self
                .storage_requests
                .get(&key)
                .or_else(|| self.storage_hints.get(&key))
                .unwrap_or(&self.locals[root].ty)
                .clone();
            if !promote(
                match &mut target {
                    Type::StoredView(t) => t,
                    _ => &mut target,
                },
                &path,
                requested,
                place.span,
                self.nominals,
            )? || matches!(target, Type::StoredView(_))
                && !self.storage_parameters.contains(&root)
            {
                continue;
            }
            if target != self.locals[root].ty {
                self.storage_requests.insert(key, target);
                changed = true;
            }
        }
        Ok(changed)
    }

    pub(super) fn request_expression_mode(
        &mut self,
        span: Span,
        actual: &Type,
        required: &Type,
    ) -> Result<bool> {
        let key = key(span);
        let known = self
            .storage_requests
            .get(&key)
            .or_else(|| self.storage_hints.get(&key))
            .unwrap_or(actual);
        let wanted = union(known, required, span)?;
        if &wanted != known {
            self.storage_requests.insert(key, wanted);
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn array_element(&mut self, source: &a::Expr, expected: Option<&Type>) -> Result<h::Expr> {
        let value = self.storage_value(source, expected)?;
        if !value.ty.copyable() && views::place(&value) && self.call_view_needed(source) {
            Ok(h::Expr {
                ty: Type::StoredView(Box::new(value.ty.clone())),
                span: value.span,
                kind: h::ExprKind::StoreView(Box::new(value)),
            })
        } else {
            Ok(value)
        }
    }

    pub(super) fn storage_array(
        &mut self,
        values: &[a::Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Result<h::Expr> {
        let hint = match expected {
            Some(Type::Array(element, _)) => Some((**element).clone()),
            _ => None,
        };
        let snapshot = self.clone();
        let mono = self.mono.borrow().clone();
        let mut logical_hint = hint.clone();
        let mut checked = Vec::new();
        for source in values {
            let value = self.array_element(source, logical_hint.as_ref())?;
            if logical_hint.is_none() {
                logical_hint = Some(value.ty.logical());
            }
            checked.push(value);
        }
        let mut element = hint;
        for value in &checked {
            element = Some(match element {
                Some(old) => union(&old, &value.ty, value.span)?,
                None => value.ty.clone(),
            });
        }
        let element =
            element.ok_or_else(|| error(span, "E0314", "empty array requires an element type"))?;
        if checked.iter().any(|value| value.ty != element) {
            let requests = self.storage_requests.clone();
            *self = snapshot;
            *self.mono.borrow_mut() = mono;
            self.storage_requests.extend(requests);
            checked = values
                .iter()
                .map(|value| self.array_element(value, Some(&element)))
                .collect::<Result<Vec<_>>>()?;
        }
        for value in &checked {
            if !self.request_storage_mode(value, &element)? {
                self.same_storage_mode(&element, &value.ty, value.span)?;
            }
        }
        let ty = Type::Array(Box::new(element), values.len() as u64);
        if let Some(expected) = expected {
            same(expected, &ty, span)?;
        }
        Ok(h::Expr {
            kind: h::ExprKind::Array(checked),
            ty,
            span,
        })
    }
}
