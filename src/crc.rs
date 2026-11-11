//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2003 Peter Brian Clements
//  Copyright (c) 2019 Michael D. Nahas
//
//  par2rust is free software; you can redistribute it and/or modify
//  it under the terms of the GNU General Public License as published by
//  the Free Software Foundation; either version 2 of the License, or
//  (at your option) any later version.
//
//  par2rust is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU General Public License for more details.
//
//  You should have received a copy of the GNU General Public License
//  along with this program; if not, write to the Free Software
//  Foundation, Inc., 59 Temple Place, Suite 330, Boston, MA  02111-1307  USA

//! CCITT CRC32 checksums of blocks of data.
//!
//! The CRC for a block of data may be computed piecemeal by repeatedly
//! calling [`crc_update_char`] and [`crc_update_block`].
//!
//! Given the CRC for a block of data in a buffer, [`crc_slide_char`] may be
//! used to quickly compute the CRC for the block of data in the buffer that
//! is the same size but offset one character later in the buffer.

// GF32 multiplication
const fn gf32_multiply(mut a: u32, mut b: u32, polynomial: u32) -> u32 {
    let mut product = 0;
    let mut i = 0;
    while i < 31 {
        product ^= (b >> 31).wrapping_neg() & a;
        a = (a >> 1) ^ (polynomial & (a & 1).wrapping_neg());
        b <<= 1;
        i += 1;
    }
    product ^ ((b >> 31).wrapping_neg() & a)
}

// A CRC32 lookup table
pub struct Crc32Table {
    pub polynom: u32,
    pub table: [u32; 256],
    pub power: [u32; 32],
}

impl Crc32Table {
    // Construct the CRC32 lookup table from the specified polynomial
    //
    // This seems to follow:
    // http://www.efg2.com/Lab/Library/UseNet/1999/0117.txt
    const fn new(polynomial: u32) -> Crc32Table {
        let mut table = [0u32; 256];
        let mut i = 0u32;
        while i <= 255 {
            let mut crc = i;

            let mut j = 0;
            while j < 8 {
                crc = (crc >> 1) ^ if crc & 1 != 0 { polynomial } else { 0 };
                j += 1;
            }

            table[i as usize] = crc;
            i += 1;
        }

        // Also construct the table used for computing power
        // Note that the table is rotated by 3 entries, since we operate on
        // bytes, i.e. 1<<3 bits
        let mut power = [0u32; 32];
        let mut k = 0x80000000u32 >> 1;
        let mut i = 0u32;
        while i < 32 {
            power[i.wrapping_sub(3) as usize & 31] = k;
            k = gf32_multiply(k, k, polynomial);
            i += 1;
        }

        Crc32Table {
            polynom: polynomial,
            table,
            power,
        }
    }
}

// The one and only CCITT CRC32 lookup table
//
// NOTE: the constant is the reversed polynomial for CRC-32
// as listed on Wikipedia's page:
// https://en.wikipedia.org/wiki/Cyclic_redundancy_check
pub static CCITT_TABLE: Crc32Table = Crc32Table::new(0xEDB88320);

// Update the CRC using one character
//
// This seems to follow:
// http://www.efg2.com/Lab/Mathematics/CRC.htm
pub fn crc_update_char(crc: u32, ch: u8) -> u32 {
    ((crc >> 8) & 0x00ffffff) ^ CCITT_TABLE.table[((crc as u8) ^ ch) as usize]
}

// Update the CRC using a block of characters in a buffer, one character at a time
pub fn crc_update_block_scalar(crc: u32, buffer: &[u8]) -> u32 {
    let mut crc = crc;

    for &ch in buffer {
        crc = crc_update_char(crc, ch);
    }

    crc
}

// Update the CRC using a block of characters in a buffer
//
// The C++ original dispatches here to hardware (PCLMUL / ARM CRC) versions
// of the loop once the CPU is known to support them; until those are ported,
// this is the scalar loop and is the point where the dispatch will go.
pub fn crc_update_block(crc: u32, buffer: &[u8]) -> u32 {
    crc_update_block_scalar(crc, buffer)
}

// Compute 2^(8n) in CRC's GF
fn crc_exp8(mut n: u64) -> u32 {
    let mut result = 0x80000000;
    let mut power = 0;
    n %= 0xffff_ffff;
    while n != 0 {
        if n & 1 != 0 {
            result = gf32_multiply(result, CCITT_TABLE.power[power], CCITT_TABLE.polynom);
        }
        n >>= 1;
        power = (power + 1) & 31;
    }
    result
}

// Construct a CRC32 lookup table for windowing
pub fn generate_window_table(window: u64) -> [u32; 256] {
    // Window coefficient
    let coeff = crc_exp8(window);
    // Extend initial CRC to window length
    let mut mask = gf32_multiply(!0, coeff, CCITT_TABLE.polynom);
    // Xor initial CRC with that extended by one byte
    mask = gf32_multiply(mask, 0x80800000, CCITT_TABLE.polynom);
    // Since we have a table, may as well invert all bits to save doing it later
    mask ^= !0;

    // Generate table
    let mut target = [0u32; 256];
    for (i, t) in target.iter_mut().enumerate() {
        *t = gf32_multiply(CCITT_TABLE.table[i], coeff, CCITT_TABLE.polynom) ^ mask;
    }

    target
}

// Update the CRC using a block of 0s.
pub fn crc_update_block_zeros(crc: u32, length: u64) -> u32 {
    gf32_multiply(crc, crc_exp8(length), CCITT_TABLE.polynom)
}

// Slide the CRC along a buffer by one character (removing the old and adding the new).
// The new character is added using the main CCITT CRC32 table, and the old character
// is removed using the windowtable.
pub fn crc_slide_char(crc: u32, ch_new: u8, ch_old: u8, windowtable: &[u32; 256]) -> u32 {
    let crc = crc ^ !0;
    ((crc >> 8) & 0x00ffffff) ^ CCITT_TABLE.table[((crc as u8) ^ ch_new) as usize]
        ^ windowtable[ch_old as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small stand-in for the srand/rand of the C++ tests: the tests only
    // compare implementations against each other, and need the data the same
    // way every run.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Rng {
            Rng(seed)
        }

        fn next_u8(&mut self) -> u8 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as u8
        }
    }

    fn random_buffer(seed: u64, length: usize) -> Vec<u8> {
        let mut rng = Rng::new(seed);
        (0..length).map(|_| rng.next_u8()).collect()
    }

    // compares crc_update_block_zeros(crc, length) to
    // crc_update_block(crc, buffer) over all zeros
    #[test]
    fn test1_zeros_by_length_equals_zeros_by_buffer() {
        let buffer = [0u8; 8];

        let checksum1 = !0 ^ crc_update_block(!0, &buffer);
        let checksum2 = !0 ^ crc_update_block_zeros(!0, buffer.len() as u64);

        assert_eq!(checksum1, checksum2);
    }

    // CRC32 of "123456789" yields 0xCBF43926
    // according to http://www.ross.net/crc/download/crc_v3.txt
    #[test]
    fn test2_known_checksum() {
        let buffer = *b"123456789";
        let expected_checksum = 0xCBF43926u32;

        let checksum1 = !0 ^ crc_update_block(!0, &buffer);

        assert_eq!(
            checksum1, expected_checksum,
            "checksum was not the precalculated value"
        );
    }

    // generate random data.
    // put it into checksum using different length blocks
    // make sure output is the same.
    #[test]
    fn test3_random_chunking_agrees() {
        let buffer = random_buffer(345087209, 32 * 1024);

        let mut rng = Rng::new(0x5eed_1234);
        let mut checksum1 = !0;
        let mut offset = 0;
        while offset < buffer.len() {
            let length = rng.next_u8() as usize;
            let length = length.min(buffer.len() - offset);
            checksum1 = crc_update_block(checksum1, &buffer[offset..offset + length]);
            offset += length;
        }
        checksum1 ^= !0;

        let mut rng = Rng::new(0x5eed_5678);
        let mut checksum2 = !0;
        let mut offset = 0;
        while offset < buffer.len() {
            let length = rng.next_u8() as usize;
            let length = length.min(buffer.len() - offset);
            checksum2 = crc_update_block(checksum2, &buffer[offset..offset + length]);
            offset += length;
        }
        checksum2 ^= !0;

        assert_eq!(checksum1, checksum2, "random checksums differ");
    }

    // generate random data.
    // compare char-at-a-time vs block
    // make sure output is the same.
    #[test]
    fn test4_char_at_a_time_equals_block() {
        let buffer = random_buffer(113450911, 32 * 1024);

        let mut rng = Rng::new(0xfeed_0001);
        let mut checksum1 = !0;
        let mut offset = 0;
        while offset < buffer.len() {
            let length = rng.next_u8() as usize;
            let length = length.min(buffer.len() - offset);
            checksum1 = crc_update_block(checksum1, &buffer[offset..offset + length]);
            offset += length;
        }
        checksum1 ^= !0;

        let mut checksum2 = !0;
        for &ch in &buffer {
            checksum2 = crc_update_char(checksum2, ch);
        }
        checksum2 ^= !0;

        assert_eq!(checksum1, checksum2, "random checksums differ");
    }

    // check windowing on random buffer
    #[test]
    fn test5_windowing() {
        let buffer = random_buffer(113450911, 32 * 1024);

        let window: usize = 1024;

        let windowtable = generate_window_table(window as u64);

        let mut crc = !0 ^ crc_update_block(!0, &buffer[..window]);
        for offset in 0..buffer.len() - window - 1 {
            // compare against reference
            let othercrc = !0 ^ crc_update_block(!0, &buffer[offset..offset + window]);
            assert_eq!(crc, othercrc, "error in window at offset {offset}");

            // slide window
            crc = crc_slide_char(crc, buffer[offset + window], buffer[offset], &windowtable);
        }
    }

    // Checksum of checksum table
    // stolen from:
    // http://www.efg2.com/Lab/Mathematics/CRC.htm
    #[test]
    fn test6_checksum_of_checksum_table() {
        // The C++ test runs the CRC over the raw memory of the u32 table,
        // which is little-endian on every machine par2cmdline targets;
        // spell that out rather than reading host-order memory.
        let table_bytes: Vec<u8> = CCITT_TABLE
            .table
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();

        let checksum1 = !0 ^ crc_update_block(!0, &table_bytes);
        let expected = 0x6FCF9E13;

        assert_eq!(
            checksum1, expected,
            "error when computing checksum of checksum table"
        );
    }

    // compare the dispatched block implementation against char-at-a-time,
    // over every length and alignment the hardware paths switch on
    #[test]
    fn test7_block_against_char_at_a_time_every_length_and_alignment() {
        let buffer = random_buffer(987654321, 1024);

        for offset in 0..16usize {
            for length in 0..=buffer.len() - offset {
                let checksum1 = crc_update_block(!0, &buffer[offset..offset + length]);

                let mut checksum2 = !0;
                for i in 0..length {
                    checksum2 = crc_update_char(checksum2, buffer[offset + i]);
                }

                assert_eq!(
                    checksum1, checksum2,
                    "offset {offset} length {length}"
                );
            }
        }
    }
}
