use super::*;

pub(super) fn build_interface_method_table(
    decls: &DeclIndex,
    resolved: &ResolvedNames,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) -> InterfaceMethodTable {
    let mut table = HashMap::new();
    for (&id, iface) in &decls.interface_decls {
        sigs.interface_generics.insert(
            id,
            iface
                .generics
                .iter()
                .map(|generic| generic.name.name.clone())
                .collect(),
        );
        let parents = iface
            .parents
            .iter()
            .filter_map(|parent| lower_generic_bound(parent, resolved, decls, diags))
            .collect();
        sigs.interface_parents.insert(id, parents);
    }

    fn build_one(
        id: DefId,
        decls: &DeclIndex,
        resolved: &ResolvedNames,
        sigs: &Signatures,
        diags: &mut Vec<Diagnostic>,
        visiting: &mut HashSet<DefId>,
        table: &mut InterfaceMethodTable,
    ) {
        if table.contains_key(&id) {
            return;
        }
        let Some(iface) = decls.interface_decls.get(&id).copied() else {
            return;
        };
        if !visiting.insert(id) {
            diags.push(
                Diagnostic::error(format!(
                    "interface inheritance cycle involving `{}`",
                    resolved.definitions.get(id).name
                ))
                .with_label(iface.span, "cycle reaches this interface"),
            );
            return;
        }

        let mut methods: HashMap<Symbol, InterfaceMethod> = HashMap::new();
        for parent in sigs.interface_parents.get(&id).cloned().unwrap_or_default() {
            build_one(
                parent.interface,
                decls,
                resolved,
                sigs,
                diags,
                visiting,
                table,
            );
            let parent_generics = sigs
                .interface_generics
                .get(&parent.interface)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let parent_subst: HashMap<Symbol, Type> = parent_generics
                .iter()
                .cloned()
                .zip(parent.args.iter().cloned())
                .collect();
            for (name, inherited) in table.get(&parent.interface).cloned().unwrap_or_default() {
                let inherited = InterfaceMethod {
                    sig: specialize_fn_sig(&inherited.sig, &parent_subst),
                    default: inherited.default.map(|default| InterfaceDefault {
                        source: default.source,
                        subst: default
                            .subst
                            .into_iter()
                            .map(|(name, ty)| (name, substitute_generic(&ty, &parent_subst)))
                            .collect(),
                    }),
                    ambiguous_default: inherited.ambiguous_default,
                };
                if let Some(existing) = methods.get_mut(&name) {
                    if !method_signatures_match(&existing.sig, &inherited.sig) {
                        diags.push(
                            Diagnostic::error(format!(
                                "inherited method `{name}` has incompatible signatures in interface `{}`",
                                iface.name.name
                            ))
                            .with_label(iface.span, "conflicting parent interfaces"),
                        );
                    }
                    existing.ambiguous_default |= inherited.ambiguous_default;
                    match (&existing.default, &inherited.default) {
                        (Some(left), Some(right))
                            if left.source != right.source || left.subst != right.subst =>
                        {
                            existing.default = None;
                            existing.ambiguous_default = true;
                        }
                        (None, Some(default)) if !existing.ambiguous_default => {
                            existing.default = Some(default.clone());
                        }
                        _ => {}
                    }
                } else {
                    methods.insert(name, inherited);
                }
            }
        }

        let identity_subst: HashMap<Symbol, Type> = iface
            .generics
            .iter()
            .map(|generic| {
                (
                    generic.name.name.clone(),
                    Type::Generic(generic.name.name.clone()),
                )
            })
            .collect();
        for method in &iface.methods {
            methods.insert(
                method.name.name.clone(),
                InterfaceMethod {
                    sig: build_fn_sig(method, resolved, decls, diags),
                    default: method.body.as_ref().map(|_| InterfaceDefault {
                        source: id,
                        subst: identity_subst.clone(),
                    }),
                    ambiguous_default: false,
                },
            );
        }
        visiting.remove(&id);
        table.insert(id, methods);
    }

    let mut visiting = HashSet::new();
    for id in decls.interface_decls.keys().copied().collect::<Vec<_>>() {
        build_one(id, decls, resolved, sigs, diags, &mut visiting, &mut table);
    }
    for (interface, methods) in &table {
        for (name, method) in methods {
            sigs.interface_methods
                .insert((*interface, name.clone()), method.sig.clone());
        }
    }
    table
}

pub(super) fn build_impl_methods(
    module: &Module,
    resolved: &ResolvedNames,
    decls: &DeclIndex,
    interface_methods: &InterfaceMethodTable,
    sigs: &mut Signatures,
    diags: &mut Vec<Diagnostic>,
) {
    // All user-written methods share one namespace per owner, regardless
    // of which freely mixed impl block contains them — except a concrete
    // specialization's own bucket, which only overrides its exact owner
    // arguments (`MethodSet`).
    struct RawMethod {
        owner: DefId,
        owner_name: Symbol,
        name: Symbol,
        name_span: Span,
        specialization: Option<Vec<Type>>,
        sig: FnSig,
    }
    let mut raw = Vec::new();
    for item in &module.items {
        let Item::Impl(b) = item else { continue };
        let Some(owner) = resolved.definitions.lookup_in(b.span.file, &b.target.name) else {
            continue;
        };
        let owner_generics = owner_generics_for_impl(b, owner, decls, resolved, diags);
        let specialization = impl_specialization_args(b, owner, decls, resolved, diags);
        if let Some(args) = &specialization {
            sigs.impl_specializations.insert(b.id, args.clone());
            if !b.interfaces.is_empty() {
                diags.push(
                    Diagnostic::error(
                        "a concrete specialization (`impl Owner<ConcreteArgs>`) cannot also \
                         implement an interface yet",
                    )
                    .with_label(b.span, "in this `impl` block"),
                );
            }
        }
        for m in &b.methods {
            if specialization.is_some() && m.self_param.is_none() {
                diags.push(
                    Diagnostic::error(
                        "a concrete specialization cannot override a static method yet — only \
                         `self`/`mut self` methods",
                    )
                    .with_label(m.name.span, "here"),
                );
                continue;
            }
            let mut sig = build_fn_sig(m, resolved, decls, diags);
            sig.generics.splice(0..0, owner_generics.clone());
            raw.push(RawMethod {
                owner,
                owner_name: b.target.name.clone(),
                name: m.name.name.clone(),
                name_span: m.name.span,
                specialization: specialization.clone(),
                sig,
            });
        }
    }

    let mut sets: HashMap<(DefId, Symbol), MethodSet> = HashMap::new();
    for entry in &raw {
        let key = (entry.owner, entry.name.clone());
        let set = sets.entry(key).or_default();
        match &entry.specialization {
            None => {
                if set.generic.is_some() {
                    diags.push(
                        Diagnostic::error(format!(
                            "method `{}` is defined more than once for `{}`",
                            entry.name, entry.owner_name
                        ))
                        .with_label(entry.name_span, "redefined here"),
                    );
                } else {
                    set.generic = Some(entry.sig.clone());
                }
            }
            Some(args) => {
                if set
                    .specializations
                    .iter()
                    .any(|(existing, _)| existing == args)
                {
                    diags.push(
                        Diagnostic::error(format!(
                            "method `{}` is defined more than once for this specialization of `{}`",
                            entry.name, entry.owner_name
                        ))
                        .with_label(entry.name_span, "redefined here"),
                    );
                } else {
                    set.specializations.push((args.clone(), entry.sig.clone()));
                }
            }
        }
    }
    for entry in &raw {
        let Some(args) = &entry.specialization else {
            continue;
        };
        let key = (entry.owner, entry.name.clone());
        let Some(generic_sig) = sets.get(&key).and_then(|set| set.generic.as_ref()) else {
            continue;
        };
        if !specialization_matches_generic(entry.owner, decls, generic_sig, args, &entry.sig) {
            diags.push(
                Diagnostic::error(format!(
                    "method `{}` on this specialization of `{}` must have the same signature as \
                     the generic `impl<...> {}<...>` version",
                    entry.name, entry.owner_name, entry.owner_name
                ))
                .with_label(entry.name_span, "signature does not match"),
            );
        }
    }
    sigs.methods = sets;

    struct Request {
        owner: DefId,
        owner_ty: Type,
        owner_name: Symbol,
        owner_span: Span,
        owner_generics: Vec<(Symbol, Option<GenericBound>)>,
        bound: GenericBound,
        allow_defaults: bool,
    }

    let mut requests = Vec::new();
    for (&owner, decl) in &decls.type_decls {
        for interface in &decl.interfaces {
            if let Some(bound) = lower_generic_bound(interface, resolved, decls, diags) {
                let owner_generics = owner_generic_params(owner, decls, resolved, diags);
                requests.push(Request {
                    owner,
                    owner_ty: owner_as_type_from_generics(owner, resolved, decls, &owner_generics),
                    owner_name: decl.name.name.clone(),
                    owner_span: interface.span(),
                    owner_generics,
                    bound,
                    allow_defaults: true,
                });
            }
        }
    }
    for (&owner, decl) in &decls.enum_decls {
        for interface in &decl.interfaces {
            if let Some(bound) = lower_generic_bound(interface, resolved, decls, diags) {
                let owner_generics = owner_generic_params(owner, decls, resolved, diags);
                requests.push(Request {
                    owner,
                    owner_ty: owner_as_type_from_generics(owner, resolved, decls, &owner_generics),
                    owner_name: decl.name.name.clone(),
                    owner_span: interface.span(),
                    owner_generics,
                    bound,
                    allow_defaults: true,
                });
            }
        }
    }
    for item in &module.items {
        let Item::Impl(block) = item else { continue };
        let Some(owner) = resolved
            .definitions
            .lookup_in(block.span.file, &block.target.name)
        else {
            continue;
        };
        for interface in &block.interfaces {
            if let Some(bound) = lower_generic_bound(interface, resolved, decls, diags) {
                let owner_generics = owner_generics_for_impl(block, owner, decls, resolved, diags);
                requests.push(Request {
                    owner,
                    owner_ty: owner_as_type_from_generics(owner, resolved, decls, &owner_generics),
                    owner_name: block.target.name.clone(),
                    owner_span: interface.span(),
                    owner_generics,
                    bound,
                    allow_defaults: false,
                });
            }
        }
    }

    let mut seen = HashSet::new();
    requests.retain(|request| {
        if seen.insert((request.owner, request.bound.clone())) {
            sigs.impls
                .insert((request.owner_ty.clone(), request.bound.clone()));
            true
        } else {
            diags.push(
                Diagnostic::error(format!(
                    "`{}` already implements `{}`",
                    request.owner_name,
                    resolved.definitions.get(request.bound.interface).name
                ))
                .with_label(request.owner_span, "duplicate implementation"),
            );
            false
        }
    });

    #[derive(Clone)]
    struct DefaultCandidate {
        source: DefId,
        subst: HashMap<Symbol, Type>,
        sig: FnSig,
    }
    struct DeclaredNeed {
        owner_name: Symbol,
        span: Span,
        interface_name: Symbol,
        sig: FnSig,
        defaults: Vec<DefaultCandidate>,
        ambiguous: bool,
    }
    let mut declared: HashMap<(DefId, Symbol), DeclaredNeed> = HashMap::new();

    for request in requests {
        let iface_id = request.bound.interface;
        let Some(methods) = interface_methods.get(&iface_id) else {
            continue;
        };
        let interface_generics = sigs
            .interface_generics
            .get(&iface_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let interface_subst: HashMap<Symbol, Type> = interface_generics
            .iter()
            .cloned()
            .zip(request.bound.args.iter().cloned())
            .collect();
        let owner_generics = request.owner_generics.clone();

        for (name, method) in methods {
            let mut expected = specialize_fn_sig(&method.sig, &interface_subst);
            expected.generics.splice(0..0, owner_generics.clone());
            let key = (request.owner, name.clone());
            if let Some(actual) = sigs.methods.get(&key).and_then(|set| set.generic.as_ref()) {
                if !method_signatures_match(actual, &expected) {
                    diags.push(
                        Diagnostic::error(format!(
                            "method `{name}` does not match its declaration in interface `{}`",
                            resolved.definitions.get(iface_id).name
                        ))
                        .with_label(request.owner_span, "implementation is here"),
                    );
                }
                continue;
            }

            if !request.allow_defaults {
                diags.push(
                    Diagnostic::error(format!(
                        "`{}` must explicitly implement method `{name}` of interface `{}`",
                        request.owner_name,
                        resolved.definitions.get(iface_id).name
                    ))
                    .with_label(request.owner_span, "explicit implementation is here"),
                );
                continue;
            }

            let candidate = method.default.as_ref().map(|default| DefaultCandidate {
                source: default.source,
                subst: default
                    .subst
                    .iter()
                    .map(|(name, ty)| (name.clone(), substitute_generic(ty, &interface_subst)))
                    .collect(),
                sig: expected.clone(),
            });
            let need = declared.entry(key).or_insert_with(|| DeclaredNeed {
                owner_name: request.owner_name.clone(),
                span: request.owner_span,
                interface_name: resolved.definitions.get(iface_id).name.clone(),
                sig: expected.clone(),
                defaults: Vec::new(),
                ambiguous: false,
            });
            if !method_signatures_match(&need.sig, &expected) {
                need.ambiguous = true;
                diags.push(
                    Diagnostic::error(format!(
                        "method `{name}` has incompatible signatures in implemented interfaces"
                    ))
                    .with_label(request.owner_span, "conflicting interface"),
                );
            }
            need.ambiguous |= method.ambiguous_default;
            if let Some(candidate) = candidate {
                if !need.defaults.iter().any(|existing| {
                    existing.source == candidate.source && existing.subst == candidate.subst
                }) {
                    need.defaults.push(candidate);
                }
            }
        }
    }

    for ((owner, name), need) in declared {
        if need.ambiguous || need.defaults.len() > 1 {
            diags.push(
                Diagnostic::error(format!(
                    "multiple default implementations of method `{name}` are available for `{}`; provide an explicit implementation",
                    need.owner_name
                ))
                .with_label(need.span, "ambiguous default"),
            );
        } else if let Some(default) = need.defaults.into_iter().next() {
            sigs.methods
                .entry((owner, name.clone()))
                .or_default()
                .generic = Some(default.sig);
            sigs.default_method_substitutions
                .insert((owner, name.clone()), default.subst);
            sigs.default_method_sources
                .insert((owner, name), default.source);
        } else {
            diags.push(
                Diagnostic::error(format!(
                    "`{}` does not implement required method `{name}` of interface `{}`",
                    need.owner_name, need.interface_name
                ))
                .with_label(need.span, "missing implementation"),
            );
        }
    }
}
