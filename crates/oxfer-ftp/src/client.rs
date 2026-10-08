use std::sync::Arc;
use std::time::Duration;

use oxfer_core::{Error, RemotePath, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{aws_lc_rs, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::{Mode, Status};

use crate::fs::{map_err, Plain, State};
use crate::FtpFs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TlsMode {
    /// Plain FTP. Credentials and data travel unencrypted.
    None,
    /// `AUTH TLS` upgrade on the regular control port (FTPES).
    #[default]
    Explicit,
    /// TLS from the first byte, usually on port 990.
    Implicit,
}

#[derive(Clone)]
pub struct FtpConnectOptions {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub tls: TlsMode,
    /// Skip certificate validation (self-signed servers). Insecure.
    pub danger_accept_invalid_certs: bool,
    /// Applies to establishing the connection and logging in.
    pub timeout: Duration,
}

impl FtpConnectOptions {
    /// Defaults: port 21, explicit TLS with certificate verification.
    pub fn new(
        host: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port: 21,
            user: user.into(),
            password: password.into(),
            tls: TlsMode::Explicit,
            danger_accept_invalid_certs: false,
            timeout: Duration::from_secs(30),
        }
    }
}

impl std::fmt::Debug for FtpConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtpConnectOptions")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"..")
            .field("tls", &self.tls)
            .field(
                "danger_accept_invalid_certs",
                &self.danger_accept_invalid_certs,
            )
            .finish()
    }
}

#[derive(Debug)]
struct AcceptAny(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn load_roots() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for e in &native.errors {
        tracing::debug!("loading native certificates: {e}");
    }
    roots.add_parsable_certificates(native.certs);
    if roots.is_empty() {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    roots
}

async fn tls_config(accept_invalid: bool) -> Result<Arc<ClientConfig>> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Protocol(e.to_string()))?;
    let config = if accept_invalid {
        tracing::warn!("TLS certificate verification is disabled");
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
            .with_no_client_auth()
    } else {
        let roots = tokio::task::spawn_blocking(load_roots)
            .await
            .map_err(|e| Error::Protocol(e.to_string()))?;
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    Ok(Arc::new(config))
}

fn has(features: &suppaftp::types::Features, name: &str) -> bool {
    features.keys().any(|k| k.eq_ignore_ascii_case(name))
}

impl FtpFs {
    /// Connect, optionally upgrade to TLS, log in and prepare the session
    /// (binary type, UTF-8, EPSV if the server supports it).
    pub async fn connect(opts: FtpConnectOptions) -> Result<Self> {
        if opts.tls == TlsMode::None {
            tracing::warn!(
                host = %opts.host,
                "connecting over plain FTP: credentials and data are not encrypted"
            );
        }
        tokio::time::timeout(opts.timeout, connect(&opts))
            .await
            .map_err(|_| Error::Connection(format!("timed out connecting to {}", opts.host)))?
    }
}

async fn connect(opts: &FtpConnectOptions) -> Result<FtpFs> {
    let root = RemotePath::root();
    let err = |e| map_err(e, &root, Plain::NotFound);
    let addr = (opts.host.as_str(), opts.port);
    let mut ftp = match opts.tls {
        TlsMode::None => AsyncRustlsFtpStream::connect(addr).await.map_err(err)?,
        TlsMode::Explicit | TlsMode::Implicit => {
            let config = tls_config(opts.danger_accept_invalid_certs).await?;
            let connector = AsyncRustlsConnector::from(tokio_rustls::TlsConnector::from(config));
            if opts.tls == TlsMode::Explicit {
                AsyncRustlsFtpStream::connect(addr)
                    .await
                    .map_err(err)?
                    .into_secure(connector, &opts.host)
                    .await
                    .map_err(err)?
            } else {
                AsyncRustlsFtpStream::connect_secure_implicit(addr, connector, &opts.host)
                    .await
                    .map_err(err)?
            }
        }
    };
    // Servers behind NAT often advertise an unroutable address in PASV replies.
    ftp.set_passive_nat_workaround(true);
    ftp.login(&opts.user, &opts.password).await.map_err(err)?;
    ftp.transfer_type(FileType::Binary).await.map_err(err)?;

    let features = ftp.feat().await.unwrap_or_default();
    if has(&features, "UTF8") {
        // Best effort: most servers default to UTF-8 anyway.
        let _ = ftp
            .custom_command("OPTS UTF8 ON", &[Status::CommandOk])
            .await;
    }
    // EPSV is not always advertised in FEAT, so probe it.
    if ftp
        .custom_command("EPSV", &[Status::ExtendedPassiveMode])
        .await
        .is_ok()
    {
        ftp.set_mode(Mode::ExtendedPassive);
    }
    Ok(FtpFs::from_state(State {
        ftp,
        mlst: has(&features, "MLST"),
    }))
}
