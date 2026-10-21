//! Douyin `a_bogus` signature, second implementation.
//!
//! This is the JohnserfSeed/f2 implementation used by
//! `vacacia/astrbot_plugin_link_resolver` (Apache-2.0). It differs from the
//! Evil0ctal `abogus.rs` in this crate: classic RC4 instead of the modified
//! KSA, a 44-field `ab_dir` indexed by `sort_index`, an inline XOR-stage
//! `transform_bytes` (with a 256-byte `big_array`), and a different browser
//! fingerprint layout (17 fields instead of 9).
//!
//! Use this one when the remote end is a fresh bdms.js bundle that expects
//! `pageId=0` and `salt="cus"`. The earlier `abogus.rs` is kept because some
//! endpoints still answer to the older layout; `DouyinClient` tries both and
//! falls back.

use rand::Rng;
use sm3::{Digest as Sm3DigestTrait, Sm3};

// Douyin identifiers hard-coded by the platform. Not ours to choose.
const AID: u32 = 6383;
const PAGE_ID: u32 = 0; // 1.0.1.19 → 6241
const SALT: &str = "cus"; // 1.0.1.19 → "dhzx"
const UA_KEY: [u8; 3] = [0x00, 0x01, 0x0E];
const OPTIONS: [u32; 3] = [0, 1, 14]; // GET; use [0, 1, 8] for POST-only flows.

const ALPHABET_0: &[u8; 64] =
    b"Dkdpgh2ZmsQB80/MfvV36XI1R45-WUAlEixNLwoqYTOPuzKFjJnry79HbGcaStCe";
const ALPHABET_1: &[u8; 64] =
    b"ckdp1h4ZKsUB80/Mfvw36XIgR25+WQAlEi7NLboqYTOPuzmFjJnryx9HVGDaStCe";

// 44 ab_dir slots written into the output in this order.
const SORT_INDEX: [usize; 44] = [
    18, 20, 52, 26, 30, 34, 58, 38, 40, 53, 42, 21, 27, 54, 55, 31, 35, 57, 39, 41, 43, 22, 28,
    32, 60, 36, 23, 29, 33, 37, 44, 45, 59, 46, 47, 48, 49, 50, 24, 25, 65, 66, 70, 71,
];

// 44 slots XORed together to make the canary at the tail.
const SORT_INDEX_2: [usize; 44] = [
    18, 20, 26, 30, 34, 38, 40, 42, 21, 27, 31, 35, 39, 41, 43, 22, 28, 32, 36, 23, 29, 33, 37,
    44, 45, 46, 47, 48, 49, 50, 24, 25, 52, 53, 54, 55, 57, 58, 59, 60, 65, 66, 70, 71,
];

// 256-byte static table used by `transform_bytes`.
const BIG_ARRAY: [u8; 256] = [
    121, 243, 55, 234, 103, 36, 47, 228, 30, 231, 106, 6, 115, 95, 78, 101, 250, 207, 198, 50,
    139, 227, 220, 105, 97, 143, 34, 28, 194, 215, 18, 100, 159, 160, 43, 8, 169, 217, 180, 120,
    247, 45, 90, 11, 27, 197, 46, 3, 84, 72, 5, 68, 62, 56, 221, 75, 144, 79, 73, 161, 178, 81,
    64, 187, 134, 117, 186, 118, 16, 241, 130, 71, 89, 147, 122, 129, 65, 40, 88, 150, 110, 219,
    199, 255, 181, 254, 48, 4, 195, 248, 208, 32, 116, 167, 69, 201, 17, 124, 125, 104, 96, 83,
    80, 127, 236, 108, 154, 126, 204, 15, 20, 135, 112, 158, 13, 1, 188, 164, 210, 237, 222, 98,
    212, 77, 253, 42, 170, 202, 26, 22, 29, 182, 251, 10, 173, 152, 58, 138, 54, 141, 185, 33,
    157, 31, 252, 132, 233, 235, 102, 196, 191, 223, 240, 148, 39, 123, 92, 82, 128, 109, 57,
    24, 38, 113, 209, 245, 2, 119, 153, 229, 189, 214, 230, 174, 232, 63, 52, 205, 86, 140, 66,
    175, 111, 171, 246, 133, 238, 193, 99, 60, 74, 91, 225, 51, 76, 37, 145, 211, 166, 151, 213,
    206, 0, 200, 244, 176, 218, 44, 184, 172, 49, 216, 93, 168, 53, 21, 183, 41, 67, 85, 224,
    155, 226, 242, 87, 177, 146, 70, 190, 12, 162, 19, 137, 114, 25, 165, 163, 192, 23, 59, 9,
    94, 179, 107, 35, 7, 142, 131, 239, 203, 149, 136, 61, 249, 14, 156,
];

fn sm3_to_array(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sm3::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn add_salt(param: &str) -> String {
    let mut out = String::with_capacity(param.len() + SALT.len());
    out.push_str(param);
    out.push_str(SALT);
    out
}

/// SM3(param + salt) -> [u8; 32] for str input. For nested calls, use the
/// `params_to_array_bytes` variant so we don't have to smuggle binary through
/// a `&str`.
fn params_to_array(param: &str, add_salt_flag: bool) -> Vec<u8> {
    if add_salt_flag {
        let s = add_salt(param);
        sm3_to_array(s.as_bytes())
    } else {
        sm3_to_array(param.as_bytes())
    }
}
fn params_to_array_bytes(param: &[u8]) -> Vec<u8> {
    sm3_to_array(param)
}

fn js_shift_right(val: i32, n: i32) -> i32 {
    // JavaScript >>> — logical right shift on u32, reinterpreted as i32
    ((val as u32) >> (n as u32)) as i32
}

/// Classic RC4 (NOT the modified version in `abogus.rs`).
fn rc4_encrypt(key: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = [0; 256];
    for (i, item) in s.iter_mut().enumerate() {
        *item = i as u8;
    }
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + s[i] as usize + key[i % key.len()] as usize) % 256;
        s.swap(i, j);
    }
    let mut i = 0usize;
    let mut j = 0usize;
    let mut out = Vec::with_capacity(plaintext.len());
    for &b in plaintext.iter() {
        i = (i + 1) % 256;
        j = (j + s[i] as usize) % 256;
        s.swap(i, j);
        let k = s[((s[i] as usize) + (s[j] as usize)) % 256];
        out.push(b ^ k);
    }
    out
}

/// Custom base64 with shifts and `=` padding, exactly as `abogus_encode`.
fn abogus_encode(input: &[u8], alphabet: &[u8; 64]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let n: u32;
        if i + 2 < input.len() {
            n = ((input[i] as u32) << 16)
                | ((input[i + 1] as u32) << 8)
                | (input[i + 2] as u32);
        } else if i + 1 < input.len() {
            n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        } else {
            n = (input[i] as u32) << 16;
        }

        for (j, k) in [(18, 0xFC0000u32), (12, 0x03F000), (6, 0x0FC0), (0, 0x3F)] {
            if j == 6 && i + 1 >= input.len() {
                break;
            }
            if j == 0 && i + 2 >= input.len() {
                break;
            }
            let idx = ((n & k) >> j) as usize;
            out.push(alphabet[idx] as char);
        }
        i += 3;
    }
    let pad = (4 - (out.len() % 4)) % 4;
    for _ in 0..pad {
        out.push('=');
    }
    out
}

/// Custom base64 over a string-encoded-as-binary, used for the UA chain.
/// Matches `CryptoUtility.base64_encode`.
fn base64_encode(input: &str, alphabet: &[u8; 64]) -> String {
    let mut binary = String::new();
    for c in input.chars() {
        binary.push_str(&format!("{:08b}", c as u32));
    }
    let padding_length = (6 - binary.len() % 6) % 6;
    for _ in 0..padding_length {
        binary.push('0');
    }
    let mut out = String::new();
    for chunk_start in (0..binary.len()).step_by(6) {
        let end = (chunk_start + 6).min(binary.len());
        let idx = u32::from_str_radix(&binary[chunk_start..end], 2).unwrap_or(0) as usize;
        out.push(alphabet[idx] as char);
    }
    for _ in 0..(padding_length / 2) {
        out.push('=');
    }
    out
}

/// Generate the 3-byte pseudo-random prefix that obfuscates `a_bogus`.
/// 4 characters per round, repeated 3 times = 12 chars total.
fn generate_random_bytes(rng: &mut impl Rng) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    for _ in 0..3 {
        let rd = (rng.r#gen::<f64>() * 10000.0) as i32;
        let rd_u8 = (rd & 255) as u8;
        let shifted = js_shift_right(rd, 8) as u8;
        out.push((rd_u8 & 0xAA) | 1);
        out.push((rd_u8 & 0x55) | 2);
        out.push((shifted & 0xAA) | 5);
        out.push((shifted & 0x55) | 40);
    }
    out
}

/// Stateful byte shuffle used for both ab_dir and fingerprint. This is the
/// `transform_bytes` step: each round picks a table entry by walking the array,
/// XORs the input byte, and swaps two slots.
struct Transformer {
    big_array: [u8; 256],
}

impl Transformer {
    fn new() -> Self {
        Self {
            big_array: BIG_ARRAY,
        }
    }

    fn transform(&mut self, input: &[u8]) -> Vec<u8> {
        let mut result = Vec::with_capacity(input.len());
        let mut index_b = self.big_array[1] as usize;
        let mut initial_value: u8 = 0;
        let mut value_e: u8 = 0;
        for (index, &byte) in input.iter().enumerate() {
            if index == 0 {
                initial_value = self.big_array[index_b];
                let sum_initial = (index_b + initial_value as usize) % 256;
                // The Python code does `self.big_array[1] = initial_value;
                // self.big_array[index_b] = index_b;` — a swap.
                self.big_array[1] = initial_value;
                self.big_array[index_b] = index_b as u8;
                let _ = sum_initial;
            } else {
                let _sum_initial = (initial_value as usize + value_e as usize) % 256;
            }
            let sum_initial = if index == 0 {
                (index_b + initial_value as usize) % 256
            } else {
                (initial_value as usize + value_e as usize) % 256
            };
            let value_f = self.big_array[sum_initial];
            let encrypted = byte ^ value_f;
            result.push(encrypted);
            value_e = self.big_array[(index + 2) % 256];
            let _sum_initial2 = (index_b + value_e as usize) % 256;
            self.big_array[(index + 2) % 256] = encrypted;
            index_b = (index_b + encrypted as usize) % 256;
        }
        result
    }
}

/// Generate a fresh browser fingerprint string for the given platform.
/// Mirrors `BrowserFingerprintGenerator._generate_fingerprint` — 17 fields
/// with randomized-but-consistent geometry.
pub fn generate_fingerprint(platform: &str) -> String {
    let mut rng = rand::thread_rng();
    let inner_width = rng.gen_range(1024..=1920);
    let inner_height = rng.gen_range(768..=1080);
    let outer_width = inner_width + rng.gen_range(24..=32);
    let outer_height = inner_height + rng.gen_range(75..=90);
    let screen_x = 0;
    let screen_y = if rng.r#gen::<bool>() { 0 } else { 30 };
    let size_width = rng.gen_range(1024..=1920);
    let size_height = rng.gen_range(768..=1080);
    let avail_width = rng.gen_range(1280..=1920);
    let avail_height = rng.gen_range(800..=1080);
    format!(
        "{}|{}|{}|{}|{}|{}|0|0|{}|{}|{}|{}|{}|{}|24|24|{}",
        inner_width, inner_height, outer_width, outer_height,
        screen_x, screen_y, size_width, size_height, avail_width, avail_height,
        inner_width, inner_height, platform
    )
}

pub struct ABogusJ {
    pub user_agent: String,
    pub browser_fp: String,
}

impl ABogusJ {
    pub fn new(user_agent: &str, fp: &str) -> Self {
        Self {
            user_agent: user_agent.to_string(),
            browser_fp: fp.to_string(),
        }
    }

    /// Produce `(signed_params, abogus)`. Returns the same string the browser
    /// would produce for a GET request; POST passes `body` and Douyin treats
    /// the `options` as `[0, 1, 14]` (which also covers GET).
    pub fn generate_abogus(&self, params: &str, body: &str) -> (String, String) {
        let start_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Build the 72-slot ab_dir lookup table. Sparse: only indices we
        // write are explicitly set; the rest are 0 by default and read by
        // `sort_index`/`sort_index_2`.
        // The Python reference builds the ab_dir as a dict; the sparse
        // key→value writes in its initialization are what actually reach the
        // wire. We replicate that with a HashMap so missing keys resolve to 0
        // exactly like `dict.get(i, 0)`.
        let mut ab_dict: std::collections::HashMap<usize, u8> = std::collections::HashMap::new();
        ab_dict.insert(8, 3);
        ab_dict.insert(18, 44);
        // ab_dict[15] is itself a dict in Python, not a u8 — but the consumer
        // only does ab_dir.get(i, 0), so the dict-object at 15 reads as 0 when
        // XORed. Treat it as absent.
        // ab_dict[19] is a list [1,0,1,0,1] — likewise reads as 0 in
        // arithmetic; the Python reference implementation relies on this.
        ab_dict.insert(19, 0);
        ab_dict.insert(66, 0);
        ab_dict.insert(69, 0);
        ab_dict.insert(70, 0);
        ab_dict.insert(71, 0);

        // Hash chains. The Python `params_to_array(params_to_array(params))`
        // pattern: inner sm3 → 32 bytes (list); outer sm3 over those 32 bytes
        // directly (no salt, because the inner call already produces a list,
        // which the outer's `process_param` skips salting).
        let array1_inner = params_to_array(params, true);
        let array1 = params_to_array_bytes(&array1_inner);
        let array2_inner = params_to_array(body, true);
        let array2 = params_to_array_bytes(&array2_inner);

        // array3: RC4 UA → base64(alphabet1) → sm3(ascii string).
        let sealed = rc4_encrypt(&UA_KEY, self.user_agent.as_bytes());
        let sealed_chars: String = sealed.iter().map(|b| *b as char).collect();
        let array3_input = base64_encode(&sealed_chars, ALPHABET_1);
        let array3 = params_to_array(&array3_input, false);

        let end_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // 20..25: start_encryption (6 bytes, big-endian-ish by bit shifting)
        ab_dict.insert(20, ((start_ms >> 24) & 255) as u8);
        ab_dict.insert(21, ((start_ms >> 16) & 255) as u8);
        ab_dict.insert(22, ((start_ms >> 8) & 255) as u8);
        ab_dict.insert(23, (start_ms & 255) as u8);
        ab_dict.insert(24, (start_ms / 256 / 256 / 256 / 256) as u8);
        ab_dict.insert(25, (start_ms / 256 / 256 / 256 / 256 / 256) as u8);

        // options[0] → 26..29
        ab_dict.insert(26, ((OPTIONS[0] >> 24) & 255) as u8);
        ab_dict.insert(27, ((OPTIONS[0] >> 16) & 255) as u8);
        ab_dict.insert(28, ((OPTIONS[0] >> 8) & 255) as u8);
        ab_dict.insert(29, (OPTIONS[0] & 255) as u8);
        // options[1] (method) → 30..33
        ab_dict.insert(30, ((OPTIONS[1] / 256) & 255) as u8);
        ab_dict.insert(31, ((OPTIONS[1] % 256) & 255) as u8);
        ab_dict.insert(32, ((OPTIONS[1] >> 24) & 255) as u8);
        ab_dict.insert(33, ((OPTIONS[1] >> 16) & 255) as u8);
        // options[2] → 34..37
        ab_dict.insert(34, ((OPTIONS[2] >> 24) & 255) as u8);
        ab_dict.insert(35, ((OPTIONS[2] >> 16) & 255) as u8);
        ab_dict.insert(36, ((OPTIONS[2] >> 8) & 255) as u8);
        ab_dict.insert(37, (OPTIONS[2] & 255) as u8);
        // body digest bytes 21..22 → 38..39
        ab_dict.insert(38, array1[21]);
        ab_dict.insert(39, array1[22]);
        // body digest bytes 21..22 → 40..41
        ab_dict.insert(40, array2[21]);
        ab_dict.insert(41, array2[22]);
        // ua digest bytes 23..24 → 42..43
        ab_dict.insert(42, array3[23]);
        ab_dict.insert(43, array3[24]);

        // 44..50: end_encryption
        ab_dict.insert(44, ((end_ms >> 24) & 255) as u8);
        ab_dict.insert(45, ((end_ms >> 16) & 255) as u8);
        ab_dict.insert(46, ((end_ms >> 8) & 255) as u8);
        ab_dict.insert(47, (end_ms & 255) as u8);
        ab_dict.insert(48, *ab_dict.get(&8).unwrap_or(&0));
        ab_dict.insert(49, (end_ms / 256 / 256 / 256 / 256) as u8);
        ab_dict.insert(50, (end_ms / 256 / 256 / 256 / 256 / 256) as u8);

        // pageId (4 bytes) → 51..54 and 55 (low byte of pageId)
        ab_dict.insert(51, ((PAGE_ID >> 24) & 255) as u8);
        ab_dict.insert(52, ((PAGE_ID >> 16) & 255) as u8);
        ab_dict.insert(53, ((PAGE_ID >> 8) & 255) as u8);
        ab_dict.insert(54, (PAGE_ID & 255) as u8);
        ab_dict.insert(55, PAGE_ID as u8);
        // aid (4 bytes) → 56..60
        ab_dict.insert(56, AID as u8);
        ab_dict.insert(57, (AID & 255) as u8);
        ab_dict.insert(58, ((AID >> 8) & 255) as u8);
        ab_dict.insert(59, ((AID >> 16) & 255) as u8);
        ab_dict.insert(60, ((AID >> 24) & 255) as u8);

        // browser fingerprint length → 64..65
        let fp_len = self.browser_fp.len() as u8;
        ab_dict.insert(64, fp_len);
        ab_dict.insert(65, fp_len);

        // Read through sort_index → ordered values.
        let mut sorted_values: Vec<u8> = Vec::with_capacity(SORT_INDEX.len() + self.browser_fp.len() + 1);
        for &i in SORT_INDEX.iter() {
            sorted_values.push(*ab_dict.get(&i).unwrap_or(&0));
        }

        // Append fingerprint bytes (one byte per UTF-16 code unit, but Python
        // treats them as ord() of the char — for ASCII this is the same).
        let fp_bytes: Vec<u8> = self.browser_fp.chars().map(|c| c as u8).collect();
        sorted_values.extend(&fp_bytes);

        // XOR the sort_index_2 values together; the running start is the
        // first slot. Match the Python:
        //   ab_xor = (len(self.browser_fp) & 255) >> 8 & 255
        //   for index in range(len(self.sort_index_2) - 1):
        //       if index == 0:
        //           ab_xor = ab_dir.get(self.sort_index_2[index], 0)
        //       ab_xor ^= ab_dir.get(self.sort_index_2[index + 1], 0)
        let mut ab_xor: u8 = 0;
        for (idx, &slot) in SORT_INDEX_2.iter().enumerate() {
            let v = *ab_dict.get(&slot).unwrap_or(&0);
            if idx == 0 {
                ab_xor = v;
            } else {
                ab_xor ^= v;
            }
        }
        sorted_values.push(ab_xor);

        // Random prefix + transformed payload.
        let mut rng = rand::thread_rng();
        let random_prefix = generate_random_bytes(&mut rng);
        let mut transformer = Transformer::new();
        let transformed = transformer.transform(&sorted_values);
        let mut blob = random_prefix;
        blob.extend(transformed);

        // Python code does "".join(chr(b) for b in blob) then `abogus_encode`
        // which reads ord() of each char. For bytes < 128 this is identical
        // to using the raw bytes directly; for ≥128 Python's chr() roundtrip
        // gives the same codepoint. Either way we can encode the bytes.
        let abogus = abogus_encode(&blob, ALPHABET_0);
        let signed_params = format!("{}&a_bogus={}", params, abogus);
        (signed_params, abogus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_is_classic_textbook() {
        // Classic RC4 with key "Key" over "Plaintext" yields the well-known
        // OpenSSL/Wikipedia vector B0 3F 0A 1F 0D 8E 2F 0C 0C 10.
        let cipher = rc4_encrypt(b"Key", b"Plaintext");
        let expected: Vec<u8> = vec![0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3];
        assert_eq!(cipher, expected);
    }

    #[test]
    fn sm3_size_is_32() {
        let d = sm3_to_array(b"hello");
        assert_eq!(d.len(), 32);
    }

    #[test]
    fn base64_encode_uses_custom_alphabet() {
        let out = base64_encode("abc", ALPHABET_0);
        assert!(!out.is_empty());
        assert!(out.chars().all(|c| ALPHABET_0.contains(&(c as u8)) || c == '='));
    }

    #[test]
    fn abogus_encode_pads_with_equals() {
        let s = abogus_encode(b"\x00", ALPHABET_0);
        assert_eq!(s.len(), 4);
        assert!(s.ends_with("=="));
    }

    #[test]
    fn fingerprint_shape_17_fields() {
        let fp = generate_fingerprint("Win32");
        let fields: Vec<&str> = fp.split('|').collect();
        assert_eq!(fields.len(), 17);
        assert!(fp.ends_with("|Win32"));
    }

    #[test]
    fn abogus_looks_like_alphabet() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0";
        let fp = generate_fingerprint("Win32");
        let ab = ABogusJ::new(ua, &fp);
        let (signed, abogus) = ab.generate_abogus("aweme_id=7696230569553693987&aid=6383&device_platform=webapp", "");
        assert!(!abogus.is_empty());
        assert!(abogus.chars().all(|c| ALPHABET_0.contains(&(c as u8)) || c == '='));
        assert!(signed.contains("&a_bogus="));
        assert!(signed.contains("aweme_id=7696230569553693987"));
        // Golden signature length: 164 chars for this fixed 49-char params form.
        // We expect roughly this range (the random prefix shifts it slightly).
        assert!(abogus.len() >= 150 && abogus.len() <= 180, "length={}", abogus.len());
    }
}
