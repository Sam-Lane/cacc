//! Thin wrapper over the macOS `security` CLI (generic passwords).

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::process::{Command, Stdio};

const SECURITY: &str = "/usr/bin/security";
/// `security -i` reads lines of at most 4096 bytes; leave headroom.
const STDIN_LIMIT: usize = 4032;

pub fn get(service: &str, account: &str) -> Result<Option<String>> {
    let out = Command::new(SECURITY)
        .args(["find-generic-password", "-a", account, "-w", "-s", service])
        .output()
        .context("running security")?;
    if out.status.code() == Some(44) {
        return Ok(None);
    }
    if !out.status.success() {
        bail!(
            "keychain read failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let s = String::from_utf8(out.stdout)?;
    Ok(Some(s.trim_end_matches('\n').to_string()))
}

pub fn set(service: &str, account: &str, value: &str) -> Result<()> {
    let hex: String = value.bytes().map(|b| format!("{b:02x}")).collect();
    let line = format!(
        "add-generic-password -U -a {} -s {} -X {}\n",
        quote(account),
        quote(service),
        hex
    );
    if line.len() <= STDIN_LIMIT {
        // Keeps the secret out of argv (visible via `ps`).
        let mut child = Command::new(SECURITY)
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("running security")?;
        child.stdin.take().unwrap().write_all(line.as_bytes())?;
        child.wait()?;
    } else {
        let st = Command::new(SECURITY)
            .args([
                "add-generic-password",
                "-U",
                "-a",
                account,
                "-s",
                service,
                "-X",
                &hex,
            ])
            .stderr(Stdio::null())
            .status()
            .context("running security")?;
        if !st.success() {
            bail!("keychain write failed");
        }
    }
    // `security -i` doesn't report errors via exit status, so verify.
    if get(service, account)?.as_deref() != Some(value) {
        bail!("keychain write for '{account}' did not stick");
    }
    Ok(())
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
