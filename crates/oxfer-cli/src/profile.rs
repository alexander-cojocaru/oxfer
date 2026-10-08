//! `oxfer profile ...`: manage saved connections.

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::{Subcommand, ValueEnum};
use oxfer_config::{AuthMethod, Config, KeyringStore, Profile, Protocol, Secret, SecretStore};

#[derive(Clone, Copy, ValueEnum)]
pub enum Proto {
    Sftp,
    Ftp,
    /// FTP over explicit TLS (AUTH TLS)
    Ftpes,
    /// FTP over implicit TLS
    Ftps,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum AuthKind {
    Agent,
    Key,
    Password,
    Interactive,
}

#[derive(Subcommand)]
pub enum Action {
    /// List saved profiles
    List,
    /// Show one profile
    Show { name: String },
    /// Add or replace a profile
    Add {
        name: String,
        host: String,
        #[arg(long, value_enum, default_value = "sftp")]
        protocol: Proto,
        #[arg(long)]
        port: Option<u16>,
        #[arg(short, long)]
        user: Option<String>,
        #[arg(long, value_enum, default_value = "agent")]
        auth: AuthKind,
        /// Key file (with --auth key)
        #[arg(long)]
        key_file: Option<PathBuf>,
        /// Initial remote directory
        #[arg(long)]
        dir: Option<String>,
        /// Replace an existing profile of the same name
        #[arg(long)]
        force: bool,
    },
    /// Remove a profile (and its stored password)
    Rm { name: String },
}

fn describe(p: &Profile) -> String {
    let auth = match &p.auth {
        AuthMethod::Agent => "agent".to_string(),
        AuthMethod::KeyFile { path } => format!("key {}", path.display()),
        AuthMethod::Password => "password (keychain)".into(),
        AuthMethod::Interactive => "interactive".into(),
    };
    format!(
        "{} -> {:?} {}@{}:{} [{}]{}",
        p.name,
        p.protocol,
        p.user.as_deref().unwrap_or("-"),
        p.host,
        p.effective_port(),
        auth,
        p.remote_dir
            .as_ref()
            .map_or(String::new(), |d| format!(" dir={d}"))
    )
}

pub fn run(action: &Action) -> Result<()> {
    let mut config = Config::load()?;
    match action {
        Action::List => {
            for p in config.list() {
                println!("{}", describe(p));
            }
        }
        Action::Show { name } => match config.get(name) {
            Some(p) => println!("{}", describe(p)),
            None => bail!("no profile named {name:?}"),
        },
        Action::Add {
            name,
            host,
            protocol,
            port,
            user,
            auth,
            key_file,
            dir,
            force,
        } => {
            let protocol = match protocol {
                Proto::Sftp => Protocol::Sftp,
                Proto::Ftp => Protocol::Ftp,
                Proto::Ftpes => Protocol::FtpsExplicit,
                Proto::Ftps => Protocol::FtpsImplicit,
            };
            let auth_method = match auth {
                AuthKind::Agent => AuthMethod::Agent,
                AuthKind::Interactive => AuthMethod::Interactive,
                AuthKind::Password => AuthMethod::Password,
                AuthKind::Key => match key_file {
                    Some(path) => AuthMethod::KeyFile { path: path.clone() },
                    None => bail!("--auth key needs --key-file"),
                },
            };
            let mut profile = Profile::new(name.clone(), protocol, host.clone(), auth_method);
            profile.port = *port;
            profile.user = user.clone();
            profile.remote_dir = dir.clone();
            if config.get(name).is_some() {
                if !force {
                    bail!("profile {name:?} exists (use --force to replace)");
                }
                config.remove(name)?;
            }
            config.add(profile)?;
            if matches!(auth, AuthKind::Password) {
                let pw = rpassword::prompt_password(format!("Password for profile {name}: "))?;
                KeyringStore.set(name, &Secret::new(pw))?;
            }
            config.save()?;
            eprintln!("saved profile {name}");
        }
        Action::Rm { name } => {
            let removed = config.remove(name)?;
            config.save()?;
            if removed.auth == AuthMethod::Password {
                if let Err(e) = KeyringStore.delete(name) {
                    eprintln!("warning: could not remove stored password: {e}");
                }
            }
        }
    }
    Ok(())
}
