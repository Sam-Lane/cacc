mod doctor;
mod kc;
mod live;
mod pick;
mod store;
mod usage;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::io::{BufRead, IsTerminal, Write};
use std::process::Command;
use store::Index;

#[derive(Parser)]
#[command(
    name = "cacc",
    version,
    about = "Claude accounts: switch between Claude Code logins"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List saved accounts
    List,
    /// Switch to an account (name, email or number)
    Set { account: String },
    /// Switch to an account; pick from an fzf menu if none given
    Switch { account: Option<String> },
    /// Show usage for all accounts, or one
    Usage { account: Option<String> },
    /// Log in to a (new or existing) account via `claude auth login` and save it
    Login { name: Option<String> },
    /// Save the account Claude Code is currently logged in to
    Add { name: Option<String> },
    /// Delete a saved account (does not log Claude Code out)
    #[command(alias = "rm")]
    Remove {
        account: String,
        /// Skip the confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Rename a saved account
    Rename { old: String, new: String },
    /// Check tools, the live login and every saved account for problems
    Doctor,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("cacc: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut idx = Index::load()?;
    match Cli::parse().cmd {
        Cmd::List => list(&mut idx),
        Cmd::Set { account } => set(&mut idx, Some(account)),
        Cmd::Switch { account } => set(&mut idx, account),
        Cmd::Usage { account } => usage(&mut idx, account),
        Cmd::Login { name } => login(&mut idx, name),
        Cmd::Remove { account, yes } => remove(&mut idx, &account, yes),
        Cmd::Rename { old, new } => {
            let i = idx.resolve(&old)?;
            let before = idx.accounts[i].name.clone();
            idx.rename(i, &new)?;
            println!("renamed '{before}' to '{new}'");
            Ok(())
        }
        Cmd::Doctor => {
            if !doctor::run(&idx)? {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Add { name } => {
            let i = idx.capture(name.as_deref())?;
            println!(
                "saved '{}' <{}>",
                idx.accounts[i].name, idx.accounts[i].email
            );
            Ok(())
        }
    }
}

fn describe(a: &store::Account, active: bool) -> String {
    let org = if a.org_name.is_empty() {
        "-"
    } else {
        &a.org_name
    };
    format!(
        "{} {:<16} {:<32} {org}",
        if active { "*" } else { " " },
        a.name,
        a.email
    )
}

fn list(idx: &mut Index) -> Result<()> {
    if idx.accounts.is_empty() {
        println!("no accounts saved. Run `cacc add` (current login) or `cacc login`.");
        return Ok(());
    }
    let active = idx.active()?;
    for (i, a) in idx.accounts.iter().enumerate() {
        println!("{:>2} {}", i + 1, describe(a, Some(i) == active));
    }
    Ok(())
}

/// Save the live login if it's one we don't track yet, so switching never loses it.
fn save_untracked(idx: &mut Index) -> Result<()> {
    if idx.sync_active()?.is_some() {
        return Ok(());
    }
    if live::read_oauth_account()?.is_some() && live::read_cred()?.is_some() {
        let i = idx.capture(None)?;
        eprintln!("saved current login as '{}'", idx.accounts[i].name);
    }
    Ok(())
}

fn set(idx: &mut Index, account: Option<String>) -> Result<()> {
    save_untracked(idx)?;
    let target = match account {
        Some(q) => idx.resolve(&q)?,
        None => {
            let active = idx.active()?;
            let items: Vec<String> = idx
                .accounts
                .iter()
                .enumerate()
                .map(|(i, a)| describe(a, Some(i) == active))
                .collect();
            match pick::pick(&items)? {
                Some(i) => i,
                None => return Ok(()),
            }
        }
    };
    idx.activate(target)?;
    let a = &idx.accounts[target];
    println!("switched to '{}' <{}>", a.name, a.email);
    println!("restart running Claude Code sessions to pick it up");
    Ok(())
}

fn usage(idx: &mut Index, account: Option<String>) -> Result<()> {
    if idx.accounts.is_empty() {
        bail!("no accounts saved");
    }
    let active = idx.sync_active()?;
    let targets: Vec<usize> = match account {
        Some(q) => vec![idx.resolve(&q)?],
        None => (0..idx.accounts.len()).collect(),
    };
    for i in targets {
        let a = &idx.accounts[i];
        println!("{}", describe(a, Some(i) == active).trim_start());
        match usage_for(idx, i, Some(i) == active) {
            Ok(u) => usage::render(&u).iter().for_each(|l| println!("{l}")),
            Err(e) => println!("  error: {e:#}"),
        }
    }
    Ok(())
}

fn usage_for(idx: &Index, i: usize, is_active: bool) -> Result<Value> {
    let name = &idx.accounts[i].name;
    let mut cred = if is_active {
        live::read_cred()?
    } else {
        store::get_cred(name)?
    }
    .with_context(|| format!("no stored credential; run `cacc login {name}`"))?;
    if usage::refresh_if_needed(&mut cred)? {
        store::put_cred(name, &cred)?;
        if is_active {
            live::write_cred(&cred)?;
        }
    }
    usage::fetch(&cred)
}

fn login(idx: &mut Index, name: Option<String>) -> Result<()> {
    save_untracked(idx)?;
    eprintln!("running `claude auth login` ...");
    let st = Command::new("claude")
        .args(["auth", "login"])
        .status()
        .context("could not run `claude` (is Claude Code installed and on PATH?)")?;
    if !st.success() {
        bail!("`claude auth login` failed");
    }
    let i = idx.capture(name.as_deref())?;
    println!(
        "saved '{}' <{}>",
        idx.accounts[i].name, idx.accounts[i].email
    );
    Ok(())
}

fn remove(idx: &mut Index, query: &str, yes: bool) -> Result<()> {
    let i = idx.resolve(query)?;
    let (name, email) = (idx.accounts[i].name.clone(), idx.accounts[i].email.clone());
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("refusing to remove without --yes when stdin is not a terminal");
        }
        eprint!("remove '{name}' <{email}>? [y/N] ");
        std::io::stderr().flush()?;
        let mut ans = String::new();
        std::io::stdin().lock().read_line(&mut ans)?;
        if !matches!(ans.trim(), "y" | "Y" | "yes") {
            eprintln!("cancelled");
            return Ok(());
        }
    }
    let was_active = idx.active()? == Some(i);
    idx.remove(i)?;
    println!("removed '{name}' <{email}>");
    if was_active {
        println!("Claude Code is still logged in as this account; it is just no longer saved");
    }
    Ok(())
}
