//! Declaration hierarchy, independent of allocation and dispatch lowering.
//! Inherited declarations retain the lexical owner of their private members.
use super::*;
use std::collections::BTreeSet;
mod signatures;
use signatures::Bindings;

#[derive(Clone)]
struct Method {
    function: a::Function,
    private: bool,
    dispatch: a::MethodDispatch,
    owner: String,
}
#[derive(Clone, Default)]
struct Node {
    fields: BTreeMap<String, (bool, Span)>,
    methods: BTreeMap<String, Method>,
}
struct Hierarchy<'a> {
    definitions: BTreeMap<String, &'a a::Item>,
    interfaces: BTreeSet<String>,
    active: BTreeSet<String>,
    nodes: BTreeMap<String, Node>,
    constants: crate::constants::Constants,
}
pub(super) fn validate(module: &a::Module) -> Result<()> {
    let mut hierarchy = Hierarchy {
        definitions: BTreeMap::new(),
        interfaces: BTreeSet::new(),
        active: BTreeSet::new(),
        nodes: BTreeMap::new(),
        constants: crate::constants::evaluate(module)?,
    };
    for item in &module.items {
        match &item.kind {
            a::ItemKind::Aggregate { name, .. } | a::ItemKind::Enum { name, .. } => {
                if hierarchy.definitions.insert(name.clone(), item).is_some() {
                    return Err(error(item.span, "E0301", "duplicate type declaration"));
                }
            }
            a::ItemKind::Interface { name, .. } => {
                hierarchy.interfaces.insert(name.clone());
            }
            _ => {}
        }
    }
    for name in hierarchy.definitions.keys().cloned().collect::<Vec<_>>() {
        if matches!(
            hierarchy.definitions[&name].kind,
            a::ItemKind::Aggregate { .. }
        ) {
            hierarchy.node(&name)?;
        }
    }
    Ok(())
}
impl Hierarchy<'_> {
    fn node(&mut self, name: &str) -> Result<Node> {
        if let Some(node) = self.nodes.get(name) {
            return Ok(node.clone());
        }
        let item = self.definitions[name];
        let a::ItemKind::Aggregate {
            class,
            interfaces,
            members,
            generics,
            ..
        } = &item.kind
        else {
            unreachable!()
        };
        if !self.active.insert(name.to_owned()) {
            return Err(error(item.span, "E0410", "cycle in class inheritance"));
        }
        if self.active.len() > 32 {
            return Err(error(
                item.span,
                "E0410",
                "class inheritance nesting limit exceeded",
            ));
        }
        let mut base = None;
        for (index, ty) in interfaces.iter().enumerate() {
            let a::TypeKind::Named { path, arguments } = &ty.kind else {
                return Err(error(
                    ty.span,
                    "E0410",
                    "base must name a class or interface",
                ));
            };
            let parent = path.join("::");
            if self.interfaces.contains(&parent) {
                continue;
            }
            let Some(declaration) = self.definitions.get(&parent) else {
                return Err(error(ty.span, "E0410", "unknown base class or interface"));
            };
            let a::ItemKind::Aggregate {
                class: true,
                generics,
                ..
            } = &declaration.kind
            else {
                return Err(error(ty.span, "E0410", "concrete parent must be a class"));
            };
            if !class {
                return Err(error(
                    ty.span,
                    "E0410",
                    "struct cannot inherit a concrete type",
                ));
            }
            if base.is_some() {
                return Err(error(
                    ty.span,
                    "E0410",
                    "class can have only one class parent",
                ));
            }
            if index != 0 {
                return Err(error(
                    ty.span,
                    "E0410",
                    "class parent must precede interfaces",
                ));
            }
            base = Some((
                parent,
                Bindings::parent(generics, arguments, ty.span, &self.constants)?,
            ));
        }
        let mut node = if let Some((parent, bindings)) = &base {
            let mut inherited = self.node(parent)?;
            for method in inherited.methods.values_mut() {
                bindings.method(&mut method.function);
                signatures::freeze_constants(&mut method.function, generics, &self.constants);
            }
            inherited
        } else {
            Node::default()
        };
        let mut seen = BTreeSet::new();
        for member in members {
            match member {
                a::Member::Field {
                    name: field,
                    private,
                    span,
                    ..
                } => {
                    if !seen.insert(field.clone()) {
                        return Err(error(*span, "E0301", "duplicate aggregate member"));
                    }
                    if node.fields.get(field).is_some_and(|(private, _)| !private)
                        || node.methods.get(field).is_some_and(|m| !m.private)
                    {
                        return Err(error(*span, "E0411", "field hides an inherited member"));
                    }
                    node.fields.insert(field.clone(), (*private, *span));
                }
                a::Member::Method {
                    function,
                    private,
                    dispatch,
                } => {
                    if !seen.insert(function.name.clone()) {
                        return Err(error(function.span, "E0301", "duplicate aggregate member"));
                    }
                    let instance = function.parameters.first().is_some_and(|p| p.receiver);
                    if *dispatch != a::MethodDispatch::Ordinary && (!class || !instance || *private)
                    {
                        return Err(error(
                            function.span,
                            "E0411",
                            "virtual/override requires a public class instance method",
                        ));
                    }
                    if node
                        .fields
                        .get(&function.name)
                        .is_some_and(|(private, _)| !private)
                    {
                        return Err(error(
                            function.span,
                            "E0411",
                            "method hides an inherited field",
                        ));
                    }
                    let mut resolved = function.clone();
                    signatures::freeze_constants(&mut resolved, generics, &self.constants);
                    let inherited = node.methods.get(&function.name).filter(|m| !m.private);
                    if *dispatch == a::MethodDispatch::Override {
                        let inherited = inherited.ok_or_else(|| {
                            error(
                                function.span,
                                "E0411",
                                "override has no accessible inherited method",
                            )
                        })?;
                        if inherited.dispatch == a::MethodDispatch::Ordinary {
                            return Err(error(
                                function.span,
                                "E0411",
                                "cannot override a non-virtual method",
                            )
                            .with_note(
                                inherited.function.span,
                                "inherited method is declared here",
                            ));
                        }
                        if !signatures::compatible(
                            &inherited.function,
                            &resolved,
                            generics,
                            &self.constants,
                        )? {
                            return Err(error(
                                function.span,
                                "E0411",
                                "override signature or receiver mode differs",
                            )
                            .with_note(
                                inherited.function.span,
                                format!("required by inherited method in '{}'", inherited.owner),
                            ));
                        }
                    } else if let Some(inherited) = inherited {
                        return Err(error(function.span, "E0411", "method hides an inherited declaration; a virtual method requires override")
                            .with_note(inherited.function.span, "inherited method is declared here"));
                    }
                    node.methods.insert(
                        function.name.clone(),
                        Method {
                            function: resolved,
                            private: *private,
                            dispatch: *dispatch,
                            owner: name.into(),
                        },
                    );
                }
                a::Member::Const { name, span, .. } => {
                    if !seen.insert(name.clone()) {
                        return Err(error(*span, "E0301", "duplicate aggregate member"));
                    }
                }
                _ => {}
            }
        }
        self.active.remove(name);
        self.nodes.insert(name.into(), node.clone());
        Ok(node)
    }
}

#[cfg(test)]
mod tests;
