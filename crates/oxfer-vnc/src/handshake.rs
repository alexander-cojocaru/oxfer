//! RFB handshake: protocol version, security (authentication) and the
//! client/server initialisation messages.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::auth::{self, SEC_ARD, SEC_NONE, SEC_VNC};
use crate::error::{Error, Result};
use crate::ConnectOptions;

/// What the server told us about its screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInit {
    pub width: u16,
    pub height: u16,
    pub name: String,
}

/// Largest frame buffer accepted (pixels); a hostile server must not make
/// us allocate gigabytes.
pub const MAX_PIXELS: usize = 64 * 1024 * 1024;

/// Longest string (error reason, desktop name) read from the server.
const MAX_STRING: usize = 64 * 1024;

pub(crate) async fn read_string<R: AsyncRead + Unpin>(r: &mut R) -> Result<String> {
    let len = r.read_u32().await? as usize;
    if len > MAX_STRING {
        return Err(Error::protocol("string from the server is too long"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// The version we answer with. Apple's servers say "003.889", others
/// "003.008"; anything below 3.7 gets the 3.3 flow.
fn parse_version(banner: &[u8; 12]) -> Result<u32> {
    let text = std::str::from_utf8(banner)
        .ok()
        .filter(|t| t.starts_with("RFB ") && t.ends_with('\n') && t.as_bytes()[7] == b'.')
        .ok_or_else(|| Error::protocol("not a VNC server (bad version banner)"))?;
    let major: u32 = text[4..7].parse().unwrap_or(0);
    let minor: u32 = text[8..11].parse().unwrap_or(0);
    // A newer major version (recent macOS announces "005.000") is answered
    // with the newest one we speak, as the RFB specification asks.
    if major < 3 {
        return Err(Error::Unsupported(format!(
            "RFB protocol version {major}.{minor}"
        )));
    }
    Ok(match (major, minor) {
        (3, 0..=6) => 3,
        (3, 7) => 7,
        _ => 8,
    })
}

pub async fn handshake<S>(s: &mut S, opts: &ConnectOptions) -> Result<ServerInit>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut banner = [0u8; 12];
    s.read_exact(&mut banner).await?;
    let minor = parse_version(&banner)?;
    s.write_all(format!("RFB 003.00{minor}\n").as_bytes())
        .await?;

    let username = opts.username.as_deref().filter(|u| !u.is_empty());
    let password = opts.password.as_deref().filter(|p| !p.is_empty());

    // Security type.
    let chosen = if minor == 3 {
        match s.read_u32().await? {
            0 => return Err(Error::protocol(read_string(s).await?)),
            t @ (1 | 2) => t as u8,
            t => return Err(Error::Unsupported(format!("security type {t}"))),
        }
    } else {
        let n = s.read_u8().await?;
        if n == 0 {
            return Err(Error::protocol(read_string(s).await?));
        }
        let mut offered = vec![0u8; n as usize];
        s.read_exact(&mut offered).await?;
        let chosen = auth::choose_security(&offered, username.is_some(), password.is_some())?;
        s.write_u8(chosen).await?;
        chosen
    };

    match chosen {
        SEC_NONE => {}
        SEC_VNC => {
            let password = password.ok_or(Error::PasswordRequired)?;
            let mut challenge = [0u8; 16];
            s.read_exact(&mut challenge).await?;
            s.write_all(&auth::vnc_response(&challenge, password))
                .await?;
        }
        SEC_ARD => {
            let (user, password) = (
                username.ok_or(Error::UsernameRequired)?,
                password.ok_or(Error::PasswordRequired)?,
            );
            let generator = s.read_u16().await?;
            let key_len = s.read_u16().await? as usize;
            if key_len == 0 || key_len > 1024 {
                return Err(Error::protocol("bad Diffie-Hellman key length"));
            }
            let mut prime = vec![0u8; key_len];
            s.read_exact(&mut prime).await?;
            let mut server_public = vec![0u8; key_len];
            s.read_exact(&mut server_public).await?;
            let reply = auth::ard_response(
                generator,
                &prime,
                &server_public,
                user,
                password,
                auth::os_random,
            )?;
            s.write_all(&reply.credentials).await?;
            s.write_all(&reply.public_key).await?;
        }
        _ => unreachable!("only supported types are chosen"),
    }

    // SecurityResult: always sent in 3.8 and for password types before that.
    if (minor == 8 || chosen != SEC_NONE) && s.read_u32().await? != 0 {
        let reason = if minor == 8 {
            read_string(s).await.unwrap_or_default()
        } else {
            String::new()
        };
        return Err(Error::AuthFailed(if reason.is_empty() {
            "the server rejected the credentials".into()
        } else {
            reason
        }));
    }

    // ClientInit: share the desktop with other clients.
    s.write_u8(1).await?;
    s.flush().await?;

    let width = s.read_u16().await?;
    let height = s.read_u16().await?;
    let mut _pixel_format = [0u8; 16];
    s.read_exact(&mut _pixel_format).await?;
    let name = read_string(s).await?;
    check_size(width, height)?;
    Ok(ServerInit {
        width,
        height,
        name,
    })
}

pub(crate) fn check_size(width: u16, height: u16) -> Result<()> {
    if width as usize * height as usize > MAX_PIXELS {
        return Err(Error::protocol("the server's screen is too large"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banner(s: &str) -> [u8; 12] {
        s.as_bytes().try_into().unwrap()
    }

    use aes::cipher::{BlockCipherDecrypt, KeyInit};
    use md5::{Digest, Md5};
    use num_bigint::BigUint;
    use tokio::io::{duplex, DuplexStream};

    fn opts(user: Option<&str>, password: Option<&str>) -> ConnectOptions {
        let mut o = ConnectOptions::new("unused", 0);
        o.username = user.map(Into::into);
        o.password = password.map(Into::into);
        o
    }

    async fn write_str(s: &mut DuplexStream, text: &str) {
        s.write_u32(text.len() as u32).await.unwrap();
        s.write_all(text.as_bytes()).await.unwrap();
    }

    async fn finish_init(s: &mut DuplexStream) {
        assert_eq!(s.read_u8().await.unwrap(), 1, "shared flag");
        s.write_u16(800).await.unwrap();
        s.write_u16(600).await.unwrap();
        s.write_all(&[0u8; 16]).await.unwrap();
        write_str(s, "desk").await;
    }

    #[tokio::test]
    async fn vnc_password_authentication() {
        for (password, ok) in [("hunter2", true), ("wrong", false)] {
            let (mut client, mut server) = duplex(4096);
            let srv = tokio::spawn(async move {
                server.write_all(b"RFB 003.008\n").await.unwrap();
                let mut v = [0u8; 12];
                server.read_exact(&mut v).await.unwrap();
                server.write_all(&[1, 2]).await.unwrap();
                assert_eq!(server.read_u8().await.unwrap(), 2);
                let challenge = [0x5Au8; 16];
                server.write_all(&challenge).await.unwrap();
                let mut response = [0u8; 16];
                server.read_exact(&mut response).await.unwrap();
                if response == auth::vnc_response(&challenge, "hunter2") {
                    server.write_u32(0).await.unwrap();
                    finish_init(&mut server).await;
                } else {
                    server.write_u32(1).await.unwrap();
                    write_str(&mut server, "bad password").await;
                }
            });
            let result = handshake(&mut client, &opts(None, Some(password))).await;
            srv.await.unwrap();
            match (ok, result) {
                (true, Ok(init)) => assert_eq!(
                    init,
                    ServerInit {
                        width: 800,
                        height: 600,
                        name: "desk".into()
                    }
                ),
                (false, Err(Error::AuthFailed(reason))) => assert_eq!(reason, "bad password"),
                (_, other) => panic!("unexpected {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn apple_remote_desktop_authentication() {
        let (mut client, mut server) = duplex(8192);
        let srv = tokio::spawn(async move {
            server.write_all(b"RFB 003.889\n").await.unwrap();
            let mut v = [0u8; 12];
            server.read_exact(&mut v).await.unwrap();
            assert_eq!(&v, b"RFB 003.008\n");
            server.write_all(&[2, 2, 30]).await.unwrap();
            assert_eq!(server.read_u8().await.unwrap(), 30);

            let mut prime = vec![0xFFu8; 128];
            prime[127] = 0xC5;
            let p = BigUint::from_bytes_be(&prime);
            let private = BigUint::from_bytes_be(&[0x37u8; 128]);
            let public = BigUint::from(2u8).modpow(&private, &p).to_bytes_be();
            let mut public_fixed = vec![0u8; 128 - public.len()];
            public_fixed.extend(public);
            server.write_u16(2).await.unwrap();
            server.write_u16(128).await.unwrap();
            server.write_all(&prime).await.unwrap();
            server.write_all(&public_fixed).await.unwrap();

            let mut credentials = [0u8; 128];
            server.read_exact(&mut credentials).await.unwrap();
            let mut client_public = [0u8; 128];
            server.read_exact(&mut client_public).await.unwrap();
            let shared = BigUint::from_bytes_be(&client_public)
                .modpow(&private, &p)
                .to_bytes_be();
            let mut shared_fixed = vec![0u8; 128 - shared.len()];
            shared_fixed.extend(shared);
            let key: [u8; 16] = Md5::digest(&shared_fixed).into();
            let cipher = aes::Aes128::new_from_slice(&key).unwrap();
            for block in credentials.as_chunks_mut::<16>().0 {
                cipher.decrypt_block(block.into());
            }
            assert_eq!(&credentials[..6], b"alice\0");
            assert_eq!(&credentials[64..74], b"wonderland");
            assert_eq!(credentials[74], 0);
            server.write_u32(0).await.unwrap();
            finish_init(&mut server).await;
        });
        let init = handshake(&mut client, &opts(Some("alice"), Some("wonderland")))
            .await
            .unwrap();
        srv.await.unwrap();
        assert_eq!(init.name, "desk");
    }

    #[tokio::test]
    async fn needs_credentials_it_does_not_have() {
        // VNC auth only, no password.
        let (mut client, mut server) = duplex(4096);
        tokio::spawn(async move {
            server.write_all(b"RFB 003.008\n").await.unwrap();
            server.write_all(&[1, 2]).await.unwrap();
            let mut sink = vec![];
            let _ = server.read_to_end(&mut sink).await;
        });
        assert!(matches!(
            handshake(&mut client, &opts(None, None)).await,
            Err(Error::PasswordRequired)
        ));

        // ARD only, no user name.
        let (mut client, mut server) = duplex(4096);
        tokio::spawn(async move {
            server.write_all(b"RFB 003.889\n").await.unwrap();
            server.write_all(&[1, 30]).await.unwrap();
            let mut sink = vec![];
            let _ = server.read_to_end(&mut sink).await;
        });
        assert!(matches!(
            handshake(&mut client, &opts(None, Some("pw"))).await,
            Err(Error::UsernameRequired)
        ));
    }

    #[tokio::test]
    async fn server_refusal_carries_its_reason() {
        let (mut client, mut server) = duplex(4096);
        tokio::spawn(async move {
            server.write_all(b"RFB 003.008\n").await.unwrap();
            let mut v = [0u8; 12];
            server.read_exact(&mut v).await.unwrap();
            server.write_u8(0).await.unwrap();
            write_str(&mut server, "too many attempts").await;
        });
        match handshake(&mut client, &opts(None, None)).await {
            Err(Error::Protocol(m)) => assert_eq!(m, "too many attempts"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn old_servers_choose_the_security_type() {
        let (mut client, mut server) = duplex(4096);
        let srv = tokio::spawn(async move {
            server.write_all(b"RFB 003.003\n").await.unwrap();
            let mut v = [0u8; 12];
            server.read_exact(&mut v).await.unwrap();
            assert_eq!(&v, b"RFB 003.003\n");
            server.write_u32(1).await.unwrap(); // None, and no SecurityResult
            finish_init(&mut server).await;
        });
        let init = handshake(&mut client, &opts(None, None)).await.unwrap();
        srv.await.unwrap();
        assert_eq!(init.width, 800);
    }

    #[tokio::test]
    async fn rejects_servers_that_are_not_vnc() {
        let (mut client, mut server) = duplex(4096);
        tokio::spawn(async move {
            server.write_all(b"SSH-2.0-Open").await.unwrap();
        });
        assert!(matches!(
            handshake(&mut client, &opts(None, None)).await,
            Err(Error::Protocol(_))
        ));
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version(&banner("RFB 003.008\n")).unwrap(), 8);
        assert_eq!(parse_version(&banner("RFB 003.889\n")).unwrap(), 8);
        assert_eq!(parse_version(&banner("RFB 003.007\n")).unwrap(), 7);
        assert_eq!(parse_version(&banner("RFB 003.003\n")).unwrap(), 3);
        assert_eq!(parse_version(&banner("RFB 005.000\n")).unwrap(), 8);
        assert!(parse_version(&banner("RFB 002.000\n")).is_err());
        assert!(parse_version(&banner("SSH-2.0-Open")).is_err());
    }
}
