//! Claude Code's live login state: OAuth credential + `oauthAccount` identity.

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const LIVE_SERVICE: &str = "Claude Code-credentials";

fn home() -> Result<PathBuf> {
    dirs::home_dir().context("no home directory")
}

fn cred_file() -> Result<PathBuf> {
    Ok(home()?.join(".claude").join(".credentials.json"))
}

fn config_file() -> Result<PathBuf> {
    Ok(home()?.join(".claude.json"))
}

fn username() -> String {
    std::env::var("USER").unwrap_or_else(|_| "claude-code-user".into())
}

/// Write `bytes` to `path` atomically with 0600 permissions.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("cacc-tmp");
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn read_cred() -> Result<Option<Value>> {
    if cfg!(target_os = "macos")
        && let Some(s) = crate::kc::get(LIVE_SERVICE, &username())?
    {
        return Ok(Some(
            serde_json::from_str(&s).context("parsing keychain credential")?,
        ));
    }
    let path = cred_file()?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_slice(&fs::read(&path)?).context("parsing .credentials.json")?,
    ))
}

pub fn write_cred(cred: &Value) -> Result<()> {
    let s = serde_json::to_string(cred)?;
    if cfg!(target_os = "macos") {
        crate::kc::set(LIVE_SERVICE, &username(), &s)
    } else {
        write_private(&cred_file()?, s.as_bytes())
    }
}

pub fn read_oauth_account() -> Result<Option<Value>> {
    let path = config_file()?;
    if !path.exists() {
        return Ok(None);
    }
    let v: Value = serde_json::from_slice(&fs::read(&path)?).context("parsing ~/.claude.json")?;
    Ok(v.get("oauthAccount").filter(|a| a.is_object()).cloned())
}

/// Replace `oauthAccount` in ~/.claude.json, preserving every other key.
pub fn write_oauth_account(acct: &Value) -> Result<()> {
    let path = config_file()?;
    let mut root: Value = if path.exists() {
        serde_json::from_slice(&fs::read(&path)?).context("parsing ~/.claude.json")?
    } else {
        Value::Object(Map::new())
    };
    root.as_object_mut()
        .context("~/.claude.json is not an object")?
        .insert("oauthAccount".into(), acct.clone());
    write_private(&path, serde_json::to_string_pretty(&root)?.as_bytes())
}
