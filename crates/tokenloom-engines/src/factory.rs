//! Family → interpreter factory (PLAN.md §5.3/§5.4). Maps registry families
//! and engine names onto concrete interpreters.

use crate::families::{
    discourse::DiscourseEngine, gitea::GiteaEngine, huggingface::HfEngine, lemmy::LemmyEngine,
    mastodon::MastodonEngine, mediawiki::MediaWikiEngine, stackexchange::StackExchangeEngine,
    wikicommons::CommonsEngine,
};
use crate::generic::{css_engine::CssEngine, json_engine::JsonEngine};
use crate::spec::EngineSpec;
use crate::specialists::{
    brave::BraveEngine,
    duckduckgo::{DdgExtraEngine, DdgHtmlEngine, DdgIaEngine},
    instant::{CurrencyEngine, LingvaEngine, TranslatedEngine},
    maps::{OsmEngine, PhotonEngine},
    mojeek::MojeekEngine,
    qwant::QwantEngine,
    science::{ArxivEngine, PubmedEngine},
    startpage::StartpageEngine,
};
use crate::trait_def::Engine;

/// Build an engine for a spec, or `None` when the family is registered but
/// has no interpreter yet (status is reported honestly — PLAN.md §15).
pub fn build(spec: &EngineSpec) -> Option<Box<dyn Engine>> {
    let family = spec.family.as_str();
    let engine: Box<dyn Engine> = match family {
        // ── family interpreters ──────────────────────────────────────────
        "mediawiki" | "archlinux" | "gentoo" | "nixos" | "wikipedia" => {
            Box::new(MediaWikiEngine::new(spec.clone()))
        }
        "wikidata" => Box::new(MediaWikiEngine::new(spec.clone())),
        "wikicommons" => Box::new(CommonsEngine::new(spec.clone())),
        "stackexchange" => Box::new(StackExchangeEngine::new(spec.clone())),
        "discourse" => Box::new(DiscourseEngine::new(spec.clone())),
        "gitea" => Box::new(GiteaEngine::new(spec.clone())),
        "lemmy" => Box::new(LemmyEngine::new(spec.clone())),
        "mastodon" => Box::new(MastodonEngine::new(spec.clone())),
        "huggingface" => Box::new(HfEngine::new(spec.clone())),

        // ── DuckDuckGo family ────────────────────────────────────────────
        "duckduckgo" | "duckduckgo_web" => Box::new(DdgHtmlEngine::new(spec.clone())),
        "duckduckgo_definitions" => Box::new(DdgIaEngine::new(spec.clone())),
        "duckduckgo_extra" => Box::new(DdgExtraEngine::new(spec.clone())),

        // ── meta search specialists ──────────────────────────────────────
        "brave" => Box::new(BraveEngine::new(spec.clone())),
        "startpage" => Box::new(StartpageEngine::new(spec.clone())),
        "mojeek" => Box::new(MojeekEngine::new(spec.clone())),
        "qwant" => Box::new(QwantEngine::new(spec.clone())),

        // ── science / maps / instant answers ─────────────────────────────
        "arxiv" => Box::new(ArxivEngine::new(spec.clone())),
        "pubmed" => Box::new(PubmedEngine::new(spec.clone())),
        "openstreetmap" => Box::new(OsmEngine::new(spec.clone())),
        "photon" => Box::new(PhotonEngine::new(spec.clone())),
        "lingva" => Box::new(LingvaEngine::new(spec.clone())),
        "translated" => Box::new(TranslatedEngine::new(spec.clone())),
        "currency_convert" => Box::new(CurrencyEngine::new(spec.clone())),

        // ── declarative interpreters ─────────────────────────────────────
        "json_engine" => Box::new(JsonEngine::new(spec.clone())?),
        "xpath" | "css_engine" => {
            let has_item = spec
                .response
                .as_ref()
                .and_then(|r| r.item.as_deref())
                .is_some();
            if has_item {
                Box::new(CssEngine::new(spec.clone())?)
            } else if spec.request.is_some() {
                Box::new(JsonEngine::new(spec.clone())?)
            } else {
                return None;
            }
        }

        // ── every other family: declarative when a spec exists ──────────
        // This is what wires the stable JSON-API engines (crates, github,
        // npm, docker_hub, openalex, crossref, semantic_scholar, openlibrary,
        // hex, gitlab, hackernews, nvd, dailymotion, deezer, mixcloud,
        // radio_browser, openverse, unsplash, tagesschau, …) whose request/
        // response specs ship in `builtin_specs`.
        _ => {
            let has_item = spec
                .response
                .as_ref()
                .and_then(|r| r.item.as_deref())
                .is_some();
            if spec.request.is_some() && spec.response.is_some() {
                if has_item {
                    Box::new(CssEngine::new(spec.clone())?)
                } else {
                    Box::new(JsonEngine::new(spec.clone())?)
                }
            } else {
                // Registered but not yet implemented (honest status).
                return None;
            }
        }
    };
    Some(engine)
}
