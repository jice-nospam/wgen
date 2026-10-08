//! How a step mask is stored in a `.wgen` file: one base64 string of little-endian `u16`
//! values (`q = round(clamp(v, 0, 1) · 65535)`), in the mask's row-major order. Reading also
//! accepts the older list of floats.

use std::fmt;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

const QUANT: f32 = 65535.0;

/// the mask as base64 of its quantized `u16` values, little-endian
pub fn encode_mask(mask: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(mask.len() * 2);
    for v in mask {
        let q = (v.clamp(0.0, 1.0) * QUANT).round() as u16;
        bytes.extend_from_slice(&q.to_le_bytes());
    }
    STANDARD.encode(bytes)
}

/// the mask values a string written by `encode_mask` holds
pub fn decode_mask(text: &str) -> Result<Vec<f32>, String> {
    let bytes = STANDARD
        .decode(text)
        .map_err(|e| format!("mask is not valid base64: {e}"))?;
    if bytes.len() % 2 != 0 {
        return Err(format!("mask has an odd byte count {}", bytes.len()));
    }
    Ok(bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b) as f32 / QUANT)
        .collect())
}

/// serde glue for `Step.mask`: writes the string form, reads either form
pub mod serde_mask {
    use super::*;
    use serde::de::{self, Deserialize, Deserializer, SeqAccess, Visitor};
    use serde::Serializer;

    pub fn serialize<S: Serializer>(mask: &Option<Vec<f32>>, s: S) -> Result<S::Ok, S::Error> {
        match mask {
            None => s.serialize_none(),
            Some(m) => s.serialize_some(&encode_mask(m)),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<f32>>, D::Error> {
        Ok(Option::<MaskText>::deserialize(d)?.map(|m| m.0))
    }

    struct MaskText(Vec<f32>);

    impl<'de> Deserialize<'de> for MaskText {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_any(MaskVisitor).map(MaskText)
        }
    }

    struct MaskVisitor;

    impl<'de> Visitor<'de> for MaskVisitor {
        type Value = Vec<f32>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a base64 mask string or a list of floats")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<f32>, A::Error> {
            let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(v) = seq.next_element::<f32>()? {
                values.push(v);
            }
            Ok(values)
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<Vec<f32>, E> {
            decode_mask(text).map_err(E::custom)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let n = 64 * 64;
        let mask: Vec<f32> = (0..n).map(|i| i as f32 / (n - 1) as f32).collect();
        let back = decode_mask(&encode_mask(&mask)).unwrap();
        assert_eq!(back.len(), n);
        for (a, b) in mask.iter().zip(&back) {
            assert!((a - b).abs() <= 0.5 / QUANT, "{a} vs {b}");
        }
        assert_eq!(back[0], 0.0);
        assert_eq!(back[n - 1], 1.0);
    }

    #[test]
    fn encoded_length() {
        assert_eq!(encode_mask(&vec![0.5; 64 * 64]).len(), 10924);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        assert_eq!(decode_mask(&encode_mask(&[-0.5, 1.5])).unwrap(), vec![0.0, 1.0]);
    }

    #[test]
    fn decode_rejects_bad_input() {
        assert!(decode_mask("!!!").is_err());
        assert!(decode_mask("AAAA").is_err());
    }
}
