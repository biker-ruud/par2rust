//  This file is part of par2cmdline (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2026 Michael Nightingale
//
//  par2cmdline is free software; you can redistribute it and/or modify
//  it under the terms of the GNU General Public License as published by
//  the Free Software Foundation; either version 2 of the License, or
//  (at your option) any later version.
//
//  par2cmdline is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU General Public License for more details.
//
//  You should have received a copy of the GNU General Public License
//  along with this program; if not, write to the Free Software
//  Foundation, Inc., 59 Temple Place, Suite 330, Boston, MA  02111-1307  USA

//! Multiplies input blocks by the Reed Solomon matrix and accumulates the
//! results. One input block is submitted against every output block at once,
//! so that an implementation chooses how to tile the work and how to spread
//! it over the threads it is given.
//!
//! Every block, in and out, is a sequence of 16 bit Galois values, each
//! little endian, which is the layout they have on disk. The values belong
//! to GF(2^16) with the generator 0x1100B.
//!
//! The work is done in chunks of up to the length `init` is given, one pass
//! over every input block for each chunk:
//!
//!   init
//!   offer_erasures                       repairing only
//!   for each chunk:
//!     set_chunk_length
//!     reset_output
//!     offer_recovery_exponents           creating only
//!     for each input block:
//!       wait_for_add
//!       add_input
//!     end_input
//!     for each output block:
//!       peek_output, and get_output where it gives nothing
//!
//! Only the methods without a body need implementing. The three carrying one
//! are chances to do less work, and an implementation which overrides none of
//! them is correct.

use std::sync::mpsc::Receiver;

pub trait Processor {
    /// Accumulate `outputcount` output blocks of up to `maxlength` bytes
    /// each. The accumulated blocks need not start at zero, `reset_output`
    /// coming before anything is submitted against them.
    fn init(&mut self, maxlength: usize, outputcount: u32) -> bool;

    /// The length of the blocks submitted until the next call, never more
    /// than the `maxlength` given to `init`.
    fn set_chunk_length(&mut self, length: usize);

    /// Set every accumulated output block back to zero.
    fn reset_output(&mut self);

    /// Offer the number of input blocks which will be submitted, and the
    /// exponents of the output blocks, which are the `outputcount` given to
    /// `init`. An implementation that would rather work the coefficients out
    /// for itself returns true, and is given no factors from then on. Only
    /// creation offers them: after a repair matrix is solved its
    /// coefficients are no longer a function of the exponents.
    fn offer_recovery_exponents(&mut self, inputcount: u32, exponents: &[u16]) -> bool {
        let _ = (inputcount, exponents);
        false
    }

    /// Offer the erasure pattern rather than a solved matrix.
    /// `present[index]` says whether input block `index` was found, and
    /// `exponents` holds those of the recovery blocks standing in for the
    /// ones that were not, in the order they are submitted after the blocks
    /// that were found. An implementation that solves the erasure for itself
    /// returns true, and is given no factors from then on; the caller does
    /// not invert the matrix at all. Only repair offers this, creation
    /// having no erasures to solve.
    fn offer_erasures(&mut self, present: &[bool], exponents: &[u16]) -> bool {
        let _ = (present, exponents);
        false
    }

    /// Wait until `add_input` will not block. The caller holds a bounded
    /// number of submissions in flight and calls this before each of them.
    fn wait_for_add(&mut self);

    /// Submit one input block against every output block. `data` is the
    /// chunk, its length the one last given to `set_chunk_length`.
    /// `factors[index]` is the matrix coefficient for output block `index`,
    /// and is `None` where an offer was taken, leaving `inputindex` to say
    /// which input block this is:
    ///
    ///   creating, it is the source block, counting from zero
    ///   repairing, it counts the submissions: first the source blocks which
    ///   were found, in the order present marks them, then one for each
    ///   exponent `offer_erasures` was given, in that order
    ///
    /// Called from one thread, and always after `wait_for_add`. The returned
    /// receiver becomes ready once data and factors may both be overwritten:
    /// it yields `Ok(())` when the implementation has finished with them, or
    /// `Err` once it has failed and dropped its end without finishing. An
    /// implementation which returns before it has multiplied the block must
    /// not need either of them afterwards.
    fn add_input(
        &mut self,
        data: &[u8],
        inputindex: u32,
        factors: Option<&[u16]>,
    ) -> Receiver<()>;

    /// Wait for every submitted input block to be processed.
    fn end_input(&mut self);

    /// Borrow accumulated output block `index` where the implementation
    /// keeps it somewhere the caller can read, saving the copy
    /// `get_output` makes. `None` means it does not, and the caller uses
    /// `get_output` instead; it never means the block could not be produced,
    /// which only `get_output` reports. The block holds the length last
    /// given to `set_chunk_length`, and stays good until the next
    /// submission.
    fn peek_output(&mut self, index: u32) -> Option<&[u8]> {
        let _ = index;
        None
    }

    /// Copy accumulated output block `index` into `out`, which holds the
    /// length last given to `set_chunk_length`. False where the block could
    /// not be produced, which ends the operation.
    fn get_output(&mut self, index: u32, out: &mut [u8]) -> bool;
}
