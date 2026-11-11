//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2003 Peter Brian Clements
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

//! Little-endian integer types.
//!
//! The PAR packet layouts are little-endian whatever the machine they are
//! read on, and these types hold a value in that order: assigning one stores
//! the bytes as the packets want them, and reading one back converts to the
//! machine's own order.

// The names match the packet layouts they are written against, 1:1 with the
// C++ original, so they keep their lower-case spelling.
#![allow(non_camel_case_types)]

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct leu16(u16);

impl leu16 {
    pub fn new(value: u16) -> leu16 {
        leu16(value.to_le())
    }

    pub fn get(self) -> u16 {
        u16::from_le(self.0)
    }
}

impl From<u16> for leu16 {
    fn from(value: u16) -> leu16 {
        leu16::new(value)
    }
}

impl From<leu16> for u16 {
    fn from(value: leu16) -> u16 {
        value.get()
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct leu32(u32);

impl leu32 {
    pub fn new(value: u32) -> leu32 {
        leu32(value.to_le())
    }

    pub fn get(self) -> u32 {
        u32::from_le(self.0)
    }
}

impl From<u32> for leu32 {
    fn from(value: u32) -> leu32 {
        leu32::new(value)
    }
}

impl From<leu32> for u32 {
    fn from(value: leu32) -> u32 {
        value.get()
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct leu64(u64);

impl leu64 {
    pub fn new(value: u64) -> leu64 {
        leu64(value.to_le())
    }

    pub fn get(self) -> u64 {
        u64::from_le(self.0)
    }
}

impl From<u64> for leu64 {
    fn from(value: u64) -> leu64 {
        leu64::new(value)
    }
}

impl From<leu64> for u64 {
    fn from(value: leu64) -> u64 {
        value.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small stand-in for the srand/rand of the C++ tests: the tests only
    // need values which are not all the same, and need them the same way
    // every run.
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

    // test one value
    #[test]
    fn test1_one_value() {
        let expected: u16 = 127 * 127;
        let x = leu16::new(expected);
        let output: u16 = x.get();

        assert_eq!(output, expected);
    }

    // test all 16 bit values
    #[test]
    fn test2_all_16_bit_values() {
        for i in 0..=u16::MAX as u32 {
            let expected = i as u16;
            let x = leu16::new(expected);
            let output: u16 = x.get();

            assert_eq!(output, expected, "value {expected} did not survive");
        }
    }

    // test one 32-bit value
    #[test]
    fn test3_one_32_bit_value() {
        let expected: u32 = 127 * 127 * 127 * 127;
        let x = leu32::new(expected);
        let output: u32 = x.get();

        assert_eq!(output, expected);
    }

    // test random 32-bit values
    #[test]
    fn test4_random_32_bit_values() {
        let mut rng = Rng::new(113450911);

        for _ in 0..256 * 256 {
            let mut z: u32 = 0;
            z = (z << 8) | rng.next_u8() as u32;
            z = (z << 8) | rng.next_u8() as u32;
            z = (z << 8) | rng.next_u8() as u32;
            z = (z << 8) | rng.next_u8() as u32;

            let expected = z;
            let x = leu32::new(expected);
            let output: u32 = x.get();

            assert_eq!(output, expected, "value {expected} did not survive");
        }
    }

    // test one 64-bit value
    #[test]
    fn test5_one_64_bit_value() {
        let expected: u64 = 127u64
            .checked_pow(8)
            .expect("127^8 fits in a u64");
        let x = leu64::new(expected);
        let output: u64 = x.get();

        assert_eq!(output, expected);
    }

    // test random 64-bit values
    #[test]
    fn test6_random_64_bit_values() {
        let mut rng = Rng::new(84395311);

        for _ in 0..256 * 256 {
            let mut z: u64 = 0;
            for _ in 0..8 {
                z = (z << 8) | rng.next_u8() as u64;
            }

            let expected = z;
            let x = leu64::new(expected);
            let output: u64 = x.get();

            assert_eq!(output, expected, "value {expected} did not survive");
        }
    }
}
