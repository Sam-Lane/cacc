//! Interactive account picker: fzf if available, numbered prompt otherwise.

use anyhow::{Result, bail};
use std::io::{BufRead, IsTerminal, Write};
use std::process::{Command, Stdio};

/// `items` are display lines. Returns the chosen index, or None if cancelled.
pub fn pick(items: &[String]) -> Result<Option<usize>> {
    if items.is_empty() {
        bail!("no accounts saved yet; run `cacc login` or `cacc add`");
    }
    if !std::io::stdin().is_terminal() {
        bail!("no account given and stdin is not a terminal");
    }
    match fzf(items) {
        Ok(r) => Ok(r),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => prompt(items),
        Err(e) => Err(e.into()),
    }
}

fn fzf(items: &[String]) -> std::io::Result<Option<usize>> {
    let mut child = Command::new("fzf")
        .args([
            "--delimiter",
            "\t",
            "--with-nth",
            "2..",
            "--reverse",
            "--height",
            "40%",
            "--prompt",
            "account> ",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    {
        let mut stdin = child.stdin.take().unwrap();
        for (i, line) in items.iter().enumerate() {
            // A closed pipe (user picked early) is fine.
            if writeln!(stdin, "{i}\t{line}").is_err() {
                break;
            }
        }
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Ok(None); // 1 = no match, 130 = Esc/Ctrl-C
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.split('\t').next().and_then(|n| n.trim().parse().ok()))
}

fn prompt(items: &[String]) -> Result<Option<usize>> {
    for (i, l) in items.iter().enumerate() {
        eprintln!("{:>2}) {l}", i + 1);
    }
    eprint!("account> ");
    std::io::stderr().flush()?;
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s)?;
    match s.trim().parse::<usize>() {
        Ok(n) if n >= 1 && n <= items.len() => Ok(Some(n - 1)),
        _ if s.trim().is_empty() => Ok(None),
        _ => bail!("invalid selection"),
    }
}
