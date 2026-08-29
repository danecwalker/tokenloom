//! Built-in declarative request/response specs (PLAN.md §5, *Declarative
//! Engines*). Merged into the registry at load time; engines.toml entries
//! carry the metadata, this module carries the extraction wiring for the
//! stable JSON/CSS engines shipped out of the box. Users can override any of
//! it via `[[engines]]` entries in their own config.

/// TOML fragments keyed by engine name.
pub const BUILTIN_SPECS: &str = r#"
[mdn]
[mdn.request]
url = "https://developer.mozilla.org/api/v1/search"
[mdn.request.params]
q = "{query}"
locale = "en-US"
[mdn.request.headers]
Accept = "application/json"
[mdn.response]
results_path = "documents"
[mdn.response.title]
path = "title"
[mdn.response.url]
path = "mdn_url"
prefix = "https://developer.mozilla.org"
[mdn.response.snippet]
path = "summary"

[packagist]
[packagist.request]
url = "https://packagist.org/search.json"
[packagist.request.params]
q = "{query}"
per_page = "15"
[packagist.response]
results_path = "results"
[packagist.response.title]
path = "name"
[packagist.response.url]
path = "url"
[packagist.response.snippet]
path = "description"

[wiby]
[wiby.request]
url = "https://wiby.me/json/"
[wiby.request.params]
q = "{query}"
[wiby.response.title]
path = "Title"
[wiby.response.url]
path = "URL"
[wiby.response.snippet]
path = "Snippet"

[crowdview]
[crowdview.request]
url = "https://crowdview.nextjson.com/search"
[crowdview.request.params]
q = "{query}"
[crowdview.response]
results_path = "results"
[crowdview.response.title]
path = "title"
[crowdview.response.url]
path = "url"
[crowdview.response.snippet]
path = "content"

[encyclosearch]
[encyclosearch.request]
url = "https://encyclosearch.org/search"
[encyclosearch.request.params]
q = "{query}"
[encyclosearch.response]
results_path = "Results"
[encyclosearch.response.title]
path = "Title"
[encyclosearch.response.url]
path = "Url"
[encyclosearch.response.snippet]
path = "Description"

[mankier]
[mankier.request]
url = "https://www.mankier.com/api/v2/q/"
[mankier.request.params]
q = "{query}"
[mankier.response]
results_path = "results"
[mankier.response.title]
path = "name"
[mankier.response.url]
path = "url"
[mankier.response.snippet]
path = "description"

["crates.io"]
["crates.io".request]
url = "https://crates.io/api/v1/crates"
["crates.io".request.params]
q = "{query}"
per_page = "15"
["crates.io".request.headers]
Accept = "application/json"
["crates.io".response]
results_path = "crates"
["crates.io".response.title]
path = "name"
["crates.io".response.url]
path = "name"
prefix = "https://crates.io/crates/"
["crates.io".response.snippet]
path = "description"
["crates.io".response.date]
path = "updated_at"
["crates.io".response.metadata.downloads]
path = "downloads"

[github]
[github.request]
url = "https://api.github.com/search/repositories"
[github.request.params]
q = "{query}"
per_page = "15"
[github.request.headers]
Accept = "application/vnd.github+json"
[github.response]
results_path = "items"
[github.response.title]
path = "full_name"
[github.response.url]
path = "html_url"
[github.response.snippet]
path = "description"
[github.response.metadata.stars]
path = "stargazers_count"
[github.response.metadata.language]
path = "language"

[npm]
[npm.request]
url = "https://registry.npmjs.org/-/v1/search"
[npm.request.params]
text = "{query}"
size = "15"
[npm.response]
results_path = "objects"
[npm.response.title]
path = "package.name"
[npm.response.url]
path = "package.name"
prefix = "https://www.npmjs.com/package/"
[npm.response.snippet]
path = "package.description"
[npm.response.metadata.version]
path = "package.version"

[docker_hub]
[docker_hub.request]
url = "https://hub.docker.com/v2/search/repositories/"
[docker_hub.request.params]
query = "{query}"
page_size = "15"
[docker_hub.response]
results_path = "results"
[docker_hub.response.title]
path = "repo_name"
[docker_hub.response.url]
path = "repo_name"
prefix = "https://hub.docker.com/r/"
[docker_hub.response.snippet]
path = "short_description"
[docker_hub.response.metadata.stars]
path = "star_count"

[openalex]
[openalex.request]
url = "https://api.openalex.org/works"
[openalex.request.params]
search = "{query}"
per-page = "15"
[openalex.response]
results_path = "results"
[openalex.response.title]
path = "display_name"
[openalex.response.url]
path = "doi"
fallback_path = "id"
[openalex.response.metadata.year]
path = "publication_year"
[openalex.response.metadata.cited_by]
path = "cited_by_count"

[crossref]
[crossref.request]
url = "https://api.crossref.org/works"
[crossref.request.params]
query = "{query}"
rows = "15"
[crossref.response]
results_path = "message.items"
[crossref.response.title]
path = "title.0"
[crossref.response.url]
path = "DOI"
prefix = "https://doi.org/"
[crossref.response.snippet]
path = "abstract"
strip_html = true
[crossref.response.date]
path = "created.date-time"

[semantic_scholar]
[semantic_scholar.request]
url = "https://api.semanticscholar.org/graph/v1/paper/search"
[semantic_scholar.request.params]
query = "{query}"
limit = "15"
[semantic_scholar.request.headers]
Accept = "application/json"
[semantic_scholar.response]
results_path = "data"
[semantic_scholar.response.title]
path = "title"
[semantic_scholar.response.url]
path = "url"
[semantic_scholar.response.snippet]
path = "abstract"
[semantic_scholar.response.date]
path = "year"

[openlibrary]
[openlibrary.request]
url = "https://openlibrary.org/search.json"
[openlibrary.request.params]
q = "{query}"
limit = "15"
[openlibrary.response]
results_path = "docs"
[openlibrary.response.title]
path = "title"
[openlibrary.response.url]
path = "key"
prefix = "https://openlibrary.org"
[openlibrary.response.metadata.author]
path = "author_name.0"
[openlibrary.response.metadata.year]
path = "first_publish_year"

[hex]
[hex.request]
url = "https://hex.pm/api/packages"
[hex.request.params]
search = "{query}"
[hex.response.title]
path = "name"
[hex.response.url]
path = "name"
prefix = "https://hex.pm/packages/"
[hex.response.snippet]
path = "meta.description"
[hex.response.date]
path = "updated_at"

[gitlab]
[gitlab.request]
url = "https://gitlab.com/api/v4/projects"
[gitlab.request.params]
search = "{query}"
per_page = "15"
[gitlab.response.title]
path = "name_with_namespace"
[gitlab.response.url]
path = "web_url"
[gitlab.response.snippet]
path = "description"

[hackernews]
[hackernews.request]
url = "https://hn.algolia.com/api/v1/search"
[hackernews.request.params]
query = "{query}"
tags = "story"
hitsPerPage = "15"
[hackernews.response]
results_path = "hits"
[hackernews.response.title]
path = "title"
fallback_path = "story_title"
[hackernews.response.url]
path = "objectID"
prefix = "https://news.ycombinator.com/item?id="
[hackernews.response.date]
path = "created_at"
[hackernews.response.metadata.points]
path = "points"
[hackernews.response.metadata.comments]
path = "num_comments"

[national_vulnerability_database]
["national_vulnerability_database".request]
url = "https://services.nvd.nist.gov/rest/json/cves/2.0"
["national_vulnerability_database".request.params]
keywordSearch = "{query}"
resultsPerPage = "15"
["national_vulnerability_database".response]
results_path = "vulnerabilities"
["national_vulnerability_database".response.title]
path = "cve.id"
["national_vulnerability_database".response.url]
path = "cve.id"
prefix = "https://nvd.nist.gov/vuln/detail/"
["national_vulnerability_database".response.snippet]
path = "cve.descriptions.0.value"

[tagesschau]
[tagesschau.request]
url = "https://www.tagesschau.de/api2u/search/"
[tagesschau.request.params]
searchtext = "{query}"
[tagesschau.response]
results_path = "search"
[tagesschau.response.title]
path = "title"
[tagesschau.response.url]
path = "shareURL"
[tagesschau.response.date]
path = "date"

[dailymotion]
[dailymotion.request]
url = "https://api.dailymotion.com/videos"
[dailymotion.request.params]
search = "{query}"
limit = "15"
[dailymotion.response]
results_path = "list"
[dailymotion.response.title]
path = "title"
[dailymotion.response.url]
path = "id"
prefix = "https://www.dailymotion.com/video/"
[dailymotion.response.thumbnail]
path = "thumbnail_240_url"

[deezer]
[deezer.request]
url = "https://api.deezer.com/search"
[deezer.request.params]
q = "{query}"
limit = "15"
[deezer.response]
results_path = "data"
[deezer.response.title]
path = "title"
[deezer.response.url]
path = "link"
[deezer.response.thumbnail]
path = "album.cover_medium"
[deezer.response.metadata.artist]
path = "artist.name"

[mixcloud]
[mixcloud.request]
url = "https://api.mixcloud.com/search/"
[mixcloud.request.params]
q = "{query}"
type = "cloudcast"
limit = "15"
[mixcloud.response]
results_path = "data"
[mixcloud.response.title]
path = "name"
[mixcloud.response.url]
path = "url"
[mixcloud.response.thumbnail]
path = "pictures.medium"
[mixcloud.response.metadata.user]
path = "user.username"

[radio_browser]
[radio_browser.request]
url = "https://de1.api.radio-browser.info/json/stations/search"
[radio_browser.request.params]
name = "{query}"
limit = "15"
hidebroken = "true"
[radio_browser.response.title]
path = "name"
[radio_browser.response.url]
path = "url_resolved"
[radio_browser.response.thumbnail]
path = "favicon"
[radio_browser.response.metadata.country]
path = "country"
[radio_browser.response.metadata.codec]
path = "codec"

[openverse]
[openverse.request]
url = "https://api.openverse.org/v1/images/"
[openverse.request.params]
q = "{query}"
page_size = "15"
[openverse.response]
results_path = "results"
[openverse.response.title]
path = "title"
[openverse.response.url]
path = "foreign_landing_url"
[openverse.response.thumbnail]
path = "thumbnail"
[openverse.response.metadata.creator]
path = "creator"
[openverse.response.metadata.license]
path = "license"

[unsplash]
[unsplash.request]
url = "https://unsplash.com/napi/search/photos"
[unsplash.request.params]
query = "{query}"
per_page = "15"
[unsplash.response]
results_path = "results"
[unsplash.response.title]
path = "alt_description"
[unsplash.response.url]
path = "links.html"
[unsplash.response.thumbnail]
path = "urls.small"
[unsplash.response.metadata.user]
path = "user.username"

[pypi]
[pypi.request]
url = "https://pypi.org/search/"
[pypi.request.params]
q = "{query}"
page = "{page1}"
[pypi.response]
item = "a.package-snippet"
[pypi.response.title]
path = "span.package-snippet__name"
[pypi.response.url]
path = "@href"
[pypi.response.snippet]
path = "p.package-snippet__description"
[pypi.response.metadata.version]
path = "span.package-snippet__version"

["pub.dev"]
["pub.dev".request]
url = "https://pub.dev/packages"
["pub.dev".request.params]
q = "{query}"
["pub.dev".response]
item = ".packages-item"
["pub.dev".response.title]
path = ".packages-title"
["pub.dev".response.url]
path = "a@href"
["pub.dev".response.snippet]
path = ".packages-description"

[rubygems]
[rubygems.request]
url = "https://rubygems.org/search"
[rubygems.request.params]
query = "{query}"
[rubygems.response]
item = "a.gems__gem"
[rubygems.response.title]
path = "h2"
[rubygems.response.url]
path = "@href"
[rubygems.response.snippet]
path = "p.gems__gem__desc__t"

["lobste.rs"]
["lobste.rs".request]
url = "https://lobste.rs/search"
["lobste.rs".request.params]
q = "{query}"
what = "stories"
order = "newest"
["lobste.rs".response]
item = "li.story"
["lobste.rs".response.title]
path = "span.link a"
["lobste.rs".response.url]
path = "span.link a@href"
["lobste.rs".response.metadata.comments]
path = "span.comments_label"

[hoogle]
[hoogle.request]
url = "https://hoogle.haskell.org/"
[hoogle.request.params]
hoogle = "{query}"
mode = "json"
[hoogle.response]
[hoogle.response.title]
path = "name"
[hoogle.response.url]
path = "url"

[searchmysite]
[searchmysite.request]
url = "https://searchmysite.net/search/"
[searchmysite.request.params]
q = "{query}"
[searchmysite.response]
item = "div.search-result"
[searchmysite.response.title]
path = "h3 a"
[searchmysite.response.url]
path = "h3 a@href"
[searchmysite.response.snippet]
path = ".result-summary"
"#;

/// Parse the built-in fragments (panics only on programmer error — validated
/// by unit test).
pub fn builtin_fragments() -> std::collections::HashMap<String, crate::spec::SpecFragment> {
    toml::from_str(BUILTIN_SPECS).expect("builtin engine specs are valid TOML")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_specs_parse() {
        let frags = builtin_fragments();
        assert!(frags.contains_key("mdn"));
        assert!(frags.contains_key("pypi"));
        assert!(frags.contains_key("hackernews"));
        assert!(
            frags.len() >= 30,
            "expected 30+ builtin fragments, got {}",
            frags.len()
        );
    }
}
