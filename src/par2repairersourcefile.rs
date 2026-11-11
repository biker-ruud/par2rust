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

//! Port of par2cmdline's par2repairersourcefile.h / .cpp.
//!
//! The Par2RepairerSourceFile object is used during verification and repair
//! to record details about a particular source file and the data blocks
//! for that file.
//!
//! C++ stored `std::vector<DataBlock>::iterator` into the repairer's block
//! vectors; Rust cannot hold such iterators, so the first block of each
//! file is recorded as an index (`firstsourceblock` / `firsttargetblock`)
//! into the vectors owned by the repairer. The C++ raw pointers to packets
//! and disk files become `Rc<RefCell<...>>` (the C++ destructor deleted the
//! packets; here the reference count does the work).

use std::cell::RefCell;
use std::cmp::min;
use std::io::Write;
use std::rc::Rc;

use crate::datablock::DataBlock;
use crate::descriptionpacket::DescriptionPacket;
use crate::diskfile::DiskFile;
use crate::noiselevel::NoiseLevel;
use crate::verificationpacket::VerificationPacket;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchType {
    NoMatch = 0,
    PartialMatch,
    FullMatch,
}

pub struct Par2RepairerSourceFile {
    descriptionpacket: Option<Rc<RefCell<DescriptionPacket>>>, // The file description packet
    verificationpacket: Option<Rc<RefCell<VerificationPacket>>>, // The file verification packet

    blockcount: u32,       // The number of DataBlocks in the file
    firstblocknumber: u32, // The block number of the first DataBlock

    firstsourceblock: usize, // Index of the first source DataBlock
    firsttargetblock: usize, // Index of the first target DataBlock

    targetexists: bool, // Whether the target file exists
    targetfile: Option<Rc<RefCell<DiskFile>>>, // The final version of the file
    completefile: Option<Rc<RefCell<DiskFile>>>, // A complete version of the file

    targetfilename: String, // The filename of the target file
    diskfilesize: u64,      // The filesize of sourcefile on disk
}

impl Par2RepairerSourceFile {
    /// Construct the object and set the description and verification packets.
    pub fn new(
        descriptionpacket: Option<Rc<RefCell<DescriptionPacket>>>,
        verificationpacket: Option<Rc<RefCell<VerificationPacket>>>,
    ) -> Par2RepairerSourceFile {
        Par2RepairerSourceFile {
            descriptionpacket,
            verificationpacket,

            blockcount: 0,
            firstblocknumber: 0,

            firstsourceblock: 0,
            firsttargetblock: 0,

            targetexists: false,
            targetfile: None,
            completefile: None,

            targetfilename: String::new(),
            diskfilesize: 0,
        }
    }

    /// Get/Set the description packet.
    pub fn descriptionpacket(&self) -> Option<Rc<RefCell<DescriptionPacket>>> {
        self.descriptionpacket.clone()
    }

    pub fn set_description_packet(
        &mut self,
        descriptionpacket: Option<Rc<RefCell<DescriptionPacket>>>,
    ) {
        self.descriptionpacket = descriptionpacket;
    }

    /// Get/Set the verification packet.
    pub fn verificationpacket(&self) -> Option<Rc<RefCell<VerificationPacket>>> {
        self.verificationpacket.clone()
    }

    pub fn set_verification_packet(
        &mut self,
        verificationpacket: Option<Rc<RefCell<VerificationPacket>>>,
    ) {
        self.verificationpacket = verificationpacket;
    }

    /// Remember which source and target blocks will be used by this file
    /// and set their lengths appropriately.
    ///
    /// `sourceblocks` must cover at least `blockcount` entries starting at
    /// the file's first source block; `firstsourceblock` / `firsttargetblock`
    /// are the indices of those first blocks in the repairer's vectors.
    pub fn set_blocks(
        &mut self,
        blocknumber: u32,
        blockcount: u32,
        firstsourceblock: usize,
        firsttargetblock: usize,
        sourceblocks: &mut [DataBlock],
        blocksize: u64,
    ) {
        self.firstblocknumber = blocknumber;
        self.blockcount = blockcount;
        self.firstsourceblock = firstsourceblock;
        self.firsttargetblock = firsttargetblock;

        if self.blockcount > 0 {
            let filesize = self
                .descriptionpacket
                .as_ref()
                .expect("set_blocks requires a description packet")
                .borrow()
                .filesize();

            for (blocknumber, datablock) in
                sourceblocks[..blockcount as usize].iter_mut().enumerate()
            {
                let blocklength =
                    min(blocksize, filesize - blocknumber as u64 * blocksize);

                datablock.set_filesize(filesize);
                datablock.set_length(blocklength);
            }
        }
    }

    /// Determine the block count from the file size and block size.
    pub fn set_block_count(&mut self, blocksize: u64) -> bool {
        if let Some(descriptionpacket) = &self.descriptionpacket {
            let filesize = descriptionpacket.borrow().filesize();
            let count = filesize / blocksize + (filesize % blocksize != 0) as u64;

            if count > u32::MAX as u64 {
                self.blockcount = 0;
                return false;
            }

            self.blockcount = count as u32;
        } else {
            self.blockcount = 0;
        }

        true
    }

    /// Set/Get which DiskFile will contain the final repaired version of the file.
    pub fn set_target_file(&mut self, diskfile: Option<Rc<RefCell<DiskFile>>>) {
        self.targetfile = diskfile;
    }

    pub fn get_target_file(&self) -> Option<Rc<RefCell<DiskFile>>> {
        self.targetfile.clone()
    }

    /// Set/Get whether or not the target file actually exists.
    pub fn set_target_exists(&mut self, exists: bool) {
        self.targetexists = exists;
    }

    pub fn get_target_exists(&self) -> bool {
        self.targetexists
    }

    /// Set/Get which DiskFile contains a full undamaged version of the source file.
    pub fn set_complete_file(&mut self, diskfile: Option<Rc<RefCell<DiskFile>>>) {
        self.completefile = diskfile;
    }

    pub fn get_complete_file(&self) -> Option<Rc<RefCell<DiskFile>>> {
        self.completefile.clone()
    }

    /// Compute the filename for the final repaired version of the file.
    pub fn compute_target_filename(
        &mut self,
        serr: &mut dyn Write,
        noiselevel: NoiseLevel,
        path: &str,
    ) {
        // Get a version of the filename compatible with the OS
        let filename = self
            .descriptionpacket
            .as_ref()
            .expect("compute_target_filename requires a description packet")
            .borrow()
            .filename();
        let filename =
            DescriptionPacket::translate_filename_from_par2_to_local(
                serr,
                noiselevel,
                &filename,
            );

        // The C++ std::string held raw bytes; this port keeps local paths as
        // String throughout, so the translated bytes are decoded as UTF-8.
        self.targetfilename =
            path.to_string() + &String::from_utf8_lossy(&filename);
    }

    pub fn targetfilename(&self) -> &str {
        &self.targetfilename
    }

    /// Get the number of blocks that the file uses.
    pub fn block_count(&self) -> u32 {
        self.blockcount
    }

    /// Get the relative block number of the first block in the file.
    pub fn first_block_number(&self) -> u32 {
        self.firstblocknumber
    }

    /// Index of the first source DataBlock for the file.
    pub fn first_source_block(&self) -> usize {
        self.firstsourceblock
    }

    /// Index of the first target DataBlock for the file.
    pub fn first_target_block(&self) -> usize {
        self.firsttargetblock
    }

    /// Set/Get "filesize on disk" needed for mt progress line.
    pub fn set_disk_file_size(&mut self) {
        self.diskfilesize = DiskFile::get_file_size(&self.targetfilename);
    }

    pub fn disk_filesize(&self) -> u64 {
        self.diskfilesize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn desc(filename: &[u8], filesize: u64) -> Rc<RefCell<DescriptionPacket>> {
        let packet = Rc::new(RefCell::new(DescriptionPacket::new()));
        assert!(packet.borrow_mut().create(filename, filesize));
        packet
    }

    #[test]
    fn new_defaults_and_packet_accessors() {
        let d = desc(b"a.bin", 10);
        let v = Rc::new(RefCell::new(VerificationPacket::new()));

        let mut sourcefile =
            Par2RepairerSourceFile::new(Some(Rc::clone(&d)), Some(Rc::clone(&v)));

        assert!(Rc::ptr_eq(
            sourcefile.descriptionpacket().as_ref().unwrap(),
            &d
        ));
        assert!(Rc::ptr_eq(
            sourcefile.verificationpacket().as_ref().unwrap(),
            &v
        ));
        assert_eq!(sourcefile.block_count(), 0);
        assert_eq!(sourcefile.first_block_number(), 0);
        assert!(!sourcefile.get_target_exists());
        assert!(sourcefile.get_target_file().is_none());
        assert!(sourcefile.get_complete_file().is_none());
        assert_eq!(sourcefile.targetfilename(), "");
        assert_eq!(sourcefile.disk_filesize(), 0);

        let d2 = desc(b"b.bin", 20);
        sourcefile.set_description_packet(Some(Rc::clone(&d2)));
        assert!(Rc::ptr_eq(
            sourcefile.descriptionpacket().as_ref().unwrap(),
            &d2
        ));
        sourcefile.set_verification_packet(None);
        assert!(sourcefile.verificationpacket().is_none());
    }

    #[test]
    fn set_block_count_rounds_up_and_handles_no_packet() {
        let d = desc(b"a.bin", 25000);
        let mut sourcefile = Par2RepairerSourceFile::new(Some(d), None);
        assert!(sourcefile.set_block_count(10000));
        assert_eq!(sourcefile.block_count(), 3);

        let d = desc(b"a.bin", 20000);
        let mut sourcefile = Par2RepairerSourceFile::new(Some(d), None);
        assert!(sourcefile.set_block_count(10000));
        assert_eq!(sourcefile.block_count(), 2);

        let d = desc(b"a.bin", 0);
        let mut sourcefile = Par2RepairerSourceFile::new(Some(d), None);
        assert!(sourcefile.set_block_count(10000));
        assert_eq!(sourcefile.block_count(), 0);

        let mut sourcefile = Par2RepairerSourceFile::new(None, None);
        assert!(sourcefile.set_block_count(10000));
        assert_eq!(sourcefile.block_count(), 0);
    }

    #[test]
    fn set_blocks_records_indices_and_sets_lengths() {
        let d = desc(b"a.bin", 25000);
        let mut sourcefile = Par2RepairerSourceFile::new(Some(d), None);

        let mut blocks: Vec<DataBlock> = (0..3).map(|_| DataBlock::new()).collect();
        sourcefile.set_blocks(7, 3, 100, 200, &mut blocks, 10000);

        assert_eq!(sourcefile.first_block_number(), 7);
        assert_eq!(sourcefile.block_count(), 3);
        assert_eq!(sourcefile.first_source_block(), 100);
        assert_eq!(sourcefile.first_target_block(), 200);

        assert_eq!(blocks[0].length(), 10000);
        assert_eq!(blocks[1].length(), 10000);
        assert_eq!(blocks[2].length(), 5000);
        for block in &blocks {
            assert_eq!(block.filesize(), 25000);
        }
    }

    #[test]
    fn set_blocks_zero_count_touches_nothing() {
        let d = desc(b"a.bin", 25000);
        let mut sourcefile = Par2RepairerSourceFile::new(Some(d), None);

        let mut blocks: Vec<DataBlock> = (0..2).map(|_| DataBlock::new()).collect();
        sourcefile.set_blocks(0, 0, 5, 9, &mut blocks, 10000);

        assert_eq!(sourcefile.block_count(), 0);
        assert_eq!(blocks[0].length(), 0);
        assert_eq!(blocks[1].length(), 0);
    }

    #[test]
    fn target_and_complete_file_accessors() {
        let mut sourcefile = Par2RepairerSourceFile::new(None, None);

        let t = Rc::new(RefCell::new(DiskFile::new()));
        let c = Rc::new(RefCell::new(DiskFile::new()));

        sourcefile.set_target_file(Some(Rc::clone(&t)));
        sourcefile.set_complete_file(Some(Rc::clone(&c)));
        sourcefile.set_target_exists(true);

        assert!(Rc::ptr_eq(sourcefile.get_target_file().as_ref().unwrap(), &t));
        assert!(Rc::ptr_eq(
            sourcefile.get_complete_file().as_ref().unwrap(),
            &c
        ));
        assert!(sourcefile.get_target_exists());

        // The C++ code compares DiskFile* pointers for equality; the Rust
        // port compares the Rc handles.
        sourcefile.set_complete_file(sourcefile.get_target_file());
        assert!(Rc::ptr_eq(
            sourcefile.get_complete_file().as_ref().unwrap(),
            sourcefile.get_target_file().as_ref().unwrap()
        ));

        sourcefile.set_target_file(None);
        sourcefile.set_target_exists(false);
        assert!(sourcefile.get_target_file().is_none());
        assert!(!sourcefile.get_target_exists());
    }

    #[test]
    fn compute_target_filename_joins_path_and_translates() {
        let d = desc(b"sub%40dir/file.bin", 10);
        let mut sourcefile = Par2RepairerSourceFile::new(Some(d), None);

        let mut serr = std::io::sink();
        sourcefile.compute_target_filename(&mut serr, NoiseLevel::Quiet, "C:\\out\\");

        let mut serr = std::io::sink();
        let want = DescriptionPacket::translate_filename_from_par2_to_local(
            &mut serr,
            NoiseLevel::Quiet,
            b"sub%40dir/file.bin",
        );
        assert_eq!(
            sourcefile.targetfilename(),
            format!("C:\\out\\{}", String::from_utf8_lossy(&want))
        );
    }

    #[test]
    fn set_disk_file_size_reads_target_file() {
        let dir = std::env::temp_dir();
        let name = format!("par2rust_prsf_{}.bin", std::process::id());
        let path = dir.join(&name);
        fs::write(&path, vec![0u8; 1234]).unwrap();

        let d = desc(name.as_bytes(), 1234);
        let mut sourcefile = Par2RepairerSourceFile::new(Some(d), None);

        let mut serr = std::io::sink();
        let base = dir.to_string_lossy().into_owned() + std::path::MAIN_SEPARATOR_STR;
        sourcefile.compute_target_filename(&mut serr, NoiseLevel::Quiet, &base);

        sourcefile.set_disk_file_size();
        assert_eq!(sourcefile.disk_filesize(), 1234);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn match_type_values() {
        assert_eq!(MatchType::NoMatch as u32, 0);
        assert_eq!(MatchType::PartialMatch as u32, 1);
        assert_eq!(MatchType::FullMatch as u32, 2);
        assert_ne!(MatchType::NoMatch, MatchType::FullMatch);
    }
}
