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

//! Port of par2cmdline's filechecksummer.h / filechecksummer.cpp.
//!
//! The FileCheckSummer slides a window one block wide along a data file,
//! maintaining the CRC of the window as it goes. When the computed CRC
//! matches a block of data from a target file, the MD5 hash of the window
//! is also computed and compared. On a match the caller jumps forward to
//! where the next block of data is expected to start. Whilst the file is
//! being scanned the MD5 hashes of the whole file and of the first 16k are
//! also accumulated.
//!
//! C++ keeps four raw pointers into a two-block buffer; the Rust port keeps
//! them as indices (`outindex`, `inindex`, `tailindex`). The C++ DiskFile
//! methods used a global error stream; here `serr` is passed explicitly.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use crate::crc::{crc_slide_char, crc_update_block, crc_update_block_zeros};
use crate::diskfile::DiskFile;
use crate::md5::{Md5Context, Md5Hash};

/// Computes the hash of the whole of a file and of its first 16k from the
/// data of the file supplied in order.
pub struct FileHasher {
    wholefile: bool,
    contextfull: Md5Context,
    context16k: Md5Context,
}

impl FileHasher {
    /// The first 16k is always hashed, the whole file only when asked for.
    pub fn new(wholefile: bool) -> FileHasher {
        FileHasher {
            wholefile,
            contextfull: Md5Context::new(),
            context16k: Md5Context::new(),
        }
    }

    /// Add the next part of the file.
    pub fn update(&mut self, offset: u64, buffer: &[u8]) {
        let length = buffer.len() as u64;

        // Are we already beyond the first 16k
        if offset >= 16384 {
            if self.wholefile {
                self.contextfull.update(buffer);
            }
        }
        // Would we reach the 16k mark
        else if offset + length >= 16384 {
            // Finish the 16k hash
            let first = (16384 - offset) as usize;
            self.context16k.update(&buffer[..first]);

            if self.wholefile {
                // Continue with the full hash
                self.contextfull = self.context16k.clone();

                // Do we go beyond the 16k mark
                if offset + length > 16384 {
                    self.contextfull.update(&buffer[first..]);
                }
            }
        } else {
            self.context16k.update(buffer);
        }
    }

    /// Return the full file hash and the 16k file hash. The full hash is
    /// only set for a whole file hasher, or a file no larger than 16k;
    /// otherwise it is `None`.
    pub fn get_hashes(&self, filesize: u64) -> (Option<Md5Hash>, Md5Hash) {
        // Compute the hash of the first 16k
        let mut context = self.context16k.clone();
        let hash16k = context.finalize();

        // Is the file smaller than 16k
        if filesize < 16384 {
            // The hashes are the same
            (Some(hash16k), hash16k)
        } else if self.wholefile {
            // Compute the hash of the full file
            let mut context = self.contextfull.clone();
            (Some(context.finalize()), hash16k)
        } else {
            (None, hash16k)
        }
    }
}

pub struct FileCheckSummer {
    diskfile: Rc<RefCell<DiskFile>>,
    blocksize: u64,
    windowtable: [u32; 256],

    filesize: u64,

    // Whether the file is being read from its start
    hashesvalid: bool,

    currentoffset: u64, // file offset for current window position
    buffer: Vec<u8>,    // buffer for reading from the file (2 blocks long)
    outindex: usize,    // position in buffer of scan window
    inindex: usize,     // outindex + blocksize
    tailindex: usize,   // after last valid data in buffer

    // File offset for next read
    readoffset: u64,

    // The current checksum
    crc: u32,

    // MD5 hash of whole file and of first 16k
    filehasher: FileHasher,
}

impl FileCheckSummer {
    /// Construct the checksummer and allocate buffers.
    pub fn new(
        diskfile: Rc<RefCell<DiskFile>>,
        blocksize: u64,
        windowtable: &[u32; 256],
        computefilehashes: bool,
    ) -> FileCheckSummer {
        let filesize = diskfile.borrow().file_size();

        FileCheckSummer {
            diskfile,
            blocksize,
            windowtable: *windowtable,
            filesize,
            hashesvalid: false,
            currentoffset: 0,
            buffer: vec![0u8; (blocksize as usize) * 2],
            outindex: 0,
            inindex: 0,
            tailindex: 0,
            readoffset: 0,
            crc: 0,
            filehasher: FileHasher::new(computefilehashes),
        }
    }

    /// Start reading the file at the beginning. The whole file and 16k
    /// hashes can only be computed when starting at 0.
    pub fn start(&mut self, serr: &mut dyn Write) -> bool {
        self.start_at(0, serr)
    }

    /// Start reading the file at the given offset.
    pub fn start_at(&mut self, startoffset: u64, serr: &mut dyn Write) -> bool {
        // The file hashes can only be computed from the start of the file
        self.hashesvalid = startoffset == 0;

        self.currentoffset = startoffset;
        self.readoffset = startoffset;

        self.tailindex = 0;
        self.outindex = 0;
        self.inindex = self.blocksize as usize;

        self.blank_past_end_of_file();

        // Fill the buffer with new data
        if !self.fill(false, serr) {
            return false;
        }

        // Compute the checksum for the block
        let bs = self.blocksize as usize;
        self.crc = !0u32 ^ crc_update_block(!0, &self.buffer[..bs]);

        true
    }

    /// Jump ahead the specified distance.
    pub fn jump(&mut self, distance: u64, serr: &mut dyn Write) -> bool {
        // Are we already at the end of the file
        if self.currentoffset >= self.filesize {
            return false;
        }

        // Special distances
        if distance == 0 {
            return false;
        }
        if distance == 1 {
            return self.step(serr);
        }

        // Not allowed to jump more than one block
        let mut distance = distance;
        if distance > self.blocksize {
            distance = self.blocksize;
        }

        let bs = self.blocksize as usize;

        // Advance the current offset and check if we have reached the end
        // of the file
        self.currentoffset += distance;
        if self.currentoffset >= self.filesize {
            self.currentoffset = self.filesize;
            self.tailindex = 0;
            self.outindex = 0;
            self.buffer[..bs].fill(0);
            self.crc = 0;

            return true;
        }

        // Move past the data being discarded
        self.outindex += distance as usize;
        debug_assert!(self.outindex <= self.tailindex);

        // Is there any data left in the buffer that we are keeping
        let keep = self.tailindex - self.outindex;
        if keep > 0 {
            // Move it back to the start of the buffer
            self.buffer.copy_within(self.outindex..self.outindex + keep, 0);
            self.tailindex = keep;
        } else {
            self.tailindex = 0;
        }

        self.outindex = 0;
        self.inindex = bs;

        self.blank_past_end_of_file();

        if !self.fill(false, serr) {
            return false;
        }

        // Compute the checksum for the block
        self.crc = !0u32 ^ crc_update_block(!0, &self.buffer[..bs]);

        true
    }

    /// Step forward one byte.
    pub fn step(&mut self, serr: &mut dyn Write) -> bool {
        let bs = self.blocksize as usize;

        // Are we already at the end of the file
        if self.currentoffset >= self.filesize {
            return false;
        }

        // Advance the file offset and check to see if
        // we have reached the end of the file
        self.currentoffset += 1;
        if self.currentoffset >= self.filesize {
            self.currentoffset = self.filesize;
            self.tailindex = 0;
            self.outindex = 0;
            self.buffer[..bs].fill(0);
            self.crc = 0;

            return true;
        }

        // Ensure we have enough data in the buffer
        if self.tailindex <= self.inindex && !self.fill(true, serr) {
            return false;
        }

        // Get the incoming and outgoing characters
        let inch = self.buffer[self.inindex];
        self.inindex += 1;
        let outch = self.buffer[self.outindex];
        self.outindex += 1;

        // Update the checksum
        self.crc = crc_slide_char(self.crc, inch, outch, &self.windowtable);

        // Can the window slide further
        if self.outindex < bs {
            return true;
        }

        debug_assert_eq!(self.outindex, bs);

        // Copy the data back to the beginning of the buffer
        self.buffer.copy_within(bs..2 * bs, 0);
        self.inindex = self.outindex;
        self.outindex = 0;
        self.tailindex -= bs;

        self.blank_past_end_of_file();

        true
    }

    /// Return the current checksum.
    pub fn checksum(&self) -> u32 {
        self.crc
    }

    /// Compute and return the current hash.
    pub fn hash(&self) -> Md5Hash {
        let bs = self.blocksize as usize;
        let mut context = Md5Context::new();
        context.update(&self.buffer[self.outindex..self.outindex + bs]);

        context.finalize()
    }

    /// Compute a short value of the checksum for a short block.
    pub fn short_checksum(&self, blocklength: u64) -> u32 {
        let bl = blocklength as usize;
        let mut crc = crc_update_block(!0, &self.buffer[self.outindex..self.outindex + bl]);

        if self.blocksize > blocklength {
            crc = crc_update_block_zeros(crc, self.blocksize - blocklength);
        }

        crc ^ !0
    }

    /// Compute a short value of the hash for a short block.
    pub fn short_hash(&self, blocklength: u64) -> Md5Hash {
        let bl = blocklength as usize;
        let mut context = Md5Context::new();
        context.update(&self.buffer[self.outindex..self.outindex + bl]);

        if self.blocksize > blocklength {
            context.update_zeros((self.blocksize - blocklength) as usize);
        }

        context.finalize()
    }

    /// Do we have less than a full block of data.
    pub fn short_block(&self) -> bool {
        self.block_length() < self.blocksize
    }

    pub fn block_length(&self) -> u64 {
        self.blocksize
            .min(self.filesize.saturating_sub(self.currentoffset))
    }

    /// Return the current file offset.
    pub fn offset(&self) -> u64 {
        self.currentoffset
    }

    /// Return the full file hash and the 16k file hash. Both need the file
    /// to have been read from its start, the full hash also needs
    /// `computefilehashes`.
    pub fn get_file_hashes(&self) -> (Option<Md5Hash>, Md5Hash) {
        debug_assert!(self.hashesvalid);

        self.filehasher.get_hashes(self.filesize)
    }

    /// Which disk file is this.
    pub fn get_diskfile(&self) -> Rc<RefCell<DiskFile>> {
        Rc::clone(&self.diskfile)
    }

    /// Blank the part of the buffer which is beyond the end of the file.
    ///
    /// The scan window slides beyond the end of the file, where the data
    /// reads as zeros. Once the whole file has been read, blank the rest of
    /// the buffer.
    fn blank_past_end_of_file(&mut self) {
        if self.readoffset >= self.filesize {
            let end = (self.blocksize as usize) * 2;
            self.buffer[self.tailindex..end].fill(0);
        }
    }

    /// Fill the buffers with more data from disk.
    /// Set longfill = true to force fill the whole buffer.
    fn fill(&mut self, longfill: bool, serr: &mut dyn Write) -> bool {
        let bs = self.blocksize as usize;

        // Have we already reached the end of the file
        if self.readoffset >= self.filesize {
            return true;
        }

        // Don't bother filling if we have enough data in the buffer
        if self.tailindex >= bs && !longfill {
            return true;
        }

        // Try reading at least one block of data
        let target = if self.tailindex == 0 { bs } else { 2 * bs };

        // How much data can we read into the buffer
        let want = std::cmp::min(
            self.filesize - self.readoffset,
            (target - self.tailindex) as u64,
        ) as usize;

        if want > 0 {
            // Read data
            if !self.diskfile.borrow_mut().read(
                self.readoffset,
                &mut self.buffer[self.tailindex..self.tailindex + want],
                serr,
            ) {
                return false;
            }

            if self.hashesvalid {
                self.filehasher
                    .update(self.readoffset, &self.buffer[self.tailindex..self.tailindex + want]);
            }
            self.readoffset += want as u64;
            self.tailindex += want;
        }

        // Blank whatever part of the buffer was not filled
        self.blank_past_end_of_file();

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc::generate_window_table;
    use std::fs;

    // Every byte is non zero, so that data left behind in the buffer cannot
    // be mistaken for the zeros beyond the end of the file
    fn test_data(filesize: usize) -> Vec<u8> {
        (0..filesize as u64)
            .map(|i| (1 + (i * 37 + (i >> 3)) % 255) as u8)
            .collect()
    }

    // The data the scan window holds at an offset: the file data, then zeros
    fn window_at(data: &[u8], offset: u64, blocksize: usize) -> Vec<u8> {
        let mut window = vec![0u8; blocksize];
        for (i, slot) in window.iter_mut().enumerate() {
            if offset + (i as u64) < data.len() as u64 {
                *slot = data[(offset + i as u64) as usize];
            }
        }
        window
    }

    fn check_window(cs: &FileCheckSummer, data: &[u8], blocksize: usize) {
        let offset = cs.offset();
        let window = window_at(data, offset, blocksize);

        let checksum = !0u32 ^ crc_update_block(!0, &window);
        assert_eq!(cs.checksum(), checksum, "checksum at offset {offset}");

        let mut context = Md5Context::new();
        context.update(&window);
        let hash = context.finalize();
        assert_eq!(cs.hash(), hash, "hash at offset {offset}");

        // The last block of a file is matched by its own length, with the
        // rest of the window taken to be zeros
        let blocklength = cs.block_length();
        assert_eq!(
            cs.short_checksum(blocklength),
            checksum,
            "short checksum at offset {offset}"
        );
        assert_eq!(
            cs.short_hash(blocklength),
            hash,
            "short hash at offset {offset}"
        );
    }

    // Walk to the end of the file from startoffset, taking a jump of
    // jumpdistance after every steps single byte steps. A jumpdistance of
    // 0 only steps.
    fn walk(
        diskfile: &Rc<RefCell<DiskFile>>,
        data: &[u8],
        windowtable: &[u32; 256],
        blocksize: u64,
        startoffset: u64,
        steps: u64,
        jumpdistance: u64,
    ) {
        let mut serr = std::io::sink();
        let mut checksummer = FileCheckSummer::new(Rc::clone(diskfile), blocksize, windowtable, true);
        assert!(checksummer.start_at(startoffset, &mut serr));

        let mut stepstaken = 0u64;
        while checksummer.offset() < data.len() as u64 {
            check_window(&checksummer, data, blocksize as usize);

            if jumpdistance > 0 {
                stepstaken += 1;
                if stepstaken > steps {
                    stepstaken = 0;
                    assert!(
                        checksummer.jump(jumpdistance, &mut serr),
                        "could not jump at offset {}",
                        checksummer.offset()
                    );
                    continue;
                }
            }
            assert!(
                checksummer.step(&mut serr),
                "could not step at offset {}",
                checksummer.offset()
            );
        }
    }

    #[test]
    fn test1_sliding_window_walk() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_filechecksummer_{}.bin", std::process::id()));

        for blocksize in [4u64, 8, 12, 20] {
            for filesize in 0..=3 * blocksize + 2 {
                let data = test_data(filesize as usize);

                let _ = fs::remove_file(&path);
                let diskfile = Rc::new(RefCell::new(DiskFile::new()));
                assert!(diskfile
                    .borrow_mut()
                    .create(path.to_str().unwrap(), filesize, &mut std::io::sink()));
                if filesize > 0 {
                    assert!(diskfile.borrow_mut().write(
                        0,
                        &data,
                        &mut std::io::sink()
                    ));
                }

                let windowtable = generate_window_table(blocksize);

                // Jumps of a whole block are what an undamaged file is
                // scanned with, the rest land the buffer at every offset
                // either side of the block boundary
                let jumpdistances = [0u64, 1, 2, 3, blocksize / 2, blocksize - 1, blocksize];

                // Starting at the end of the file leaves nothing to scan
                for startoffset in 0..=filesize {
                    for &jumpdistance in &jumpdistances {
                        for &steps in &[0u64, 3] {
                            walk(&diskfile, &data, &windowtable, blocksize, startoffset, steps, jumpdistance);
                        }
                    }
                }

                let _ = fs::remove_file(&path);
            }
        }
    }

    #[test]
    fn test2_file_hasher_small_file() {
        let data = test_data(1000);

        // Feed the file in order
        let mut hasher = FileHasher::new(true);
        let mut offset = 0u64;
        for chunk in data.chunks(300) {
            hasher.update(offset, chunk);
            offset += chunk.len() as u64;
        }
        let (hashfull, hash16k) = hasher.get_hashes(data.len() as u64);

        let mut ctx = Md5Context::new();
        ctx.update(&data);
        let whole = ctx.finalize();

        // A file smaller than 16k hashes the same both ways
        assert_eq!(hash16k, whole);
        assert_eq!(hashfull, Some(whole));
    }

    #[test]
    fn test3_file_hasher_large_file() {
        let data = test_data(20000);

        let mut ctx16 = Md5Context::new();
        ctx16.update(&data[..16384]);
        let hash16k = ctx16.finalize();
        let mut ctxall = Md5Context::new();
        ctxall.update(&data);
        let hashfull = ctxall.finalize();

        // Chunk sizes chosen to straddle the 16k mark
        for chunksize in [5000usize, 16384, 16383, 16385] {
            let mut hasher = FileHasher::new(true);
            let mut offset = 0u64;
            for chunk in data.chunks(chunksize) {
                hasher.update(offset, chunk);
                offset += chunk.len() as u64;
            }
            let (full, sixteen) = hasher.get_hashes(data.len() as u64);
            assert_eq!(sixteen, hash16k, "chunksize {chunksize}");
            assert_eq!(full, Some(hashfull), "chunksize {chunksize}");
        }

        // Without the whole file hash only the 16k hash is available
        let mut hasher = FileHasher::new(false);
        let mut offset = 0u64;
        for chunk in data.chunks(5000) {
            hasher.update(offset, chunk);
            offset += chunk.len() as u64;
        }
        let (full, sixteen) = hasher.get_hashes(data.len() as u64);
        assert_eq!(sixteen, hash16k);
        assert_eq!(full, None);
    }

    #[test]
    fn test4_file_hashes_after_full_scan() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_filechecksummer_h_{}.bin", std::process::id()));
        let _ = fs::remove_file(&path);

        let data = test_data(5000);
        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .create(path.to_str().unwrap(), data.len() as u64, &mut std::io::sink()));
        assert!(diskfile.borrow_mut().write(0, &data, &mut std::io::sink()));

        let windowtable = generate_window_table(1024);
        let mut serr = std::io::sink();
        let mut checksummer =
            FileCheckSummer::new(Rc::clone(&diskfile), 1024, &windowtable, true);
        assert!(checksummer.start(&mut serr));
        while checksummer.step(&mut serr) {}

        let (full, sixteen) = checksummer.get_file_hashes();

        let mut ctx = Md5Context::new();
        ctx.update(&data);
        let whole = ctx.finalize();

        // A file smaller than 16k hashes the same both ways
        assert_eq!(sixteen, whole);
        assert_eq!(full, Some(whole));

        let _ = fs::remove_file(&path);
    }
}
