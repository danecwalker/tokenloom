//! Engine registry: loads the 248-engine `engines.toml` master registry at
//! compile time, merges built-in declarative specs, and constructs engines
//! via the family factory (PLAN.md §5, §Appendix A).

use crate::builtin_specs::builtin_fragments;
use crate::spec::{apply_fragment, EngineSpec};
use crate::trait_def::Engine;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use tokenloom_core::{url_util, Category, TokenloomError};

/// The engines.toml file shape.
#[derive(Debug, Deserialize)]
struct EnginesFile {
    #[serde(default)]
    #[allow(dead_code)]
    schema_version: u8,
    engines: Vec<EngineSpec>,
}

const ENGINES_TOML: &str = include_str!("../../../engines.toml");

/// Families that only answer specialized query syntaxes; they never join a
/// default category search (bang-only, mirroring SearXNG behavior).
const EXCLUSIVE_INSTANT_FAMILIES: &[&str] = &[
    "currency_convert",
    "translated",
    "lingva",
    "dictzone",
    "tineye",
    "mozhi",
];

pub struct Registry {
    specs: Vec<EngineSpec>,
    by_name: HashMap<String, usize>,
    by_bang: HashMap<String, usize>,
    implemented: HashSet<String>,
    fragments: HashMap<String, crate::spec::SpecFragment>,
}

impl Registry {
    /// Load + validate the master registry (248 engines expected).
    pub fn load() -> Result<Self, TokenloomError> {
        let file: EnginesFile = toml::from_str(ENGINES_TOML)
            .map_err(|e| TokenloomError::Config(format!("engines.toml invalid: {e}")))?;
        Self::from_specs(file.engines)
    }

    /// Registry from an explicit spec list (used by tests and by user-config
    /// additions).
    pub fn from_specs(specs: Vec<EngineSpec>) -> Result<Self, TokenloomError> {
        let mut by_name = HashMap::new();
        let mut by_bang = HashMap::new();
        let mut implemented = HashSet::new();
        let fragments = builtin_fragments();

        let mut specs = specs;
        for (idx, spec) in specs.iter_mut().enumerate() {
            if let Some(frag) = fragments.get(&spec.name) {
                apply_fragment(spec, frag);
            }
            if by_name.insert(spec.name.clone(), idx).is_some() {
                return Err(TokenloomError::Config(format!(
                    "duplicate engine name '{}'",
                    spec.name
                )));
            }
            if by_bang.insert(spec.bang.clone(), idx).is_some() {
                return Err(TokenloomError::Config(format!(
                    "duplicate bang '!{}'",
                    spec.bang
                )));
            }
            if crate::factory::build(spec).is_some() {
                implemented.insert(spec.name.clone());
            }
        }

        Ok(Self {
            specs,
            by_name,
            by_bang,
            implemented,
            fragments,
        })
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    pub fn specs(&self) -> &[EngineSpec] {
        &self.specs
    }

    pub fn get(&self, name: &str) -> Option<&EngineSpec> {
        self.by_name.get(name).map(|&i| &self.specs[i])
    }

    pub fn get_by_bang(&self, bang: &str) -> Option<&EngineSpec> {
        self.by_bang.get(bang).map(|&i| &self.specs[i])
    }

    pub fn is_implemented(&self, name: &str) -> bool {
        self.implemented.contains(name)
    }

    /// Construct an engine instance (None when the family has no interpreter).
    pub fn build(&self, name: &str) -> Option<Box<dyn Engine>> {
        let spec = self.get(name)?;
        crate::factory::build(spec)
    }

    /// All implemented engines matching a category, sorted by weight (desc).
    pub fn engines_for_category(
        &self,
        category: Category,
        include_disabled: bool,
    ) -> Vec<&EngineSpec> {
        let mut list: Vec<&EngineSpec> = self
            .specs
            .iter()
            .filter(|s| {
                s.categories.contains(&category)
                    && self.implemented.contains(&s.name)
                    && (include_disabled || s.enabled)
                    // Instant-answer engines only make sense for matching
                    // queries; they fire via bang or explicit engine name.
                    && !EXCLUSIVE_INSTANT_FAMILIES.contains(&s.family.as_str())
            })
            .collect();
        list.sort_by(|a, b| {
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
        });
        list
    }

    /// Resolve parsed bangs into engine names + a category
    /// (PLAN.md §5 *Bangs & Category Routing*).
    pub fn resolve_bangs(&self, parsed: &url_util::BangParse) -> url_util::BangResolution {
        let mut res = url_util::BangResolution::default();
        for bang in &parsed.bangs {
            if let Some(cat) = url_util::category_from_bang(bang) {
                res.category = Some(cat);
            } else if let Some(spec) = self.get_by_bang(bang) {
                res.engines.push(spec.name.clone());
                // An engine bang implies its primary category when no
                // category bang was given.
                if res.category.is_none() {
                    res.category = spec.categories.first().copied();
                }
            }
            // Unknown bangs are ignored (SearXNG semantics: fall through).
        }
        res
    }

    /// Iterate (name, bang) pairs for `tokenloom bangs`.
    pub fn bangs(&self) -> impl Iterator<Item = (&str, &str, bool)> {
        self.specs.iter().map(|s| {
            (
                s.name.as_str(),
                s.bang.as_str(),
                self.implemented.contains(&s.name),
            )
        })
    }

    /// Merge user-config engine additions (custom [[engines]] entries) and
    /// spec fragments (overrides for request/response wiring).
    pub fn with_user_spec_fragments(
        mut self,
        fragments: HashMap<String, crate::spec::SpecFragment>,
    ) -> Self {
        for (name, frag) in fragments {
            if let Some(&idx) = self.by_name.get(&name) {
                apply_fragment(&mut self.specs[idx], &frag);
                // Re-evaluate implemented status for overridden engines.
                if crate::factory::build(&self.specs[idx]).is_some() {
                    self.implemented.insert(name.clone());
                }
            }
            self.fragments.insert(name, frag);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_248_engines() {
        let reg = Registry::load().unwrap();
        assert_eq!(reg.len(), 248, "expected exactly 248 engines");
    }

    #[test]
    fn bang_lookup_and_resolution() {
        let reg = Registry::load().unwrap();
        assert_eq!(reg.get_by_bang("ddg").unwrap().name, "duckduckgo");
        assert_eq!(reg.get_by_bang("arx").unwrap().name, "arxiv");

        let parsed = url_util::parse_bangs("!ddg !news ukraine");
        let res = reg.resolve_bangs(&parsed);
        assert_eq!(res.engines, vec!["duckduckgo"]);
        assert_eq!(res.category, Some(Category::News));

        let parsed = url_util::parse_bangs("!science crispr");
        let res = reg.resolve_bangs(&parsed);
        assert!(res.engines.is_empty());
        assert_eq!(res.category, Some(Category::Science));
    }

    #[test]
    fn core_engines_are_implemented() {
        let reg = Registry::load().unwrap();
        for name in [
            "duckduckgo",
            "wikipedia",
            "arxiv",
            "crates.io",
            "github",
            "stackoverflow",
            "pypi",
            "mdn",
            "hackernews",
            "lemmy_posts",
            "mankier",
            "openstreetmap",
            "currency",
            "photon",
            "huggingface",
            "codeberg",
            "caddy.community",
            "national_vulnerability_database",
            "openalex",
            "docker_hub",
        ] {
            assert!(reg.is_implemented(name), "{name} should be implemented");
        }
        for name in ["google", "bing", "baidu", "yandex", "piratebay", "1337x"] {
            assert!(
                !reg.is_implemented(name),
                "{name} is wave-3/adversarial and should NOT be implemented yet"
            );
        }
    }

    #[test]
    fn implemented_count_is_substantial() {
        let reg = Registry::load().unwrap();
        let implemented = reg
            .specs()
            .iter()
            .filter(|s| reg.is_implemented(&s.name))
            .count();
        assert!(
            implemented >= 85,
            "expected 85+ implemented engines, got {implemented}"
        );
    }

    #[test]
    fn default_general_search_set_is_reasonable() {
        let reg = Registry::load().unwrap();
        let engines = reg.engines_for_category(Category::General, false);
        let names: Vec<&str> = engines.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"duckduckgo"), "{names:?}");
        assert!(names.contains(&"wikipedia"), "{names:?}");
        assert!(names.len() >= 5 && names.len() <= 30, "{names:?}");
    }
}
