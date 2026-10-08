//! Offline health checks for cacc's saved accounts and the tools it relies on.

use crate::{live, store};
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;

#[derive(Debug, PartialEq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug)]
pub struct Finding {
    pub level: Level,
    pub msg: String,
}

fn f(level: Level, msg: impl Into<String>) -> Finding {
    Finding {
        level,
        msg: msg.into(),
    }
}

/// Inspect one stored credential blob. Pure so it can be tested without a keychain.
pub fn check_cred(cred: &Value, now_ms: i64) -> Vec<Finding> {
    let Some(oauth) = cred.get("claudeAiOauth").filter(|o| o.is_object()) else {
        return vec![f(Level::Fail, "credential has no claudeAiOauth login")];
    };
    let has = |k: &str| {
        oauth
            .get(k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    let mut out = Vec::new();
    if !has("accessToken") {
        out.push(f(Level::Fail, "no access token"));
    }
    if !has("refreshToken") {
        out.push(f(Level::Fail, "no refresh token, so it cannot renew"));
    }
    if out.is_empty() {
        match oauth.get("expiresAt").and_then(Value::as_i64) {
            Some(exp) if exp <= now_ms => out.push(f(
                Level::Ok,
                "access token expired; will refresh on next use",
            )),
            Some(exp) => out.push(f(
                Level::Ok,
                format!("token valid for another {} min", (exp - now_ms) / 60_000),
            )),
            None => out.push(f(Level::Warn, "no expiry recorded")),
        }
    }
    out
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

fn show(level: &Level, msg: &str) {
    let tag = match level {
        Level::Ok => " ok ",
        Level::Warn => "warn",
        Level::Fail => "FAIL",
    };
    println!("[{tag}] {msg}");
}

/// Print a report. Returns true when nothing failed.
pub fn run(idx: &store::Index) -> Result<bool> {
    let mut all: Vec<Finding> = Vec::new();

    all.push(if on_path("claude") {
        f(Level::Ok, "`claude` found on PATH")
    } else {
        f(Level::Fail, "`claude` not on PATH (`cacc login` needs it)")
    });
    all.push(if on_path("fzf") {
        f(Level::Ok, "`fzf` found on PATH")
    } else {
        f(
            Level::Warn,
            "`fzf` not on PATH; `cacc switch` falls back to a numbered prompt",
        )
    });

    match (live::read_oauth_account(), live::read_cred()) {
        (Ok(Some(_)), Ok(Some(_))) => match idx.active() {
            Ok(Some(i)) => all.push(f(
                Level::Ok,
                format!("live login is saved as '{}'", idx.accounts[i].name),
            )),
            Ok(None) => all.push(f(
                Level::Warn,
                "live login is not saved; run `cacc add` to keep it",
            )),
            Err(e) => all.push(f(Level::Fail, format!("could not match live login: {e:#}"))),
        },
        (Err(e), _) | (_, Err(e)) => {
            all.push(f(Level::Fail, format!("could not read live login: {e:#}")))
        }
        _ => all.push(f(
            Level::Warn,
            "Claude Code is not logged in (no live credential)",
        )),
    }

    if idx.accounts.is_empty() {
        all.push(f(Level::Warn, "no saved accounts"));
    }
    let now = Utc::now().timestamp_millis();
    for (i, a) in idx.accounts.iter().enumerate() {
        let label = format!("'{}' <{}>", a.name, a.email);
        if idx.accounts[..i].iter().any(|b| b.same_identity(a)) {
            all.push(f(
                Level::Warn,
                format!("{label} duplicates an earlier account"),
            ));
        }
        match store::get_cred(&a.name) {
            Ok(Some(c)) => {
                for mut x in check_cred(&c, now) {
                    x.msg = format!("{label}: {}", x.msg);
                    if x.level == Level::Fail {
                        x.msg = format!("{} (run `cacc login {}`)", x.msg, a.name);
                    }
                    all.push(x);
                }
            }
            Ok(None) => all.push(f(
                Level::Fail,
                format!(
                    "{label}: stored credential missing (run `cacc login {}`)",
                    a.name
                ),
            )),
            Err(e) => all.push(f(
                Level::Fail,
                format!("{label}: could not read credential: {e:#}"),
            )),
        }
    }

    for x in &all {
        show(&x.level, &x.msg);
    }
    Ok(!all.iter().any(|x| x.level == Level::Fail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn worst(v: &[Finding]) -> Level {
        if v.iter().any(|x| x.level == Level::Fail) {
            Level::Fail
        } else if v.iter().any(|x| x.level == Level::Warn) {
            Level::Warn
        } else {
            Level::Ok
        }
    }

    #[test]
    fn healthy() {
        let c = json!({"claudeAiOauth": {"accessToken": "a", "refreshToken": "r", "expiresAt": 10_000_000}});
        assert_eq!(worst(&check_cred(&c, 1_000)), Level::Ok);
    }

    #[test]
    fn expired_access_with_refresh_is_ok() {
        let c = json!({"claudeAiOauth": {"accessToken": "a", "refreshToken": "r", "expiresAt": 5}});
        assert_eq!(worst(&check_cred(&c, 1_000)), Level::Ok);
    }

    #[test]
    fn missing_refresh_token_fails() {
        let c = json!({"claudeAiOauth": {"accessToken": "a", "expiresAt": 5}});
        assert_eq!(worst(&check_cred(&c, 1_000)), Level::Fail);
    }

    #[test]
    fn malformed_fails() {
        assert_eq!(worst(&check_cred(&json!({}), 0)), Level::Fail);
        assert_eq!(
            worst(&check_cred(&json!({"claudeAiOauth": "x"}), 0)),
            Level::Fail
        );
    }
}
