//! Google refresh tokens at rest: XChaCha20-Poly1305 with the member id as associated data, so a
//! ciphertext copied onto another member's row does not decrypt. The key is never stored with the
//! data. Tokens and keys are never logged or put in error text.
use crate::db::now_ms;
use anyhow::{anyhow, bail, Result};
use chacha20poly1305::{
    aead::{Aead, Generate, Payload},
    KeyInit, XChaCha20Poly1305, XNonce,
};
use rusqlite::{params, Connection, OptionalExtension};

/// The 32-byte secret key, read from `RECAPZ_SECRET_KEY` as 64 hex characters.
pub struct SecretKey([u8; 32]);

impl SecretKey {
    pub fn from_hex(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("secret key must be 64 hex characters");
        }
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
                .map_err(|_| anyhow!("secret key must be 64 hex characters"))?;
        }
        Ok(Self(k))
    }
}

/// Stores or replaces the member's refresh token. `scopes` is the space-separated granted list.
pub fn save(
    c: &Connection,
    key: &SecretKey,
    member_id: &str,
    refresh_token: &str,
    scopes: &str,
) -> Result<()> {
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    let nonce = XNonce::try_generate().map_err(|_| anyhow!("no randomness"))?;
    let blob = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: refresh_token.as_bytes(),
                aad: member_id.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("could not encrypt token"))?;
    let now = now_ms();
    c.execute(
        "INSERT INTO google_tokens (member_id, refresh_token, nonce, scopes, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(member_id) DO UPDATE SET refresh_token = ?2, nonce = ?3, scopes = ?4, updated_at = ?5",
        params![member_id, blob, nonce.as_slice(), scopes, now],
    )?;
    Ok(())
}

/// The member's refresh token and granted scopes, or `None` when not connected. A wrong key or a
/// tampered row is an error, not `None`.
pub fn load(c: &Connection, key: &SecretKey, member_id: &str) -> Result<Option<(String, String)>> {
    let row: Option<(Vec<u8>, Vec<u8>, String)> = c
        .query_row(
            "SELECT refresh_token, nonce, scopes FROM google_tokens WHERE member_id = ?1",
            [member_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((blob, nonce, scopes)) = row else {
        return Ok(None);
    };
    if nonce.len() != 24 {
        bail!("stored token is damaged");
    }
    let plain = XChaCha20Poly1305::new((&key.0).into())
        .decrypt(
            &XNonce::try_from(nonce.as_slice()).map_err(|_| anyhow!("stored token is damaged"))?,
            Payload {
                msg: &blob,
                aad: member_id.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("could not decrypt token (wrong key or damaged row)"))?;
    Ok(Some((String::from_utf8(plain)?, scopes)))
}

/// Hard delete (disconnect or revoked grant).
pub fn delete(c: &Connection, member_id: &str) -> Result<()> {
    c.execute(
        "DELETE FROM google_tokens WHERE member_id = ?1",
        [member_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_tenant;

    const KEY_A: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const KEY_B: &str = "ff02030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f00";

    fn setup() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let c = open_tenant(dir.path(), "t1").unwrap();
        c.execute_batch(
            "INSERT INTO members (id, email, role) VALUES ('u1','a@example.com','member'), ('u2','b@example.com','member');",
        )
        .unwrap();
        (dir, c)
    }

    fn key(h: &str) -> SecretKey {
        SecretKey::from_hex(h).unwrap()
    }

    #[test]
    fn a_token_round_trips_and_is_not_stored_in_plain() {
        let (_d, c) = setup();
        save(
            &c,
            &key(KEY_A),
            "u1",
            "1//refresh-secret",
            "openid calendar",
        )
        .unwrap();
        assert_eq!(
            load(&c, &key(KEY_A), "u1").unwrap(),
            Some((
                "1//refresh-secret".to_string(),
                "openid calendar".to_string()
            ))
        );
        let stored: Vec<u8> = c
            .query_row("SELECT refresh_token FROM google_tokens", [], |r| r.get(0))
            .unwrap();
        assert!(!String::from_utf8_lossy(&stored).contains("refresh-secret"));
        assert_eq!(load(&c, &key(KEY_A), "u2").unwrap(), None);
    }

    #[test]
    fn the_wrong_key_fails_and_so_does_a_row_moved_to_another_member() {
        let (_d, c) = setup();
        save(&c, &key(KEY_A), "u1", "tok", "s").unwrap();
        assert!(load(&c, &key(KEY_B), "u1").is_err());
        c.execute("UPDATE google_tokens SET member_id = 'u2'", [])
            .unwrap();
        assert!(load(&c, &key(KEY_A), "u2").is_err());
    }

    #[test]
    fn saving_again_replaces_the_token_and_keeps_one_row() {
        let (_d, c) = setup();
        save(&c, &key(KEY_A), "u1", "old", "s1").unwrap();
        save(&c, &key(KEY_A), "u1", "new", "s2").unwrap();
        assert_eq!(
            load(&c, &key(KEY_A), "u1").unwrap(),
            Some(("new".to_string(), "s2".to_string()))
        );
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM google_tokens", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn delete_and_member_removal_hard_delete_the_row() {
        let (_d, c) = setup();
        save(&c, &key(KEY_A), "u1", "a", "s").unwrap();
        save(&c, &key(KEY_A), "u2", "b", "s").unwrap();
        delete(&c, "u1").unwrap();
        c.execute("DELETE FROM members WHERE id = 'u2'", [])
            .unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM google_tokens", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn keys_must_be_64_hex_characters() {
        for bad in [
            "",
            "abcd",
            &KEY_A[..63],
            &format!("{}g", &KEY_A[..63]),
            &format!("+{}", &KEY_A[1..]),
        ] {
            assert!(SecretKey::from_hex(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_save_uses_a_fresh_nonce() {
        let (_d, c) = setup();
        let nonce = |c: &Connection| -> Vec<u8> {
            c.query_row("SELECT nonce FROM google_tokens", [], |r| r.get(0))
                .unwrap()
        };
        save(&c, &key(KEY_A), "u1", "same", "s").unwrap();
        let first = nonce(&c);
        save(&c, &key(KEY_A), "u1", "same", "s").unwrap();
        assert_eq!(first.len(), 24);
        assert_ne!(first, nonce(&c));
    }
}
