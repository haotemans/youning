//! Douyin `a_bogus` request signature, in pure Rust.
//!
//! Ported from `Evil0ctal/Douyin_TikTok_Download_API`'s `dtk/signing/native/abogus.py`
//! (Apache-2.0). Douyin's website front-end signs every `/aweme/v1/web/*` data
//! endpoint with this token; without it the WAF returns either an empty JSON or
//! a JS-VM shell. The algorithm is self-contained: RC4-with-a-twist, a custom
//! base64 alphabet, SM3, a fifty-field layout, an internal checksum.
//!
//! Douyin rotates the parameters every few months. The ones below are current
//! as of the `bdms.js` revision "1.0.1.19-fix.01" (2026-09). If Douyin ships a
//! new bundle the constants here will need refreshing.

use rand::Rng;
use sm3::{Digest as Sm3DigestTrait, Sm3};
use std::time::{SystemTime, UNIX_EPOCH};

const ALPHABET_S3: &[u8; 64] =
    b"ckdp1h4ZKsUB80/Mfvw36XIgR25+WQAlEi7NLboqYTOPuzmFjJnryx9HVGDaStCe";
const ALPHABET_S4: &[u8; 64] =
    b"Dkdpgh2ZmsQB80/MfvV36XI1R45-WUAlEixNLwoqYTOPuzKFjJnry79HbGcaStCe";
const SALT: &str = "dhzx";
const HEADER_MAGIC: (u8, u8) = (3, 82);
const SDK_VERSION: (u8, u8, u8, u8) = (1, 0, 1, 0);
const PAYLOAD_KEY: u8 = 0xD3;
const FORTNIGHT_EPOCH_MS: u64 = 1_721_836_800_000;
const PAGE_ID: u32 = 6241;
const AID: u32 = 6383;
const NOISE_MASKS: [u8; 3] = [0x91, 0x42, 0x2C];
const DATA_MASKS: [u8; 3] = [0x6E, 0xBD, 0xD3];
/// Order in which the 50 scalar fields are written into the payload.
/// Each entry is the numeric suffix of the field name (34 = "L34").
const FIELD_ORDER: [u8; 50] = [
    34, 44, 56, 61, 73, 29, 70, 45, 35, 49, 38, 66, 51, 68, 28, 48, 64, 47, 30, 71, 26, 55, 31,
    69, 59, 40, 62, 63, 27, 72, 41, 74, 57, 52, 42, 39, 33, 67, 53, 43, 65, 46, 36, 24, 60, 32,
    79, 80, 84, 85,
];
const ENV_FLAGS: u16 = 1;
const DETECT_FLAGS: u32 = 14;
const NR_FLAGS: u16 = 0x21;
const NR_TAG: (u8, u8, u8, u8) = (0, 0, 0, 0);
const TRIPWIRE_LOCKED: u8 = 3;
const CALL_BUCKET: u8 = 6;
const TASKBAR_PX: i32 = 48;
const CHROME_PX: i32 = 85;
const TRIPWIRE_SET: u8 = 0xB2;
const TRIPWIRE_FREE: u8 = 0x4D;

/// Nine-field `navigator`/`screen` string the signature carries verbatim.
fn browser_info_from_screen(width: i32, height: i32, platform: &str) -> String {
    let avail_height = (height - TASKBAR_PX).max(1);
    let fields = [
        width,
        (avail_height - CHROME_PX).max(1),
        width,
        avail_height,
        width,
        avail_height,
        width,
        height,
    ];
    let mut out = fields
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("|");
    out.push('|');
    out.push_str(platform);
    out
}

/// Douyin's RC4: descending-identity S-box, multiplicative KSA. The stream
/// cipher itself is textbook.
fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut box0 = [0u8; 256];
    for i in 0..256 {
        box0[255 - i] = i as u8;
    }
    let mut j = 0usize;
    for i in 0..256 {
        j = (j * box0[i] as usize + j + key[i % key.len()] as usize) % 256;
        box0.swap(i, j);
    }
    let mut out = vec![0u8; data.len()];
    let mut i = 0usize;
    let mut j = 0usize;
    for (idx, byte) in data.iter().enumerate() {
        i = (i + 1) % 256;
        j = (j + box0[i] as usize) % 256;
        box0.swap(i, j);
        out[idx] = byte ^ box0[(box0[i] as usize + box0[j] as usize) % 256];
    }
    out
}

fn encode_base64(data: &[u8], alphabet: &[u8; 64]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let mut block = [0u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let b24 = ((block[0] as u32) << 16) | ((block[1] as u32) << 8) | (block[2] as u32);
        let digits = [
            ((b24 >> 18) & 0x3F) as usize,
            ((b24 >> 12) & 0x3F) as usize,
            ((b24 >> 6) & 0x3F) as usize,
            (b24 & 0x3F) as usize,
        ];
        for &d in digits.iter().take(chunk.len() + 1) {
            out.push(alphabet[d] as char);
        }
    }
    let pad = (4 - (out.len() % 4)) % 4;
    for _ in 0..pad {
        out.push('=');
    }
    out
}

/// The SDK's string-to-bytes rule: walk UTF-16 code units, emit one byte for
/// units below U+0100 and two big-endian bytes above. NOT UTF-8.
fn js_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for unit in text.encode_utf16() {
        if unit & 0xFF00 != 0 {
            out.push((unit >> 8) as u8);
        }
        out.push((unit & 0xFF) as u8);
    }
    out
}

fn le_bytes(value: u64, count: usize) -> Vec<u8> {
    (0..count).map(|i| ((value >> (8 * i)) & 0xFF) as u8).collect()
}

fn sm3_to_array(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sm3::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn digest_of(text: &str) -> Vec<u8> {
    let mut input = String::with_capacity(text.len() + SALT.len());
    input.push_str(text);
    input.push_str(SALT);
    let first = sm3_to_array(input.as_bytes());
    sm3_to_array(&first)
}

fn user_agent_digest(user_agent: &str) -> Vec<u8> {
    let key = [
        (ENV_FLAGS / 256) as u8,
        (ENV_FLAGS % 256) as u8,
        (DETECT_FLAGS % 256) as u8,
    ];
    let ua_units = js_bytes(user_agent.trim());
    let ua_low_bytes: Vec<u8> = ua_units.iter().map(|b| b & 0xFF).collect();
    let sealed = rc4(&key, &ua_low_bytes);
    let encoded = encode_base64(&sealed, ALPHABET_S3);
    sm3_to_array(encoded.as_bytes())
}

fn canary(digest: &[u8], offset: usize, sentinel: u8, fallback: u8) -> u8 {
    for &byte in digest.iter().skip(offset) {
        if byte != sentinel {
            return byte;
        }
    }
    fallback
}

struct DigestChain {
    slots: [u8; 3],
    indices: (usize, usize),
    canary: (usize, u8, u8),
}

fn chain_bytes(chain: &DigestChain, digest: &[u8]) -> (u8, u8, u8) {
    let (a, b) = chain.indices;
    let (off, sen, fb) = chain.canary;
    (digest[a], digest[b], canary(digest, off, sen, fb))
}

fn header_noise_band(ua: &str) -> i32 {
    let lower = ua.to_ascii_lowercase();
    for name in ["edg", "huawei", "firefox", "chrome", "safari"] {
        if lower.contains(name) {
            return match name {
                "edg" => 125,
                "firefox" => 40,
                "chrome" => 0,
                "safari" => 81,
                "huawei" => 170,
                _ => 210,
            };
        }
    }
    210
}

fn mask_pair(pair: (u8, u8), rng: &mut impl Rng, low: Option<u8>, high: Option<u8>) -> Vec<u8> {
    let noise = rng.r#gen::<u16>();
    let low = low.unwrap_or((noise & 0xFF) as u8);
    let high = high.unwrap_or(((noise >> 8) & 0xFF) as u8);
    vec![
        (low & 0xAA) | (pair.0 & 0x55),
        (low & 0x55) | (pair.0 & 0xAA),
        (high & 0xAA) | (pair.1 & 0x55),
        (high & 0x55) | (pair.1 & 0xAA),
    ]
}

fn expand_noise(body: &[u8], rng: &mut impl Rng) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() * 4 / 3 + 4);
    for chunk in body.chunks(3) {
        if chunk.len() < 3 {
            out.push(chunk[0]);
            if chunk.len() > 1 && chunk[1] != 0 {
                out.push(chunk[1]);
            }
            continue;
        }
        let noise = (rng.r#gen::<u32>() % 1000) as u8;
        out.push((noise & NOISE_MASKS[0]) | (chunk[0] & DATA_MASKS[0]));
        out.push((noise & NOISE_MASKS[1]) | (chunk[1] & DATA_MASKS[1]));
        out.push((noise & NOISE_MASKS[2]) | (chunk[2] & DATA_MASKS[2]));
        out.push(
            (chunk[0] & NOISE_MASKS[0]) | (chunk[1] & NOISE_MASKS[1]) | (chunk[2] & NOISE_MASKS[2]),
        );
    }
    out
}

pub struct ABogus {
    user_agent: String,
    browser_info: String,
}

impl ABogus {
    pub fn new(user_agent: &str) -> Self {
        Self {
            user_agent: user_agent.to_string(),
            browser_info: browser_info_from_screen(1920, 1080, "Win32"),
        }
    }

    /// Assemble the 50 scalar fields keyed by their numeric suffix (L34 -> 34).
    fn fields(&self, query: &str, body: &str, now_ms: u64) -> [u8; 50] {
        let query_digest = digest_of(query);
        let body_digest = digest_of(body);
        let ua_digest = user_agent_digest(&self.user_agent);

        let ink: u64 = now_ms.saturating_sub(1);
        let fortnights: u64 = (now_ms - FORTNIGHT_EPOCH_MS) / (1000 * 60 * 60 * 24 * 14);
        let info_bytes = js_bytes(&self.browser_info);
        let tail_str = format!("{},", ((now_ms + 3) & 0xFF) as u8);
        let tail_bytes = js_bytes(&tail_str);

        // A byte-indexed lookup (max field id is 85).
        let mut map = [0u8; 86];
        map[24] = 41;
        map[26] = (fortnights & 0xFF) as u8;
        map[27] = CALL_BUCKET;
        map[28] = 3;
        map[35] = (ENV_FLAGS & 0xFF) as u8;
        map[36] = ((ENV_FLAGS / 256) & 0xFF) as u8;
        map[38] = (NR_FLAGS & 0xFF) as u8;
        map[39] = ((NR_FLAGS >> 8) & 0xFF) as u8;
        map[66] = TRIPWIRE_LOCKED;
        map[79] = (info_bytes.len() & 0xFF) as u8;
        map[80] = ((info_bytes.len() >> 8) & 0xFF) as u8;
        map[84] = (tail_bytes.len() & 0xFF) as u8;
        map[85] = ((tail_bytes.len() >> 8) & 0xFF) as u8;

        for (i, byte) in le_bytes(now_ms, 6).iter().enumerate() {
            map[29 + i] = *byte;
        }
        for (i, byte) in le_bytes(DETECT_FLAGS as u64, 4).iter().enumerate() {
            map[44 + i] = *byte;
        }
        let nr = [NR_TAG.0, NR_TAG.1, NR_TAG.2, NR_TAG.3];
        for (i, byte) in nr.iter().enumerate() {
            map[40 + i] = *byte;
        }
        for (i, byte) in le_bytes(ink, 6).iter().enumerate() {
            map[60 + i] = *byte;
        }
        for (i, byte) in le_bytes(PAGE_ID as u64, 4).iter().enumerate() {
            map[67 + i] = *byte;
        }
        for (i, byte) in le_bytes(AID as u64, 4).iter().enumerate() {
            map[71 + i] = *byte;
        }

        let chains = [
            (DigestChain { slots: [48, 49, 51], indices: (9, 18), canary: (3, 11, 12) }, query_digest),
            (DigestChain { slots: [52, 53, 55], indices: (10, 19), canary: (4, 8, 9) }, body_digest),
            (DigestChain { slots: [56, 57, 59], indices: (11, 21), canary: (5, 12, 13) }, ua_digest),
        ];
        for (chain, digest) in chains {
            let (a, b, c) = chain_bytes(&chain, &digest);
            map[chain.slots[0] as usize] = a;
            map[chain.slots[1] as usize] = b;
            map[chain.slots[2] as usize] = c;
        }

        let mut out = [0u8; 50];
        for (i, &slot) in FIELD_ORDER.iter().enumerate() {
            out[i] = map[slot as usize];
        }
        out
    }

    pub fn get_value(&self, query: &str, body: &str, content_type: &str) -> String {
        let body = if content_type.to_ascii_lowercase().contains("multipart/form-data") {
            ""
        } else {
            body
        };
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        assert!(now_ms >= FORTNIGHT_EPOCH_MS, "clock before a_bogus epoch");

        let mut rng = rand::thread_rng();
        let fields = self.fields(query, body, now_ms);

        let probe_low: u8 = {
            let v = (rng.r#gen::<f64>() * 240.0) as i32;
            let v = if v > 109 { v + v % 2 + 1 } else { v };
            (v & 0xFF) as u8
        };
        let tripwire_high: u8 = {
            let base = (rng.r#gen::<f64>() * 255.0) as u8;
            (base & TRIPWIRE_FREE) | TRIPWIRE_SET
        };

        let mut version = mask_pair((SDK_VERSION.0, SDK_VERSION.1), &mut rng, None, None);
        version.extend(mask_pair(
            (SDK_VERSION.2, SDK_VERSION.3),
            &mut rng,
            Some(probe_low),
            Some(tripwire_high),
        ));

        let mut checksum = 0u8;
        for &b in &version {
            checksum ^= b;
        }
        for &b in &fields {
            checksum ^= b;
        }

        let mut body_bytes: Vec<u8> = fields.to_vec();
        body_bytes.extend(js_bytes(&self.browser_info));
        let tail_str = format!("{},", ((now_ms + 3) & 0xFF) as u8);
        body_bytes.extend(js_bytes(&tail_str));
        body_bytes.push(checksum);

        let header_high: u8 = {
            let base = header_noise_band(&self.user_agent);
            let v = base + ((rng.r#gen::<f64>() * 40.0) as i32);
            (v & 0xFF) as u8
        };
        let header = mask_pair(HEADER_MAGIC, &mut rng, None, Some(header_high));
        let frame = expand_noise(&body_bytes, &mut rng);
        let sealed = rc4(&[PAYLOAD_KEY], &[version.as_slice(), frame.as_slice()].concat());
        let mut out = header;
        out.extend(sealed);
        encode_base64(&out, ALPHABET_S4)
    }
}

/// Compute `a_bogus` for a request under a given User-Agent.
pub fn sign(user_agent: &str, query: &str) -> String {
    ABogus::new(user_agent).get_value(query, "", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_alternate_ksa_is_deterministic() {
        let a = rc4(&[0x42], b"hello");
        let b = rc4(&[0x42], b"hello");
        let c = rc4(&[0x43], b"hello");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 5);
    }

    #[test]
    fn base64_custom_alphabet_wellformed() {
        let s = encode_base64(b"\x00\x01\x02", ALPHABET_S4);
        assert_eq!(s.len(), 4);
        assert!(s.chars().all(|c| ALPHABET_S4.contains(&(c as u8))));
        let s = encode_base64(b"\x00", ALPHABET_S4);
        assert_eq!(s.len(), 4);
        assert!(s.ends_with("=="));
    }

    #[test]
    fn js_bytes_not_utf8_for_non_ascii() {
        let out = js_bytes("中");
        assert_eq!(out.len(), 2);
        assert_eq!(js_bytes("abc"), vec![b'a', b'b', b'c']);
    }

    #[test]
    fn a_bogus_shape_and_randomness() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36";
        let a = sign(ua, "a=1&b=2");
        assert!(a.chars().all(|c| ALPHABET_S4.contains(&(c as u8)) || c == '='));
        assert!(a.len() >= 60);
        let b = sign(ua, "a=1&b=2");
        assert_ne!(a, b);
    }

    #[test]
    fn browser_info_consistent() {
        let s = browser_info_from_screen(1920, 1080, "Win32");
        assert!(s.ends_with("|Win32"));
        let fields: Vec<&str> = s.split('|').collect();
        assert_eq!(fields.len(), 9);
    }
}
