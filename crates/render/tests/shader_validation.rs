//! Static validation of every WGSL shader (SPEC §8, "Shader static").
//!
//! Runs in plain `cargo test` on any machine — no GPU, no window. Catches parse
//! errors, type errors, missing entry points, and binding mistakes before a
//! human ever launches the app. Sources come from `render::shaders::PASSES`, so
//! a pass cannot be added to the renderer without also being checked here.

use farfall_render::shaders::{compose, COMMON, PASSES};

fn validate(name: &str, src: &str) -> naga::Module {
    let module = naga::front::wgsl::parse_str(src)
        .unwrap_or_else(|e| panic!("{name}: WGSL parse error:\n{}", e.emit_to_string(src)));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: validation error: {e:?}"));
    module
}

#[test]
fn every_pass_compiles_with_the_prelude() {
    assert!(!PASSES.is_empty());
    for (name, src, entries) in PASSES {
        let module = validate(name, &compose(src));
        for entry in *entries {
            assert!(
                module.entry_points.iter().any(|ep| ep.name == *entry),
                "{name}: missing entry point `{entry}`"
            );
        }
    }
}

/// The prelude must stand on its own, so a broken helper is reported once
/// rather than N times with a confusing pass name attached.
#[test]
fn the_prelude_is_self_contained() {
    validate("common", COMMON);
}

/// Passes must not redefine prelude helpers: two divergent copies of a noise
/// function is how two parts of the same world stop agreeing.
#[test]
fn passes_do_not_shadow_prelude_helpers() {
    let helpers = [
        "fn hash31(",
        "fn vnoise(",
        "fn fbm3(",
        "fn fbm5(",
        "fn tonemap(",
    ];
    for (name, src, _) in PASSES {
        for helper in helpers {
            assert!(
                !src.contains(helper),
                "{name} redefines prelude helper `{helper}` — use the shared one"
            );
        }
    }
}
