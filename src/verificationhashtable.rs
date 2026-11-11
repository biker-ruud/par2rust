//  This file is part of par2cmdline (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2003 Peter Brian Clements
//  Copyright (c) 2019 Michael D. Nahas
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

//! Port of verificationhashtable.h / verificationhashtable.cpp.
//!
//! The C++ implementation links `VerificationHashEntry` objects with raw
//! pointers (binary tree nodes plus `same`/`next` list links). The Rust port
//! stores every entry in an arena (`Vec<VerificationHashEntry>`) owned by the
//! table and replaces each pointer with an arena index (`Option<usize>` for
//! null). The table owns all entries, so dropping the table frees them, just
//! like the C++ destructor deleting the tree roots.
//!
//! Two C++ pointer fields are adapted:
//! - `sourcefile` (`Par2RepairerSourceFile*`) becomes a cloned
//!   `Rc<RefCell<Par2RepairerSourceFile>>`.
//! - `datablock` (`DataBlock*`) becomes a `usize` index into the repairer's
//!   source blocks vector, because the repairer (not the table) owns the
//!   blocks. Methods that dereference the block (`is_set`, `set_block`, and
//!   the block-length checks inside `find_match`) therefore take the block
//!   slice as an extra parameter.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use crate::datablock::DataBlock;
use crate::diskfile::DiskFile;
use crate::filechecksummer::FileCheckSummer;
use crate::md5::Md5Hash;
use crate::par2repairersourcefile::Par2RepairerSourceFile;

/// A node of the binary trees stored in a `VerificationHashTable`.
///
/// There is one entry for each data block of the original source files.
pub struct VerificationHashEntry {
    // Which source file the block belongs to
    sourcefile: Rc<RefCell<Par2RepairerSourceFile>>,
    // Index of the associated data block in the repairer's source blocks
    datablock: usize,
    // Is this the first block of its source file
    firstblock: bool,

    // The verification data for the block
    crc: u32,
    hash: Md5Hash,

    // Binary tree
    left: Option<usize>,
    right: Option<usize>,

    // Linked list of entries with the same crc and hash
    same: Option<usize>,

    // Linked list of entries in sequence for the same file
    next: Option<usize>,
}

impl VerificationHashEntry {
    pub fn sourcefile(&self) -> &Rc<RefCell<Par2RepairerSourceFile>> {
        &self.sourcefile
    }

    /// Index of the associated data block in the repairer's source blocks.
    pub fn datablock(&self) -> usize {
        self.datablock
    }

    pub fn firstblock(&self) -> bool {
        self.firstblock
    }

    pub fn checksum(&self) -> u32 {
        self.crc
    }

    pub fn hash(&self) -> &Md5Hash {
        &self.hash
    }

    pub fn same(&self) -> Option<usize> {
        self.same
    }

    pub fn next(&self) -> Option<usize> {
        self.next
    }

    pub fn set_next(&mut self, next: Option<usize>) {
        self.next = next;
    }

    // Comparison used for searching the tree: order by crc, then by hash
    fn key_cmp(&self, other: &VerificationHashEntry) -> Ordering {
        self.crc.cmp(&other.crc).then_with(|| self.hash.cmp(&other.hash))
    }
}

// Which field of the table (or of an entry) a search cursor points at
#[derive(Clone, Copy)]
enum Slot {
    Bucket(usize),
    Left(usize),
    Right(usize),
    Same(usize),
}

/// Contains all of the `VerificationHashEntry` objects and is used to find
/// matches for blocks of data in a target file that is being scanned.
///
/// It is initialised by loading data from all available verification packets
/// for the source files.
pub struct VerificationHashTable {
    // Arena of all entries; indices replace the C++ pointers
    entries: Vec<VerificationHashEntry>,
    // Roots of the binary trees, indexed by crc & hashmask
    hashtable: Vec<Option<usize>>,
    hashmask: usize,
}

impl VerificationHashTable {
    /// Construct an empty hash table.
    pub fn new() -> VerificationHashTable {
        VerificationHashTable {
            entries: Vec::new(),
            hashtable: Vec::new(),
            hashmask: 0,
        }
    }

    /// Allocate the hash table with a reasonable size.
    pub fn set_limit(&mut self, limit: u32) {
        // Pick a good size for the hash table
        let mut hashmask: usize = 256;
        while (hashmask as u64) < (limit as u64) && hashmask < 65536 {
            hashmask <<= 1;
        }

        // Allocate and clear the hash table
        self.hashtable = vec![None; hashmask];

        self.hashmask = hashmask - 1;
    }

    /// Load data from a verification packet.
    ///
    /// `blocksize` is unused (the C++ parameter is unused too) and kept only
    /// to mirror the C++ signature.
    pub fn load(&mut self, sourcefile: &Rc<RefCell<Par2RepairerSourceFile>>, _blocksize: u64) {
        let mut preventry: Option<usize> = None;

        // Get information from the sourcefile
        let (verificationpacket, firstsourceblock) = {
            let sf = sourcefile.borrow();
            let verificationpacket = sf
                .verificationpacket()
                .expect("load requires a verification packet");
            (verificationpacket, sf.first_source_block())
        };
        let blockcount = verificationpacket.borrow().block_count();

        // Iterate through the data blocks for the source file and the
        // verification entries in the verification packet.
        let mut blocknumber = 0u32;
        while blocknumber < blockcount {
            let verificationentry = verificationpacket.borrow().verification_entry(blocknumber);

            // Create a new VerificationHashEntry with the details for the
            // current data block and verification entry.
            let entry = VerificationHashEntry {
                sourcefile: Rc::clone(sourcefile),
                datablock: firstsourceblock + blocknumber as usize,
                firstblock: blocknumber == 0,
                crc: verificationentry.crc,
                hash: verificationentry.hash,
                left: None,
                right: None,
                same: None,
                next: None,
            };

            // Insert the entry in the hash table
            let index = self.entries.len();
            self.entries.push(entry);
            self.insert(index);

            // Make the previous entry point forwards to this one
            if let Some(pre) = preventry {
                self.entries[pre].next = Some(index);
            }
            preventry = Some(index);

            blocknumber += 1;
        }
    }

    /// Try to find a match.
    ///
    /// - `suggestedentry` - The entry which we expect to find next. This is
    ///   used when a sequence of matches are found.
    /// - `sourcefile` - Which source file we would prefer to find a match for
    ///   if there are more than one possible match (with the same crc and
    ///   hash).
    /// - `checksummer` - Provides the crc and hash values being tested.
    /// - `duplicate` - Set on return if the match would have been valid
    ///   except for the fact that the block has already been found.
    /// - `datablocks` - The repairer's source blocks vector, which the
    ///   entries index into (replaces the C++ `DataBlock*` field).
    pub fn find_match(
        &self,
        suggestedentry: Option<usize>,
        sourcefile: Option<&Rc<RefCell<Par2RepairerSourceFile>>>,
        checksummer: &FileCheckSummer,
        duplicate: &mut bool,
        datablocks: &[DataBlock],
    ) -> Option<usize> {
        *duplicate = false;

        // Get the current checksum from the checksummer
        let crc = checksummer.checksum();

        let mut hash = Md5Hash::default();
        let mut havehash = false;

        // Do we know what the next entry should be
        if let Some(suggestedentry) = suggestedentry {
            let entry = &self.entries[suggestedentry];

            // Is the suggested match supposed to be the last one in the file
            if entry.next.is_none() {
                // How long should the last block be
                let length = datablocks[entry.datablock].length();

                // Get a short checksum from the checksummer
                let checksum = checksummer.short_checksum(length);

                // Is the checksum correct
                if checksum == entry.crc {
                    havehash = true;
                    // Get a short hash from the checksummer
                    hash = checksummer.short_hash(length);

                    // If the hash matches as well, then return it
                    if hash == entry.hash {
                        return Some(suggestedentry);
                    }
                }
            }
            // If the suggested entry has not already been found, compare the
            // checksum
            else if !datablocks[entry.datablock].is_set() && entry.crc == crc {
                havehash = true;
                // Get the hash value from the checksummer
                hash = checksummer.hash();

                // If the hash value matches, then return it.
                if hash == entry.hash {
                    return Some(suggestedentry);
                }
            }
        }

        // Look for other possible matches for the checksum
        let mut nextentry = self.search_crc(self.hashtable[crc as usize & self.hashmask], crc)?;

        // If we don't have the hash yet, get it
        if !havehash {
            hash = checksummer.hash();
        }

        // Look for an entry with a matching hash
        nextentry = self.search_hash(nextentry, &hash)?;

        // Is there one match with the same checksum and hash, or many
        if self.entries[nextentry].same.is_none() {
            // If the match is for a block that is part of a target file
            // for which we already have a complete match, then don't
            // return it.
            if self.entries[nextentry]
                .sourcefile
                .borrow()
                .get_complete_file()
                .is_some()
            {
                *duplicate = true;
                return None;
            }

            // If we are close to the end of the file and the block
            // length is wrong, don't return it because it is an
            // invalid match
            if checksummer.short_block()
                && checksummer.block_length() != datablocks[self.entries[nextentry].datablock].length()
            {
                return None;
            }

            // If the match was at the start of the file and it is the first
            // block for a target file, then return it.
            if self.entries[nextentry].firstblock && checksummer.offset() == 0 {
                return Some(nextentry);
            }

            // Was this match actually the one which had originally been
            // suggested but which has presumably already been found
            if Some(nextentry) == suggestedentry {
                // Was the existing match in the same file as the new match
                if datablocks[self.entries[nextentry].datablock].is_set() {
                    let checksummed = checksummer.get_diskfile();
                    let same_file = datablocks[self.entries[nextentry].datablock]
                        .diskfile()
                        .map(|df| Rc::ptr_eq(&df, &checksummed))
                        .unwrap_or(false);
                    if same_file {
                        // Yes. Don't return it
                        *duplicate = true;
                        return None;
                    }
                }
                // No, it was in a different file. Return it.
                // This ensures that we can find a perfect match for a target
                // file even if some of the blocks had already been found
                // in a different file.
                return Some(nextentry);
            } else {
                // return it only if it has not already been used
                if datablocks[self.entries[nextentry].datablock].is_set() {
                    *duplicate = true;
                    return None;
                }

                return Some(nextentry);
            }
        }

        let mut nextentry = Some(nextentry);

        // Do we prefer to match entries for a particular source file
        if let Some(sourcefile) = sourcefile {
            let mut currententry = nextentry;
            nextentry = None;

            // We don't want entries for the wrong source file, ones that
            // have already been matched, or ones that are the wrong length
            while let Some(current) = currententry {
                let entry = &self.entries[current];
                let wrong_file = !Rc::ptr_eq(&entry.sourcefile, sourcefile);
                let is_set = datablocks[entry.datablock].is_set();
                let wrong_length = checksummer.short_block()
                    && checksummer.block_length() != datablocks[entry.datablock].length();
                if !wrong_file && !is_set && !wrong_length {
                    break;
                }

                // If we found an unused entry (which was presumably for the
                // wrong source file) remember it (providing it is the
                // correct length).
                if nextentry.is_none() && !is_set && !wrong_length {
                    nextentry = Some(current);
                }

                currententry = entry.same;
            }

            // If we found an unused entry for the source file we want,
            // return it
            if currententry.is_some() {
                return currententry;
            }
        }

        // Check for an unused entry which is the correct length
        while let Some(next) = nextentry {
            let entry = &self.entries[next];
            let is_set = datablocks[entry.datablock].is_set();
            let wrong_length = checksummer.short_block()
                && checksummer.block_length() != datablocks[entry.datablock].length();
            if !is_set && !wrong_length {
                break;
            }

            nextentry = entry.same;
        }

        // Return what we have found
        nextentry
    }

    /// Look up based on the block crc.
    pub fn lookup(&self, crc: u32) -> Option<usize> {
        if self.hashmask != 0 {
            return self.search_crc(self.hashtable[crc as usize & self.hashmask], crc);
        }

        None
    }

    /// Continue lookup based on the block hash.
    pub fn lookup_hash(&self, entry: usize, hash: &Md5Hash) -> Option<usize> {
        self.search_hash(entry, hash)
    }

    /// Is the data block associated with the entry already matched.
    pub fn is_set(&self, entry: usize, datablocks: &[DataBlock]) -> bool {
        datablocks[self.entries[entry].datablock].is_set()
    }

    /// Set the location of the data block associated with the entry.
    pub fn set_block(
        &self,
        entry: usize,
        diskfile: Rc<RefCell<DiskFile>>,
        offset: u64,
        datablocks: &mut [DataBlock],
    ) {
        datablocks[self.entries[entry].datablock].set_location(diskfile, offset);
    }

    /// Get the entry with the given arena index.
    pub fn entry(&self, index: usize) -> &VerificationHashEntry {
        &self.entries[index]
    }

    /// How many entries have been loaded.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Has no entry been loaded yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // Insert an entry into the tree of its bucket
    fn insert(&mut self, entry: usize) {
        let bucket = self.entries[entry].crc as usize & self.hashmask;
        let mut slot = Slot::Bucket(bucket);

        while let Some(node) = self.slot_ref(slot) {
            match self.entries[node].key_cmp(&self.entries[entry]) {
                Ordering::Less => slot = Slot::Right(node),
                Ordering::Greater => slot = Slot::Left(node),
                Ordering::Equal => break,
            }
        }

        while let Some(node) = self.slot_ref(slot) {
            slot = Slot::Same(node);
        }

        *self.slot_mut(slot) = Some(entry);
    }

    // Search the tree for an entry with the correct crc
    fn search_crc(&self, mut entry: Option<usize>, crc: u32) -> Option<usize> {
        while let Some(index) = entry {
            let entry_crc = self.entries[index].crc;
            if entry_crc < crc {
                entry = self.entries[index].right;
            } else if entry_crc > crc {
                entry = self.entries[index].left;
            } else {
                break;
            }
        }

        entry
    }

    // Search the tree for an entry with the correct hash. The crc is taken
    // from the entry the search starts at.
    fn search_hash(&self, entry: usize, hash: &Md5Hash) -> Option<usize> {
        let crc = self.entries[entry].crc;

        let mut entry = Some(entry);
        while let Some(index) = entry {
            let node = &self.entries[index];
            if node.crc < crc || (node.crc == crc && node.hash < *hash) {
                entry = node.right;
            } else if node.crc > crc || (node.crc == crc && node.hash > *hash) {
                entry = node.left;
            } else {
                break;
            }
        }

        entry
    }

    fn slot_ref(&self, slot: Slot) -> Option<usize> {
        match slot {
            Slot::Bucket(b) => self.hashtable[b],
            Slot::Left(n) => self.entries[n].left,
            Slot::Right(n) => self.entries[n].right,
            Slot::Same(n) => self.entries[n].same,
        }
    }

    fn slot_mut(&mut self, slot: Slot) -> &mut Option<usize> {
        match slot {
            Slot::Bucket(b) => &mut self.hashtable[b],
            Slot::Left(n) => &mut self.entries[n].left,
            Slot::Right(n) => &mut self.entries[n].right,
            Slot::Same(n) => &mut self.entries[n].same,
        }
    }
}

impl Default for VerificationHashTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc::generate_window_table;
    use crate::descriptionpacket::DescriptionPacket;
    use crate::md5::{Md5Context, Md5Hash};
    use crate::verificationpacket::VerificationPacket;
    use std::fs;

    fn hash_of(bytes: &[u8]) -> Md5Hash {
        let mut context = Md5Context::new();
        context.update(bytes);
        context.finalize()
    }

    // Build a source file with a description packet of the given size and a
    // verification packet holding the given (crc, hash) entries, wired to a
    // fresh source blocks vector starting at firstsourceblock.
    fn make_sourcefile(
        name: &str,
        filesize: u64,
        entries: &[(u32, Md5Hash)],
        firstsourceblock: usize,
        blocksize: u64,
    ) -> (
        Rc<RefCell<Par2RepairerSourceFile>>,
        Vec<DataBlock>,
    ) {
        let mut descriptionpacket = DescriptionPacket::new();
        assert!(descriptionpacket.create(name.as_bytes(), filesize));

        let mut verificationpacket = VerificationPacket::new();
        assert!(verificationpacket.create(entries.len() as u32));
        for (i, (crc, hash)) in entries.iter().enumerate() {
            verificationpacket.set_block_hash_and_crc(i as u32, hash, *crc);
        }

        let mut sourcefile = Par2RepairerSourceFile::new(
            Some(Rc::new(RefCell::new(descriptionpacket))),
            Some(Rc::new(RefCell::new(verificationpacket))),
        );

        let mut sourceblocks: Vec<DataBlock> =
            (0..entries.len()).map(|_| DataBlock::new()).collect();
        sourcefile.set_blocks(
            0,
            entries.len() as u32,
            firstsourceblock,
            0,
            &mut sourceblocks,
            blocksize,
        );

        (Rc::new(RefCell::new(sourcefile)), sourceblocks)
    }

    #[test]
    fn test_set_limit_sizes() {
        let mut table = VerificationHashTable::new();
        assert_eq!(table.hashmask, 0);
        assert!(table.hashtable.is_empty());

        table.set_limit(0);
        assert_eq!(table.hashmask, 255);
        assert_eq!(table.hashtable.len(), 256);

        table.set_limit(1000);
        assert_eq!(table.hashmask, 1023);
        assert_eq!(table.hashtable.len(), 1024);

        table.set_limit(100000);
        assert_eq!(table.hashmask, 65535);
        assert_eq!(table.hashtable.len(), 65536);
    }

    #[test]
    fn test_lookup_without_set_limit() {
        let table = VerificationHashTable::new();
        assert_eq!(table.lookup(1234), None);
    }

    #[test]
    fn test_load_and_lookup() {
        let entries = [
            (0x11111111u32, hash_of(b"block one")),
            (0x22222222u32, hash_of(b"block two")),
            (0x33333333u32, hash_of(b"block three")),
        ];
        let (sourcefile, sourceblocks) = make_sourcefile("file1", 48, &entries, 0, 16);

        let mut table = VerificationHashTable::new();
        table.set_limit(16);
        table.load(&sourcefile, 16);

        assert_eq!(table.len(), 3);

        let e0 = table.lookup(0x11111111).expect("entry 0");
        let e1 = table.lookup(0x22222222).expect("entry 1");
        let e2 = table.lookup(0x33333333).expect("entry 2");

        assert_eq!(table.entry(e0).checksum(), 0x11111111);
        assert_eq!(table.entry(e0).hash(), &entries[0].1);
        assert!(table.entry(e0).firstblock());
        assert!(!table.entry(e1).firstblock());
        assert_eq!(table.entry(e0).datablock(), 0);
        assert_eq!(table.entry(e1).datablock(), 1);
        assert_eq!(table.entry(e2).datablock(), 2);

        // The entries are chained in file order
        assert_eq!(table.entry(e0).next(), Some(e1));
        assert_eq!(table.entry(e1).next(), Some(e2));
        assert_eq!(table.entry(e2).next(), None);

        // No duplicates
        assert_eq!(table.entry(e0).same(), None);

        // Unknown crc
        assert_eq!(table.lookup(0xdeadbeef), None);

        // lookup_hash finds the same entry
        let found = table
            .lookup_hash(e1, &entries[1].1)
            .expect("hash match");
        assert_eq!(found, e1);

        assert!(!table.is_set(e0, &sourceblocks));
    }

    #[test]
    fn test_load_with_sourceblock_offset() {
        let entries = [(7u32, hash_of(b"a")), (8u32, hash_of(b"b"))];
        let (sourcefile, _blocks) = make_sourcefile("file1", 32, &entries, 5, 16);

        let mut table = VerificationHashTable::new();
        table.set_limit(16);
        table.load(&sourcefile, 16);

        let e0 = table.lookup(7).expect("entry 0");
        let e1 = table.lookup(8).expect("entry 1");
        assert_eq!(table.entry(e0).datablock(), 5);
        assert_eq!(table.entry(e1).datablock(), 6);
    }

    #[test]
    fn test_same_chain_for_identical_blocks() {
        let shared = (0x55555555u32, hash_of(b"same data"));
        let (sourcefile1, _blocks1) = make_sourcefile("file1", 16, &[shared], 0, 16);
        let (sourcefile2, _blocks2) = make_sourcefile("file2", 16, &[shared], 1, 16);

        let mut table = VerificationHashTable::new();
        table.set_limit(16);
        table.load(&sourcefile1, 16);
        table.load(&sourcefile2, 16);

        let first = table.lookup(0x55555555).expect("entry");
        let second = table
            .entry(first)
            .same()
            .expect("duplicate entry on same chain");

        // The two entries belong to different source files
        assert!(!Rc::ptr_eq(
            table.entry(first).sourcefile(),
            table.entry(second).sourcefile()
        ));
        assert_eq!(table.entry(second).same(), None);

        // lookup_hash from either finds a matching entry
        let found = table.lookup_hash(first, &shared.1).expect("hash");
        assert_eq!(table.entry(found).hash(), &shared.1);
    }

    #[test]
    fn test_tree_search_with_bucket_collisions() {
        // All three crcs share the same bucket for a 256 slot table
        let entries = [
            (1u32, hash_of(b"one")),
            (257u32, hash_of(b"two")),
            (513u32, hash_of(b"three")),
        ];
        let (sourcefile, _blocks) = make_sourcefile("file1", 48, &entries, 0, 16);

        let mut table = VerificationHashTable::new();
        table.set_limit(0);
        table.load(&sourcefile, 16);

        for (crc, hash) in entries.iter() {
            let found = table.lookup(*crc).unwrap_or_else(|| panic!("crc {crc} not found"));
            assert_eq!(table.entry(found).checksum(), *crc);
            assert_eq!(table.entry(found).hash(), hash);
        }
    }

    // Scan a real file with a FileCheckSummer and check find_match
    #[test]
    fn test_find_match_against_real_file() {
        let blocksize = 16u64;
        let data: Vec<u8> = (0..48u8).collect();

        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_vhashtest_{}.bin", std::process::id()));
        let _ = fs::remove_file(&path);
        fs::write(&path, &data).unwrap();

        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .open(path.to_str().unwrap(), &mut std::io::sink()));

        // Compute the verification entries from the checksummer itself
        let windowtable = generate_window_table(blocksize);
        let mut serr = std::io::sink();
        let mut checksummer =
            FileCheckSummer::new(Rc::clone(&diskfile), blocksize, &windowtable, false);
        assert!(checksummer.start(&mut serr));

        let mut entries = Vec::new();
        for i in 0..3 {
            entries.push((checksummer.checksum(), checksummer.hash()));
            if i < 2 {
                assert!(checksummer.jump(blocksize, &mut serr));
            }
        }

        let (sourcefile, mut sourceblocks) =
            make_sourcefile("target", data.len() as u64, &entries, 0, blocksize);

        let mut table = VerificationHashTable::new();
        table.set_limit(16);
        table.load(&sourcefile, blocksize);

        // Rescan from the start and match every block in sequence
        assert!(checksummer.start(&mut serr));
        let mut nextentry: Option<usize> = None;
        for i in 0..3 {
            let mut duplicate = false;
            let found = table.find_match(
                nextentry,
                Some(&sourcefile),
                &checksummer,
                &mut duplicate,
                &sourceblocks,
            );
            let found = found.expect("block should match");
            assert!(!duplicate);
            assert_eq!(table.entry(found).checksum(), checksummer.checksum());

            table.set_block(found, Rc::clone(&diskfile), checksummer.offset(), &mut sourceblocks);
            nextentry = table.entry(found).next();

            if i < 2 {
                assert!(checksummer.jump(blocksize, &mut serr));
            }
        }
        assert!(sourceblocks.iter().all(|b| b.is_set()));

        // The first block at offset 0 is returned even when already set
        // (C++ FindMatch returns first blocks at the start of the file).
        assert!(checksummer.start(&mut serr));
        let mut duplicate = false;
        let found = table.find_match(None, None, &checksummer, &mut duplicate, &sourceblocks);
        assert_eq!(found, Some(0));
        assert!(!duplicate);

        // A non-first block that is already set is reported as a duplicate
        assert!(checksummer.jump(blocksize, &mut serr));
        let mut duplicate = false;
        let found = table.find_match(None, None, &checksummer, &mut duplicate, &sourceblocks);
        assert_eq!(found, None);
        assert!(duplicate);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_find_match_no_crc_match() {
        let entries = [(0x1234u32, hash_of(b"known"))];
        let (sourcefile, sourceblocks) = make_sourcefile("file1", 16, &entries, 0, 16);

        let mut table = VerificationHashTable::new();
        table.set_limit(16);
        table.load(&sourcefile, 16);

        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_vhashtest2_{}.bin", std::process::id()));
        let _ = fs::remove_file(&path);
        fs::write(&path, vec![0xabu8; 16]).unwrap();

        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .open(path.to_str().unwrap(), &mut std::io::sink()));

        let windowtable = generate_window_table(16);
        let mut serr = std::io::sink();
        let mut checksummer = FileCheckSummer::new(Rc::clone(&diskfile), 16, &windowtable, false);
        assert!(checksummer.start(&mut serr));

        let mut duplicate = false;
        let found = table.find_match(None, None, &checksummer, &mut duplicate, &sourceblocks);
        assert_eq!(found, None);
        assert!(!duplicate);

        let _ = fs::remove_file(&path);
    }
}
