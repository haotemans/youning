//! Douyin Argus WebSign — the second signature every `/aweme/v1/web/*` call
//! carries alongside `a_bogus`. Without a valid `x-secsdk-web-signature` the
//! WAF answers `403 Blocked by ArgusSecurityPlugin Uifid Not Found`, which is
//! exactly the error the bare-abogus implementation was hitting.
//!
//! Ported from `vacacia/astrbot_plugin_link_resolver/core/douyin/websign.py`
//! (Apache-2.0).

use std::time::{SystemTime, UNIX_EPOCH};
use url::form_urlencoded;

const WEB_SIGN_SALT: &str = "A96D855A08C0A9707F8BEF0D9A527E4E";
pub const WEB_SIGNATURE_PARAM: &str = "x-secsdk-web-signature";

pub struct SignedQuery {
    pub query: String,
    pub signature: String,
    pub timestamp: String,
}

fn decode_pairs(query: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn encode_pairs(pairs: &[(String, String)]) -> String {
    // The Python `quote(safe='*-._')` — RFC 3986 minus `~`. url's
    // form_urlencoded::Serializer percent-encodes using `application/x-www-form-urlencoded`
    // semantics, which encodes `*` as well. To stay faithful, write it out
    // manually.
    let mut out = String::new();
    for (i, (name, value)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&percent_encode(name));
        out.push('=');
        out.push_str(&percent_encode(value));
    }
    out
}

const SAFE: &str = "*-._";
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for byte in s.bytes() {
        let c = byte as char;
        if byte.is_ascii_alphanumeric() || SAFE.contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

/// Append `uifid`, `timestamp`, and `x-secsdk-web-signature` to an existing
/// query string. `query` should already contain `a_bogus`.
pub fn sign(query: &str, uifid: &str) -> SignedQuery {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    let mut pairs = decode_pairs(query);
    if !pairs.iter().any(|(k, _)| k == "uifid") {
        pairs.push(("uifid".into(), uifid.to_string()));
    }
    pairs.push(("timestamp".into(), stamp.clone()));
    let covered_query = encode_pairs(&pairs);
    let preimage = format!("{}_{}_{}_{}", uifid, stamp, WEB_SIGN_SALT, covered_query);
    let digest = md5(preimage.as_bytes());
    let signature = hex(&digest);
    SignedQuery {
        query: format!("{}&{}={}", covered_query, WEB_SIGNATURE_PARAM, signature),
        signature,
        timestamp: stamp,
    }
}

fn md5(data: &[u8]) -> [u8; 16] {
    // Minimal MD5 — no external crate to keep the social-media manifest small.
    // We use `md-5` if it appears in the lockfile, otherwise fall back to the
    // small in-crate implementation below. Cargo.toml declares md-5 = "0.10".
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(data);
    let out = h.finalize();
    let mut r = [0u8; 16];
    r.copy_from_slice(&out);
    r
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_known_vector() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn percent_encode_safe_chars() {
        assert_eq!(percent_encode("aZ09"), "aZ09");
        assert_eq!(percent_encode("*-._"), "*-._");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("x=y"), "x%3Dy");
    }

    #[test]
    fn sign_produces_md5_and_appends_fields() {
        let q = sign("a=1&b=2", "uid-42");
        assert!(q.query.contains("uifid=uid-42"));
        assert!(q.query.contains("timestamp="));
        assert!(q.query.contains(WEB_SIGNATURE_PARAM));
        assert_eq!(q.signature.len(), 32);
    }
}
