//! Instant-answer specialists: currency conversion (frankfurter.app),
//! MyMemory translation and Lingva translation.

use crate::http_util::json_get;
use crate::spec::{urlencoding_lite, EngineSpec};
use crate::trait_def::{result, Engine, EngineError};
use async_trait::async_trait;
use serde_json::Value;
use tokenloom_core::{Category, SearchQuery, SearchResult};

// ── currency ─────────────────────────────────────────────────────────────────

pub struct CurrencyEngine {
    spec: EngineSpec,
}

impl CurrencyEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self { spec }
    }
}

/// Parse "100 usd to eur" / "usd to eur" / "100 usd eur" → (amount, from, to).
fn parse_currency_query(q: &str) -> Option<(f64, String, String)> {
    let tokens: Vec<String> = q
        .split_whitespace()
        .map(|t| {
            t.trim_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_uppercase()
        })
        .filter(|t| !t.is_empty())
        .collect();
    let mut amount = 1.0f64;
    let mut codes: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if t == "TO" || t == "IN" || t == "=" {
            i += 1;
            continue;
        }
        if let Ok(v) = t.parse::<f64>() {
            amount = v;
        } else if t.len() == 3 && t.chars().all(|c| c.is_ascii_alphabetic()) {
            codes.push(t.clone());
        }
        i += 1;
    }
    if codes.len() >= 2 {
        Some((amount, codes[0].clone(), codes[1].clone()))
    } else {
        None
    }
}

#[async_trait]
impl Engine for CurrencyEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let Some((amount, from, to)) = parse_currency_query(&query.clean_query) else {
            return Err(EngineError::Parse(
                "currency query must look like '100 usd to eur'".into(),
            ));
        };
        let url = format!("https://api.frankfurter.app/latest?from={from}&to={to}&amount={amount}");
        let json = json_get(http, &url, self.timeout(), &[]).await?;
        let rate = json
            .pointer(&format!("/rates/{to}"))
            .and_then(Value::as_f64)
            .ok_or_else(|| EngineError::Parse("missing rate".into()))?;
        let mut r = result(
            &self.spec.name,
            Category::General,
            format!("{amount} {from} = {rate:.4} {to}"),
            format!("https://frankfurter.app/{from}/{to}/"),
            format!(
                "Exchange rate 1 {from} = {:.6} {to} (ECB reference rates)",
                rate / amount
            ),
        );
        r.metadata.insert("kind".into(), "instant_answer".into());
        r.metadata
            .insert("rate".into(), format!("{:.6}", rate / amount));
        Ok(vec![r])
    }
}

// ── translation (MyMemory) ───────────────────────────────────────────────────

pub struct TranslatedEngine {
    spec: EngineSpec,
}

impl TranslatedEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self { spec }
    }
}

/// Split "hello world en de" / "en de hello world" → (text, from, to).
fn parse_translation_query(q: &str) -> Option<(String, String, String)> {
    let tokens: Vec<&str> = q.split_whitespace().collect();
    let is_code = |t: &str| t.len() == 2 && t.chars().all(|c| c.is_ascii_alphabetic());
    if tokens.len() >= 4 && is_code(tokens[0]) && is_code(tokens[1]) {
        return Some((
            tokens[2..].join(" "),
            tokens[0].to_string(),
            tokens[1].to_string(),
        ));
    }
    if tokens.len() >= 4 && is_code(tokens[tokens.len() - 2]) && is_code(tokens[tokens.len() - 1]) {
        let n = tokens.len();
        return Some((
            tokens[..n - 2].join(" "),
            tokens[n - 2].to_string(),
            tokens[n - 1].to_string(),
        ));
    }
    None
}

#[async_trait]
impl Engine for TranslatedEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let Some((text, from, to)) = parse_translation_query(&query.clean_query) else {
            return Err(EngineError::Parse(
                "translation query must include source and target codes, e.g. '!tl en de hello'"
                    .into(),
            ));
        };
        let url = format!(
            "https://api.mymemory.translated.net/get?q={}&langpair={}|{}",
            urlencoding_lite(&text),
            from,
            to
        );
        let json = json_get(http, &url, self.timeout(), &[]).await?;
        let translated = json
            .pointer("/responseData/translatedText")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if translated.is_empty() {
            return Err(EngineError::Parse("empty translation".into()));
        }
        let mut r = result(
            &self.spec.name,
            Category::General,
            translated,
            format!("https://mymemory.translated.net/en/{from}/{to}/"),
            format!("“{text}” ({from} → {to})"),
        );
        r.metadata.insert("kind".into(), "instant_answer".into());
        Ok(vec![r])
    }
}

// ── translation (Lingva) ─────────────────────────────────────────────────────

pub struct LingvaEngine {
    spec: EngineSpec,
}

impl LingvaEngine {
    pub fn new(spec: EngineSpec) -> Self {
        Self { spec }
    }
}

#[async_trait]
impl Engine for LingvaEngine {
    fn spec(&self) -> &EngineSpec {
        &self.spec
    }

    async fn search(
        &self,
        query: &SearchQuery,
        http: &reqwest::Client,
    ) -> Result<Vec<SearchResult>, EngineError> {
        // Query format: "text…" or "text… [target-lang]".
        let mut tokens: Vec<&str> = query.clean_query.split_whitespace().collect();
        let mut target = "en".to_string();
        if let Some(last) = tokens.last() {
            if last.len() == 2 && last.chars().all(|c| c.is_ascii_alphabetic()) {
                target = last.to_lowercase();
                tokens.pop();
            }
        }
        let text = tokens.join(" ");
        if text.is_empty() {
            return Err(EngineError::Parse("empty translation query".into()));
        }
        let url = format!(
            "https://lingva.ml/api/v1/auto/{target}/{}",
            urlencoding_lite(&text)
        );
        let json = json_get(http, &url, self.timeout(), &[]).await?;
        let translated = json
            .get("translation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if translated.is_empty() {
            return Err(EngineError::Parse("empty translation".into()));
        }
        let share_url = format!("https://lingva.ml/en/{target}/{}", urlencoding_lite(&text));
        let mut r = result(
            &self.spec.name,
            Category::General,
            translated,
            share_url,
            format!("“{text}” (auto → {target})"),
        );
        r.metadata.insert("kind".into(), "instant_answer".into());
        Ok(vec![r])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_query_parsing() {
        assert_eq!(
            parse_currency_query("100 usd to eur"),
            Some((100.0, "USD".into(), "EUR".into()))
        );
        assert_eq!(
            parse_currency_query("usd = gbp"),
            Some((1.0, "USD".into(), "GBP".into()))
        );
        assert_eq!(parse_currency_query("hello world"), None);
    }

    #[test]
    fn translation_query_parsing() {
        assert_eq!(
            parse_translation_query("en de hello world"),
            Some(("hello world".into(), "en".into(), "de".into()))
        );
        assert_eq!(
            parse_translation_query("hello world en de"),
            Some(("hello world".into(), "en".into(), "de".into()))
        );
        assert_eq!(parse_translation_query("just text"), None);
    }
}
