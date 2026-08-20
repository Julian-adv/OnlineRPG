// FNV-1a (64-bit), spelled out because `DefaultHasher` is not stable across
// Rust releases and independently-built server and client binaries must agree
// bit-for-bit. Also `include!`d by build.rs, which cannot depend on this crate
// — hence plain comments and no inner attributes.

pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

pub fn fnv1a64(seed: u64, bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut hash = seed;
    for b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
