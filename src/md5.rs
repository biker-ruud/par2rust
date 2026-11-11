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

//! This file defines the [`Md5Hash`] and [`Md5Context`] objects which are used
//! to compute and manipulate the MD5 Hash values for a block of data.
//!
//! Usage:
//!
//! ```
//! use par2rust::md5::{Md5Context, Md5Hash};
//!
//! let mut context = Md5Context::new();
//! context.update(b"buffer");
//!
//! let hash: Md5Hash = context.finalize();
//! let _ = hash;
//! ```

use std::fmt;

// MD5 Hash value
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Md5Hash {
    // 16 byte MD5 Hash value
    pub hash: [u8; 16],
}

// Compare hash values. The ordering compares the bytes from the last to the
// first, so that the hash reads as a big number.
impl Md5Hash {
    fn compare(&self, other: &Md5Hash) -> std::cmp::Ordering {
        let mut index = 15usize;
        while index > 0 && self.hash[index] == other.hash[index] {
            index -= 1;
        }

        self.hash[index].cmp(&other.hash[index])
    }
}

impl std::cmp::Ord for Md5Hash {
    fn cmp(&self, other: &Md5Hash) -> std::cmp::Ordering {
        self.compare(other)
    }
}

impl std::cmp::PartialOrd for Md5Hash {
    fn partial_cmp(&self, other: &Md5Hash) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Write length bytes into buffer as hex digits, and return where that left off
fn print_bytes(bytes: &[u8], buffer: &mut String) {
    const HEXDIGITS: &[u8; 16] = b"0123456789ABCDEF";

    for b in bytes {
        buffer.push(HEXDIGITS[(b >> 4) as usize] as char);
        buffer.push(HEXDIGITS[(b & 0xf) as usize] as char);
    }
}

// The 32 hex digits of a 16 byte hash, in the order its bytes are stored
fn print_hash(hash: &[u8; 16]) -> String {
    let mut buffer = String::with_capacity(32);
    print_bytes(hash, &mut buffer);
    buffer
}

// Convert hash values to hex
impl fmt::Display for Md5Hash {
    fn fmt(&self, result: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(result, "{}", print_hash(&self.hash))
    }
}

impl Md5Hash {
    // Convert value to hex
    pub fn print(&self) -> String {
        print_hash(&self.hash)
    }
}

const BUFFERSIZE: usize = 64;

// MD5 computation context with 64 byte buffer
#[derive(Clone, Debug)]
pub struct Md5Context {
    block: [u8; BUFFERSIZE],
    used: usize,

    // 16 byte MD5 computation state
    state: [u32; 4],

    bytes: u64,
}

// Primitive operations
fn f1(x: u32, y: u32, z: u32) -> u32 {
    ((y ^ z) & x) ^ z
}
fn f2(x: u32, y: u32, z: u32) -> u32 {
    (x & z).wrapping_add((!z) & y)
}
fn f3(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}
fn f4(x: u32, y: u32, z: u32) -> u32 {
    y ^ (x | !z)
}

macro_rules! round {
    ($f:path, $w:ident, $x:ident, $y:ident, $z:ident, $k:expr, $s:expr, $ti:expr, $current:expr) => {
        // Read the source data in little endian format
        let input = u32::from_le_bytes([
            $current[$k * 4],
            $current[$k * 4 + 1],
            $current[$k * 4 + 2],
            $current[$k * 4 + 3],
        ]);
        $w = $x.wrapping_add(
            $w.wrapping_add($f($x, $y, $z))
                .wrapping_add(input)
                .wrapping_add($ti)
                .rotate_left($s),
        );
    };
}

impl Md5Context {
    pub fn new() -> Md5Context {
        let mut context = Md5Context {
            block: [0; BUFFERSIZE],
            used: 0,
            state: [0; 4],
            bytes: 0,
        };
        context.reset();
        context
    }

    // Initialise the 16 byte state
    pub fn reset(&mut self) {
        self.state[0] = 0x67452301;
        self.state[1] = 0xefcdab89;
        self.state[2] = 0x98badcfe;
        self.state[3] = 0x10325476;
        self.used = 0;
        self.bytes = 0;
    }

    // Update the state using 64 bytes of new data
    fn update_state(&mut self, current: &[u8]) {
        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];

        round!(f1, a, b, c, d, 0, 7, 0xd76aa478, current);
        round!(f1, d, a, b, c, 1, 12, 0xe8c7b756, current);
        round!(f1, c, d, a, b, 2, 17, 0x242070db, current);
        round!(f1, b, c, d, a, 3, 22, 0xc1bdceee, current);

        round!(f1, a, b, c, d, 4, 7, 0xf57c0faf, current);
        round!(f1, d, a, b, c, 5, 12, 0x4787c62a, current);
        round!(f1, c, d, a, b, 6, 17, 0xa8304613, current);
        round!(f1, b, c, d, a, 7, 22, 0xfd469501, current);

        round!(f1, a, b, c, d, 8, 7, 0x698098d8, current);
        round!(f1, d, a, b, c, 9, 12, 0x8b44f7af, current);
        round!(f1, c, d, a, b, 10, 17, 0xffff5bb1, current);
        round!(f1, b, c, d, a, 11, 22, 0x895cd7be, current);

        round!(f1, a, b, c, d, 12, 7, 0x6b901122, current);
        round!(f1, d, a, b, c, 13, 12, 0xfd987193, current);
        round!(f1, c, d, a, b, 14, 17, 0xa679438e, current);
        round!(f1, b, c, d, a, 15, 22, 0x49b40821, current);

        round!(f2, a, b, c, d, 1, 5, 0xf61e2562, current);
        round!(f2, d, a, b, c, 6, 9, 0xc040b340, current);
        round!(f2, c, d, a, b, 11, 14, 0x265e5a51, current);
        round!(f2, b, c, d, a, 0, 20, 0xe9b6c7aa, current);

        round!(f2, a, b, c, d, 5, 5, 0xd62f105d, current);
        round!(f2, d, a, b, c, 10, 9, 0x02441453, current);
        round!(f2, c, d, a, b, 15, 14, 0xd8a1e681, current);
        round!(f2, b, c, d, a, 4, 20, 0xe7d3fbc8, current);

        round!(f2, a, b, c, d, 9, 5, 0x21e1cde6, current);
        round!(f2, d, a, b, c, 14, 9, 0xc33707d6, current);
        round!(f2, c, d, a, b, 3, 14, 0xf4d50d87, current);
        round!(f2, b, c, d, a, 8, 20, 0x455a14ed, current);

        round!(f2, a, b, c, d, 13, 5, 0xa9e3e905, current);
        round!(f2, d, a, b, c, 2, 9, 0xfcefa3f8, current);
        round!(f2, c, d, a, b, 7, 14, 0x676f02d9, current);
        round!(f2, b, c, d, a, 12, 20, 0x8d2a4c8a, current);

        round!(f3, a, b, c, d, 5, 4, 0xfffa3942, current);
        round!(f3, d, a, b, c, 8, 11, 0x8771f681, current);
        round!(f3, c, d, a, b, 11, 16, 0x6d9d6122, current);
        round!(f3, b, c, d, a, 14, 23, 0xfde5380c, current);

        round!(f3, a, b, c, d, 1, 4, 0xa4beea44, current);
        round!(f3, d, a, b, c, 4, 11, 0x4bdecfa9, current);
        round!(f3, c, d, a, b, 7, 16, 0xf6bb4b60, current);
        round!(f3, b, c, d, a, 10, 23, 0xbebfbc70, current);

        round!(f3, a, b, c, d, 13, 4, 0x289b7ec6, current);
        round!(f3, d, a, b, c, 0, 11, 0xeaa127fa, current);
        round!(f3, c, d, a, b, 3, 16, 0xd4ef3085, current);
        round!(f3, b, c, d, a, 6, 23, 0x04881d05, current);

        round!(f3, a, b, c, d, 9, 4, 0xd9d4d039, current);
        round!(f3, d, a, b, c, 12, 11, 0xe6db99e5, current);
        round!(f3, c, d, a, b, 15, 16, 0x1fa27cf8, current);
        round!(f3, b, c, d, a, 2, 23, 0xc4ac5665, current);

        round!(f4, a, b, c, d, 0, 6, 0xf4292244, current);
        round!(f4, d, a, b, c, 7, 10, 0x432aff97, current);
        round!(f4, c, d, a, b, 14, 15, 0xab9423a7, current);
        round!(f4, b, c, d, a, 5, 21, 0xfc93a039, current);

        round!(f4, a, b, c, d, 12, 6, 0x655b59c3, current);
        round!(f4, d, a, b, c, 3, 10, 0x8f0ccc92, current);
        round!(f4, c, d, a, b, 10, 15, 0xffeff47d, current);
        round!(f4, b, c, d, a, 1, 21, 0x85845dd1, current);

        round!(f4, a, b, c, d, 8, 6, 0x6fa87e4f, current);
        round!(f4, d, a, b, c, 15, 10, 0xfe2ce6e0, current);
        round!(f4, c, d, a, b, 6, 15, 0xa3014314, current);
        round!(f4, b, c, d, a, 13, 21, 0x4e0811a1, current);

        round!(f4, a, b, c, d, 4, 6, 0xf7537e82, current);
        round!(f4, d, a, b, c, 11, 10, 0xbd3af235, current);
        round!(f4, c, d, a, b, 2, 15, 0x2ad7d2bb, current);
        round!(f4, b, c, d, a, 9, 21, 0xeb86d391, current);

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
    }

    // Update using 0 bytes
    pub fn update_zeros(&mut self, mut length: usize) {
        // Update the total amount of data processed.
        self.bytes = self.bytes.wrapping_add(length as u64);

        // If there is already some data in the buffer, update
        // enough 0 bytes to take us to a whole buffer
        if self.used > 0 {
            if self.used + length >= BUFFERSIZE {
                let have = BUFFERSIZE - self.used;
                self.block[self.used..self.used + have].fill(0);
                length -= have;
                let block = self.block;
                self.update_state(&block);
                self.used = 0;
            } else {
                // Don't have enough 0 bytes for a whole buffer
                self.block[self.used..self.used + length].fill(0);
                self.used += length;
                return;
            }
        }

        // Update as many whole buffers as possible
        self.block.fill(0);
        while length >= BUFFERSIZE {
            let block = self.block;
            self.update_state(&block);

            length -= BUFFERSIZE;
        }

        // Note down the remainder
        self.used = length;
    }

    // Update using data from a buffer
    pub fn update(&mut self, buffer: &[u8]) {
        let mut current = buffer;
        let mut length = buffer.len();

        // Update the total amount of data processed.
        self.bytes = self.bytes.wrapping_add(length as u64);

        // Process first whole block if we have something in the buffer
        if self.used > 0 && self.used + length >= BUFFERSIZE {
            let have = BUFFERSIZE - self.used;

            self.block[self.used..BUFFERSIZE].copy_from_slice(&current[..have]);

            current = &current[have..];
            length -= have;

            let block = self.block;
            self.update_state(&block);

            self.used = 0;
        }

        // Process all subsequent whole blocks
        while length >= BUFFERSIZE {
            self.update_state(&current[..BUFFERSIZE]);
            current = &current[BUFFERSIZE..];
            length -= BUFFERSIZE;
        }

        // Store any remainder
        if length > 0 {
            self.block[self.used..self.used + length].copy_from_slice(current);
            self.used += length;
        }
    }

    // Finalise the computation and extract the Hash value
    // The `>> 0` and `+ 0` keep the 1:1 shift pattern of the C++ original
    #[allow(clippy::identity_op)]
    pub fn finalize(&mut self) -> Md5Hash {
        // Temporary work buffer
        let mut buffer = [0u8; 64];

        // How many bits were processed
        let bits = self.bytes << 3;

        // Pad as much as needed so that there are exactly 8 bytes needed to fill the buffer
        let padding = if self.used >= BUFFERSIZE - 8 {
            BUFFERSIZE - 8 + BUFFERSIZE - self.used
        } else {
            BUFFERSIZE - 8 - self.used
        };
        for b in buffer[..padding].iter_mut() {
            *b = 0;
        }
        buffer[0] = 0x80;
        self.update(&buffer[..padding]);

        // Pad with an additional 8 bytes containing the bit count in little endian format
        buffer[..8].copy_from_slice(&bits.to_le_bytes());
        self.update(&buffer[..8]);

        let mut output = Md5Hash::default();
        for i in 0..4 {
            // Read out the state and convert it from internal format to little endian format
            output.hash[4 * i + 3] = ((self.state[i] >> 24) & 0xFF) as u8;
            output.hash[4 * i + 2] = ((self.state[i] >> 16) & 0xFF) as u8;
            output.hash[4 * i + 1] = ((self.state[i] >> 8) & 0xFF) as u8;
            output.hash[4 * i + 0] = ((self.state[i] >> 0) & 0xFF) as u8;
        }

        output
    }

    // Get the Hash value
    // The `>> 0` and `+ 0` keep the 1:1 shift pattern of the C++ original
    #[allow(clippy::identity_op)]
    pub fn hash(&self) -> Md5Hash {
        let mut output = Md5Hash::default();

        for i in 0..4 {
            // Read out the state and convert it from internal format to little endian format
            output.hash[4 * i + 3] = ((self.state[i] >> 24) & 0xFF) as u8;
            output.hash[4 * i + 2] = ((self.state[i] >> 16) & 0xFF) as u8;
            output.hash[4 * i + 1] = ((self.state[i] >> 8) & 0xFF) as u8;
            output.hash[4 * i + 0] = ((self.state[i] >> 0) & 0xFF) as u8;
        }

        output
    }

    // Get the total number of bytes processed
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    // The state, in the order Final stores its bytes, followed by the number
    // of bytes processed
    fn print_context(&self) -> String {
        let hash = self.hash();

        let mut buffer = String::with_capacity(49);
        print_bytes(&hash.hash, &mut buffer);
        buffer.push(':');
        for index in 0..8 {
            // Most significant byte first, so the count reads as a number
            let byte = ((self.bytes >> (56 - 8 * index)) & 0xFF) as u8;
            print_bytes(&[byte], &mut buffer);
        }
        buffer
    }

    pub fn print(&self) -> String {
        self.print_context()
    }
}

impl Default for Md5Context {
    fn default() -> Md5Context {
        Md5Context::new()
    }
}

impl fmt::Display for Md5Context {
    fn fmt(&self, result: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(result, "{}", self.print_context())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small stand-in for the srand/rand of the C++ tests.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Rng {
            Rng(seed)
        }

        fn below(&mut self, max: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % max
        }
    }

    // compares update_zeros(length) to update(buffer, buffersize)
    #[test]
    fn test1_zeros_via_length_equals_zeros_via_buffer() {
        let buffer: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 0];

        let mut context1 = Md5Context::new();
        context1.update(&buffer);
        let hash1 = context1.finalize();

        let mut context2 = Md5Context::new();
        context2.update_zeros(buffer.len());
        let hash2 = context2.finalize();

        assert_eq!(hash1, hash2, "hash1 = {hash1}\nhash2 = {hash2}");
    }

    // "MD5 of a null string is d41d8cd98f00b204e9800998ecf8427e"
    // according to https://news.ycombinator.com/item?id=5653698
    #[test]
    fn test2_null_string_hash() {
        let mut context1 = Md5Context::new();
        let hash1 = context1.finalize();

        let hash2 = Md5Hash {
            hash: [
                0xd4, 0x1d, 0x8c, 0xd9, 0x8f, 0x00, 0xb2, 0x04, 0xe9, 0x80, 0x09, 0x98, 0xec,
                0xf8, 0x42, 0x7e,
            ],
        };

        for i in 0..16 {
            assert_eq!(
                hash1.hash[i], hash2.hash[i],
                "hash1 and hash2 differ in location {i}"
            );
        }
    }

    // test comparison operators
    #[test]
    fn test3_comparison_operators() {
        let mut context1 = Md5Context::new();
        let mut hash1 = context1.finalize();

        let mut hash2 = hash1;

        assert!(hash1 == hash2, "equal fail");
        assert!(!(hash1 != hash2), "not equal fail");
        assert!(!(hash1 < hash2), "less than fail");
        assert!(!(hash1 > hash2), "greater than fail");
        assert!(hash1 <= hash2, "less than or equal fail");
        assert!(hash1 >= hash2, "greater than or equal fail");

        // make hash1 less than hash2 in first place
        hash1.hash[0] = 0x0;
        hash2.hash[0] = 0x1;
        assert!(!(hash1 == hash2), "equal fail 2");
        assert!(hash1 != hash2, "not equal fail 2");
        assert!(hash1 < hash2, "less than fail 2");
        assert!(!(hash1 > hash2), "greater than fail 2");
        assert!(hash1 <= hash2, "less than or equal fail 2");
        assert!(!(hash1 >= hash2), "greater than or equal fail 2");

        // make them equal again
        hash1.hash[0] = 0x0;
        hash2.hash[0] = 0x0;
        // now make hash1 less than in 15th place
        hash1.hash[15] = 0x0;
        hash2.hash[15] = 0x1;

        assert!(!(hash1 == hash2), "equal fail 3");
        assert!(hash1 != hash2, "not equal fail 3");
        assert!(hash1 < hash2, "less than fail 3");
        assert!(!(hash1 > hash2), "greater than fail 3");
        assert!(hash1 <= hash2, "less than or equal fail 3");
        assert!(!(hash1 >= hash2), "greater than or equal fail 3");
    }

    // generate random data.
    // put it into two different contexts in different lengths
    // make sure output is the same.
    #[test]
    fn test4_random_chunking_agrees() {
        let mut rng = Rng::new(345087209);
        let mut buffer = [0u8; 32 * 1024];

        for b in buffer.iter_mut() {
            *b = rng.below(256) as u8;
        }

        let mut context1 = Md5Context::new();
        let mut offset = 0;
        while offset < buffer.len() {
            let mut length = rng.below(256);
            if offset + length > buffer.len() {
                length = buffer.len() - offset;
            }
            context1.update(&buffer[offset..offset + length]);
            offset += length;
        }
        let hash1 = context1.finalize();

        let mut context2 = Md5Context::new();
        offset = 0;
        while offset < buffer.len() {
            let mut length = rng.below(256);
            if offset + length > buffer.len() {
                length = buffer.len() - offset;
            }
            context2.update(&buffer[offset..offset + length]);
            offset += length;
        }
        let hash2 = context2.finalize();

        assert_eq!(hash1, hash2, "random hash1 = {hash1}\nrandom hash2 = {hash2}");
    }

    // Check that a hash prints as the bytes it stores, so it reads the same as
    // it does from md5sum and every other tool
    fn checkprinted(input: &str, expected: &str) {
        let mut context = Md5Context::new();
        context.update(input.as_bytes());
        let hash = context.finalize();

        assert_eq!(hash.print(), expected, "print gave wrong value");

        assert_eq!(format!("{hash}"), expected, "Display gave wrong value");

        // The context prints those same bytes, then the number it has processed
        let wanted = format!("{expected}:{:016X}", context.bytes());

        let printed = context.print();
        assert_eq!(printed, wanted, "context print gave wrong value");

        assert_eq!(format!("{context}"), printed, "context Display gave wrong value");
    }

    #[test]
    fn test5_printing() {
        checkprinted("", "D41D8CD98F00B204E9800998ECF8427E");
        checkprinted("abc", "900150983CD24FB0D6963F7D28E17F72");
    }
}
