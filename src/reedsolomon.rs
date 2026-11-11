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

//! 1:1 portage van reedsolomon.h en reedsolomon.cpp.
//!
//! De C++-template-parameter `g` (Galois8 / Galois16) is hier het
//! `GaloisSpec`-type: `ReedSolomon<Spec8>` / `ReedSolomon<Spec16>`.
//!
//! Afwijkingen (documenteel, zoals in eerdere modules):
//! - `ProgressMeter` is weggelaten; de `sout`-teksten die de C++-code
//!   eromheen schrijft worden wel geprint. Foutmeldingen van `serr`
//!   worden als `Err(String)` teruggegeven.
//! - De `LONGMULTIPLY`-optimalisatie van `InternalProcess` is niet
//!   geporteerd; het scalar-pad (de C++ `#else`-tak) wordt gebruikt.
//! - De C++-overloads van `SetInput` heten hier `set_input_present` en
//!   `set_input_all`.
//! - De C++-velden `parpresentindex` / `parmissingindex` zijn nooit in
//!   gebruik genomen en zijn weggelaten.

use crate::galois::{Galois, GaloisSpec, GaloisValueType};
use crate::noiselevel::NoiseLevel;
use crate::par1repairer::{sout_line, SharedSout};

#[derive(Clone, Copy, Debug, Default)]
pub struct RSOutputRow {
    pub present: bool,
    pub exponent: u16,
}

impl RSOutputRow {
    pub fn new(present: bool, exponent: u16) -> RSOutputRow {
        RSOutputRow { present, exponent }
    }
}

pub fn gcd(mut a: u32, mut b: u32) -> u32 {
    if a != 0 && b != 0 {
        while a != 0 && b != 0 {
            if a > b {
                a %= b;
            } else {
                b %= a;
            }
        }

        a + b
    } else {
        0
    }
}

/// Cloning takes the matrix as it stands: a repairer solves its own matrix
/// while a processor, which is built from a snapshot of it, only ever uses
/// the stateless `multiply_add` (as the C++ reference processor does).
#[derive(Clone)]
pub struct ReedSolomon<S: GaloisSpec> {
    input_count: u32, // Total number of input blocks

    data_present: u32,  // Number of input blocks that are present
    data_missing: u32,  // Number of input blocks that are missing
    data_present_index: Vec<u32>, // The index numbers of the data blocks that are present
    data_missing_index: Vec<u32>, // The index numbers of the data blocks that are missing

    database: Vec<S::Value>, // The "base" value to use for each input block

    output_count: u32, // Total number of output blocks

    par_present: u32, // Number of output blocks that are present
    par_missing: u32, // Number of output blocks that are missing

    output_rows: Vec<RSOutputRow>, // Details of the output blocks

    left_matrix: Vec<Galois<S>>, // The main matrix

    // When the matrices are initialised: values of the form base ^ exponent are
    // stored (where the base values are obtained from database[] and the exponent
    // values are obtained from outputrows[]).
}

impl<S: GaloisSpec> Default for ReedSolomon<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: GaloisSpec> ReedSolomon<S> {
    pub fn new() -> ReedSolomon<S> {
        ReedSolomon {
            input_count: 0,

            data_present: 0,
            data_missing: 0,
            data_present_index: Vec::new(),
            data_missing_index: Vec::new(),
            database: Vec::new(),

            output_count: 0,

            par_present: 0,
            par_missing: 0,

            output_rows: Vec::new(),

            left_matrix: Vec::new(),
        }
    }

    // Set which input blocks are present or missing
    // Some input blocks are present
    pub fn set_input_present(&mut self, present: &[bool]) -> Result<(), String> {
        self.input_count = present.len() as u32;

        self.data_present = 0;
        self.data_missing = 0;
        self.data_present_index.clear();
        self.data_missing_index.clear();
        self.database.clear();
        self.database
            .resize(self.input_count as usize, S::Value::from_u32(0));

        if S::BITS == 8 {
            for (base, (index, &is_present)) in (1_u32..).zip(present.iter().enumerate()) {
                // Record the index of the file in the data_present_index array
                // or the data_missing_index array
                if is_present {
                    self.data_present_index.push(index as u32);
                    self.data_present += 1;
                } else {
                    self.data_missing_index.push(index as u32);
                    self.data_missing += 1;
                }

                self.database[index] = S::Value::from_u32(base);
            }
        } else {
            let limit = S::table().limit as u32;
            let mut logbase: u32 = 0;

            for index in 0..self.input_count {
                // Record the index of the file in the data_present_index array
                // or the data_missing_index array
                if present[index as usize] {
                    self.data_present_index.push(index);
                    self.data_present += 1;
                } else {
                    self.data_missing_index.push(index);
                    self.data_missing += 1;
                }

                // Determine the next useable base value.
                // Its log must must be relatively prime to 65535
                while gcd(limit, logbase) != 1 {
                    logbase += 1;
                }
                if logbase >= limit {
                    return Err("Too many input blocks for Reed Solomon matrix.".to_string());
                }
                self.database[index as usize] =
                    Galois::<S>::new(S::Value::from_u32(logbase)).alog();
                logbase += 1;
            }
        }

        Ok(())
    }

    // All input blocks are present
    pub fn set_input_all(&mut self, count: u32) -> Result<(), String> {
        self.input_count = count;

        self.data_present = 0;
        self.data_missing = 0;
        self.data_present_index.clear();
        self.data_missing_index.clear();
        self.database.clear();
        self.database
            .resize(self.input_count as usize, S::Value::from_u32(0));

        if S::BITS == 8 {
            for (base, (index, db)) in (1_u32..).zip(self.database.iter_mut().enumerate()) {
                // Record that the file is present
                self.data_present_index.push(index as u32);
                self.data_present += 1;

                *db = S::Value::from_u32(base);
            }
        } else {
            let limit = S::table().limit as u32;
            let mut logbase: u32 = 0;

            for index in 0..count {
                // Record that the file is present
                self.data_present_index.push(index);
                self.data_present += 1;

                // Determine the next useable base value.
                // Its log must must be relatively prime to 65535
                while gcd(limit, logbase) != 1 {
                    logbase += 1;
                }
                if logbase >= limit {
                    return Err("Too many input blocks for Reed Solomon matrix.".to_string());
                }
                self.database[index as usize] =
                    Galois::<S>::new(S::Value::from_u32(logbase)).alog();
                logbase += 1;
            }
        }

        Ok(())
    }

    // Record whether the recovery block with the specified
    // exponent values is present or missing.
    pub fn set_output(&mut self, present: bool, exponent: u16) -> bool {
        // Store the exponent and whether or not the recovery block is present or missing
        self.output_rows.push(RSOutputRow::new(present, exponent));

        self.output_count += 1;

        // Update the counts.
        if present {
            self.par_present += 1;
        } else {
            self.par_missing += 1;
        }

        true
    }

    // Record whether the recovery blocks with the specified
    // range of exponent values are present or missing.
    pub fn set_output_range(&mut self, present: bool, low_exponent: u16, high_exponent: u16) -> bool {
        for exponent in low_exponent..=high_exponent {
            if !self.set_output(present, exponent) {
                return false;
            }
        }

        true
    }

    // Construct the Vandermonde matrix and solve it if necessary
    pub fn compute(
        &mut self,
        noiselevel: NoiseLevel,
        sout: &SharedSout,
    ) -> Result<(), String> {
        let outcount = (self.data_missing + self.par_missing) as usize;
        let incount = (self.data_present + self.data_missing) as usize;

        if self.data_missing > self.par_present {
            return Err("Not enough recovery blocks.".to_string());
        } else if outcount == 0 {
            return Err("No output blocks.".to_string());
        }

        if noiselevel > NoiseLevel::Quiet {
            sout_line(sout, "Computing Reed Solomon matrix.");
        }

        let zero = Galois::<S>::new(S::Value::from_u32(0));
        let one = Galois::<S>::new(S::Value::from_u32(1));

        let datapresent = self.data_present as usize;
        let datamissing = self.data_missing as usize;
        let parmissing = self.par_missing as usize;

        /*  Layout of RS Matrix:
            NOTE: The second set of columns represents the parity vectors present,
            but this only uses datamissing of them.  Otherwise, it is over constrained.

                                             datamissing
                           datapresent      <=parpresent        datamissing       parmissing
                     /                     |             \ /                     |           \
                     |           (ppi[row])|             | |           (ppi[row])|           |
         datamissing |          ^          |      I      | |          ^          |     0     |
                     |(dpi[col])           |             | |(dmi[col])           |           |
                     +---------------------+-------------+ +---------------------+-----------+
                     |           (pmi[row])|             | |           (pmi[row])|           |
         parmissing  |          ^          |      0      | |          ^          |     I     |
                     |(dpi[col])           |             | |(dmi[col])           |           |
                     \                     |             / \                     |           /
        */

        // Allocate the left hand matrix

        self.left_matrix = vec![zero; outcount * incount];

        // Allocate the right hand matrix only if we are recovering

        let mut rightmatrix: Vec<Galois<S>> = Vec::new();
        if datamissing > 0 {
            rightmatrix = vec![zero; outcount * outcount];
        }

        // Fill in the two matrices:

        let mut outputrow = 0usize;

        // One row for each present recovery block that will be used for a missing data block
        for row in 0..datamissing {
            // Get the exponent of the next present recovery block
            while !self.output_rows[outputrow].present {
                outputrow += 1;
            }
            let exponent = self.output_rows[outputrow].exponent as u32;

            // One column for each present data block
            for col in 0..datapresent {
                let base = self.database[self.data_present_index[col] as usize];
                self.left_matrix[row * incount + col] = Galois::<S>::new(base).pow(exponent);
            }
            // One column for each each present recovery block that will be used for a missing data block
            for col in 0..datamissing {
                self.left_matrix[row * incount + col + datapresent] =
                    if row == col { one } else { zero };
            }

            if datamissing > 0 {
                // One column for each missing data block
                for col in 0..datamissing {
                    let base = self.database[self.data_missing_index[col] as usize];
                    rightmatrix[row * outcount + col] = Galois::<S>::new(base).pow(exponent);
                }
                // One column for each missing recovery block
                for col in 0..parmissing {
                    rightmatrix[row * outcount + col + datamissing] = zero;
                }
            }

            outputrow += 1;
        }

        // One row for each recovery block being computed
        outputrow = 0;
        for row in 0..parmissing {
            // Get the exponent of the next missing recovery block
            while self.output_rows[outputrow].present {
                outputrow += 1;
            }
            let exponent = self.output_rows[outputrow].exponent as u32;

            // One column for each present data block
            for col in 0..datapresent {
                let base = self.database[self.data_present_index[col] as usize];
                self.left_matrix[(row + datamissing) * incount + col] =
                    Galois::<S>::new(base).pow(exponent);
            }
            // One column for each each present recovery block that will be used for a missing data block
            for col in 0..datamissing {
                self.left_matrix[(row + datamissing) * incount + col + datapresent] = zero;
            }

            if datamissing > 0 {
                // One column for each missing data block
                for col in 0..datamissing {
                    let base = self.database[self.data_missing_index[col] as usize];
                    rightmatrix[(row + datamissing) * outcount + col] =
                        Galois::<S>::new(base).pow(exponent);
                }
                // One column for each missing recovery block
                for col in 0..parmissing {
                    rightmatrix[(row + datamissing) * outcount + col + datamissing] =
                        if row == col { one } else { zero };
                }
            }

            outputrow += 1;
        }

        if noiselevel > NoiseLevel::Quiet {
            sout_line(sout, "Constructing: done.");
        }

        // Solve the matrices only if recovering data
        if datamissing > 0 {
            // Perform Gaussian Elimination and then delete the right matrix (which
            // will no longer be required).
            self.gauss_elim(noiselevel, sout, outcount, incount, &mut rightmatrix, datamissing)
        } else {
            Ok(())
        }
    }

    // Process a block of data
    pub fn process(
        &self,
        size: usize,          // The size of the block of data
        input_index: u32,     // The column in the RS matrix
        input: &[u8],         // Buffer containing input data
        output_index: u32,    // The row in the RS matrix
        output: &mut [u8],    // Buffer containing output data
    ) -> bool {
        self.multiply_add(self.get_factor(input_index, output_index), size, input, output)
    }

    // Multiply a block of data by one coefficient and add it to the output
    pub fn multiply_add(
        &self,
        factor: S::Value,
        size: usize,
        input: &[u8],
        output: &mut [u8],
    ) -> bool {
        // Do nothing if the factor happens to be 0
        if factor.is_zero() {
            return true;
        }
        self.internal_process(Galois::<S>::new(factor), size, input, output)
    }

    // The matrix coefficient for one input block against one output block
    pub fn get_factor(&self, input_index: u32, output_index: u32) -> S::Value {
        self.left_matrix[output_index as usize * (self.data_present + self.data_missing) as usize
            + input_index as usize]
            .value()
    }

    #[allow(clippy::needless_range_loop)]
    fn internal_process(
        &self,
        factor: Galois<S>,
        size: usize,
        input: &[u8],
        output: &mut [u8],
    ) -> bool {
        if S::BITS == 8 {
            // Treat the buffers as arrays of 8-bit Galois values.
            for i in 0..size {
                let s = Galois::<S>::new(S::Value::from_u32(input[i] as u32));
                let d = Galois::<S>::new(S::Value::from_u32(output[i] as u32));
                output[i] = (d + s * factor).value().to_u32() as u8;
            }
        } else {
            // Treat the buffers as arrays of 16-bit Galois values.

            // BUG: This only works for __LITTLE_ENDIAN
            let count = size / 2;
            for i in 0..count {
                let s = Galois::<S>::new(S::Value::from_u32(u16::from_le_bytes([
                    input[2 * i],
                    input[2 * i + 1],
                ]) as u32));
                let d = Galois::<S>::new(S::Value::from_u32(u16::from_le_bytes([
                    output[2 * i],
                    output[2 * i + 1],
                ]) as u32));
                let v = (d + s * factor).value().to_u32() as u16;
                output[2 * i] = (v & 0xff) as u8;
                output[2 * i + 1] = (v >> 8) as u8;
            }
        }

        true
    }

    // Perform Gaussian Elimination
    fn gauss_elim(
        &mut self,
        noiselevel: NoiseLevel,
        sout: &SharedSout,
        rows: usize,
        leftcols: usize,
        rightmatrix: &mut [Galois<S>],
        datamissing: usize,
    ) -> Result<(), String> {
        // Because the matrices being operated on are Vandermonde matrices
        // they are guaranteed not to be singular.

        // Additionally, because Galois arithmetic is being used, all calculations
        // involve exact values with no loss of precision. It is therefore
        // not necessary to carry out any row or column swapping.

        let zero = Galois::<S>::new(S::Value::from_u32(0));
        let one = Galois::<S>::new(S::Value::from_u32(1));

        // Solve one row at a time

        // For each row in the matrix
        for row in 0..datamissing {
            // NB Row and column swapping to find a non zero pivot value or to find the largest value
            // is not necessary due to the nature of the arithmetic and construction of the RS matrix.

            // Get the pivot value.
            let pivotvalue = rightmatrix[row * rows + row];
            if pivotvalue == zero {
                return Err("RS computation error.".to_string());
            }

            // If the pivot value is not 1, then the whole row has to be scaled
            if pivotvalue != one {
                for col in 0..leftcols {
                    let v = self.left_matrix[row * leftcols + col];
                    if v != zero {
                        self.left_matrix[row * leftcols + col] = v / pivotvalue;
                    }
                }
                rightmatrix[row * rows + row] = one;
                for col in (row + 1)..rows {
                    let v = rightmatrix[row * rows + col];
                    if v != zero {
                        rightmatrix[row * rows + col] = v / pivotvalue;
                    }
                }
            }

            // For every other row in the matrix
            for row2 in 0..rows {
                if row != row2 {
                    // Get the scaling factor for this row.
                    let scalevalue = rightmatrix[row2 * rows + row];

                    if scalevalue == one {
                        // If the scaling factor happens to be 1, just subtract rows
                        for col in 0..leftcols {
                            let v = self.left_matrix[row * leftcols + col];
                            if v != zero {
                                let d = self.left_matrix[row2 * leftcols + col];
                                self.left_matrix[row2 * leftcols + col] = d - v;
                            }
                        }

                        for col in row..rows {
                            let v = rightmatrix[row * rows + col];
                            if v != zero {
                                let d = rightmatrix[row2 * rows + col];
                                rightmatrix[row2 * rows + col] = d - v;
                            }
                        }
                    } else if scalevalue != zero {
                        // If the scaling factor is not 0, then compute accordingly.
                        for col in 0..leftcols {
                            let v = self.left_matrix[row * leftcols + col];
                            if v != zero {
                                let d = self.left_matrix[row2 * leftcols + col];
                                self.left_matrix[row2 * leftcols + col] = d - v * scalevalue;
                            }
                        }

                        for col in row..rows {
                            let v = rightmatrix[row * rows + col];
                            if v != zero {
                                let d = rightmatrix[row2 * rows + col];
                                rightmatrix[row2 * rows + col] = d - v * scalevalue;
                            }
                        }
                    }
                }
            }
        }

        if noiselevel > NoiseLevel::Quiet {
            sout_line(sout, "Solving: done.");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::galois::{Spec8, Spec16};

    // A small stand-in for the srand/rand of the C++ tests: the tests only
    // need the same values the same way every run.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Rng {
            Rng(seed)
        }

        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
    }

    const BUF_SIZE: usize = 1024;

    fn shared_sout() -> SharedSout {
        let writer: Box<dyn std::io::Write + Send> = Box::new(Vec::<u8>::new());
        std::sync::Arc::new(std::sync::Mutex::new(writer))
    }

    fn generate_data<S: GaloisSpec>(
        seed: u64,
        data: &mut [Vec<u8>],
        in_count: usize,
        recovery_count: usize,
        low_exponent: u16,
    ) -> Result<(), String> {
        let high_exponent = low_exponent + recovery_count as u16 - 1;

        // random input data
        let mut rng = Rng::new(seed);

        for row in data.iter_mut().take(in_count) {
            for byte in row.iter_mut() {
                *byte = (rng.next() % 256) as u8;
            }
        }
        // zero recovery
        for row in data.iter_mut().skip(in_count).take(recovery_count) {
            row.fill(0);
        }

        let mut rs_creator = ReedSolomon::<S>::new();

        rs_creator.set_input_all(in_count as u32)?;
        if !rs_creator.set_output_range(false, low_exponent, high_exponent) {
            return Err("rs_creator.SetOutput returned false".to_string());
        }
        rs_creator.compute(NoiseLevel::Quiet, &shared_sout())?;

        for i in 0..in_count {
            for j in 0..recovery_count {
                let (inputs, recoveries) = data.split_at_mut(in_count);
                rs_creator.process(BUF_SIZE, i as u32, &inputs[i], j as u32, &mut recoveries[j]);
            }
        }

        Ok(())
    }

    fn init_repair_rs<S: GaloisSpec>(
        rs_repair: &mut ReedSolomon<S>,
        in_present: &[bool],
        recovery_present: &[bool],
        low_exponent: u16,
    ) -> Result<(), String> {
        rs_repair.set_input_present(in_present)?;

        for (j, &present) in recovery_present.iter().enumerate() {
            if present && !rs_repair.set_output(true, low_exponent + j as u16) {
                return Err(format!("rs_repair.SetOutput returned false for {}", j));
            }
        }

        rs_repair.compute(NoiseLevel::Quiet, &shared_sout())
    }

    // Strange that "missing place" is 0,1,2,3...
    // no matter what exponent or which in_present are false.
    fn recover_data<S: GaloisSpec>(
        rs_repair: &ReedSolomon<S>,
        missing_place: u32,
        buffer: &mut [u8],
        data: &[Vec<u8>],
        in_present: &[bool],
        recovery_present: &[bool],
    ) {
        buffer.fill(0);

        let mut index = 0u32;
        for i in 0..in_present.len() {
            if in_present[i] {
                rs_repair.process(BUF_SIZE, index, &data[i], missing_place, buffer);
                index += 1;
            }
        }
        for j in 0..recovery_present.len() {
            if recovery_present[j] {
                rs_repair.process(
                    BUF_SIZE,
                    index,
                    &data[in_present.len() + j],
                    missing_place,
                    buffer,
                );
                index += 1;
            }
        }
    }

    fn compare_buffer(buffer: &[u8], expected: &[u8], error_prefix: &str) {
        for k in 0..BUF_SIZE {
            assert_eq!(
                buffer[k], expected[k],
                "{} mismatch at place {}: buffer had {}, expected {}",
                error_prefix, k, buffer[k], expected[k]
            );
        }
    }

    // 4 inputs, recover all possible cases of 2 missing inputs
    fn test1<S: GaloisSpec>() {
        const NUM_IN: usize = 4;
        const NUM_REC: usize = 2; // recovery
        const LOW_EXPONENT: u16 = 0;

        let mut data: Vec<Vec<u8>> = vec![vec![0u8; BUF_SIZE]; NUM_IN + NUM_REC];

        generate_data::<S>(873945932, &mut data, NUM_IN, NUM_REC, LOW_EXPONENT).unwrap();

        // loop over missing input blocks
        for missing1 in 0..NUM_IN {
            for missing2 in missing1 + 1..NUM_IN {
                let in_present: Vec<bool> = (0..NUM_IN)
                    .map(|i| i != missing1 && i != missing2)
                    .collect();
                let recovery_present: Vec<bool> = vec![true; NUM_REC];

                let mut rs_repair = ReedSolomon::<S>::new();
                init_repair_rs(
                    &mut rs_repair,
                    &in_present,
                    &recovery_present,
                    LOW_EXPONENT,
                )
                .unwrap();

                let mut result = vec![0u8; BUF_SIZE];
                recover_data(
                    &rs_repair,
                    0,
                    &mut result,
                    &data,
                    &in_present,
                    &recovery_present,
                );
                compare_buffer(&result, &data[missing1], "test1 - missing1");

                recover_data(
                    &rs_repair,
                    1,
                    &mut result,
                    &data,
                    &in_present,
                    &recovery_present,
                );
                compare_buffer(&result, &data[missing2], "test1 - missing2");
            }
        }
    }

    // recover when all inputs are missing
    fn test2<S: GaloisSpec>() {
        const NUM_IN: usize = 5;
        const NUM_REC: usize = 5; // recovery
        const LOW_EXPONENT: u16 = 0;

        let mut data: Vec<Vec<u8>> = vec![vec![0u8; BUF_SIZE]; NUM_IN + NUM_REC];

        generate_data::<S>(873945932, &mut data, NUM_IN, NUM_REC, LOW_EXPONENT).unwrap();

        let in_present: Vec<bool> = vec![false; NUM_IN];
        let recovery_present: Vec<bool> = vec![true; NUM_REC];

        let mut rs_repair = ReedSolomon::<S>::new();
        init_repair_rs(&mut rs_repair, &in_present, &recovery_present, LOW_EXPONENT).unwrap();

        for i in 0..NUM_IN {
            let mut result = vec![0u8; BUF_SIZE];
            recover_data(
                &rs_repair,
                i as u32,
                &mut result,
                &data,
                &in_present,
                &recovery_present,
            );
            compare_buffer(&result, &data[i], "test2 - missing");
        }
    }

    // Check that the correct constants are being used for Par2

    // The test pretends there are 10 input blocks and 1 recovery block, each
    // 1024 bytes long. The input blocks are zeroed out and a (little endian) 1
    // is written into the i-th location of the i-th input block. The recovery
    // block is generated with many calls to ReedSolomon, so those 1s are
    // multiplied by the coefficients for each input block. The recovery file
    // has exponent 1 and should contain each base to the power 1; those values
    // are checked against the expected values passed to the function.
    fn test4<S: GaloisSpec>(expected_bases: &[u32]) {
        const NUM_REC: usize = 1; // recovery
        const LOW_EXPONENT: u16 = 1;

        let num_in = expected_bases.len();
        let elem = (S::BITS / 8) as usize;

        let high_exponent = LOW_EXPONENT + NUM_REC as u16 - 1;

        let mut data: Vec<Vec<u8>> = vec![vec![0u8; BUF_SIZE]; num_in + NUM_REC];

        for i in 0..num_in {
            // fill with zeros, EXCEPT write a (little endian) 1 in a different
            // place for each file: in the i-th file, it is written into the
            // i-th location
            data[i][elem * i] = 1;
        }
        // zero recovery
        for j in 0..NUM_REC {
            data[num_in + j].fill(0);
        }

        let mut rs_creator = ReedSolomon::<S>::new();

        rs_creator.set_input_all(num_in as u32).unwrap();
        assert!(rs_creator.set_output_range(false, LOW_EXPONENT, high_exponent));
        rs_creator.compute(NoiseLevel::Quiet, &shared_sout()).unwrap();

        for i in 0..num_in {
            for j in 0..NUM_REC {
                let (inputs, recoveries) = data.split_at_mut(num_in);
                rs_creator.process(BUF_SIZE, i as u32, &inputs[i], j as u32, &mut recoveries[j]);
            }
        }

        for i in 0..num_in {
            // read little-endian value
            let mut v: u32 = 0;
            for byte_index in 0..elem {
                v |= (data[num_in][elem * i + byte_index] as u32) << (byte_index * 8);
            }
            assert_eq!(
                v, expected_bases[i],
                "base at location {} did not match expected: base = {}, expected = {}",
                i, v, expected_bases[i]
            );
        }
    }

    #[test]
    fn test1_galois8() {
        test1::<Spec8>();
    }

    #[test]
    fn test1_galois16() {
        test1::<Spec16>();
    }

    #[test]
    fn test2_galois8() {
        test2::<Spec8>();
    }

    #[test]
    fn test2_galois16() {
        test2::<Spec16>();
    }

    #[test]
    fn test4_galois8() {
        // the values for Par1
        test4::<Spec8>(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[test]
    fn test4_galois16() {
        // from the Par2 standard
        test4::<Spec16>(&[2, 4, 16, 128, 256, 2048, 8192, 16384, 4107, 32856]);
    }
}
