//! Map specialists: OpenStreetMap (Nominatim) and Photon.

use crate::http_util::{json_get, json_get_with_params};
use crate::spec::EngineSpec;
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{Category, SearchQuery, SearchResult};

pub struct OsmEngine {
    spec: EngineSpec,
}

impl OsmEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self { spec }
    }
}

#[async_trait]
impl Engine for OsmEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let json = json_get_with_params(
            http,
            "https://nominatim.openstreetmap.org/search",
            &[
                ("q", query.clean_query.clone()),
                ("format", "jsonv2".into()),
                ("limit", "10".into()),
            ],
            self.timeout(),
            &[],
        )
        .await?;

        let mut out = Vec::new();
        let Some(items) = json.as_array() else {
            return Ok(out);
        };
        for item in items {
            let Some(display) = item.get("display_name").and_then(Value::as_str) else {
                continue;
            };
            let osm_type = item
                .get("osm_type")
                .and_then(Value::as_str)
                .unwrap_or("node");
            let osm_id = item.get("osm_id").and_then(Value::as_i64).unwrap_or(0);
            if osm_id == 0 {
                continue;
            }
            let url = format!("https://www.openstreetmap.org/{osm_type}/{osm_id}");
            let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
            let lat = item.get("lat").and_then(Value::as_str).unwrap_or("0");
            let lon = item.get("lon").and_then(Value::as_str).unwrap_or("0");
            let mut r = result(
                &self.spec.name,
                Category::Map,
                display,
                url,
                format!("({lat}, {lon})"),
            );
            r.metadata.insert("osm_type".into(), osm_type.into());
            if !kind.is_empty() {
                r.metadata.insert("type".into(), kind.into());
            }
            out.push(r);
        }
        Ok(out)
    }
}

pub struct PhotonEngine {
    spec: EngineSpec,
}

impl PhotonEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self { spec }
    }
}

#[async_trait]
impl Engine for PhotonEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let url = format!(
            "https://photon.komoot.io/api/?q={}&limit=10",
            crate::spec::urlencoding_lite(&query.clean_query)
        );
        let json = json_get(http, &url, self.timeout(), &[]).await?;

        let mut out = Vec::new();
        let Some(features) = json.get("features").and_then(Value::as_array) else {
            return Ok(out);
        };
        for feature in features {
            let Some(props) = feature.get("properties").and_then(Value::as_object) else {
                continue;
            };
            let Some(name) = props.get("name").and_then(Value::as_str) else {
                continue;
            };
            let osm_id = props.get("osm_id").and_then(Value::as_i64).unwrap_or(0);
            if osm_id == 0 {
                continue;
            }
            let osm_type = match props.get("osm_type").and_then(Value::as_str) {
                Some("W") => "way",
                Some("R") => "relation",
                _ => "node",
            };
            let parts: Vec<&str> = ["street", "city", "county", "state", "country"]
                .iter()
                .filter_map(|k| props.get(*k).and_then(Value::as_str))
                .collect();
            let url = format!("https://www.openstreetmap.org/{osm_type}/{osm_id}");
            let mut r = result(&self.spec.name, Category::Map, name, url, parts.join(", "));
            let osm_key = props.get("osm_key").and_then(Value::as_str).unwrap_or("");
            let osm_value = props.get("osm_value").and_then(Value::as_str).unwrap_or("");
            if !osm_key.is_empty() {
                r.metadata
                    .insert("kind".into(), format!("{osm_key}/{osm_value}"));
            }
            out.push(r);
        }
        Ok(out)
    }
}
