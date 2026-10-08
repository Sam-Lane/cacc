//! Token refresh and the OAuth usage endpoint.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const BETA: &str = "oauth-2025-04-20";
const EXPIRY_BUFFER_MS: i64 = 5 * 60 * 1000;
const UA: &str = concat!("cacc/", env!("CARGO_PKG_VERSION"));

pub fn needs_refresh(cred: &Value, now_ms: i64) -> bool {
    match cred
        .pointer("/claudeAiOauth/expiresAt")
        .and_then(Value::as_i64)
    {
        Some(exp) => now_ms + EXPIRY_BUFFER_MS >= exp,
        None => false,
    }
}

/// Refresh the access token in place. Returns true if `cred` changed.
pub fn refresh_if_needed(cred: &mut Value) -> Result<bool> {
    let now = Utc::now().timestamp_millis();
    if !needs_refresh(cred, now) {
        return Ok(false);
    }
    let refresh = cred
        .pointer("/claudeAiOauth/refreshToken")
        .and_then(Value::as_str)
        .context("no refresh token stored")?
        .to_string();
    let mut resp = ureq::post(TOKEN_URL)
        .header("User-Agent", UA)
        .send_json(json!({"grant_type": "refresh_token", "refresh_token": refresh, "client_id": CLIENT_ID}))
        .map_err(|e| match e {
            ureq::Error::StatusCode(400 | 401) => anyhow::anyhow!("refresh token rejected; re-login needed"),
            e => anyhow::anyhow!("token refresh failed: {e}"),
        })?;
    let body: Value = resp.body_mut().read_json()?;
    let oauth = cred["claudeAiOauth"]
        .as_object_mut()
        .context("malformed credential")?;
    oauth.insert("accessToken".into(), body["access_token"].clone());
    let ttl = body["expires_in"].as_i64().unwrap_or(3600);
    oauth.insert("expiresAt".into(), json!(now + ttl * 1000));
    if let Some(r) = body.get("refresh_token").filter(|r| r.is_string()) {
        oauth.insert("refreshToken".into(), r.clone());
    }
    if let Some(s) = body.get("scope").and_then(Value::as_str) {
        oauth.insert(
            "scopes".into(),
            json!(s.split_whitespace().collect::<Vec<_>>()),
        );
    }
    Ok(true)
}

pub fn fetch(cred: &Value) -> Result<Value> {
    let token = cred
        .pointer("/claudeAiOauth/accessToken")
        .and_then(Value::as_str)
        .context("no access token stored")?;
    let mut resp = ureq::get(USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", BETA)
        .header("User-Agent", UA)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401 | 403) => {
                anyhow::anyhow!("token rejected; run `cacc login <name>`")
            }
            e => anyhow::anyhow!("usage request failed: {e}"),
        })?;
    Ok(resp.body_mut().read_json()?)
}

fn bar(pct: f64) -> String {
    let filled = ((pct / 100.0) * 20.0).round().clamp(0.0, 20.0) as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(20 - filled))
}

fn until(resets_at: Option<&str>) -> String {
    let Some(t) = resets_at.and_then(|s| DateTime::parse_from_rfc3339(s).ok()) else {
        return String::new();
    };
    let mins = (t.with_timezone(&Utc) - Utc::now()).num_minutes().max(0);
    let (d, h, m) = (mins / 1440, mins % 1440 / 60, mins % 60);
    let s = if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    };
    format!("resets in {s}")
}

fn line(label: &str, window: &Value) -> Option<String> {
    let pct = window.get("utilization")?.as_f64()?;
    let reset = until(window.get("resets_at").and_then(Value::as_str));
    Some(format!("  {label:<10} {} {pct:>5.1}%  {reset}", bar(pct)))
}

pub fn render(u: &Value) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(u.get("five_hour").and_then(|w| line("5-hour", w)));
    out.extend(u.get("seven_day").and_then(|w| line("7-day", w)));
    if let Some(limits) = u.get("limits").and_then(Value::as_array) {
        for l in limits {
            let name = l
                .pointer("/scope/model/display_name")
                .and_then(Value::as_str)
                .unwrap_or("limit");
            if let Some(pct) = l.get("percent").and_then(Value::as_f64) {
                let reset = until(l.get("resets_at").and_then(Value::as_str));
                out.push(format!("  {name:<10} {} {pct:>5.1}%  {reset}", bar(pct)));
            }
        }
    }
    if let Some(x) = u
        .get("extra_usage")
        .filter(|x| x["is_enabled"].as_bool() == Some(true))
    {
        let used = x["used_credits"].as_f64().unwrap_or(0.0);
        let cap = x["monthly_limit"].as_f64().unwrap_or(0.0);
        let cur = x["currency"].as_str().unwrap_or("USD");
        out.push(format!("  extra      {used:.2} / {cap:.2} {cur}"));
    }
    if out.is_empty() {
        out.push("  (no usage data returned)".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_window() {
        let c = json!({"claudeAiOauth": {"expiresAt": 1_000_000}});
        assert!(needs_refresh(&c, 1_000_000 - EXPIRY_BUFFER_MS));
        assert!(!needs_refresh(&c, 1_000_000 - EXPIRY_BUFFER_MS - 1));
    }

    #[test]
    fn renders_windows() {
        let u = json!({"five_hour": {"utilization": 50.0}, "seven_day": {"utilization": 100.0}});
        let r = render(&u);
        assert_eq!(r.len(), 2);
        assert!(r[0].contains("50.0%") && r[0].contains("██████████░░░░░░░░░░"));
    }
}
