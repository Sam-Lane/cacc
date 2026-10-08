//! Saved accounts: a plain index plus per-account secret credential blobs.

use crate::{kc, live};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const SERVICE: &str = "cacc";

#[derive(Serialize, Deserialize, Clone)]
pub struct Account {
    pub name: String,
    pub email: String,
    pub account_uuid: String,
    #[serde(default)]
    pub org_uuid: String,
    #[serde(default)]
    pub org_name: String,
    /// Copy of `oauthAccount` from ~/.claude.json (not secret).
    pub oauth_account: Value,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Index {
    pub accounts: Vec<Account>,
}

fn dir() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("no home directory")?
        .join(".config")
        .join("cacc"))
}

fn str_field(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Names must be safe as keychain/file keys and must not collide with how
/// `Index::resolve` interprets a query (number, other account's name or email).
pub fn validate_name(name: &str, accounts: &[Account], except: Option<usize>) -> Result<()> {
    if name.is_empty() {
        bail!("account name cannot be empty");
    }
    if name
        .chars()
        .any(|c| c.is_whitespace() || c == '/' || c == '\\')
    {
        bail!("account name '{name}' cannot contain whitespace or slashes");
    }
    if name.parse::<usize>().is_ok() {
        bail!("account name '{name}' cannot be a number (numbers select by position)");
    }
    let clash = accounts
        .iter()
        .enumerate()
        .any(|(i, a)| Some(i) != except && (a.name == name || a.email == name));
    if clash {
        bail!("an account named '{name}' already exists");
    }
    Ok(())
}

impl Account {
    pub fn from_oauth(name: String, oauth: &Value) -> Result<Self> {
        let email = str_field(oauth, "emailAddress");
        let account_uuid = str_field(oauth, "accountUuid");
        if email.is_empty() || account_uuid.is_empty() {
            bail!("~/.claude.json has no usable oauthAccount (are you logged in?)");
        }
        Ok(Self {
            name,
            email,
            account_uuid,
            org_uuid: str_field(oauth, "organizationUuid"),
            org_name: str_field(oauth, "organizationName"),
            oauth_account: oauth.clone(),
        })
    }

    pub fn same_identity(&self, other: &Account) -> bool {
        self.account_uuid == other.account_uuid && self.org_uuid == other.org_uuid
    }
}

impl Index {
    pub fn load() -> Result<Self> {
        let p = dir()?.join("accounts.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&fs::read(&p)?).context("parsing accounts.json")
    }

    pub fn save(&self) -> Result<()> {
        live::write_private(
            &dir()?.join("accounts.json"),
            serde_json::to_string_pretty(self)?.as_bytes(),
        )
    }

    /// Resolve by exact name, exact email, 1-based index, then unique prefix.
    pub fn resolve(&self, q: &str) -> Result<usize> {
        if let Some(i) = self.accounts.iter().position(|a| a.name == q) {
            return Ok(i);
        }
        if let Some(i) = self.accounts.iter().position(|a| a.email == q) {
            return Ok(i);
        }
        if let Ok(n) = q.parse::<usize>()
            && (1..=self.accounts.len()).contains(&n)
        {
            return Ok(n - 1);
        }
        let hits: Vec<usize> = (0..self.accounts.len())
            .filter(|&i| {
                self.accounts[i].name.starts_with(q) || self.accounts[i].email.starts_with(q)
            })
            .collect();
        match hits.as_slice() {
            [i] => Ok(*i),
            [] => bail!("no account matching '{q}' (see `cacc list`)"),
            _ => bail!("'{q}' is ambiguous (see `cacc list`)"),
        }
    }

    /// Index of the saved account matching the live login, if any.
    pub fn active(&self) -> Result<Option<usize>> {
        let Some(oauth) = live::read_oauth_account()? else {
            return Ok(None);
        };
        let Ok(probe) = Account::from_oauth(String::new(), &oauth) else {
            return Ok(None);
        };
        Ok(self.accounts.iter().position(|a| a.same_identity(&probe)))
    }

    /// If the live login is a saved account, refresh its snapshot (tokens rotate).
    pub fn sync_active(&mut self) -> Result<Option<usize>> {
        let Some(i) = self.active()? else {
            return Ok(None);
        };
        if let Some(cred) = live::read_cred()? {
            put_cred(&self.accounts[i].name, &cred)?;
        }
        if let Some(oauth) = live::read_oauth_account()? {
            self.accounts[i].oauth_account = oauth;
            self.save()?;
        }
        Ok(Some(i))
    }

    /// Save the live login. Existing identity -> update in place (renamed if
    /// `name` given); otherwise a new account named `name` or the email's local part.
    pub fn capture(&mut self, name: Option<&str>) -> Result<usize> {
        let oauth = live::read_oauth_account()?
            .context("not logged in to Claude Code (no oauthAccount)")?;
        let cred = live::read_cred()?.context("no Claude Code credential found")?;
        let mut acct = Account::from_oauth(String::new(), &oauth)?;
        let existing = self.accounts.iter().position(|a| a.same_identity(&acct));

        let new_name = match (name, existing) {
            (Some(n), _) => n.to_string(),
            (None, Some(i)) => self.accounts[i].name.clone(),
            (None, None) => acct
                .email
                .split('@')
                .next()
                .unwrap_or("account")
                .to_string(),
        };
        // An existing account keeps its (already accepted) name unless one is given.
        if name.is_some() || existing.is_none() {
            validate_name(&new_name, &self.accounts, existing).map_err(|e| {
                if name.is_none() {
                    e.context("pass an explicit name")
                } else {
                    e
                }
            })?;
        }
        acct.name = new_name;

        let idx = match existing {
            Some(i) => {
                if self.accounts[i].name != acct.name {
                    drop_cred(&self.accounts[i].name)?;
                }
                self.accounts[i] = acct;
                i
            }
            None => {
                self.accounts.push(acct);
                self.accounts.len() - 1
            }
        };
        put_cred(&self.accounts[idx].name, &cred)?;
        self.save()?;
        Ok(idx)
    }

    /// Delete a saved account and its stored credential. Never touches the live login.
    pub fn remove(&mut self, i: usize) -> Result<Account> {
        drop_cred(&self.accounts[i].name)?;
        let a = self.accounts.remove(i);
        self.save()?;
        Ok(a)
    }

    /// Rename account `i`, moving its stored credential. The old copy is only
    /// dropped once the new one is verified, so a failure never loses the login.
    pub fn rename(&mut self, i: usize, new: &str) -> Result<()> {
        validate_name(new, &self.accounts, Some(i))?;
        let old = self.accounts[i].name.clone();
        if old == new {
            return Ok(());
        }
        let cred = get_cred(&old)?
            .with_context(|| format!("no stored credential for '{old}'; run `cacc login {old}`"))?;
        put_cred(new, &cred)?;
        if get_cred(new)?.as_ref() != Some(&cred) {
            bail!("could not verify the copied credential; '{old}' left unchanged");
        }
        self.accounts[i].name = new.to_string();
        self.save()?;
        drop_cred(&old)
    }

    /// Make account `i` the live login.
    pub fn activate(&self, i: usize) -> Result<()> {
        let a = &self.accounts[i];
        let saved = get_cred(&a.name)?.with_context(|| {
            format!(
                "no stored credential for '{}'; run `cacc login {}`",
                a.name, a.name
            )
        })?;
        // Keep machine-shared keys (mcpOAuth etc.) from the live blob; swap only the login.
        let mut next = live::read_cred()?.unwrap_or_else(|| saved.clone());
        match (next.as_object_mut(), saved.get("claudeAiOauth")) {
            (Some(o), Some(login)) => {
                o.insert("claudeAiOauth".into(), login.clone());
            }
            _ => next = saved,
        }
        live::write_cred(&next)?;
        live::write_oauth_account(&a.oauth_account)
    }
}

fn cred_file(name: &str) -> Result<PathBuf> {
    Ok(dir()?.join("creds").join(format!("{name}.json")))
}

pub fn get_cred(name: &str) -> Result<Option<Value>> {
    let raw = if cfg!(target_os = "macos") {
        kc::get(SERVICE, &format!("account-{name}"))?
    } else {
        let p = cred_file(name)?;
        if p.exists() {
            Some(fs::read_to_string(p)?)
        } else {
            None
        }
    };
    raw.map(|s| serde_json::from_str(&s).context("parsing stored credential"))
        .transpose()
}

pub fn put_cred(name: &str, cred: &Value) -> Result<()> {
    let s = serde_json::to_string(cred)?;
    if cfg!(target_os = "macos") {
        kc::set(SERVICE, &format!("account-{name}"), &s)
    } else {
        live::write_private(&cred_file(name)?, s.as_bytes())
    }
}

pub(crate) fn drop_cred(name: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("/usr/bin/security")
            .args([
                "delete-generic-password",
                "-a",
                &format!("account-{name}"),
                "-s",
                SERVICE,
            ])
            .output();
    } else {
        let _ = fs::remove_file(cred_file(name)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn acct(name: &str, email: &str) -> Account {
        Account {
            name: name.into(),
            email: email.into(),
            account_uuid: format!("uuid-{name}"),
            org_uuid: String::new(),
            org_name: String::new(),
            oauth_account: json!({}),
        }
    }

    fn two() -> Vec<Account> {
        vec![acct("main", "a@x.com"), acct("work", "b@x.com")]
    }

    #[test]
    fn name_rules() {
        let a = two();
        assert!(validate_name("side", &a, None).is_ok());
        for bad in ["", "has space", "a/b", "a\\b", "3", "main", "b@x.com"] {
            assert!(validate_name(bad, &a, None).is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn renaming_to_own_name_is_allowed() {
        let a = two();
        assert!(validate_name("main", &a, Some(0)).is_ok());
        assert!(validate_name("work", &a, Some(0)).is_err());
    }

    #[test]
    fn resolve_by_name_email_number_prefix() {
        let idx = Index { accounts: two() };
        assert_eq!(idx.resolve("work").unwrap(), 1);
        assert_eq!(idx.resolve("a@x.com").unwrap(), 0);
        assert_eq!(idx.resolve("2").unwrap(), 1);
        assert_eq!(idx.resolve("ma").unwrap(), 0);
        assert!(idx.resolve("nope").is_err());
    }
}
