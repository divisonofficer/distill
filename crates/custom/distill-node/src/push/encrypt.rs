//! RFC 8291 payload encryption with RFC 8188 `aes128gcm` framing (single record).

use anyhow::anyhow;
use ring::aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::agreement::{self, ECDH_P256, EphemeralPrivateKey, UnparsedPublicKey};
use ring::error::Unspecified;
use ring::hkdf::{HKDF_SHA256, KeyType, Salt};
use ring::rand::{SecureRandom, SystemRandom};

const RECORD_SIZE: u32 = 4096;
const TAG_LEN: usize = 16;
const PAD_DELIMITER: u8 = 0x02;

struct Len(usize);

impl KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

/// Encrypts `plaintext` for a subscriber with uncompressed P-256 key `ua_public` and `auth_secret`.
pub fn encrypt(ua_public: &[u8], auth_secret: &[u8], plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let rng = SystemRandom::new();
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| anyhow!("random salt"))?;
    let private =
        EphemeralPrivateKey::generate(&ECDH_P256, &rng).map_err(|_| anyhow!("ephemeral key"))?;
    let as_public = private
        .compute_public_key()
        .map_err(|_| anyhow!("ephemeral public key"))?;
    let peer = UnparsedPublicKey::new(&ECDH_P256, ua_public);
    let body = agreement::agree_ephemeral(private, &peer, |secret| {
        encrypt_with_secret(
            secret,
            auth_secret,
            ua_public,
            as_public.as_ref(),
            &salt,
            plaintext,
        )
    })
    .map_err(|_| anyhow!("ECDH with subscriber key failed"))?;
    body.map_err(|_| anyhow!("payload encryption failed"))
}

/// Key derivation, header and AEAD step for a known ECDH secret (RFC 8291 §3.3, RFC 8188 §2).
/// `as_public` is the sender's 65-byte ephemeral public key, written as the header keyid.
pub fn encrypt_with_secret(
    ecdh_secret: &[u8],
    auth_secret: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, Unspecified> {
    if plaintext.len() + 1 + TAG_LEN > RECORD_SIZE as usize {
        return Err(Unspecified);
    }
    let idlen = u8::try_from(as_public.len()).map_err(|_| Unspecified)?;

    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    Salt::new(HKDF_SHA256, auth_secret)
        .extract(ecdh_secret)
        .expand(&[key_info.as_slice()], Len(ikm.len()))?
        .fill(&mut ikm)?;

    let prk = Salt::new(HKDF_SHA256, salt).extract(&ikm);
    let mut cek = [0u8; 16];
    prk.expand(
        &[b"Content-Encoding: aes128gcm\0".as_slice()],
        Len(cek.len()),
    )?
    .fill(&mut cek)?;
    let mut nonce = [0u8; 12];
    prk.expand(&[b"Content-Encoding: nonce\0".as_slice()], Len(nonce.len()))?
        .fill(&mut nonce)?;

    let mut record = Vec::with_capacity(plaintext.len() + 1 + TAG_LEN);
    record.extend_from_slice(plaintext);
    record.push(PAD_DELIMITER);
    let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek)?);
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::empty(),
        &mut record,
    )?;

    let mut out = Vec::with_capacity(16 + 4 + 1 + as_public.len() + record.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(idlen);
    out.extend_from_slice(as_public);
    out.extend_from_slice(&record);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn d(s: &str) -> Vec<u8> {
        URL_SAFE_NO_PAD.decode(s).unwrap()
    }

    // RFC 8291 Appendix A / Section 5.
    const AS_PUBLIC: &str =
        "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const AUTH_SECRET: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const ECDH_SECRET: &str = "kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs";
    const PLAINTEXT: &str = "When I grow up, I want to be a watermelon";
    const BODY: &str = concat!(
        "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml",
        "mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT",
        "pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN",
    );

    #[test]
    fn matches_rfc8291_vector() {
        let salt: [u8; 16] = d(SALT).try_into().unwrap();
        let out = encrypt_with_secret(
            &d(ECDH_SECRET),
            &d(AUTH_SECRET),
            &d(UA_PUBLIC),
            &d(AS_PUBLIC),
            &salt,
            PLAINTEXT.as_bytes(),
        )
        .unwrap();
        assert_eq!(URL_SAFE_NO_PAD.encode(&out), BODY);
    }
}
