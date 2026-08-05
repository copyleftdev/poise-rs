use std::hash::{BuildHasherDefault, Hasher};

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// A deterministic 64-bit FNV-1a hasher.
///
/// This is provided for repeatable policy decisions, not for adversarial hash
/// tables or cryptographic use. Its byte-level algorithm is part of Poise's
/// compatibility contract.
#[derive(Clone, Debug)]
pub struct Fnv1a64(u64);

impl Default for Fnv1a64 {
    fn default() -> Self {
        Self(OFFSET_BASIS)
    }
}

impl Hasher for Fnv1a64 {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(PRIME);
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.write(&value.to_le_bytes());
    }

    fn write_u16(&mut self, value: u16) {
        self.write(&value.to_le_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    fn write_u128(&mut self, value: u128) {
        self.write(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_i8(&mut self, value: i8) {
        self.write(&value.to_le_bytes());
    }

    fn write_i16(&mut self, value: i16) {
        self.write(&value.to_le_bytes());
    }

    fn write_i32(&mut self, value: i32) {
        self.write(&value.to_le_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }

    fn write_i128(&mut self, value: i128) {
        self.write(&value.to_le_bytes());
    }

    fn write_isize(&mut self, value: isize) {
        self.write_i64(value as i64);
    }
}

/// The deterministic default hash builder for keyed policies.
pub type FnvBuildHasher = BuildHasherDefault<Fnv1a64>;

/// Applies a deterministic avalanche finalizer to a 64-bit hash.
///
/// FNV is inexpensive and stable but its nearby inputs can retain visible bit
/// relationships. Keyed policies use this bijective `SplitMix64` finalizer before
/// interpreting hash magnitude, preserving all 64 bits while improving
/// diffusion. The constants and wrapping operations are part of Poise's
/// compatibility contract.
#[must_use]
pub const fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_published_fnv1a_vector() {
        let mut hasher = Fnv1a64::default();
        hasher.write(b"hello");
        assert_eq!(hasher.finish(), 0xa430_d846_80aa_bd0b);
    }

    #[test]
    fn integer_encoding_is_explicitly_little_endian() {
        let mut through_method = Fnv1a64::default();
        through_method.write_u64(0x0102_0304_0506_0708);

        let mut through_bytes = Fnv1a64::default();
        through_bytes.write(&[8, 7, 6, 5, 4, 3, 2, 1]);

        assert_eq!(through_method.finish(), through_bytes.finish());
    }

    #[test]
    fn avalanche_finalizer_has_stable_vectors() {
        assert_eq!(mix64(0), 0);
        assert_eq!(mix64(1), 0x5692_161d_100b_05e5);
        assert_eq!(mix64(u64::MAX), 0xb4d0_55fc_f2cb_bd7b);
    }
}
