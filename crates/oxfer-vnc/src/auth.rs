//! The two password authentications: classic VNC authentication (a DES
//! challenge) and Apple Remote Desktop (Diffie-Hellman + AES, with a user
//! name), which is what macOS Screen Sharing speaks.

use aes::cipher::{BlockCipherEncrypt, KeyInit};
use md5::{Digest, Md5};
use num_bigint::BigUint;

use crate::error::{Error, Result};

pub const SEC_NONE: u8 = 1;
pub const SEC_VNC: u8 = 2;
pub const SEC_ARD: u8 = 30;

/// Pick the security type to use out of what the server offers.
pub fn choose_security(offered: &[u8], has_user: bool, has_password: bool) -> Result<u8> {
    let has = |t: u8| offered.contains(&t);
    if has_user && has(SEC_ARD) {
        return if has_password {
            Ok(SEC_ARD)
        } else {
            Err(Error::PasswordRequired)
        };
    }
    if has(SEC_VNC) {
        if has_password {
            return Ok(SEC_VNC);
        }
        return if has(SEC_NONE) {
            Ok(SEC_NONE)
        } else {
            Err(Error::PasswordRequired)
        };
    }
    if has(SEC_NONE) {
        return Ok(SEC_NONE);
    }
    if has(SEC_ARD) {
        return Err(Error::UsernameRequired);
    }
    Err(Error::Unsupported(format!(
        "none of the server's security types {offered:?} is supported \
         (supported: none, VNC password, Apple Remote Desktop)"
    )))
}

/// The response to a classic VNC authentication challenge: the challenge
/// DES-encrypted with the password (first 8 bytes, each byte bit-reversed).
pub fn vnc_response(challenge: &[u8; 16], password: &str) -> [u8; 16] {
    let mut key = [0u8; 8];
    for (k, b) in key.iter_mut().zip(password.bytes()) {
        *k = b.reverse_bits();
    }
    let cipher = des::Des::new_from_slice(&key).expect("DES takes an 8-byte key");
    let mut out = *challenge;
    for block in out.as_chunks_mut::<8>().0 {
        cipher.encrypt_block(block.into());
    }
    out
}

/// What the client sends for Apple Remote Desktop authentication.
pub struct ArdReply {
    /// 128 bytes: user name and password, AES-128-ECB encrypted.
    pub credentials: [u8; 128],
    /// The client's Diffie-Hellman public key (as long as the prime).
    pub public_key: Vec<u8>,
}

fn to_fixed_be(n: &BigUint, len: usize) -> Vec<u8> {
    let bytes = n.to_bytes_be();
    let mut out = vec![0u8; len.saturating_sub(bytes.len())];
    out.extend_from_slice(&bytes[bytes.len().saturating_sub(len)..]);
    out
}

/// Apple Remote Desktop authentication: Diffie-Hellman with the server's
/// parameters, MD5 of the shared secret as an AES-128 key for the credentials.
/// `fill_random` supplies random bytes (the OS RNG outside of tests).
pub fn ard_response(
    generator: u16,
    prime: &[u8],
    server_public: &[u8],
    username: &str,
    password: &str,
    mut fill_random: impl FnMut(&mut [u8]),
) -> Result<ArdReply> {
    if prime.is_empty() || prime.len() > 1024 || server_public.len() != prime.len() {
        return Err(Error::protocol("bad Diffie-Hellman parameters"));
    }
    let p = BigUint::from_bytes_be(prime);
    let g = BigUint::from(generator);
    let server_public = BigUint::from_bytes_be(server_public);
    if p < BigUint::from(5u8) || g < BigUint::from(2u8) || server_public <= BigUint::from(1u8) {
        return Err(Error::protocol("bad Diffie-Hellman parameters"));
    }

    let mut private = vec![0u8; prime.len()];
    fill_random(&mut private);
    let private = BigUint::from_bytes_be(&private);
    let public_key = to_fixed_be(&g.modpow(&private, &p), prime.len());
    let shared = to_fixed_be(&server_public.modpow(&private, &p), prime.len());
    let key: [u8; 16] = Md5::digest(&shared).into();

    // Fixed-size fields, padded with random bytes after the terminating NUL.
    let mut credentials = [0u8; 128];
    fill_random(&mut credentials);
    for (field, value) in [(0usize, username), (64, password)] {
        let bytes = value.as_bytes();
        let n = bytes.len().min(63);
        credentials[field..field + n].copy_from_slice(&bytes[..n]);
        credentials[field + n] = 0;
    }
    let cipher = aes::Aes128::new_from_slice(&key).expect("AES-128 takes a 16-byte key");
    for block in credentials.as_chunks_mut::<16>().0 {
        cipher.encrypt_block(block.into());
    }
    Ok(ArdReply {
        credentials,
        public_key,
    })
}

/// Random bytes from the operating system.
pub fn os_random(buf: &mut [u8]) {
    getrandom::fill(buf).expect("the operating system provides random bytes");
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockCipherDecrypt;

    #[test]
    fn security_choice() {
        use Error::*;
        assert_eq!(choose_security(&[1], false, false).unwrap(), SEC_NONE);
        assert_eq!(choose_security(&[2], false, true).unwrap(), SEC_VNC);
        assert!(matches!(
            choose_security(&[2], false, false),
            Err(PasswordRequired)
        ));
        // None wins over VNC auth only when there is no password.
        assert_eq!(choose_security(&[1, 2], false, false).unwrap(), SEC_NONE);
        assert_eq!(choose_security(&[1, 2], false, true).unwrap(), SEC_VNC);
        // A user name selects ARD.
        assert_eq!(choose_security(&[2, 30], true, true).unwrap(), SEC_ARD);
        assert!(matches!(
            choose_security(&[30], true, false),
            Err(PasswordRequired)
        ));
        // ARD alone needs a user name; VNC auth is used when it is offered too.
        assert!(matches!(
            choose_security(&[30], false, true),
            Err(UsernameRequired)
        ));
        assert_eq!(choose_security(&[30, 2], false, true).unwrap(), SEC_VNC);
        assert!(matches!(
            choose_security(&[18], true, true),
            Err(Unsupported(_))
        ));
    }

    #[test]
    fn des_matches_the_standard_test_vector() {
        // FIPS 46 example: key 133457799BBCDFF1 encrypts 0123456789ABCDEF
        // to 85E813540F0AB405.
        let key = [0x13u8, 0x34, 0x57, 0x79, 0x9B, 0xBC, 0xDF, 0xF1];
        let cipher = des::Des::new_from_slice(&key).unwrap();
        let mut block = [0x01u8, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        cipher.encrypt_block((&mut block).into());
        assert_eq!(block, [0x85, 0xE8, 0x13, 0x54, 0x0F, 0x0A, 0xB4, 0x05]);
    }

    #[test]
    fn vnc_response_is_per_block_and_password_dependent() {
        let challenge = [7u8; 16];
        let a = vnc_response(&challenge, "secret");
        assert_eq!(a[..8], a[8..], "equal blocks encrypt equally (ECB)");
        assert_ne!(a, challenge);
        assert_ne!(a, vnc_response(&challenge, "Secret"));
        // Only the first 8 characters count.
        assert_eq!(
            vnc_response(&challenge, "12345678"),
            vnc_response(&challenge, "12345678XYZ")
        );
    }

    #[test]
    fn ard_round_trip_with_a_server() {
        let mut prime = vec![0xFFu8; 128];
        prime[127] = 0xC5;
        let generator = 2u16;
        let p = BigUint::from_bytes_be(&prime);
        let g = BigUint::from(generator);
        let server_private = BigUint::from_bytes_be(&[0x42u8; 128]);
        let server_public = to_fixed_be(&g.modpow(&server_private, &p), 128);

        let mut counter = 0u8;
        let reply = ard_response(
            generator,
            &prime,
            &server_public,
            "alice",
            "wonderland",
            |b| {
                for x in b.iter_mut() {
                    counter = counter.wrapping_mul(31).wrapping_add(17);
                    *x = counter;
                }
            },
        )
        .unwrap();
        assert_eq!(reply.public_key.len(), 128);

        // The server derives the same key and can read the credentials.
        let shared = to_fixed_be(
            &BigUint::from_bytes_be(&reply.public_key).modpow(&server_private, &p),
            128,
        );
        let key: [u8; 16] = Md5::digest(&shared).into();
        let cipher = aes::Aes128::new_from_slice(&key).unwrap();
        let mut plain = reply.credentials;
        for block in plain.as_chunks_mut::<16>().0 {
            cipher.decrypt_block(block.into());
        }
        let cstr = |s: &[u8]| {
            String::from_utf8(s.iter().copied().take_while(|b| *b != 0).collect()).unwrap()
        };
        assert_eq!(cstr(&plain[..64]), "alice");
        assert_eq!(cstr(&plain[64..]), "wonderland");
    }

    #[test]
    fn ard_rejects_bad_parameters() {
        let prime = vec![0xFFu8; 16];
        assert!(ard_response(2, &prime, &[1u8; 8], "u", "p", os_random).is_err());
        assert!(ard_response(2, &[], &[], "u", "p", os_random).is_err());
        assert!(ard_response(2, &prime, &[0u8; 16], "u", "p", os_random).is_err());
    }

    #[test]
    fn long_credentials_are_truncated() {
        let mut prime = vec![0xFFu8; 16];
        prime[15] = 0xC5;
        let public = vec![5u8; 16];
        let long = "x".repeat(200);
        assert!(ard_response(2, &prime, &public, &long, &long, os_random).is_ok());
    }
}
