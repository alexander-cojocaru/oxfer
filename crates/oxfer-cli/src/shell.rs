//! Interactive shell: `oxfer shell user@host`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::{Parser, Subcommand};
use oxfer_core::{LocalFs, RemoteFs, RemotePath};
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::{Context, Editor, Helper};
use rustyline::{Highlighter, Hinter, Validator};
use tokio::runtime::Handle;

use crate::ops::{self, TransferArgs};

#[derive(Parser)]
#[command(no_binary_name = true, name = "", disable_help_flag = true)]
struct Line {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List a remote directory
    Ls { path: Option<String> },
    /// List a local directory
    Lls { path: Option<String> },
    /// Change the remote directory
    Cd { path: Option<String> },
    /// Change the local directory
    Lcd { path: Option<String> },
    /// Print the remote directory
    Pwd,
    /// Print the local directory
    Lpwd,
    /// Show metadata of a remote path
    Stat { path: String },
    /// Download: get [options] REMOTE... [LOCAL]
    Get {
        #[arg(required = true)]
        paths: Vec<String>,
        #[command(flatten)]
        transfer: TransferArgs,
    },
    /// Upload: put [options] LOCAL... [REMOTE]
    Put {
        #[arg(required = true)]
        paths: Vec<String>,
        #[command(flatten)]
        transfer: TransferArgs,
    },
    /// Create remote directories
    Mkdir {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Remove remote files or directories
    Rm {
        #[arg(short, long)]
        recursive: bool,
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Rename a remote path
    Mv { from: String, to: String },
    /// Leave the shell
    #[command(aliases = ["quit", "bye"])]
    Exit,
}

struct Paths {
    cwd: RemotePath,
    home: RemotePath,
    lcwd: PathBuf,
}

type Shared = Arc<Mutex<Paths>>;

fn rpath(st: &Shared, arg: &str) -> RemotePath {
    let st = st.lock().unwrap();
    if arg.starts_with('/') {
        RemotePath::new(arg).normalize()
    } else if arg == "~" {
        st.home.clone()
    } else if let Some(rest) = arg.strip_prefix("~/") {
        st.home.join(rest).normalize()
    } else {
        st.cwd.join(arg).normalize()
    }
}

fn lpath(st: &Shared, arg: &str) -> RemotePath {
    let st = st.lock().unwrap();
    let p = std::path::Path::new(arg);
    let full = if p.is_absolute() {
        p.to_path_buf()
    } else {
        st.lcwd.join(p)
    };
    RemotePath::from_local(&full.to_string_lossy()).normalize()
}

#[derive(Helper, Highlighter, Hinter, Validator)]
struct Complete {
    fs: Arc<dyn RemoteFs>,
    rt: Handle,
    st: Shared,
}

const COMMANDS: &[&str] = &[
    "ls", "lls", "cd", "lcd", "pwd", "lpwd", "stat", "get", "put", "mkdir", "rm", "mv", "help",
    "exit", "quit", "bye",
];

/// Start of the word being completed (spaces escaped with `\` are part of it).
fn word_start(before: &str) -> usize {
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in before.char_indices() {
        match c {
            '\\' if !escaped => {
                escaped = true;
                continue;
            }
            ' ' if !escaped => start = i + 1,
            _ => {}
        }
        escaped = false;
    }
    start
}

fn escape(s: &str) -> String {
    s.replace(' ', "\\ ")
}

impl Completer for Complete {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let before = &line[..pos];
        let start = word_start(before);
        let word = before[start..].replace("\\ ", " ");
        let head = before[..start].trim();
        if head.is_empty() {
            let c = COMMANDS
                .iter()
                .filter(|c| c.starts_with(&word))
                .map(|c| Pair {
                    display: c.to_string(),
                    replacement: format!("{c} "),
                })
                .collect();
            return Ok((start, c));
        }
        let local = matches!(head.split(' ').next(), Some("lcd" | "lls" | "put"));
        let (dir_part, prefix) = match word.rfind('/') {
            Some(i) => (&word[..=i], &word[i + 1..]),
            None => ("", word.as_str()),
        };
        let names: Vec<(String, bool)> = if local {
            let dir = lpath(&self.st, if dir_part.is_empty() { "." } else { dir_part });
            std::fs::read_dir(dir.as_str())
                .map(|rd| {
                    rd.flatten()
                        .map(|e| {
                            (
                                e.file_name().to_string_lossy().into_owned(),
                                e.path().is_dir(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            let dir = rpath(&self.st, if dir_part.is_empty() { "." } else { dir_part });
            self.rt
                .block_on(self.fs.list(&dir))
                .map(|v| {
                    v.into_iter()
                        .map(|e| (e.name().to_string(), e.is_dir()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut out: Vec<Pair> = names
            .into_iter()
            .filter(|(n, _)| {
                n.starts_with(prefix) && (prefix.starts_with('.') || !n.starts_with('.'))
            })
            .map(|(n, is_dir)| Pair {
                display: if is_dir { format!("{n}/") } else { n.clone() },
                replacement: format!(
                    "{}{}",
                    escape(&format!("{dir_part}{n}")),
                    if is_dir { "/" } else { " " }
                ),
            })
            .collect();
        out.sort_by(|a, b| a.display.cmp(&b.display));
        Ok((start, out))
    }
}

fn history_file() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "oxfer")?;
    let dir = dirs.data_local_dir();
    std::fs::create_dir_all(dir).ok()?;
    Some(dir.join("history.txt"))
}

pub async fn run(fs: Arc<dyn RemoteFs>, label: String, start_dir: &str) -> Result<()> {
    let home = fs.realpath(&RemotePath::new(".")).await?;
    let cwd = if start_dir == "." {
        home.clone()
    } else {
        fs.realpath(&RemotePath::new(start_dir)).await?
    };
    let st: Shared = Arc::new(Mutex::new(Paths {
        cwd,
        home,
        lcwd: std::env::current_dir()?,
    }));
    let rt = Handle::current();
    tokio::task::spawn_blocking(move || repl(fs, label, st, rt)).await??;
    Ok(())
}

fn repl(fs: Arc<dyn RemoteFs>, label: String, st: Shared, rt: Handle) -> Result<()> {
    let mut rl: Editor<Complete, rustyline::history::DefaultHistory> = Editor::new()?;
    rl.set_helper(Some(Complete {
        fs: fs.clone(),
        rt: rt.clone(),
        st: st.clone(),
    }));
    let history = history_file();
    if let Some(h) = &history {
        let _ = rl.load_history(h);
    }
    loop {
        let prompt = format!("oxfer {label}:{}> ", st.lock().unwrap().cwd);
        let line = match rl.readline(&prompt) {
            Ok(l) => l,
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(line);
        let words = match shell_words::split(line) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("parse error: {e}");
                continue;
            }
        };
        let cmd = match Line::try_parse_from(&words) {
            Ok(l) => l.cmd,
            Err(e) => {
                let _ = e.print();
                continue;
            }
        };
        if matches!(cmd, Cmd::Exit) {
            break;
        }
        if let Err(e) = rt.block_on(execute(&fs, &st, cmd)) {
            eprintln!("{e:#}");
        }
    }
    if let Some(h) = &history {
        let _ = rl.save_history(h);
    }
    Ok(())
}

async fn execute(fs: &Arc<dyn RemoteFs>, st: &Shared, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Ls { path } => {
            let target = path.map_or_else(|| st.lock().unwrap().cwd.clone(), |p| rpath(st, &p));
            list(&**fs, &target).await?;
        }
        Cmd::Lls { path } => {
            let target = lpath(st, path.as_deref().unwrap_or("."));
            list(&LocalFs, &target).await?;
        }
        Cmd::Cd { path } => {
            let target = path.map_or_else(|| st.lock().unwrap().home.clone(), |p| rpath(st, &p));
            let target = fs.realpath(&target).await?;
            if !fs.stat(&target).await?.is_dir() {
                anyhow::bail!("{target} is not a directory");
            }
            st.lock().unwrap().cwd = target;
        }
        Cmd::Lcd { path } => {
            let target = lpath(st, path.as_deref().unwrap_or("~"));
            let target = LocalFs.realpath(&target).await?;
            if !LocalFs.stat(&target).await?.is_dir() {
                anyhow::bail!("{target} is not a directory");
            }
            st.lock().unwrap().lcwd = PathBuf::from(target.as_str());
        }
        Cmd::Pwd => println!("{}", st.lock().unwrap().cwd),
        Cmd::Lpwd => println!("{}", st.lock().unwrap().lcwd.display()),
        Cmd::Stat { path } => {
            println!("{}", ops::format_entry(&fs.stat(&rpath(st, &path)).await?));
        }
        Cmd::Get { paths, transfer } => {
            let (sources, dest) = match paths.as_slice() {
                [one] => (vec![rpath(st, one)], lpath(st, ".")),
                [src @ .., dest] => (src.iter().map(|s| rpath(st, s)).collect(), lpath(st, dest)),
                [] => unreachable!("clap requires one"),
            };
            ops::copy(fs.clone(), Arc::new(LocalFs), &sources, &dest, &transfer).await?;
        }
        Cmd::Put { paths, transfer } => {
            let cwd = st.lock().unwrap().cwd.clone();
            let (sources, dest) = match paths.as_slice() {
                [one] => (vec![lpath(st, one)], cwd),
                [src @ .., dest] => (src.iter().map(|s| lpath(st, s)).collect(), rpath(st, dest)),
                [] => unreachable!("clap requires one"),
            };
            ops::copy(Arc::new(LocalFs), fs.clone(), &sources, &dest, &transfer).await?;
        }
        Cmd::Mkdir { paths } => {
            for p in paths {
                fs.mkdir(&rpath(st, &p)).await?;
            }
        }
        Cmd::Rm { recursive, paths } => {
            for p in paths {
                let p = rpath(st, &p);
                if recursive {
                    oxfer_core::remove_all(&**fs, &p).await?;
                } else {
                    fs.remove(&p).await?;
                }
            }
        }
        Cmd::Mv { from, to } => fs.rename(&rpath(st, &from), &rpath(st, &to)).await?,
        Cmd::Exit => {}
    }
    Ok(())
}

async fn list(fs: &dyn RemoteFs, path: &RemotePath) -> Result<()> {
    let e = fs.stat(path).await?;
    if !e.is_dir() {
        println!("{}", ops::format_entry(&e));
        return Ok(());
    }
    let mut entries = fs.list(path).await?;
    entries.sort_by(|a, b| a.name().cmp(b.name()));
    for e in &entries {
        println!("{}", ops::format_entry(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_start_handles_escapes() {
        assert_eq!(word_start("get fo"), 4);
        assert_eq!(word_start("get my\\ fi"), 4);
        assert_eq!(word_start("ge"), 0);
        assert_eq!(word_start("get "), 4);
    }

    #[test]
    fn parses_shell_lines() {
        let p = |s: &str| Line::try_parse_from(shell_words::split(s).unwrap());
        assert!(p("get -r --overwrite skip a b").is_ok());
        assert!(p("put x").is_ok());
        assert!(p("rm -r a b").is_ok());
        assert!(matches!(p("quit").unwrap().cmd, Cmd::Exit));
        assert!(p("bogus").is_err());
        assert!(p("mv onlyone").is_err());
    }
}
