use super::*;

#[test]
fn generic_param_resolves_in_fn_signature() {
    let resolved = resolve_ok(
        r#"
trait Sound {
    sound() String {
        "..."
    }
}
fn f<T: Sound>(x T) {
    println(x);
}
"#,
    );
    let sound_id = resolved.definitions.lookup(&"Sound".into()).unwrap();
    assert_eq!(resolved.definitions.get(sound_id).kind, DefKind::Trait);
}

#[test]
fn explicit_impl_generics_bind_a_name_for_a_declaration_less_owner() {
    // The explicit `impl<T> Owner<T> { ... }` form exists specifically
    // for an owner with no local declaration to read a parameter name
    // from — `Option`/`Result` used to be the only real example (compiler
    // builtins with no `TypeDecl`/`EnumDecl`), but they're ordinary
    // prelude `enum`s now (`stdlib/option.nr`/`result.nt`), so nothing
    // reachable from a single self-contained module is truly
    // declaration-less anymore. This keeps the explicit-form mechanism
    // itself covered with a local stand-in.
    resolve_ok(
        r#"
enum MyOption<T> {
    MySome(T),
    MyNone,
}
impl<T> MyOption<T> {
    unwrap_or(self, fallback: T): T {
        match self {
            MySome(item) => item,
            MyNone => fallback,
        }
    }
}
"#,
    );
}

/// Builds a merged multi-file `Module` the way the driver would: `real.nr`
/// declares `helper`, `prelude.nr` re-exports it with `use real.helper;`,
/// and `consumer.nr` is the file under test. Returns the merged module and
/// `prelude.nr`'s own `FileId`, ready for `resolve_with_prelude`.
fn parse_prelude_fixture(consumer_source: &str) -> (Module, nether_diagnostics::FileId) {
    let mut map = SourceMap::new();

    let real_source = "pub fn helper(): i32 { 1 }\n";
    let real_file = map.add_file("real.nr", real_source);
    let (real_module, real_diags) = nether_parser::parse_module(real_source, real_file);
    assert!(real_diags.is_empty(), "{real_diags:?}");

    let prelude_source = "pub use real.helper;\n";
    let prelude_file = map.add_file("prelude.nr", prelude_source);
    let (prelude_module, prelude_diags) = nether_parser::parse_module(prelude_source, prelude_file);
    assert!(prelude_diags.is_empty(), "{prelude_diags:?}");
    let Item::Use(use_decl) = &prelude_module.items[0] else {
        panic!("expected a Use item")
    };
    let use_id = use_decl.id;

    let consumer_file = map.add_file("consumer.nr", consumer_source);
    let (consumer_module, consumer_diags) =
        nether_parser::parse_module(consumer_source, consumer_file);
    assert!(consumer_diags.is_empty(), "{consumer_diags:?}");

    let mut items = real_module.items;
    items.extend(prelude_module.items);
    items.extend(consumer_module.items);
    let mut imports = std::collections::HashMap::new();
    imports.insert(use_id, real_file);
    (
        Module {
            file: consumer_file,
            items,
            imports,
            variant_imports: std::collections::HashMap::new(),
        },
        prelude_file,
    )
}

#[test]
fn prelude_file_reexports_are_visible_everywhere_with_no_use() {
    let (module, prelude_file) = parse_prelude_fixture("fn use_it(): i32 { helper() }\n");
    let (_resolved, diags) = nether_resolver::resolve_with_prelude(&module, Some(prelude_file));
    assert!(
        diags.is_empty(),
        "unexpected diagnostics: {}",
        messages(&diags)
    );
}

#[test]
fn prelude_names_are_invisible_without_an_explicit_prelude_file() {
    // The same fixture resolved through plain `resolve` (as if the driver
    // had found no bundled `stdlib/mod.nr`) must NOT see `helper` — proves
    // the visibility genuinely comes from the prelude promotion, not from
    // `helper` being reachable some other way.
    let (module, _prelude_file) = parse_prelude_fixture("fn use_it(): i32 { helper() }\n");
    let (_resolved, diags) = resolve(&module);
    assert!(!diags.is_empty());
}

/// Builds a merged multi-file `Module` the way the driver would for a
/// `use module.Enum.Variant;` path — `enums.nr` declares `enum Color {
/// Red, Green }`, `prelude.nr` re-exports `Red` with `use enums.Color.Red;`
/// (recorded in `variant_imports`, not `imports` — mirrors what
/// `nether_driver`'s use-loop does after its longer-prefix attempt fails),
/// and `consumer.nr` is the file under test.
fn parse_variant_prelude_fixture(consumer_source: &str) -> (Module, nether_diagnostics::FileId) {
    let mut map = SourceMap::new();

    let enums_source = "pub enum Color { Red, Green }\n";
    let enums_file = map.add_file("enums.nr", enums_source);
    let (enums_module, enums_diags) = nether_parser::parse_module(enums_source, enums_file);
    assert!(enums_diags.is_empty(), "{enums_diags:?}");

    let prelude_source = "pub use enums.Color.Red;\n";
    let prelude_file = map.add_file("prelude.nr", prelude_source);
    let (prelude_module, prelude_diags) = nether_parser::parse_module(prelude_source, prelude_file);
    assert!(prelude_diags.is_empty(), "{prelude_diags:?}");
    let Item::Use(use_decl) = &prelude_module.items[0] else {
        panic!("expected a Use item")
    };
    let use_id = use_decl.id;

    let consumer_file = map.add_file("consumer.nr", consumer_source);
    let (consumer_module, consumer_diags) =
        nether_parser::parse_module(consumer_source, consumer_file);
    assert!(consumer_diags.is_empty(), "{consumer_diags:?}");

    let mut items = enums_module.items;
    items.extend(prelude_module.items);
    items.extend(consumer_module.items);
    let mut variant_imports = std::collections::HashMap::new();
    variant_imports.insert(use_id, enums_file);
    (
        Module {
            file: consumer_file,
            items,
            imports: std::collections::HashMap::new(),
            variant_imports,
        },
        prelude_file,
    )
}

#[test]
fn bare_variant_name_from_use_module_enum_variant_resolves_to_enum_variant() {
    let (module, prelude_file) = parse_variant_prelude_fixture("fn use_it() { Red }\n");
    let (resolved, diags) = nether_resolver::resolve_with_prelude(&module, Some(prelude_file));
    assert!(
        diags.is_empty(),
        "unexpected diagnostics: {}",
        messages(&diags)
    );
    let use_it = module
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(f) if f.name.name.as_str() == "use_it" => Some(f),
            _ => None,
        })
        .expect("expected a fn named use_it");
    let tail = use_it
        .body
        .as_ref()
        .and_then(|b| b.tail.as_ref())
        .expect("expected a tail expression");
    let path = first_path(tail);
    let res = resolved
        .path_res
        .get(&path.id)
        .expect("expected a resolution for the bare `Red`");
    assert!(
        matches!(res.base, Resolution::EnumVariant(_, 0)),
        "expected EnumVariant(_, 0) for `Red`, got {:?}",
        res.base
    );
}

#[test]
fn a_local_declaration_shadows_a_prelude_reexport_without_conflict() {
    let (module, prelude_file) =
        parse_prelude_fixture("fn helper(): i32 { 2 }\nfn use_it(): i32 { helper() }\n");
    let (_resolved, diags) = nether_resolver::resolve_with_prelude(&module, Some(prelude_file));
    assert!(
        diags.is_empty(),
        "unexpected diagnostics: {}",
        messages(&diags)
    );
}

#[test]
fn implicit_impl_form_on_a_builtin_owner_leaves_its_type_unbound() {
    // Without the explicit `<T>` form, `Option` has no declaration to read
    // a parameter name from, so `T` is an ordinary unresolved name.
    let (_, _, diags) = resolve_with_diagnostics(
        r#"
impl Option {
    unwrap_or(self, fallback: T): T {
        fallback
    }
}
"#,
    );
    assert!(!diags.is_empty());
}

#[test]
fn builtins_are_available_without_use() {
    // `Option`/`Result`/`Array` are deliberately not in this list — they're
    // ordinary `enum`/`type` declarations in the bundled prelude
    // (`stdlib/option.nr`/`result.nt`/`array.nt`), not builtins;
    // `resolve_ok` resolves a bare `Module` directly with no driver and no
    // prelude loading, so they are genuinely unavailable here (see
    // `prelude_file_reexports_are_visible_everywhere_with_no_use` for
    // prelude-specific coverage instead).
    let resolved = resolve_ok("fn main() { println(\"hi\"); }");
    let println_id = resolved
        .definitions
        .lookup(&"println".into())
        .expect("println should be builtin");
    assert_eq!(resolved.definitions.get(println_id).kind, DefKind::Fn);
    for name in ["i32", "bool", "String"] {
        assert!(
            resolved.definitions.lookup(&name.into()).is_some(),
            "missing builtin {name}"
        );
    }
}
