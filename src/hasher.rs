//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2026 Michael Nightingale
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

//! Hashes the blocks of one file, and the file itself. Several consecutive
//! blocks are submitted at once, so that an implementation may hash them
//! alongside each other.
//!
//! A file is hashed one of two ways, and an implementation serves both:
//!
//!   creating, where the blocks are submitted and their hashes taken, and the
//!   same submissions advance the hashes of the file
//!
//!   ```text
//!   init, with wholefile set
//!   for each batch:
//!     submit_blocks
//!     collect_blocks
//!   end_file
//!   ```
//!
//!   verifying, where the blocks are checked against what the set records for
//!   them, and the caller hashes the file itself
//!
//!   ```text
//!   init, with wholefile clear
//!   for each batch:
//!     check_blocks
//!   ```
//!
//! A submission says which of the two it belongs to by its filelength, and the
//! two are worth hashing differently. Where it is set, the MD5 of the file is a
//! second chain over the same bytes the block's is taken from, and carrying
//! both through one pass costs little more than the block on its own. Where it
//! is zero only the hashes of the block are wanted, and they come cheaper taken
//! alone. Only a caller which set it reaches end_file, so nothing may be left
//! until then, and collect_blocks never follows check_blocks.
//!
//! One instance takes one file at a time, and init may start another when that
//! one is done. It is used by one thread at a time, but not always the same
//! one: a caller may pass it between the threads checking a file, so nothing
//! may be tied to the thread which called init.

use crate::crc::crc_update_block;
use crate::filechecksummer::FileHasher;
use crate::md5::{Md5Context, Md5Hash};

pub trait Hasher {
    /// Start on a file of filesize bytes whose blocks are blocklength bytes
    /// each. The first 16k of the file is always hashed, the whole of it only
    /// when wholefile is set. filesize is what the hashes of the file are
    /// finished against, and does not limit what may be submitted. False where
    /// the file cannot be taken on, which is the only refusal an
    /// implementation can make: nothing after this reports one.
    fn init(&mut self, filesize: u64, blocklength: usize, wholefile: bool) -> bool;

    /// Submit count consecutive blocks, in file order, laid out end to end in
    /// data. Where the file runs out part way through the last of them the
    /// remainder is already zero, because a block is hashed padded out to
    /// blocklength. filelength is how many of the submitted bytes belong to the
    /// file and advances its hashes; zero leaves them alone, for a caller which
    /// hashes the file elsewhere.
    fn submit_blocks(&mut self, data: &[u8], count: u32, filelength: usize);

    /// Take the hashes of the count blocks submitted earliest and not taken
    /// yet, 20 bytes each in submission order: 16 bytes of MD5 then the CRC32
    /// little endian, which is the layout of a verification packet entry.
    /// Taking them gives them up, so submitting and taking work a queue between
    /// them, and count reaches no further than what is waiting in it.
    fn collect_blocks(&mut self, out: &mut [u8], count: u32);

    /// Submit blocks as submit_blocks does, but only far enough to say whether
    /// each one hashes to the 20 bytes at expected for it, setting
    /// matched[index] to true or false. An implementation may stop on a block
    /// as soon as it knows, and need not keep what it computed: collect_blocks
    /// does not follow this. One instance is asked for this or for
    /// submit_blocks, never for both.
    fn check_blocks(
        &mut self,
        data: &[u8],
        count: u32,
        filelength: usize,
        expected: &[u8],
        matched: &mut [bool],
    );

    /// Take the hashes of the file, 16 bytes each. The full hash is only set
    /// for a hasher asked for it, or a file no larger than 16k. Only a caller
    /// whose submissions advanced the hashes of the file reaches this.
    fn end_file(&mut self, hashfull: &mut [u8; 16], hash16k: &mut [u8; 16]);
}

/// Hashes each block of a batch on its own, and the file alongside them.
pub struct ReferenceHasher {
    filesize: u64,
    blocklength: usize,
    fileoffset: u64,
    filehasher: FileHasher,
    results: Vec<u8>,
}

impl ReferenceHasher {
    pub fn new() -> ReferenceHasher {
        ReferenceHasher {
            filesize: 0,
            blocklength: 0,
            fileoffset: 0,
            filehasher: FileHasher::new(true),
            results: Vec::new(),
        }
    }

    fn block_hash(&self, at: &[u8]) -> Md5Hash {
        let mut context = Md5Context::new();
        context.update(at);

        context.finalize()
    }

    fn block_crc(&self, at: &[u8]) -> u32 {
        !0 ^ crc_update_block(!0, at)
    }

    fn update_file(&mut self, data: &[u8], filelength: usize) {
        if filelength > 0 {
            self.filehasher.update(self.fileoffset, &data[..filelength]);
            self.fileoffset += filelength as u64;
        }
    }
}

impl Default for ReferenceHasher {
    fn default() -> ReferenceHasher {
        ReferenceHasher::new()
    }
}

impl Hasher for ReferenceHasher {
    fn init(&mut self, filesize: u64, blocklength: usize, wholefile: bool) -> bool {
        self.filesize = filesize;
        self.blocklength = blocklength;
        self.fileoffset = 0;
        self.filehasher = FileHasher::new(wholefile);
        self.results.clear();

        blocklength > 0
    }

    fn submit_blocks(&mut self, data: &[u8], count: u32, filelength: usize) {
        debug_assert!(data.len() >= count as usize * self.blocklength);

        for block in 0..count {
            let at = block as usize * self.blocklength;
            let block_data = &data[at..at + self.blocklength];

            let was = self.results.len();
            self.results.resize(was + 20, 0);

            let hash = self.block_hash(block_data);
            self.results[was..was + 16].copy_from_slice(&hash.hash);

            let crc = self.block_crc(block_data);
            self.results[was + 16..was + 20].copy_from_slice(&crc.to_le_bytes());
        }

        self.update_file(data, filelength);
    }

    fn check_blocks(
        &mut self,
        data: &[u8],
        count: u32,
        filelength: usize,
        expected: &[u8],
        matched: &mut [bool],
    ) {
        debug_assert!(data.len() >= count as usize * self.blocklength);
        debug_assert!(expected.len() >= count as usize * 20);
        debug_assert!(matched.len() >= count as usize);

        for block in 0..count {
            let at = block as usize * self.blocklength;
            let block_data = &data[at..at + self.blocklength];
            let want = &expected[block as usize * 20..block as usize * 20 + 20];

            let crc = self.block_crc(block_data);

            // The crc settles most blocks, so the hash is only worth taking
            // when it does not
            if crc.to_le_bytes() == want[16..20] {
                let hash = self.block_hash(block_data);

                matched[block as usize] = hash.hash == want[..16];
            } else {
                matched[block as usize] = false;
            }
        }

        self.update_file(data, filelength);
    }

    fn collect_blocks(&mut self, out: &mut [u8], count: u32) {
        let taken = count as usize * 20;
        debug_assert!(out.len() >= taken);
        debug_assert!(self.results.len() >= taken);

        out[..taken].copy_from_slice(&self.results[..taken]);
        self.results.drain(..taken);
    }

    fn end_file(&mut self, hashfull: &mut [u8; 16], hash16k: &mut [u8; 16]) {
        let (full, first16k) = self.filehasher.get_hashes(self.filesize);

        if let Some(full) = full {
            hashfull.copy_from_slice(&full.hash);
        }
        hash16k.copy_from_slice(&first16k.hash);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md5::{Md5Context, Md5Hash};

    fn md5(data: &[u8]) -> Md5Hash {
        let mut context = Md5Context::new();
        context.update(data);
        context.finalize()
    }

    fn crc(data: &[u8]) -> u32 {
        !0 ^ crc_update_block(!0, data)
    }

    #[test]
    fn init_refuses_zero_blocklength() {
        let mut hasher = ReferenceHasher::new();
        assert!(!hasher.init(100, 0, true));
        assert!(hasher.init(100, 16, true));
    }

    #[test]
    fn submit_collect_matches_manual_hashes() {
        let blocklength = 64usize;
        let data: Vec<u8> = (0..3 * blocklength).map(|i| (i * 7 + 3) as u8).collect();
        let filesize = data.len() as u64;

        let mut hasher = ReferenceHasher::new();
        assert!(hasher.init(filesize, blocklength, true));

        hasher.submit_blocks(&data, 3, data.len());

        let mut out = [0u8; 60];
        hasher.collect_blocks(&mut out, 3);

        for block in 0..3 {
            let at = block * blocklength;
            let entry = &out[block * 20..block * 20 + 20];
            assert_eq!(&md5(&data[at..at + blocklength]).hash, &entry[..16]);
            assert_eq!(
                crc(&data[at..at + blocklength]).to_le_bytes(),
                entry[16..20]
            );
        }

        // The queue is emptied by taking
        let mut out2 = [0u8; 20];
        hasher.collect_blocks(&mut out2, 0);
    }

    #[test]
    fn end_file_matches_file_hasher() {
        let data: Vec<u8> = (0..40000u32).map(|i| (i * 13 + 5) as u8).collect();
        let filesize = data.len() as u64;

        let mut hasher = ReferenceHasher::new();
        assert!(hasher.init(filesize, 1024, true));
        let mut padded = data.clone();
        padded.resize(40 * 1024, 0);
        hasher.submit_blocks(&padded, 40, data.len());
        let mut out = [0u8; 800];
        hasher.collect_blocks(&mut out, 40);

        let mut hashfull = [0u8; 16];
        let mut hash16k = [0u8; 16];
        hasher.end_file(&mut hashfull, &mut hash16k);

        assert_eq!(hashfull, md5(&data).hash);
        assert_eq!(hash16k, md5(&data[..16384]).hash);
    }

    #[test]
    fn check_blocks_sets_matched_flags() {
        let blocklength = 32usize;
        let data: Vec<u8> = (0..4 * blocklength).map(|i| (i * 11 + 9) as u8).collect();

        // Expected: block 0 correct, block 1 wrong crc, block 2 right crc
        // but wrong hash, block 3 correct.
        let mut expected = Vec::new();
        for block in 0..4 {
            let at = block * blocklength;
            let mut entry = [0u8; 20];
            match block {
                1 => {
                    entry[..16].copy_from_slice(&md5(&data[at..at + blocklength]).hash);
                    entry[16..20].copy_from_slice(&crc(&[0u8; 32]).to_le_bytes());
                }
                2 => {
                    entry[..16].copy_from_slice(&md5(&[0u8; 32]).hash);
                    entry[16..20].copy_from_slice(&crc(&data[at..at + blocklength]).to_le_bytes());
                }
                _ => {
                    entry[..16].copy_from_slice(&md5(&data[at..at + blocklength]).hash);
                    entry[16..20].copy_from_slice(&crc(&data[at..at + blocklength]).to_le_bytes());
                }
            }
            expected.extend_from_slice(&entry);
        }

        let mut hasher = ReferenceHasher::new();
        assert!(hasher.init(0, blocklength, false));

        let mut matched = [true; 4];
        hasher.check_blocks(&data, 4, 0, &expected, &mut matched);

        assert_eq!(matched, [true, false, false, true]);
    }
}
