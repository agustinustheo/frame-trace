//! SS58 address encoding, used to show `AccountId32` map keys.

use sp_crypto_hashing::blake2_512;

/// Encodes a 32-byte account id as an SS58 address with the given network prefix.
pub fn encode(prefix: u16, account: &[u8; 32]) -> String {
    let mut data = match prefix {
        0..=63 => vec![prefix as u8],
        // Two-byte form for prefixes 64..=16383 (see the SS58 spec).
        _ => vec![
            ((prefix & 0b1111_1100) >> 2) as u8 | 0b0100_0000,
            (prefix >> 8) as u8 | ((prefix & 0b11) << 6) as u8,
        ],
    };
    data.extend_from_slice(account);
    let hash = blake2_512(&[b"SS58PRE".as_slice(), &data].concat());
    data.extend_from_slice(&hash[..2]);
    bs58::encode(data).into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: [u8; 32] = [
        0xd4, 0x35, 0x93, 0xc7, 0x15, 0xfd, 0xd3, 0x1c, 0x61, 0x14, 0x1a, 0xbd, 0x04, 0xa9, 0x9f,
        0xd6, 0x82, 0x2c, 0x85, 0x58, 0x85, 0x4c, 0xcd, 0xe3, 0x9a, 0x56, 0x84, 0xe7, 0xa5, 0x6d,
        0xa2, 0x7d,
    ];

    #[test]
    fn encodes_alice() {
        assert_eq!(
            encode(42, &ALICE),
            "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
        );
        assert_eq!(
            encode(0, &ALICE),
            "15oF4uVJwmo4TdGW7VfQxNLavjCXviqxT9S1MgbjMNHr6Sp5"
        );
    }
}
