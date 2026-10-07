//! Compile-time module linking, including cycles between declarations.
use crate::{
    ast::*,
    parser::parse,
    source::{Diagnostic, Source, SourceId, Span},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

pub struct Loaded {
    pub sources: Vec<Source>,
    pub module: Option<Module>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn load(entry: &Path, package: &str) -> Loaded {
    let mut sources = Vec::new();
    let mut diagnostics = Vec::new();
    let entry = match entry.canonicalize() {
        Ok(path) => path,
        Err(e) => {
            diagnostics.push(Diagnostic::new(
                "E0500",
                e.to_string(),
                Span {
                    source: SourceId(0),
                    start: 0,
                    end: 0,
                },
            ));
            sources.push(Source::new(entry.to_string_lossy(), ""));
            return Loaded {
                sources,
                module: None,
                diagnostics,
            };
        }
    };
    let root = entry.parent().unwrap().to_path_buf();
    let mut pending = vec![entry.clone()];
    let mut modules = BTreeMap::new();
    let mut paths = BTreeSet::new();
    let mut imports = BTreeMap::new();
    paths.insert(entry.clone());
    while let Some(path) = pending.pop() {
        let id = SourceId(sources.len());
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                sources.push(Source::new(path.to_string_lossy(), ""));
                diagnostics.push(Diagnostic::new(
                    "E0500",
                    e.to_string(),
                    Span {
                        source: id,
                        start: 0,
                        end: 0,
                    },
                ));
                continue;
            }
        };
        let source = Source::new(path.to_string_lossy(), text);
        let parsed = parse(id, &source.text);
        sources.push(source);
        diagnostics.extend(parsed.diagnostics);
        let Some(module) = parsed.module else {
            continue;
        };
        for item in &module.items {
            if let ItemKind::Import { from, .. } = &item.kind {
                match import_path(&root, &path, from, package)
                    .and_then(|p| p.canonicalize().map_err(|e| e.to_string()))
                {
                    Ok(import) if import.starts_with(&root) => {
                        imports.insert((path.clone(), from.clone()), import.clone());
                        if paths.insert(import.clone()) {
                            pending.push(import);
                        }
                    }
                    Ok(_) => diagnostics.push(Diagnostic::new(
                        "E0501",
                        "import escapes package root",
                        item.span,
                    )),
                    Err(message) => diagnostics.push(Diagnostic::new("E0501", message, item.span)),
                }
            }
        }
        modules.insert(path, module);
    }
    if !diagnostics.is_empty() {
        return Loaded {
            sources,
            module: None,
            diagnostics,
        };
    }
    match link(&entry, modules, imports) {
        Ok(module) => Loaded {
            sources,
            module: Some(module),
            diagnostics,
        },
        Err(error) => {
            diagnostics.push(error);
            Loaded {
                sources,
                module: None,
                diagnostics,
            }
        }
    }
}

fn import_path(
    root: &Path,
    current: &Path,
    import: &str,
    package: &str,
) -> Result<PathBuf, String> {
    let relative = if import.starts_with("./") || import.starts_with("../") {
        current.parent().unwrap().join(import)
    } else if let Some(path) = import.strip_prefix(&format!("{package}/")) {
        root.join(path)
    } else {
        return Err(format!("unknown package alias in import '{import}'"));
    };
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if !normalized.starts_with(root) {
        return Err("import escapes package root".into());
    }
    if normalized.extension().is_none() {
        normalized.set_extension("nr");
    }
    Ok(normalized)
}

#[derive(Clone)]
struct Symbol {
    canonical: String,
    private: bool,
}
type Symbols = BTreeMap<String, Symbol>;

fn link(
    entry: &Path,
    modules: BTreeMap<PathBuf, Module>,
    imports: BTreeMap<(PathBuf, String), PathBuf>,
) -> Result<Module, Diagnostic> {
    let mut visible: BTreeMap<PathBuf, Symbols> = BTreeMap::new();
    for (index, (path, module)) in modules.iter().enumerate() {
        let mut names = Symbols::new();
        for item in &module.items {
            let Some(name) = item_name(&item.kind) else {
                continue;
            };
            let canonical = if path == entry && name == "main" {
                "main".into()
            } else {
                format!("m{index}::{name}")
            };
            if names
                .insert(
                    name.into(),
                    Symbol {
                        canonical,
                        private: item.private,
                    },
                )
                .is_some()
            {
                return Err(Diagnostic::new(
                    "E0301",
                    "duplicate module declaration",
                    item.span,
                ));
            }
        }
        visible.insert(path.clone(), names);
    }
    // Re-exports resolve monotonically. A cycle with no definition cannot invent one.
    loop {
        let mut progress = false;
        for (path, module) in &modules {
            for item in &module.items {
                let ItemKind::Import {
                    export: true,
                    names,
                    from,
                } = &item.kind
                else {
                    continue;
                };
                let imported = imports[&(path.clone(), from.clone())].clone();
                for name in names {
                    if let Some(symbol) =
                        visible[&imported].get(name).filter(|s| !s.private).cloned()
                    {
                        match visible.get_mut(path).unwrap().entry(name.clone()) {
                            std::collections::btree_map::Entry::Vacant(entry) => {
                                entry.insert(symbol);
                                progress = true;
                            }
                            std::collections::btree_map::Entry::Occupied(entry)
                                if entry.get().canonical != symbol.canonical =>
                            {
                                return Err(Diagnostic::new(
                                    "E0502",
                                    "conflicting re-export",
                                    item.span,
                                ))
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if !progress {
            break;
        }
    }
    let mut result = Vec::new();
    for (path, module) in modules {
        let mut scope = visible[&path].clone();
        for item in &module.items {
            let ItemKind::Import { names, from, .. } = &item.kind else {
                continue;
            };
            let imported = imports[&(path.clone(), from.clone())].clone();
            for name in names {
                let symbol = visible[&imported].get(name).ok_or_else(|| {
                    Diagnostic::new(
                        "E0503",
                        format!("module does not export '{name}'"),
                        item.span,
                    )
                })?;
                if symbol.private {
                    return Err(Diagnostic::new(
                        "E0504",
                        format!("'{name}' is private"),
                        item.span,
                    ));
                }
                if let Some(previous) = scope.insert(name.clone(), symbol.clone()) {
                    if previous.canonical != symbol.canonical {
                        return Err(Diagnostic::new(
                            "E0502",
                            "import conflicts with a local name",
                            item.span,
                        ));
                    }
                }
            }
        }
        let mut rewrite = Rewrite {
            globals: &scope,
            scopes: vec![BTreeSet::new()],
        };
        for mut item in module.items {
            if matches!(item.kind, ItemKind::Import { .. }) {
                continue;
            }
            rewrite.item(&mut item);
            result.push(item);
        }
    }
    Ok(Module { items: result })
}

fn item_name(item: &ItemKind) -> Option<&str> {
    Some(match item {
        ItemKind::Function(function) => &function.name,
        ItemKind::Aggregate { name, .. }
        | ItemKind::Enum { name, .. }
        | ItemKind::Interface { name, .. }
        | ItemKind::Const { name, .. } => name,
        _ => return None,
    })
}

struct Rewrite<'a> {
    globals: &'a Symbols,
    scopes: Vec<BTreeSet<String>>,
}
impl Rewrite<'_> {
    fn name(&self, name: &mut String) {
        if !self.scopes.iter().rev().any(|scope| scope.contains(name)) {
            if let Some(symbol) = self.globals.get(name) {
                *name = symbol.canonical.clone();
            }
        }
    }
    fn declaration(&self, name: &mut String) {
        if let Some(symbol) = self.globals.get(name) {
            *name = symbol.canonical.clone();
        }
    }
    fn generics(&mut self, generics: &mut [Generic]) {
        for generic in generics {
            match &mut generic.kind {
                GenericKind::Type {
                    constraints,
                    default,
                } => {
                    for constraint in constraints {
                        self.ty(constraint);
                    }
                    if let Some(default) = default {
                        self.ty(default);
                    }
                }
                GenericKind::Const { ty, default } => {
                    self.ty(ty);
                    if let Some(default) = default {
                        self.expr(default);
                    }
                }
            }
            self.scopes.last_mut().unwrap().insert(generic.name.clone());
        }
    }
    fn item(&mut self, item: &mut Item) {
        self.scopes.push(BTreeSet::new());
        match &mut item.kind {
            ItemKind::Function(function) => {
                self.declaration(&mut function.name);
                self.function(function);
            }
            ItemKind::Const { name, ty, value } => {
                self.declaration(name);
                self.ty(ty);
                self.expr(value);
            }
            ItemKind::Aggregate {
                name,
                generics,
                interfaces,
                members,
                ..
            } => {
                self.declaration(name);
                self.generics(generics);
                for interface in interfaces {
                    self.ty(interface);
                }
                let member_names: BTreeSet<String> = members
                    .iter()
                    .filter_map(|member| match member {
                        Member::Field { name, .. } | Member::Const { name, .. } => {
                            Some(name.clone())
                        }
                        Member::Method { function, .. } => Some(function.name.clone()),
                        _ => None,
                    })
                    .collect();
                for member in members {
                    match member {
                        Member::Field { ty, .. } => self.ty(ty),
                        Member::Method { function, .. } => {
                            let instance = function.parameters.iter().any(|p| p.receiver);
                            if instance {
                                self.scopes.push(member_names.clone());
                            }
                            self.function(function);
                            if instance {
                                self.scopes.pop();
                            }
                        }
                        Member::Constructor {
                            parameters, body, ..
                        } => {
                            self.scopes.push(BTreeSet::new());
                            self.parameters(parameters);
                            self.block(body);
                            self.scopes.pop();
                        }
                        Member::Destructor(body) => self.block(body),
                        Member::Const { ty, value, .. } => {
                            self.ty(ty);
                            self.expr(value);
                        }
                    }
                }
            }
            ItemKind::Enum {
                name,
                generics,
                variants,
            } => {
                self.declaration(name);
                self.generics(generics);
                for variant in variants {
                    match &mut variant.fields {
                        VariantFields::Unit => {}
                        VariantFields::Tuple(fields) => {
                            for ty in fields {
                                self.ty(ty);
                            }
                        }
                        VariantFields::Named(fields) => {
                            for (_, ty) in fields {
                                self.ty(ty);
                            }
                        }
                    }
                }
            }
            ItemKind::Interface {
                name,
                generics,
                methods,
            } => {
                self.declaration(name);
                self.generics(generics);
                for method in methods {
                    self.function(method);
                }
            }
            ItemKind::Extern(functions) => {
                for function in functions {
                    self.function(function);
                }
            }
            ItemKind::Import { .. } => {}
        }
        self.scopes.pop();
    }
    fn function(&mut self, function: &mut Function) {
        self.scopes.push(BTreeSet::new());
        self.generics(&mut function.generics);
        self.parameters(&mut function.parameters);
        self.ty(&mut function.result);
        if let Some(body) = &mut function.body {
            self.block(body);
        }
        self.scopes.pop();
    }
    fn parameters(&mut self, parameters: &mut [Parameter]) {
        for parameter in parameters {
            if let Some(ty) = &mut parameter.ty {
                self.ty(ty);
            }
            self.pattern(&mut parameter.pattern);
        }
    }
    fn ty(&mut self, ty: &mut Type) {
        match &mut ty.kind {
            TypeKind::Named { path, arguments } => {
                if let Some(name) = path.first_mut() {
                    self.name(name);
                }
                self.arguments(arguments);
            }
            TypeKind::Tuple(fields) => {
                for field in fields {
                    self.ty(field);
                }
            }
            TypeKind::Array { element, length } => {
                self.ty(element);
                self.expr(length);
            }
            TypeKind::Pointer { pointee, .. } => self.ty(pointee),
            TypeKind::Function { parameters, result } => {
                for (_, ty) in parameters {
                    self.ty(ty);
                }
                self.ty(result);
            }
        }
    }
    fn arguments(&mut self, arguments: &mut [TypeArgument]) {
        for argument in arguments {
            match argument {
                TypeArgument::Type(ty) => self.ty(ty),
                TypeArgument::Const(value) => self.expr(value),
            }
        }
    }
    fn pattern(&mut self, pattern: &mut Pattern) {
        match &mut pattern.kind {
            PatternKind::Binding { name, .. } => {
                self.scopes.last_mut().unwrap().insert(name.clone());
            }
            PatternKind::Tuple(fields) | PatternKind::Array(fields) => {
                for field in fields {
                    self.pattern(field);
                }
            }
            PatternKind::Variant { path, fields } => {
                if let Some(name) = path.first_mut() {
                    self.name(name);
                }
                match fields {
                    PatternFields::Unit => {}
                    PatternFields::Tuple(fields) => {
                        for field in fields {
                            self.pattern(field);
                        }
                    }
                    PatternFields::Named(fields, _) => {
                        for (_, field) in fields {
                            self.pattern(field);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn block(&mut self, block: &mut Block) {
        self.scopes.push(BTreeSet::new());
        for statement in &mut block.statements {
            match &mut statement.kind {
                StatementKind::Binding {
                    pattern, ty, value, ..
                } => {
                    if let Some(ty) = ty {
                        self.ty(ty);
                    }
                    self.expr(value);
                    self.pattern(pattern);
                }
                StatementKind::Expression(value) => self.expr(value),
                StatementKind::Return(Some(value)) => self.expr(value),
                StatementKind::Unsafe(block) => self.block(block),
                _ => {}
            }
        }
        self.scopes.pop();
    }
    fn expr(&mut self, value: &mut Expr) {
        match &mut value.kind {
            ExprKind::Name(name) => self.name(name),
            ExprKind::Literal(_) => {}
            ExprKind::Group(value)
            | ExprKind::Unary { value, .. }
            | ExprKind::Member { value, .. } => self.expr(value),
            ExprKind::Specialize { value, arguments } => {
                self.expr(value);
                self.arguments(arguments);
            }
            ExprKind::Tuple(values) | ExprKind::Array(values) => {
                for value in values {
                    self.expr(value);
                }
            }
            ExprKind::Construct { ty, fields } => {
                self.ty(ty);
                for (_, value) in fields {
                    self.expr(value);
                }
            }
            ExprKind::New { ty, arguments } => {
                self.ty(ty);
                for value in arguments {
                    self.expr(value);
                }
            }
            ExprKind::Call {
                callee,
                types,
                arguments,
            } => {
                self.expr(callee);
                self.arguments(types);
                for value in arguments {
                    self.expr(value);
                }
            }
            ExprKind::Index { value, index } => {
                self.expr(value);
                self.expr(index);
            }
            ExprKind::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Assign { place, value, .. } => {
                self.expr(place);
                self.expr(value);
            }
            ExprKind::Cast { value, ty } => {
                self.expr(value);
                self.ty(ty);
            }
            ExprKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.expr(condition);
                self.block(then_block);
                if let Some(branch) = else_branch {
                    self.expr(branch);
                }
            }
            ExprKind::While { condition, body } => {
                self.expr(condition);
                self.block(body);
            }
            ExprKind::Branch(block) => self.block(block),
            ExprKind::Match { value, arms } => {
                self.expr(value);
                for arm in arms {
                    self.scopes.push(BTreeSet::new());
                    self.pattern(&mut arm.pattern);
                    if let Some(guard) = &mut arm.guard {
                        self.expr(guard);
                    }
                    self.expr(&mut arm.value);
                    self.scopes.pop();
                }
            }
        }
    }
}
