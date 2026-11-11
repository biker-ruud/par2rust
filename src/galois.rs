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

//! Galois field arithmetic.
//!
//! This defines the Galois object for carrying out arithmetic in GF(2^8)
//! using the generator 0x11D, and in GF(2^16) using the generator 0x1100B.
//!
//! [`GaloisTable`] contains the log and anti log tables used for carrying
//! out multiplication and division.
//!
//! The C++ template parameters (bits, generator, value type) become a
//! [`GaloisSpec`] implementor here, with [`Galois8`] and [`Galois16`] as the
//! two instantiations the rest of the program uses.

use std::sync::OnceLock;

// The value type a Galois field is built from (u8 or u16).
pub trait GaloisValueType: Copy + PartialEq + Eq + std::fmt::Debug + 'static {
    fn from_u32(v: u32) -> Self;
    fn to_u32(self) -> u32;
    fn to_usize(self) -> usize;
    fn is_zero(self) -> bool {
        self.to_u32() == 0
    }
    // Addition in the field is the bit-wise exclusive or of the values.
    fn field_add(self, other: Self) -> Self;
}

impl GaloisValueType for u8 {
    fn from_u32(v: u32) -> u8 {
        v as u8
    }
    fn to_u32(self) -> u32 {
        self as u32
    }
    fn to_usize(self) -> usize {
        self as usize
    }
    fn field_add(self, other: u8) -> u8 {
        self ^ other
    }
}

impl GaloisValueType for u16 {
    fn from_u32(v: u32) -> u16 {
        v as u16
    }
    fn to_u32(self) -> u32 {
        self as u32
    }
    fn to_usize(self) -> usize {
        self as usize
    }
    fn field_add(self, other: u16) -> u16 {
        self ^ other
    }
}

// The parameters of one Galois field, and its one and only log/antilog table.
pub trait GaloisSpec: Copy + std::fmt::Debug {
    const BITS: u32;
    const GENERATOR: u32;
    type Value: GaloisValueType;

    fn table() -> &'static GaloisTable<Self::Value>;
}

// The log and antilog tables for one field.
pub struct GaloisTable<V: GaloisValueType> {
    pub bits: u32,
    pub count: usize,
    pub limit: usize,
    pub generator: u32,

    pub log: Vec<V>,
    pub antilog: Vec<V>,
}

impl<V: GaloisValueType> GaloisTable<V> {
    // Construct the log and antilog tables from the generator
    fn new(bits: u32, generator: u32) -> GaloisTable<V> {
        let count = 1usize << bits;
        let limit = count - 1;

        let mut log = vec![V::from_u32(0); count];
        let mut antilog = vec![V::from_u32(0); count];

        let mut b: u32 = 1;

        for l in 0..limit as u32 {
            log[b as usize] = V::from_u32(l);
            antilog[l as usize] = V::from_u32(b);

            b <<= 1;
            if b & count as u32 != 0 {
                b ^= generator;
            }
        }

        log[0] = V::from_u32(limit as u32);
        antilog[limit] = V::from_u32(0);

        GaloisTable {
            bits,
            count,
            limit,
            generator,
            log,
            antilog,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Spec8 {}

impl GaloisSpec for Spec8 {
    const BITS: u32 = 8;
    const GENERATOR: u32 = 0x11D;
    type Value = u8;

    fn table() -> &'static GaloisTable<u8> {
        static TABLE: OnceLock<GaloisTable<u8>> = OnceLock::new();
        TABLE.get_or_init(|| GaloisTable::new(Self::BITS, Self::GENERATOR))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Spec16 {}

impl GaloisSpec for Spec16 {
    const BITS: u32 = 16;
    const GENERATOR: u32 = 0x1100B;
    type Value = u16;

    fn table() -> &'static GaloisTable<u16> {
        static TABLE: OnceLock<GaloisTable<u16>> = OnceLock::new();
        TABLE.get_or_init(|| GaloisTable::new(Self::BITS, Self::GENERATOR))
    }
}

// A value in a Galois field.
#[derive(Clone, Copy, Debug)]
pub struct Galois<S: GaloisSpec> {
    value: S::Value,
}

impl<S: GaloisSpec> PartialEq for Galois<S> {
    fn eq(&self, other: &Galois<S>) -> bool {
        self.value == other.value
    }
}

impl<S: GaloisSpec> Eq for Galois<S> {}

pub type Galois8 = Galois<Spec8>;
pub type Galois16 = Galois<Spec16>;

impl<S: GaloisSpec> Galois<S> {
    pub fn new(value: S::Value) -> Galois<S> {
        Galois { value }
    }

    // Cast to value and value access
    pub fn value(self) -> S::Value {
        self.value
    }

    // Direct log and antilog
    pub fn log(self) -> S::Value {
        S::table().log[self.value.to_usize()]
    }

    pub fn alog(self) -> S::Value {
        S::table().antilog[self.value.to_usize()]
    }

    // Power
    pub fn pow(self, right: u32) -> Galois<S> {
        if right == 0 {
            return Galois::new(S::Value::from_u32(1));
        }
        if self.value.is_zero() {
            return Galois::new(S::Value::from_u32(0));
        }

        let table = S::table();
        let mut sum = table.log[self.value.to_usize()].to_u32().wrapping_mul(right);

        sum = (sum >> S::BITS) + (sum & table.limit as u32);
        if sum as usize >= table.limit {
            Galois::new(table.antilog[sum as usize - table.limit])
        } else {
            Galois::new(table.antilog[sum as usize])
        }
    }
}

// Addition
impl<S: GaloisSpec> std::ops::Add<Galois<S>> for Galois<S> {
    type Output = Galois<S>;
    fn add(self, right: Galois<S>) -> Galois<S> {
        Galois::new(self.value.field_add(right.value))
    }
}

impl<S: GaloisSpec> std::ops::AddAssign<Galois<S>> for Galois<S> {
    fn add_assign(&mut self, right: Galois<S>) {
        self.value = self.value.field_add(right.value);
    }
}

// Subtraction
impl<S: GaloisSpec> std::ops::Sub<Galois<S>> for Galois<S> {
    type Output = Galois<S>;
    fn sub(self, right: Galois<S>) -> Galois<S> {
        Galois::new(self.value.field_add(right.value))
    }
}

impl<S: GaloisSpec> std::ops::SubAssign<Galois<S>> for Galois<S> {
    fn sub_assign(&mut self, right: Galois<S>) {
        self.value = self.value.field_add(right.value);
    }
}

// Multiplication
impl<S: GaloisSpec> std::ops::Mul<Galois<S>> for Galois<S> {
    type Output = Galois<S>;
    fn mul(self, right: Galois<S>) -> Galois<S> {
        if self.value.is_zero() || right.value.is_zero() {
            return Galois::new(S::Value::from_u32(0));
        }
        let table = S::table();
        let sum = table.log[self.value.to_usize()].to_usize()
            + table.log[right.value.to_usize()].to_usize();
        if sum >= table.limit {
            Galois::new(table.antilog[sum - table.limit])
        } else {
            Galois::new(table.antilog[sum])
        }
    }
}

impl<S: GaloisSpec> std::ops::MulAssign<Galois<S>> for Galois<S> {
    fn mul_assign(&mut self, right: Galois<S>) {
        *self = *self * right;
    }
}

// Division
impl<S: GaloisSpec> std::ops::Div<Galois<S>> for Galois<S> {
    type Output = Galois<S>;
    fn div(self, right: Galois<S>) -> Galois<S> {
        if self.value.is_zero() {
            return Galois::new(S::Value::from_u32(0));
        }

        debug_assert!(!right.value.is_zero(), "Division by 0!");
        if right.value.is_zero() {
            return Galois::new(S::Value::from_u32(0)); // Division by 0!
        }

        let table = S::table();
        let sum = table.log[self.value.to_usize()].to_u32() as i32
            - table.log[right.value.to_usize()].to_u32() as i32;
        if sum < 0 {
            Galois::new(table.antilog[(sum + table.limit as i32) as usize])
        } else {
            Galois::new(table.antilog[sum as usize])
        }
    }
}

impl<S: GaloisSpec> std::ops::DivAssign<Galois<S>> for Galois<S> {
    fn div_assign(&mut self, right: Galois<S>) {
        if self.value.is_zero() {
            return;
        }

        debug_assert!(!right.value.is_zero(), "Division by 0!");
        if right.value.is_zero() {
            return; // Division by 0!
        }

        let table = S::table();
        let sum = table.log[self.value.to_usize()].to_u32() as i32
            - table.log[right.value.to_usize()].to_u32() as i32;
        self.value = if sum < 0 {
            table.antilog[(sum + table.limit as i32) as usize]
        } else {
            table.antilog[sum as usize]
        };
    }
}

// Power (the C++ operator ^ raises to a power, it is not an exclusive or)
impl<S: GaloisSpec> std::ops::BitXor<u32> for Galois<S> {
    type Output = Galois<S>;
    fn bitxor(self, right: u32) -> Galois<S> {
        self.pow(right)
    }
}

impl<S: GaloisSpec> Galois<S> {
    pub fn pow_assign(&mut self, right: u32) {
        if right == 0 {
            self.value = S::Value::from_u32(1);
            return;
        }
        if self.value.is_zero() {
            return;
        }

        let table = S::table();
        let mut sum = table.log[self.value.to_usize()].to_u32().wrapping_mul(right);

        sum = (sum >> S::BITS) + (sum & table.limit as u32);
        self.value = if sum as usize >= table.limit {
            table.antilog[sum as usize - table.limit]
        } else {
            table.antilog[sum as usize]
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small stand-in for the srand/rand of the C++ tests: the tests only
    // compare the field against its own axioms, and need the values the same
    // way every run.
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

    // Galois are finite fields.  (A field with a finite number of elements.)
    // A field has:
    //    addition
    //    multiplication
    // such that
    //    + and * are associative
    //    + and * are commutative
    //    exists 0 such that a+0 = 0+a = a
    //    exists 1 such that a*1 = 1*a = 1
    //    for all a, exists addative inverse -a such that a + -a = 0
    //    for all a, exists multiplicative inverse a^-1 such that a * a^-1 = 1
    //    * is distributive over +
    fn test_field<S: GaloisSpec>(
        a: Galois<S>,
        b: Galois<S>,
        c: Galois<S>,
        zero: Galois<S>,
        one: Galois<S>,
        max_value: usize,
    ) {
        assert_eq!((a + b) + c, a + (b + c), "addative associativity");

        assert_eq!((a * b) * c, a * (b * c), "multiplicative associativity");

        assert_eq!(a + b, b + a, "addative commutivity");

        assert_eq!(a * b, b * a, "multiplicative commutivity");

        assert_eq!(a * (b + c), (a * b) + (a * c), "distributivity");

        assert_eq!(a + zero, a, "addative identity (right)");
        assert_eq!(zero + a, a, "addative identity (left)");

        assert_eq!(a * one, a, "multiplicative identity (right)");
        assert_eq!(one * a, a, "multiplicative identity (left)");

        // search for inverses
        let mut addative_inverse = -1i32;
        let mut multiplicative_inverse = -1i32;
        for i in 0..max_value {
            let other = Galois::new(S::Value::from_u32(i as u32));

            if a + other == zero {
                assert_eq!(
                    addative_inverse, -1,
                    "value {a:?} has two addative inverses at {addative_inverse} and {i}"
                );
                addative_inverse = i as i32;
            }

            if a * other == one {
                assert_eq!(
                    multiplicative_inverse, -1,
                    "value {a:?} has two multiplicative inverses at {multiplicative_inverse} and {i}"
                );
                multiplicative_inverse = i as i32;
            }
        }
        assert_ne!(addative_inverse, -1, "value {a:?} has no addative inverse!");
        if a != zero {
            assert_ne!(
                multiplicative_inverse, -1,
                "value {a:?} has no multiplicative inverse!"
            );
        }
    }

    fn test_field_many<S: GaloisSpec>(zero: Galois<S>, one: Galois<S>, max_value: usize) {
        let mut rng = Rng::new(345087209);

        // The C++ test draws 65536 samples; the inverse search makes every
        // sample cost max_value field operations, so the 16-bit field samples
        // less often to keep the test quick.
        let samples = if max_value > 256 { 256 } else { 256 * 256 };

        for i in 0..samples {
            test_field::<S>(
                Galois::new(S::Value::from_u32((i % max_value) as u32)),
                Galois::new(S::Value::from_u32(rng.below(max_value) as u32)),
                Galois::new(S::Value::from_u32(rng.below(max_value) as u32)),
                zero,
                one,
                max_value,
            );
        }
    }

    #[test]
    fn test1_field_8() {
        test_field_many::<Spec8>(Galois8::new(0), Galois8::new(1), 256);
    }

    #[test]
    fn test2_field_16() {
        test_field_many::<Spec16>(Galois16::new(0), Galois16::new(1), 256 * 256);
    }

    // `a - a` and `a / a` are the point of these checks, not accidents
    #[allow(clippy::eq_op)]
    fn test_operators<S: GaloisSpec>(
        a: Galois<S>,
        b: Galois<S>,
        power: u32,
        zero: Galois<S>,
        one: Galois<S>,
        max_value: usize,
    ) {
        let mut c = a;
        c += b;
        assert_eq!(c, a + b, "+= fails");

        assert_eq!(a - a, zero, "- fails");

        c = a;
        c -= a;
        assert_eq!(c, zero, "-= fails");

        c = a;
        c *= b;
        assert_eq!(c, a * b, "*= fails");

        if a != zero {
            assert_eq!(a / a, one, "/ fails");

            c = a;
            c /= a;
            assert_eq!(c, one, "/= fails");
        }

        c = one;
        for _ in 0..power {
            c *= a;
        }
        assert_eq!(a ^ power, c, "^ fails");
        assert_eq!(a.pow(power), c, "pow() fails");

        c = a;
        c.pow_assign(power);
        assert_eq!(c, a ^ power, "pow_assign() fails");

        if a != zero && b != zero {
            let la = a.log().to_u32();
            let lb = b.log().to_u32();
            let lab = (a * b).log().to_u32();
            assert_eq!(
                (la + lb) % (max_value as u32 - 1),
                lab,
                "Log fails"
            );
        }

        c = Galois::new(Galois::<S>::new(a.log()).alog());
        assert_eq!(c, a, "ALog fails");
    }

    fn test_operators_many<S: GaloisSpec>(zero: Galois<S>, one: Galois<S>, max_value: usize) {
        let mut rng = Rng::new(14531119);

        for i in 0..256 * 256 {
            let power = if i % 64 == 0 {
                rng.below(max_value) as u32
            } else {
                rng.below(256) as u32
            };
            test_operators::<S>(
                Galois::new(S::Value::from_u32((i % max_value) as u32)),
                Galois::new(S::Value::from_u32(rng.below(max_value) as u32)),
                power,
                zero,
                one,
                max_value,
            );
        }
    }

    #[test]
    fn test3_operators_8() {
        test_operators_many::<Spec8>(Galois8::new(0), Galois8::new(1), 256);
    }

    #[test]
    fn test4_operators_16() {
        test_operators_many::<Spec16>(Galois16::new(0), Galois16::new(1), 256 * 256);
    }

    // test powers of Galois
    // 2^pow should be unique for pow in range 1 to N-1.
    // and 2^N = 2
    // and 2^pow != 0 for any pow
    fn test_powers<S: GaloisSpec>(two: Galois<S>, max_value: usize) {
        let mut g = two;

        let mut used = vec![0i32; max_value];

        // mark 0 as used, so we get an error if it reaches it.
        used[0] = -1;

        for power in 1..max_value {
            let index = g.value().to_usize();
            assert_eq!(
                used[index], 0,
                "error at power {power} was already used by power {}",
                used[index]
            );

            used[index] = power as i32;
            g *= two;
        }

        assert_eq!(g, two, "error after power {max_value}");
    }

    #[test]
    fn test5_powers_8() {
        test_powers::<Spec8>(Galois8::new(2), 256);
    }

    #[test]
    fn test6_powers_16() {
        test_powers::<Spec16>(Galois16::new(2), 256 * 256);
    }
}
