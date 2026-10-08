//! FTPS (explicit `AUTH TLS` and implicit) against an in-process TLS mock with
//! a certificate generated at test time.

mod common;

use std::sync::Arc;

use common::*;
use oxfer_core::{EntryKind, Error, RemoteFs, RemotePath};
use oxfer_ftp::{FtpFs, TlsMode};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::ServerConfig;

const FLAVOR: Flavor = Flavor {
    mlsd: true,
    epsv: true,
};

struct Cert {
    pem: Vec<u8>,
    config: Arc<ServerConfig>,
}

/// Self-signed certificate for `127.0.0.1` / `localhost`.
fn make_cert() -> Cert {
    let ck =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".into(), "localhost".into()]).unwrap();
    let der = CertificateDer::from(ck.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()));
    let config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![der], key)
    .unwrap();
    Cert {
        pem: ck.cert.pem().into_bytes(),
        config: Arc::new(config),
    }
}

fn mode(sec: Security) -> TlsMode {
    match sec {
        Security::Explicit => TlsMode::Explicit,
        Security::Implicit => TlsMode::Implicit,
        Security::Plain => unreachable!(),
    }
}

const MODES: [Security; 2] = [Security::Explicit, Security::Implicit];

async fn exercise(fs: &FtpFs, mock: &Mock) {
    std::fs::write(mock.dir.join("a.txt"), b"hello").unwrap();
    let entries = fs.list(&RemotePath::root()).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name(), "a.txt");
    assert_eq!(entries[0].kind, EntryKind::File);

    let data = pattern(300_000);
    put(fs, "/big.bin", 0, &data).await;
    assert_eq!(fs.stat(&rp("/big.bin")).await.unwrap().size, 300_000);
    assert_eq!(get(fs, "/big.bin", 0).await, data);
    // the bytes on disk are the plaintext, i.e. the TLS layer was transparent
    assert_eq!(std::fs::read(mock.dir.join("big.bin")).unwrap(), data);

    fs.mkdir(&rp("/d")).await.unwrap();
    assert!(fs.stat(&rp("/d")).await.unwrap().is_dir());
    fs.remove(&rp("/d")).await.unwrap();
    fs.remove(&rp("/big.bin")).await.unwrap();
    assert!(!mock.dir.join("d").exists());
    assert!(!mock.dir.join("big.bin").exists());
}

#[tokio::test]
async fn full_session_accepting_invalid_certs() {
    let cert = make_cert();
    for sec in MODES {
        let mock = Mock::start_secure(FLAVOR, sec, Some(cert.config.clone())).await;
        let mut o = mock.opts();
        o.tls = mode(sec);
        o.danger_accept_invalid_certs = true;
        let fs = FtpFs::connect(o).await.unwrap();
        exercise(&fs, &mock).await;
    }
}

#[tokio::test]
async fn default_verification_rejects_self_signed() {
    let cert = make_cert();
    for sec in MODES {
        let mock = Mock::start_secure(FLAVOR, sec, Some(cert.config.clone())).await;
        let mut o = mock.opts();
        o.tls = mode(sec);
        assert!(!o.danger_accept_invalid_certs && o.extra_root_certs_pem.is_none());
        let r = FtpFs::connect(o).await;
        let Err(e) = r else {
            panic!("{sec:?}: connect succeeded without trusting the certificate")
        };
        assert!(!matches!(e, Error::Auth(_)), "{sec:?}: {e:?}");
        let msg = format!("{e:?}").to_lowercase();
        eprintln!("{sec:?} rejected: {e:?}");
        assert!(
            msg.contains("certificate") || msg.contains("unknownissuer"),
            "{sec:?}: {msg}"
        );
    }
}

#[tokio::test]
async fn verification_on_with_trusted_extra_root() {
    let cert = make_cert();
    for sec in MODES {
        let mock = Mock::start_secure(FLAVOR, sec, Some(cert.config.clone())).await;
        let mut o = mock.opts();
        o.tls = mode(sec);
        o.extra_root_certs_pem = Some(cert.pem.clone());
        let fs = FtpFs::connect(o).await.unwrap();
        exercise(&fs, &mock).await;
    }
}

#[tokio::test]
async fn trusting_a_different_cert_still_fails() {
    let server = make_cert();
    let other = make_cert();
    for sec in MODES {
        let mock = Mock::start_secure(FLAVOR, sec, Some(server.config.clone())).await;
        let mut o = mock.opts();
        o.tls = mode(sec);
        o.extra_root_certs_pem = Some(other.pem.clone());
        assert!(FtpFs::connect(o).await.is_err(), "{sec:?}");
    }
}

#[tokio::test]
async fn invalid_extra_root_pem_is_an_error() {
    let cert = make_cert();
    let mock = Mock::start_secure(FLAVOR, Security::Explicit, Some(cert.config.clone())).await;
    let mut o = mock.opts();
    o.tls = TlsMode::Explicit;
    o.extra_root_certs_pem =
        Some(b"-----BEGIN CERTIFICATE-----\n!!!notbase64\n-----END CERTIFICATE-----\n".to_vec());
    assert!(FtpFs::connect(o).await.is_err());
}
