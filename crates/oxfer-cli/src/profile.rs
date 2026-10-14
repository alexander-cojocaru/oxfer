//! `oxfer profile ...`: manage saved connections.

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::{Subcommand, ValueEnum};
use oxfer_config::{
    AuthMethod, Config, Credential, KeyringStore, Profile, Protocol, Secret, SecretStore,
};

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
    /// List saved profiles, by group
    List {
        /// Only profiles matching all these words (name, host, user, group, directory)
        query: Vec<String>,
        /// Only profiles in this group
        #[arg(short, long)]
        group: Option<String>,
    },
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
        /// Group (folder) to list the profile under; use `/` for sub groups (`Customers/ACME`)
        #[arg(short, long)]
        group: Option<String>,
        /// Use a saved credential (see `oxfer credential`) for user and authentication
        #[arg(short, long, conflicts_with_all = ["auth", "key_file"])]
        credential: Option<String>,
        /// Replace an existing profile of the same name
        #[arg(long)]
        force: bool,
    },
    /// Remove a profile (and its stored password)
    Rm { name: String },
    /// Write all profiles and credentials to a file (passwords are never included)
    Export { file: PathBuf },
    /// Merge profiles and credentials from an exported file
    Import {
        file: PathBuf,
        /// Replace existing profiles/credentials of the same name
        #[arg(long)]
        overwrite: bool,
    },
}

/// Commands of `oxfer credential`.
#[derive(Subcommand)]
pub enum CredentialAction {
    /// List saved credentials
    List,
    /// Add or replace a credential
    Add {
        name: String,
        #[arg(short, long)]
        user: Option<String>,
        #[arg(long, value_enum, default_value = "password")]
        auth: AuthKind,
        /// Key file (with --auth key)
        #[arg(long)]
        key_file: Option<PathBuf>,
        /// Replace an existing credential of the same name
        #[arg(long)]
        force: bool,
    },
    /// Remove a credential no profile uses (and its stored password)
    Rm { name: String },
}

fn auth_method(auth: AuthKind, key_file: &Option<PathBuf>) -> Result<AuthMethod> {
    Ok(match auth {
        AuthKind::Agent => AuthMethod::Agent,
        AuthKind::Interactive => AuthMethod::Interactive,
        AuthKind::Password => AuthMethod::Password,
        AuthKind::Key => match key_file {
            Some(path) => AuthMethod::KeyFile { path: path.clone() },
            None => bail!("--auth key needs --key-file"),
        },
    })
}

fn describe_auth(auth: &AuthMethod) -> String {
    match auth {
        AuthMethod::Agent => "agent".to_string(),
        AuthMethod::KeyFile { path } => format!("key {}", path.display()),
        AuthMethod::Password => "password (keychain)".into(),
        AuthMethod::Interactive => "interactive".into(),
    }
}

fn describe(p: &Profile) -> String {
    let auth = match &p.credential {
        Some(c) => format!("credential {c}"),
        None => describe_auth(&p.auth),
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
        Action::List { query, group } => {
            let query = query.join(" ");
            let mut found: Vec<&Profile> = config
                .list()
                .iter()
                .filter(|p| group.is_none() || p.group == *group)
                .filter(|p| p.matches(&query))
                .collect();
            // Ungrouped first, then by group and name.
            found.sort_by_cached_key(|p| {
                (
                    p.group.as_ref().map(|g| g.to_lowercase()),
                    p.name.to_lowercase(),
                )
            });
            let mut current = None;
            for p in found {
                if p.group.is_some() && p.group.as_ref() != current {
                    println!("{}:", p.group.as_deref().unwrap_or_default());
                    current = p.group.as_ref();
                }
                let indent = if p.group.is_some() { "  " } else { "" };
                println!("{indent}{}", describe(p));
            }
        }
        Action::Export { file } => {
            config.save_to(file)?;
            println!(
                "exported {} profiles and {} credentials to {} (without passwords)",
                config.list().len(),
                config.credentials().len(),
                file.display()
            );
        }
        Action::Import { file, overwrite } => {
            if !file.exists() {
                bail!("{} does not exist", file.display());
            }
            let incoming = Config::load_from(file)?;
            let report = config.merge(&incoming, *overwrite)?;
            config.save()?;
            println!(
                "imported: {} added, {} replaced, {} skipped (already exist; use --overwrite)",
                report.added.len(),
                report.replaced.len(),
                report.skipped.len()
            );
            println!("passwords are not exported: set them again for password logins");
        }
        Action::Show { name } => match config.get(name) {
            Some(p) => {
                println!("{}", describe(p));
                if let Some(g) = &p.group {
                    println!("group: {g}");
                }
            }
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
            group,
            credential,
            force,
        } => {
            let protocol = match protocol {
                Proto::Sftp => Protocol::Sftp,
                Proto::Ftp => Protocol::Ftp,
                Proto::Ftpes => Protocol::FtpsExplicit,
                Proto::Ftps => Protocol::FtpsImplicit,
            };
            let auth_method = match credential {
                Some(c) if config.credential(c).is_none() => {
                    bail!("no credential named {c:?} (see `oxfer credential list`)")
                }
                // Ignored while a credential is set.
                Some(_) => AuthMethod::Agent,
                None => auth_method(*auth, key_file)?,
            };
            let mut profile = Profile::new(name.clone(), protocol, host.clone(), auth_method);
            profile.port = *port;
            profile.user = user.clone();
            profile.remote_dir = dir.clone();
            profile.group = group.clone();
            profile.credential = credential.clone();
            if config.get(name).is_some() {
                if !force {
                    bail!("profile {name:?} exists (use --force to replace)");
                }
                config.remove(name)?;
            }
            config.add(profile)?;
            if credential.is_none() && matches!(auth, AuthKind::Password) {
                let pw = rpassword::prompt_password(format!("Password for profile {name}: "))?;
                KeyringStore.set(name, &Secret::new(pw))?;
            }
            config.save()?;
            eprintln!("saved profile {name}");
        }
        Action::Rm { name } => {
            let removed = config.remove(name)?;
            config.save()?;
            if removed.credential.is_none() && removed.auth == AuthMethod::Password {
                if let Err(e) = KeyringStore.delete(name) {
                    eprintln!("warning: could not remove stored password: {e}");
                }
            }
        }
    }
    Ok(())
}

pub fn run_credential(action: &CredentialAction) -> Result<()> {
    let mut config = Config::load()?;
    match action {
        CredentialAction::List => {
            for c in config.credentials() {
                let used = config
                    .list()
                    .iter()
                    .filter(|p| p.credential.as_deref() == Some(c.name.as_str()))
                    .count();
                println!(
                    "{} -> {} [{}] used by {used} profile(s)",
                    c.name,
                    c.user.as_deref().unwrap_or("-"),
                    describe_auth(&c.auth)
                );
            }
        }
        CredentialAction::Add {
            name,
            user,
            auth,
            key_file,
            force,
        } => {
            let credential =
                Credential::new(name.clone(), user.clone(), auth_method(*auth, key_file)?);
            if config.credential(name).is_some() {
                if !force {
                    bail!("credential {name:?} exists (use --force to replace)");
                }
                config.replace_credential(name, credential.clone())?;
            } else {
                config.add_credential(credential.clone())?;
            }
            if matches!(auth, AuthKind::Password) {
                let pw = rpassword::prompt_password(format!("Password for credential {name}: "))?;
                KeyringStore.set(&credential.secret_key(), &Secret::new(pw))?;
            } else if let Err(e) = KeyringStore.delete(&credential.secret_key()) {
                eprintln!("warning: could not remove stored password: {e}");
            }
            config.save()?;
            eprintln!("saved credential {name}");
        }
        CredentialAction::Rm { name } => {
            let removed = config.remove_credential(name)?;
            config.save()?;
            if let Err(e) = KeyringStore.delete(&removed.secret_key()) {
                eprintln!("warning: could not remove stored password: {e}");
            }
        }
    }
    Ok(())
}
