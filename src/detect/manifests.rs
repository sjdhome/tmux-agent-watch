//! Bundled agent-detection manifests, vendored verbatim from herdr's
//! `src/detect/manifests/` (Apache-2.0; see NOTICE). Refresh with
//! `scripts/refresh-manifests.sh`; `cargo test` is the drift gate.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::engine::{self, CompiledManifest};

pub const BUNDLED: &[(&str, &str)] = &[
    ("amp", include_str!("manifests/amp.toml")),
    ("antigravity", include_str!("manifests/antigravity.toml")),
    ("claude", include_str!("manifests/claude.toml")),
    ("cline", include_str!("manifests/cline.toml")),
    ("codex", include_str!("manifests/codex.toml")),
    ("cursor", include_str!("manifests/cursor.toml")),
    ("devin", include_str!("manifests/devin.toml")),
    ("droid", include_str!("manifests/droid.toml")),
    ("gemini", include_str!("manifests/gemini.toml")),
    (
        "github-copilot",
        include_str!("manifests/github-copilot.toml"),
    ),
    ("grok", include_str!("manifests/grok.toml")),
    ("hermes", include_str!("manifests/hermes.toml")),
    ("kilo", include_str!("manifests/kilo.toml")),
    ("kimi", include_str!("manifests/kimi.toml")),
    ("kiro", include_str!("manifests/kiro.toml")),
    ("maki", include_str!("manifests/maki.toml")),
    ("opencode", include_str!("manifests/opencode.toml")),
    ("pi", include_str!("manifests/pi.toml")),
    ("qodercli", include_str!("manifests/qodercli.toml")),
];

/// Compiled manifest for a manifest id (the TOML `id` field, which is the
/// canonical agent label — e.g. "agy", "copilot"). `None` when the manifest
/// failed to parse/compile at startup; the agent then falls back to
/// known-agent-Idle.
pub fn get(manifest_id: &str) -> Option<&'static CompiledManifest> {
    compiled().get(manifest_id)
}

fn compiled() -> &'static HashMap<String, CompiledManifest> {
    static COMPILED: OnceLock<HashMap<String, CompiledManifest>> = OnceLock::new();
    COMPILED.get_or_init(|| {
        let mut map = HashMap::new();
        for (name, text) in BUNDLED {
            match engine::parse_manifest(text).and_then(|manifest| engine::compile(&manifest)) {
                Ok(compiled) => {
                    map.insert(compiled.id.clone(), compiled);
                }
                Err(err) => {
                    eprintln!("warning: bundled manifest {name} failed to load: {err}");
                }
            }
        }
        map
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn all_bundled_manifests_parse_and_compile() {
        assert_eq!(BUNDLED.len(), 19);
        for (name, text) in BUNDLED {
            let manifest = engine::parse_manifest(text)
                .unwrap_or_else(|err| panic!("manifest {name} failed to parse: {err}"));
            let agent = crate::detect::parse_agent_label(&manifest.id).unwrap_or_else(|| {
                panic!("manifest {name} id {:?} is not a known agent", manifest.id)
            });
            assert_eq!(
                crate::detect::manifest_id(agent),
                Some(manifest.id.as_str()),
                "manifest {name}: id must be the agent's canonical label"
            );
            engine::compile(&manifest)
                .unwrap_or_else(|err| panic!("manifest {name} failed to compile: {err}"));
        }
        assert_eq!(compiled().len(), 19);
    }

    #[test]
    fn every_used_region_is_implemented() {
        for (name, text) in BUNDLED {
            let manifest = engine::parse_manifest(text).expect("parses");
            for region in manifest.rule_regions() {
                assert!(
                    engine::region_is_supported(region),
                    "manifest {name} uses unimplemented region {region:?}"
                );
            }
        }
    }
}
