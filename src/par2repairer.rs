//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2003 Peter Brian Clements
//  Copyright (c) 2019 Michael D. Nahas
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

//! C++ Par2Repairer - verifies the files of a PAR 2.0 recovery set and
//! repairs the ones that are damaged.
//!
//! The ownership of the C++ objects maps onto Rust the same way it does for
//! [`crate::par1repairer`]:
//!
//! - Every `DiskFile` the repairer creates is owned by
//!   [`Par2Repairer::diskfiles`] (a `Vec<Rc<RefCell<DiskFile>>>`), and
//!   [`Par2Repairer::diskfilemap`] tracks which of them are in use by name,
//!   holding the index of the file in that vector.
//! - The recovery packets live in [`Par2Repairer::recoverypacketmap`] keyed
//!   by exponent, and the source files in
//!   [`Par2Repairer::sourcefilemap`] keyed by file id. Both share each
//!   object through an `Rc`, and [`Par2Repairer::sourcefiles`] is the
//!   ordered list the main packet describes — it may hold `None` where a
//!   file id has no packets at all, which is what the C++ `NULL` entries
//!   mean.
//! - `verifylist`, `backuplist` and `unverifiablesourcefiles` are index
//!   lists into `sourcefiles` and `diskfiles` where C++ kept pointer lists.
//! - `inputblocks`, `copyblocks` and `outputblocks` hold cloned
//!   [`DataBlock`]s; a clone describes the same bytes on the same
//!   [`DiskFile`] as its original, like a copied C++ `DataBlock*`.
//! - The C++ `sout` member stream is shared with the `ProgressMeter`
//!   objects the repairer creates, so it is passed around as a shared
//!   stream ([`SharedSout`]) while `serr` stays an explicit `&mut dyn
//!   Write` like in the rest of the crate.
//!
//! The C++ reads several files at once and checks their blocks on worker
//! threads (`blockpool`, `scanbuffers` and `activereaders`). This port
//! scans one file at a time on the calling thread, so those members are
//! not needed and no scan result differs for it: the blocks it finds, and
//! the files it repairs, are the same either way. The thread count still
//! reaches the processor, as it does in C++.

use std::cell::RefCell;
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::backends::{Backends, ProcessorConfig};
use crate::creatorpacket::CreatorPacket;
use crate::crc::generate_window_table;
use crate::datablock::DataBlock;
use crate::descriptionpacket::DescriptionPacket;
use crate::diskfile::{DiskFile, DiskFileMap};
use crate::filechecksummer::{FileCheckSummer, FileHasher};
use crate::galois::Spec16;
use crate::libpar2::Result;
use crate::mainpacket::MainPacket;
use crate::md5::{Md5Context, Md5Hash};
use crate::noiselevel::NoiseLevel;
use crate::par1repairer::{sout_line, sout_write, SharedSout};
use crate::par2fileformat::*;
use crate::par2repairersourcefile::{MatchType, Par2RepairerSourceFile};
use crate::processor::Processor;
use crate::progressmeter::ProgressMeter;
use crate::recoverypacket::RecoveryPacket;
use crate::reedsolomon::ReedSolomon;
use crate::taskpool::{resolve_threads};
use crate::verificationhashtable::VerificationHashTable;
use crate::verificationpacket::VerificationPacket;

/// The number of files read at once when no particular number was asked
/// for (the C++ `_FILE_THREADS`).
pub const FILE_THREADS: u32 = 2;

/// Too large chunks are likely detrimental to performance; set to 0 to
/// disable (the C++ `MAX_CHUNK_SIZE`).
pub const MAX_CHUNK_SIZE: u64 = 32 * 1048576;

/// Blocks in a batch for each thread checking it, so one which finishes
/// early has more to take (the C++ `SCAN_BATCH_PER_THREAD`).
pub const SCAN_BATCH_PER_THREAD: usize = 2;

/// Input blocks in flight (the C++ `NUM_TRANSFER_BUFFERS`).
pub const NUM_TRANSFER_BUFFERS: usize = 2;

/// C++ Par2Repairer
pub struct Par2Repairer {
    noiselevel: NoiseLevel,

    /// The implementations the application supplied
    backends: Backends,

    searchpath: String, // Where to look for the input files
    basepath: String,

    totalthreads: u32, // Number of threads the whole repair may use

    skipdata: bool,   // Should we skip data whilst scanning
    skipleaway: u64,  // The leaway +/- we should allow whilst scanning
    fullhash: bool,   // Should the whole of each file be hashed too

    firstpacket: bool, // Whether or not a valid packet has been found
    setid: Md5Hash,    // The SetId extracted from the first packet

    /// One recovery packet for each exponent value
    recoverypacketmap: BTreeMap<u32, Rc<RefCell<RecoveryPacket>>>,
    mainpacket: Option<MainPacket>,    // One copy of the main packet
    creatorpacket: Option<CreatorPacket>, // One copy of the creator packet

    diskfiles: Vec<Rc<RefCell<DiskFile>>>, // All of the DiskFiles created by the repairer
    diskfilemap: DiskFileMap,              // Track which disk files are in use

    /// Map from FileId to SourceFile
    sourcefilemap: BTreeMap<Md5Hash, Rc<RefCell<Par2RepairerSourceFile>>>,
    sourcefiles: Vec<Option<Rc<RefCell<Par2RepairerSourceFile>>>>, // The source files
    verifylist: Vec<usize>,            // Those source files that are being repaired
    backuplist: Vec<usize>,            // Those source files backups
    par2list: Vec<String>,             // list of par2 files

    blocksize: u64, // The block size
    chunksize: u64, // How much of a block can be processed

    sourceblockcount: u32,    // The total number of blocks
    availableblockcount: u32, // How many undamaged blocks have been found
    missingblockcount: u32,   // How many blocks are missing

    blocksallocated: bool,        // Whether or not the DataBlocks have been allocated
    sourceblocks: Vec<DataBlock>, // The DataBlocks that will be read from disk
    targetblocks: Vec<DataBlock>, // The DataBlocks that will be written to disk

    windowtable: [u32; 256], // Table for sliding CRCs

    blockverifiable: bool, // Whether any files can be verified at the block level
    verificationhashtable: VerificationHashTable, // Hash table for block verification
    /// Files that are not block verifiable (indices into `sourcefiles`)
    unverifiablesourcefiles: Vec<usize>,

    completefilecount: u32, // How many files are fully verified
    renamedfilecount: u32,  // How many files are verified but have the wrong name
    damagedfilecount: u32,  // How many files exist but are damaged
    missingfilecount: u32,  // How many files are completely missing

    inputblocks: Vec<DataBlock>,  // Which DataBlocks will be read from disk
    copyblocks: Vec<DataBlock>,   // Which DataBlocks will copied back to disk
    outputblocks: Vec<DataBlock>, // Which DataBlocks have to calculated using RS

    rs: ReedSolomon<Spec16>, // The Reed Solomon matrix

    transferbuffer: Vec<u8>, // Input blocks in flight (chunksize * NUM_TRANSFER_BUFFERS)
    outputbuffer: Vec<u8>,   // Buffer for writing DataBlocks (chunksize)
    processor: Option<Box<dyn Processor>>, // Multiplies the input blocks by the RS matrix
    ownfactors: bool,        // Whether the processor solved the erasure itself
}

impl Par2Repairer {
    pub fn new(noiselevel: NoiseLevel, backends: Backends) -> Par2Repairer {
        Par2Repairer {
            noiselevel,
            backends,

            searchpath: String::new(),
            basepath: String::new(),

            totalthreads: crate::taskpool::default_threads(),

            skipdata: false,
            skipleaway: 0,
            fullhash: false,

            firstpacket: true,
            setid: Md5Hash::default(),

            recoverypacketmap: BTreeMap::new(),
            mainpacket: None,
            creatorpacket: None,

            diskfiles: Vec::new(),
            diskfilemap: DiskFileMap::new(),

            sourcefilemap: BTreeMap::new(),
            sourcefiles: Vec::new(),
            verifylist: Vec::new(),
            backuplist: Vec::new(),
            par2list: Vec::new(),

            blocksize: 0,
            chunksize: 0,

            sourceblockcount: 0,
            availableblockcount: 0,
            missingblockcount: 0,

            blocksallocated: false,
            sourceblocks: Vec::new(),
            targetblocks: Vec::new(),

            windowtable: [0u32; 256],

            blockverifiable: false,
            verificationhashtable: VerificationHashTable::new(),
            unverifiablesourcefiles: Vec::new(),

            completefilecount: 0,
            renamedfilecount: 0,
            damagedfilecount: 0,
            missingfilecount: 0,

            inputblocks: Vec::new(),
            copyblocks: Vec::new(),
            outputblocks: Vec::new(),

            rs: ReedSolomon::new(),

            transferbuffer: Vec::new(),
            outputbuffer: Vec::new(),
            processor: None,
            ownfactors: false,
        }
    }

    // C++ creates the DiskFile objects with `new`; here they are owned by
    // the repairer and shared through Rc.
    fn add_diskfile(&mut self) -> Rc<RefCell<DiskFile>> {
        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        self.diskfiles.push(diskfile.clone());
        diskfile
    }

    // C++ `diskfilemap.Insert(diskfile)` (which keys on the file's current
    // name) with the C++ `assert(success)`.
    fn map_insert(&mut self, diskfile: &Rc<RefCell<DiskFile>>) {
        let success = self.try_map_insert(diskfile);
        debug_assert!(success);
    }

    // C++ `diskFileMap.Insert(diskfile)` where the caller checks the
    // result: false means the file is already known under that name.
    fn try_map_insert(&mut self, diskfile: &Rc<RefCell<DiskFile>>) -> bool {
        let index = self
            .diskfiles
            .iter()
            .position(|d| Rc::ptr_eq(d, diskfile))
            .expect("disk file not owned by repairer");
        let filename = diskfile.borrow().file_name().to_string();
        self.diskfilemap.insert(&filename, index)
    }

    // C++ `diskfilemap.Remove(diskfile)`.
    fn map_remove(&mut self, diskfile: &Rc<RefCell<DiskFile>>) {
        let filename = diskfile.borrow().file_name().to_string();
        self.diskfilemap.remove(&filename);
    }

    /// Test whether filename has a .par2 / .PAR2 / .Par2 extension.
    ///
    /// C++ Par2Repairer::IsPar2Filename
    pub fn is_par2_filename(filename: &str) -> bool {
        if filename.len() < 5 {
            return false;
        }

        // Check that filename ends with ".par2" (case-insensitive).
        let bytes = filename.as_bytes();
        let ext = &bytes[bytes.len() - 5..];
        if ext[0] != b'.' {
            return false;
        }

        ext[1].eq_ignore_ascii_case(&b'p')
            && ext[2].eq_ignore_ascii_case(&b'a')
            && ext[3].eq_ignore_ascii_case(&b'r')
            && ext[4] == b'2'
    }

    // --------------------------------__Process__-------------------------------------

    /// C++ Par2Repairer::Process
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        memorylimit: usize,
        basepath: &str,
        nthreads: u32,
        filethreads: u32,
        parfilename: String,
        extrafiles: &[String],
        dorepair: bool, // derived from operation
        purgefiles: bool,
        renameonly: bool,
        skipdata: bool,
        skipleaway: u64,
        fullhash: bool,
    ) -> Result {
        // Should we skip data whilst scanning files
        self.skipdata = skipdata;

        // How much leaway should we allow when scanning files
        self.skipleaway = skipleaway;

        // Should the whole of each file be hashed as well as its blocks
        self.fullhash = fullhash;

        // Get filenames from the command line
        self.basepath = basepath.to_string();
        let mut extrafiles: Vec<String> = extrafiles.to_vec();

        // Set the number of threads
        self.totalthreads = resolve_threads(nthreads);

        // The C++ takes no more files at once than there are threads to
        // hash them with; this port reads them one at a time, so the
        // request only reaches the processor through totalthreads.
        let _ = filethreads;

        // Determine the searchpath from the location of the main PAR2 file
        let (searchpath, name) = DiskFile::split_filename(&parfilename);
        self.searchpath = searchpath;

        self.par2list.push(parfilename);

        // Load packets from the main PAR2 file
        let pathname = self.searchpath.clone() + &name;
        if !self.load_packets_from_file(sout, serr, pathname) {
            return Result::LogicError;
        }

        // Load packets from other PAR2 files with names based on the original PAR2 file
        let parfilename = self.par2list[0].clone();
        if !self.load_packets_from_other_files(sout, serr, parfilename) {
            return Result::LogicError;
        }

        // Load packets from any other PAR2 files whose names are given on the command line
        if !self.load_packets_from_extra_files(sout, serr, &extrafiles) {
            return Result::LogicError;
        }

        if self.noiselevel > NoiseLevel::Quiet {
            sout_write(sout, "\n");
        }

        // Check that the packets are consistent and discard any that are not
        if !self.check_packet_consistency(sout, serr) {
            return Result::InsufficientCriticalData;
        }

        // Use the information in the main packet to get the source files
        // into the correct order and determine their filenames
        if !self.create_source_file_list(serr) {
            return Result::LogicError;
        }

        // Determine the total number of DataBlocks for the recoverable source files
        // The allocate the DataBlocks and assign them to each source file
        if !self.allocate_source_blocks(sout, serr) {
            return Result::LogicError;
        }

        // Create a verification hash table for all files for which we have not
        // found a complete version of the file and for which we have
        // a verification packet
        if !self.prepare_verification_hash_table(sout) {
            return Result::LogicError;
        }

        // Compute the table for the sliding CRC computation
        if !self.compute_window_table(sout) {
            return Result::LogicError;
        }

        // Attempt to verify all of the source files
        if !self.verify_source_files(sout, serr, &self.basepath.clone(), &mut extrafiles) {
            return Result::FileIOError;
        }

        let recoverable = self
            .mainpacket
            .as_ref()
            .expect("process requires a main packet")
            .recoverable_file_count();
        if self.completefilecount < recoverable {
            // Scan any extra files specified on the command line
            if !self.verify_extra_files(sout, serr, &extrafiles, &self.basepath.clone(), renameonly)
            {
                return Result::LogicError;
            }
        }

        // Find out how much data we have found
        self.update_verification_results();

        if self.noiselevel > NoiseLevel::Silent {
            sout_write(sout, "\n");
        }

        // Check the verification results and report the results
        if !self.check_verification_results(sout) {
            return Result::RepairNotPossible;
        }

        let recoverable = self
            .mainpacket
            .as_ref()
            .expect("process requires a main packet")
            .recoverable_file_count();

        // Are any of the files incomplete
        if self.completefilecount < recoverable {
            // Do we want to carry out a repair
            if dorepair {
                if self.noiselevel > NoiseLevel::Silent {
                    sout_write(sout, "\n");
                }

                // Rename any damaged or missnamed target files.
                if !self.rename_target_files(serr) {
                    return Result::FileIOError;
                }

                let recoverable = self
                    .mainpacket
                    .as_ref()
                    .expect("process requires a main packet")
                    .recoverable_file_count();

                // Are we still missing any files
                if self.completefilecount < recoverable {
                    // Work out which files are being repaired, create them, and allocate
                    // target DataBlocks to them, and remember them for later verification.
                    if !self.create_target_files(serr) {
                        return Result::FileIOError;
                    }

                    // Allocate memory buffers for reading and writing data to disk, and
                    // build the processor, which is offered the erasures below.
                    if !self.allocate_buffers(sout, serr, memorylimit) {
                        // Delete all of the partly reconstructed files
                        self.delete_incomplete_target_files(serr);
                        return Result::MemoryError;
                    }

                    // Work out which data blocks are available, which need to be copied
                    // directly to the output, and which need to be recreated, and compute
                    // the appropriate Reed Solomon matrix.
                    if !self.compute_rs_matrix(sout, serr) {
                        // Delete all of the partly reconstructed files
                        self.delete_incomplete_target_files(serr);
                        return Result::FileIOError;
                    }

                    if self.noiselevel > NoiseLevel::Silent {
                        sout_write(sout, "\n");
                    }

                    // Set the total amount of data to be processed.
                    let message = if self.missingblockcount > 0 {
                        "Repairing: "
                    } else {
                        "Processing: "
                    };
                    let progress = ProgressMeter::new_shared(
                        Arc::clone(sout),
                        message,
                        self.blocksize * self.sourceblockcount as u64,
                    );

                    // Start at an offset of 0 within a block.
                    let mut blockoffset = 0u64;
                    while blockoffset < self.blocksize {
                        // Work out how much data to process this time.
                        let blocklength =
                            std::cmp::min(self.chunksize, self.blocksize - blockoffset);

                        // Read source data, process it through the RS matrix and write it to disk.
                        if !self.process_data(sout, serr, blockoffset, blocklength as usize, &progress)
                        {
                            // Delete all of the partly reconstructed files
                            self.delete_incomplete_target_files(serr);
                            return Result::FileIOError;
                        }

                        // Advance to the need offset within each block
                        blockoffset += blocklength;
                    }

                    if self.noiselevel > NoiseLevel::Silent {
                        sout_line(sout, "\nVerifying repaired files:\n");
                    }

                    // The repaired files are scanned into buffers of their own, so the ones
                    // the repair read and wrote through are given up first
                    self.transferbuffer = Vec::new();
                    self.outputbuffer = Vec::new();
                    self.processor = None;

                    // Verify that all of the reconstructed target files are now correct
                    if !self.verify_target_files(sout, serr, &self.basepath.clone()) {
                        // Delete all of the partly reconstructed files
                        self.delete_incomplete_target_files(serr);
                        return Result::FileIOError;
                    }
                }

                let recoverable = self
                    .mainpacket
                    .as_ref()
                    .expect("process requires a main packet")
                    .recoverable_file_count();

                // Are all of the target files now complete?
                if self.completefilecount < recoverable {
                    writeln!(serr, "Repair Failed.").expect("cannot write to serr");
                    return Result::RepairFailed;
                } else if self.noiselevel > NoiseLevel::Silent {
                    sout_line(sout, "\nRepair complete.");
                }
            } else {
                return Result::RepairPossible;
            }
        }

        if purgefiles {
            self.remove_backup_files(sout, serr);
            self.remove_par_files(sout, serr);
        }

        Result::Success
    }

    // -----------------------------__LoadPacketsFromFile__-----------------------------

    /// Load the packets from the specified file
    ///
    /// C++ Par2Repairer::LoadPacketsFromFile
    fn load_packets_from_file(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        filename: String,
    ) -> bool {
        // Skip the file if it has already been processed
        if self.diskfilemap.find(&filename).is_some() {
            return true;
        }

        let diskfile = self.add_diskfile();

        // Open the file
        if !diskfile.borrow_mut().open(&filename, serr) {
            // If we could not open the file, ignore the error and
            // proceed to the next file
            return true;
        }

        if self.noiselevel > NoiseLevel::Silent {
            let (_, name) = DiskFile::split_filename(&filename);
            sout_line(sout, &format!("Loading \"{name}\"."));
        }

        // How many useable packets have we found
        let mut packets = 0u32;

        // How many recovery packets were there
        let mut recoverypackets = 0u32;

        // How big is the file
        let filesize = diskfile.borrow().file_size();
        if filesize > 0 {
            // Allocate a buffer to read data into
            // The buffer should be large enough to hold a whole
            // critical packet (i.e. file verification, file description, main,
            // and creator), but not necessarily a whole recovery packet.
            let buffersize = std::cmp::min(1048576u64, filesize) as usize;
            let mut buffer = vec![0u8; buffersize];

            // Progress indicator
            let progress = ProgressMeter::new_shared(Arc::clone(sout), "Loading: ", filesize);

            // Start at the beginning of the file
            let mut offset = 0u64;

            // Continue as long as there is at least enough for the packet header
            while offset + PACKET_HEADER_SIZE as u64 <= filesize {
                if self.noiselevel > NoiseLevel::Quiet {
                    progress.update(offset);
                }

                // Attempt to read the next packet header
                let mut header = [0u8; PACKET_HEADER_SIZE];
                if !diskfile.borrow_mut().read(offset, &mut header, serr) {
                    break;
                }

                // Does this look like it might be a packet
                if packet_magic(&header) != PACKET_MAGIC {
                    offset += 1;

                    // Is there still enough for at least a whole packet header
                    while offset + PACKET_HEADER_SIZE as u64 <= filesize {
                        // How much can we read into the buffer
                        let want = std::cmp::min(buffersize as u64, filesize - offset) as usize;

                        // Fill the buffer
                        if !diskfile.borrow_mut().read(offset, &mut buffer[..want], serr) {
                            offset = filesize;
                            break;
                        }

                        // Scan the buffer for the magic value
                        let limit = want - PACKET_HEADER_SIZE;
                        let mut current = 0usize;
                        while current <= limit
                            && buffer[current..current + 8] != PACKET_MAGIC
                        {
                            current += 1;
                        }

                        // What file offset did we reach
                        offset += current as u64;

                        // Did we find the magic
                        if current <= limit {
                            header.copy_from_slice(&buffer[current..current + PACKET_HEADER_SIZE]);
                            break;
                        }
                    }

                    // Did we reach the end of the file
                    if offset + PACKET_HEADER_SIZE as u64 > filesize {
                        break;
                    }
                }

                // We have found the magic

                let length = packet_length(&header);

                // Check the packet length
                if (PACKET_HEADER_SIZE as u64) > length  // packet length is too small
                    || 0 != (length & 3)                 // packet length is not a multiple of 4
                    || filesize < offset + length
                // packet would extend beyond the end of the file
                {
                    offset += 1;
                    continue;
                }

                // Compute the MD5 Hash of the packet
                let mut context = Md5Context::new();
                context.update(&header[HDR_SETID_OFFSET..PACKET_HEADER_SIZE]);

                // How much more do I need to read to get the whole packet
                let mut current = offset + PACKET_HEADER_SIZE as u64;
                let limit = offset + length;
                while current < limit {
                    let want = std::cmp::min(buffersize as u64, limit - current) as usize;

                    if !diskfile.borrow_mut().read(current, &mut buffer[..want], serr) {
                        break;
                    }

                    context.update(&buffer[..want]);

                    current += want as u64;
                }

                // Did the whole packet get processed
                if current < limit {
                    offset += 1;
                    continue;
                }

                // Check the calculated packet hash against the value in the header
                let hash = context.finalize();
                if hash != packet_hash(&header) {
                    offset += 1;
                    continue;
                }

                // If this is the first packet that we have found then record the setid
                if self.firstpacket {
                    self.setid = packet_setid(&header);
                    self.firstpacket = false;
                }

                // Is the packet from the correct set
                if self.setid == packet_setid(&header) {
                    let packettype = packet_type(&header);

                    // Is it a packet type that we are interested in
                    if packettype == RECOVERYBLOCKPACKET_TYPE {
                        if self.load_recovery_packet(serr, &diskfile, offset, &header) {
                            recoverypackets += 1;
                            packets += 1;
                        }
                    } else if packettype == FILEVERIFICATIONPACKET_TYPE {
                        if self.load_verification_packet(serr, &diskfile, offset, &header) {
                            packets += 1;
                        }
                    } else if packettype == FILEDESCRIPTIONPACKET_TYPE {
                        if self.load_description_packet(serr, &diskfile, offset, &header) {
                            packets += 1;
                        }
                    } else if packettype == MAINPACKET_TYPE {
                        if self.load_main_packet(serr, &diskfile, offset, &header) {
                            packets += 1;
                        }
                    } else if packettype == CREATORPACKET_TYPE
                        && self.load_creator_packet(serr, &diskfile, offset, &header)
                    {
                        packets += 1;
                    }
                }

                // Advance to the next packet
                offset += length;
            }
            if self.noiselevel > NoiseLevel::Quiet {
                progress.update(offset);
            }
        }

        // We have finished with the file for now
        diskfile.borrow_mut().close();

        // Did we actually find any interesting packets
        if packets > 0 {
            if self.noiselevel > NoiseLevel::Quiet {
                let mut message = format!("Loaded {packets} new packets");
                if recoverypackets > 0 {
                    message.push_str(&format!(
                        " including {recoverypackets} recovery blocks"
                    ));
                }
                sout_line(sout, &message);
            }

            // Remember that the file was processed
            self.map_insert(&diskfile);
        } else {
            if self.noiselevel > NoiseLevel::Quiet {
                sout_line(sout, "No new packets found");
            }
        }

        true
    }

    /// Finish loading a recovery packet
    ///
    /// C++ Par2Repairer::LoadRecoveryPacket
    fn load_recovery_packet(
        &mut self,
        serr: &mut dyn Write,
        diskfile: &Rc<RefCell<DiskFile>>,
        offset: u64,
        header: &[u8],
    ) -> bool {
        let mut packet = RecoveryPacket::new();

        // Load the packet from disk
        if !packet.load(Rc::clone(diskfile), offset, header, serr) {
            return false;
        }

        // What is the exponent value of this recovery packet
        let exponent = packet.exponent();

        // Try to insert the new packet into the recovery packet map
        match self.recoverypacketmap.entry(exponent) {
            // The packet must be a duplicate of one we already have
            Entry::Occupied(_) => false,
            Entry::Vacant(slot) => {
                slot.insert(Rc::new(RefCell::new(packet)));
                true
            }
        }
    }

    /// Finish loading a file description packet
    ///
    /// C++ Par2Repairer::LoadDescriptionPacket
    fn load_description_packet(
        &mut self,
        serr: &mut dyn Write,
        diskfile: &Rc<RefCell<DiskFile>>,
        offset: u64,
        header: &[u8],
    ) -> bool {
        let mut packet = DescriptionPacket::new();

        // Load the packet from disk
        if !packet.load(&mut diskfile.borrow_mut(), offset, header, serr) {
            return false;
        }

        // What is the fileid
        let fileid = packet.fileid();

        // Look up the fileid in the source file map for an existing source file entry
        if let Some(sourcefile) = self.sourcefilemap.get(&fileid).cloned() {
            // Does the source file already have a description packet
            if sourcefile.borrow().descriptionpacket().is_some() {
                // Yes. We don't need another copy
                false
            } else {
                // No. Store the packet in the source file
                sourcefile
                    .borrow_mut()
                    .set_description_packet(Some(Rc::new(RefCell::new(packet))));
                true
            }
        } else {
            // Create a new source file for the packet
            let sourcefile = Par2RepairerSourceFile::new(Some(Rc::new(RefCell::new(packet))), None);

            // Record the source file in the source file map
            self.sourcefilemap.insert(fileid, Rc::new(RefCell::new(sourcefile)));

            true
        }
    }

    /// Finish loading a file verification packet
    ///
    /// C++ Par2Repairer::LoadVerificationPacket
    fn load_verification_packet(
        &mut self,
        serr: &mut dyn Write,
        diskfile: &Rc<RefCell<DiskFile>>,
        offset: u64,
        header: &[u8],
    ) -> bool {
        let mut packet = VerificationPacket::new();

        // Load the packet from disk
        if !packet.load(&mut diskfile.borrow_mut(), offset, header, serr) {
            return false;
        }

        // What is the fileid
        let fileid = packet.fileid();

        // Look up the fileid in the source file map for an existing source file entry
        if let Some(sourcefile) = self.sourcefilemap.get(&fileid).cloned() {
            // Does the source file already have a verification packet
            if sourcefile.borrow().verificationpacket().is_some() {
                // Yes. We don't need another copy.
                false
            } else {
                // No. Store the packet in the source file
                sourcefile
                    .borrow_mut()
                    .set_verification_packet(Some(Rc::new(RefCell::new(packet))));

                true
            }
        } else {
            // Create a new source file for the packet
            let sourcefile = Par2RepairerSourceFile::new(None, Some(Rc::new(RefCell::new(packet))));

            // Record the source file in the source file map
            self.sourcefilemap.insert(fileid, Rc::new(RefCell::new(sourcefile)));

            true
        }
    }

    /// Finish loading the main packet
    ///
    /// C++ Par2Repairer::LoadMainPacket
    fn load_main_packet(
        &mut self,
        serr: &mut dyn Write,
        diskfile: &Rc<RefCell<DiskFile>>,
        offset: u64,
        header: &[u8],
    ) -> bool {
        // Do we already have a main packet
        if self.mainpacket.is_some() {
            return false;
        }

        let mut packet = MainPacket::new();

        // Load the packet from disk;
        if !packet.load(&mut diskfile.borrow_mut(), offset, header, serr) {
            return false;
        }

        self.mainpacket = Some(packet);

        true
    }

    /// Finish loading the creator packet
    ///
    /// C++ Par2Repairer::LoadCreatorPacket
    fn load_creator_packet(
        &mut self,
        serr: &mut dyn Write,
        diskfile: &Rc<RefCell<DiskFile>>,
        offset: u64,
        header: &[u8],
    ) -> bool {
        // Do we already have a creator packet
        if self.creatorpacket.is_some() {
            return false;
        }

        let mut packet = CreatorPacket::new();

        // Load the packet from disk;
        if !packet.load(&mut diskfile.borrow_mut(), offset, header, serr) {
            return false;
        }

        self.creatorpacket = Some(packet);

        true
    }

    // ---------------------------__LoadPacketsFromOtherFiles__--------------------------

    /// Load packets from other PAR2 files with names based on the original
    /// PAR2 file
    ///
    /// C++ Par2Repairer::LoadPacketsFromOtherFiles
    fn load_packets_from_other_files(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        filename: String,
    ) -> bool {
        // Split the original PAR2 filename into path and name parts
        let (path, mut name) = DiskFile::split_filename(&filename);

        // Trim ".par2" off of the end original name

        // Look for the last "." in the filename
        while let Some(where_) = name.rfind('.') {
            // Trim what follows the last .
            let tail = name[where_ + 1..].to_string();
            name = name[..where_].to_string();

            // Was what followed the last "." "par2"
            if tail.eq_ignore_ascii_case("par2") {
                break;
            }
        }

        // If what is left ends in ".volNNN-NNN" or ".volNNN+NNN" strip
        // that as well

        // Is there another "."
        if let Some(where_) = name.rfind('.') {
            // What follows the "."
            let tail = &name[where_ + 1..];

            // Scan what follows the last "." to see of it matches
            // vol123-456 or vol123+456
            let mut n = 0u32;
            let mut matched = true;
            for ch in tail.bytes() {
                match n {
                    0 => {
                        if ch.eq_ignore_ascii_case(&b'v') {
                            n = 1;
                        } else {
                            matched = false;
                            break;
                        }
                    }
                    1 => {
                        if ch.eq_ignore_ascii_case(&b'o') {
                            n = 2;
                        } else {
                            matched = false;
                            break;
                        }
                    }
                    2 => {
                        if ch.eq_ignore_ascii_case(&b'l') {
                            n = 3;
                        } else {
                            matched = false;
                            break;
                        }
                    }
                    3 => {
                        if ch.is_ascii_digit() {} else if ch == b'-' || ch == b'+' {
                            n = 4;
                        } else {
                            matched = false;
                            break;
                        }
                    }
                    _ => {
                        if !ch.is_ascii_digit() {
                            matched = false;
                            break;
                        }
                    }
                }
            }

            // If we matched then retain only what precedes the "."
            if matched {
                name = name[..where_].to_string();
            }
        }

        // Find files called "*.par2" or "name.*.par2"
        {
            let wildcard = if name.is_empty() {
                "*.par2".to_string()
            } else {
                format!("{name}.*.par2")
            };
            let files = DiskFile::find_files(&path, &wildcard, false);
            self.par2list.extend(files);

            let wildcardu = if name.is_empty() {
                "*.PAR2".to_string()
            } else {
                format!("{name}.*.PAR2")
            };
            let filesu = DiskFile::find_files(&path, &wildcardu, false);
            self.par2list.extend(filesu);

            // Load packets from each file that was found (the original
            // file is at the front of the list and is skipped because it
            // was already processed)
            let count = self.par2list.len();
            for index in 0..count {
                let filename = self.par2list[index].clone();
                self.load_packets_from_file(sout, serr, filename);
            }
        }

        true
    }

    // ---------------------------__LoadPacketsFromExtraFiles__--------------------------

    /// Load packets from any other PAR2 files whose names are given on the
    /// command line
    ///
    /// C++ Par2Repairer::LoadPacketsFromExtraFiles
    fn load_packets_from_extra_files(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        extrafiles: &[String],
    ) -> bool {
        for filename in extrafiles {
            // If the filename has a .par2 / .PAR2 / .Par2 extension
            if Self::is_par2_filename(filename) {
                self.load_packets_from_file(sout, serr, filename.clone());
            }
        }

        true
    }

    // ----------------------------__CheckPacketConsistency__---------------------------

    /// Check that the packets are consistent and discard any that are not
    ///
    /// C++ Par2Repairer::CheckPacketConsistency
    fn check_packet_consistency(&mut self, sout: &SharedSout, serr: &mut dyn Write) -> bool {
        // Do we have a main packet
        let Some(mainpacket) = &self.mainpacket else {
            // If we don't have a main packet, then there is nothing more
            // that we can do. We cannot verify or repair any files.
            writeln!(serr, "Main packet not found.").expect("cannot write to serr");
            return false;
        };

        // Remember the block size from the main packet
        let blocksize = mainpacket.block_size();
        self.blocksize = blocksize;

        // Check that the recovery blocks have the correct amount of data
        // and discard any that don't
        {
            let discarded: Vec<u32> = self
                .recoverypacketmap
                .iter()
                .filter(|(_, packet)| packet.borrow().block_size() != blocksize)
                .map(|(exponent, _)| *exponent)
                .collect();
            for exponent in discarded {
                let packet = self
                    .recoverypacketmap
                    .remove(&exponent)
                    .expect("discarded recovery packet");
                writeln!(
                    serr,
                    "Incorrect sized recovery block for exponent {} discarded",
                    packet.borrow().exponent()
                )
                .expect("cannot write to serr");
            }
        }

        // Check for source files that have no description packet or where
        // the verification packet has the wrong number of entries and
        // discard them.
        {
            let entries: Vec<(Md5Hash, Rc<RefCell<Par2RepairerSourceFile>>)> = self
                .sourcefilemap
                .iter()
                .map(|(fileid, sourcefile)| (*fileid, Rc::clone(sourcefile)))
                .collect();

            let mut discarded: Vec<Md5Hash> = Vec::new();
            for (fileid, sourcefile) in entries {
                // Do we have a description packet
                let Some(descriptionpacket) = sourcefile.borrow().descriptionpacket() else {
                    // No description packet. Discard the source file
                    discarded.push(fileid);
                    continue;
                };

                // Compute and store the block count from the filesize
                // and blocksize
                if !sourcefile.borrow_mut().set_block_count(blocksize) {
                    writeln!(
                        serr,
                        "Too many blocks in source file \"{}\" discarded",
                        String::from_utf8_lossy(&descriptionpacket.borrow().filename())
                    )
                    .expect("cannot write to serr");

                    discarded.push(fileid);
                    continue;
                }

                // Do we have a verification packet. That is ok, but we
                // won't be able to use block verification.
                let verificationpacket = sourcefile.borrow().verificationpacket();
                if let Some(verificationpacket) = verificationpacket {
                    // Compare the calculated block count with the
                    // verification packet.
                    if sourcefile.borrow().block_count()
                        != verificationpacket.borrow().block_count()
                    {
                        // The block counts are different! Discard the
                        // source file.
                        writeln!(
                            serr,
                            "Incorrectly sized verification packet for \"{}\" discarded",
                            String::from_utf8_lossy(&descriptionpacket.borrow().filename())
                        )
                        .expect("cannot write to serr");

                        discarded.push(fileid);
                        continue;
                    }
                }

                // Everything is ok.
            }

            for fileid in discarded {
                self.sourcefilemap.remove(&fileid);
            }
        }

        if self.noiselevel > NoiseLevel::Quiet {
            let mainpacket = self
                .mainpacket
                .as_ref()
                .expect("check_packet_consistency requires a main packet");
            sout_line(
                sout,
                &format!(
                    "There are {} recoverable files and {} other files.\nThe block size used was {} bytes.",
                    mainpacket.recoverable_file_count(),
                    mainpacket.total_file_count() - mainpacket.recoverable_file_count(),
                    self.blocksize
                ),
            );
        }

        true
    }

    // -----------------------------__CreateSourceFileList__-----------------------------

    /// Use the information in the main packet to get the source files into
    /// the correct order and determine their filenames
    ///
    /// C++ Par2Repairer::CreateSourceFileList
    fn create_source_file_list(&mut self, serr: &mut dyn Write) -> bool {
        let total_file_count = self
            .mainpacket
            .as_ref()
            .expect("create_source_file_list requires a main packet")
            .total_file_count();

        // For each FileId entry in the main packet
        for filenumber in 0..total_file_count {
            let fileid = self
                .mainpacket
                .as_ref()
                .expect("create_source_file_list requires a main packet")
                .fileid(filenumber);

            // Look up the fileid in the source file map
            let sourcefile = self.sourcefilemap.get(&fileid).cloned();

            if let Some(sourcefile) = &sourcefile {
                let basepath = self.basepath.clone();
                sourcefile
                    .borrow_mut()
                    .compute_target_filename(serr, self.noiselevel, &basepath);

                // Need actual filesize on disk for mt-progress line
                sourcefile.borrow_mut().set_disk_file_size();
            }

            self.sourcefiles.push(sourcefile);
        }

        true
    }

    // ------------------------------__AllocateSourceBlocks__---------------------------

    /// Determine the total number of DataBlocks for the recoverable source
    /// files then allocate the DataBlocks and assign them to each source
    /// file
    ///
    /// C++ Par2Repairer::AllocateSourceBlocks
    fn allocate_source_blocks(&mut self, sout: &SharedSout, serr: &mut dyn Write) -> bool {
        self.sourceblockcount = 0;

        let recoverable_file_count = self
            .mainpacket
            .as_ref()
            .expect("allocate_source_blocks requires a main packet")
            .recoverable_file_count();

        // For each recoverable source file
        for (filenumber, sourcefile) in self.sourcefiles.iter().enumerate() {
            let filenumber = filenumber as u32;
            if filenumber >= recoverable_file_count {
                break;
            }

            // Do we have a source file
            if let Some(sourcefile) = sourcefile {
                let blockcount = sourcefile.borrow().block_count();
                if blockcount > u32::MAX - self.sourceblockcount {
                    writeln!(serr, "Too many source blocks in recovery set.")
                        .expect("cannot write to serr");
                    return false;
                }

                self.sourceblockcount += blockcount;
            } else {
                // No details for this source file so we don't know what
                // the total number of source blocks is
                //      sourceblockcount = 0;
                //      break;
            }
        }

        // Did we determine the total number of source blocks
        if self.sourceblockcount > 0 {
            // Yes.

            // Allocate all of the Source and Target DataBlocks (which will
            // be used to read and write data to disk).
            self.sourceblocks = vec![DataBlock::new(); self.sourceblockcount as usize];
            self.targetblocks = vec![DataBlock::new(); self.sourceblockcount as usize];

            // Which DataBlocks will be allocated first
            let mut totalsize = 0u64;
            let mut blocknumber = 0u32;
            let mut blockindex = 0usize;

            for (filenumber, sourcefile) in self.sourcefiles.iter().enumerate() {
                let filenumber = filenumber as u32;
                if filenumber >= recoverable_file_count {
                    break;
                }

                if let Some(sourcefile) = sourcefile {
                    totalsize += sourcefile
                        .borrow()
                        .descriptionpacket()
                        .expect("allocate_source_blocks requires a description packet")
                        .borrow()
                        .filesize();
                    let blockcount = sourcefile.borrow().block_count();

                    // Allocate the source and target DataBlocks to the
                    // sourcefile
                    sourcefile.borrow_mut().set_blocks(
                        blocknumber,
                        blockcount,
                        blockindex,
                        blockindex,
                        &mut self.sourceblocks[blockindex..],
                        self.blocksize,
                    );

                    blocknumber += 1;
                    blockindex += blockcount as usize;
                }
            }

            self.blocksallocated = true;

            if self.noiselevel > NoiseLevel::Quiet {
                sout_line(
                    sout,
                    &format!(
                        "There are a total of {} data blocks.\nThe total size of the data files is {} bytes.",
                        self.sourceblockcount, totalsize
                    ),
                );
            }
        }

        true
    }

    // --------------------------__PrepareVerificationHashTable__-----------------------

    /// Create a verification hash table for all files for which we have not
    /// found a complete version of the file and for which we have a
    /// verification packet
    ///
    /// C++ Par2Repairer::PrepareVerificationHashTable
    fn prepare_verification_hash_table(&mut self, sout: &SharedSout) -> bool {
        if self.noiselevel >= NoiseLevel::Debug {
            sout_line(sout, "[DEBUG] Prepare verification hashtable");
        }

        // Choose a size for the hash table
        self.verificationhashtable.set_limit(self.sourceblockcount);

        // Will any files be block verifiable
        self.blockverifiable = false;

        // For each source file
        for (index, sourcefile) in self.sourcefiles.iter().enumerate() {
            if let Some(sourcefile) = sourcefile {
                // Do we have a verification packet
                if sourcefile.borrow().verificationpacket().is_some() {
                    // Yes. Load the verification entries into the hash table
                    self.verificationhashtable
                        .load(sourcefile, self.blocksize);

                    self.blockverifiable = true;
                } else {
                    // No. We can only check the whole file
                    self.unverifiablesourcefiles.push(index);
                }
            }
        }

        true
    }

    // --------------------------------__ComputeWindowTable__---------------------------

    /// Compute the table for the sliding CRC computation
    ///
    /// C++ Par2Repairer::ComputeWindowTable
    fn compute_window_table(&mut self, sout: &SharedSout) -> bool {
        if self.noiselevel >= NoiseLevel::Debug {
            sout_line(sout, "[DEBUG] compute window table");
        }

        if self.blockverifiable {
            self.windowtable = generate_window_table(self.blocksize);
        }

        true
    }

    // -------------------------------__VerifySourceFiles__------------------------------

    /// Attempt to verify all of the source files
    ///
    /// C++ Par2Repairer::VerifySourceFiles
    fn verify_source_files(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        basepath: &str,
        extrafiles: &mut Vec<String>,
    ) -> bool {
        if self.noiselevel > NoiseLevel::Quiet {
            sout_line(sout, "\nVerifying source files:\n");
        }

        let mut finalresult = true;

        // Created a sorted list of the source files and verify them in
        // that order rather than the order they are in the main packet.
        let mut sortedfiles: Vec<Rc<RefCell<Par2RepairerSourceFile>>> = Vec::new();

        let recoverable_file_count = self
            .mainpacket
            .as_ref()
            .expect("verify_source_files requires a main packet")
            .recoverable_file_count();

        let mut mttotalsize = 0u64;

        for (filenumber, sourcefile) in self.sourcefiles.iter().enumerate() {
            // Do we have a source file
            if let Some(sourcefile) = sourcefile {
                sortedfiles.push(Rc::clone(sourcefile));
                // Total filesizes for mt-progress line
                mttotalsize += sourcefile.borrow().disk_filesize();
            } else {
                // Was this one of the recoverable files
                if (filenumber as u32) < recoverable_file_count {
                    writeln!(
                        serr,
                        "No details available for recoverable file number {}.\nRecovery will not be possible.",
                        filenumber + 1
                    )
                    .expect("cannot write to serr");

                    // Set error but let verification of other files continue
                    finalresult = false;
                } else {
                    writeln!(
                        serr,
                        "No details available for non-recoverable file number {}",
                        filenumber as u32 - recoverable_file_count + 1
                    )
                    .expect("cannot write to serr");
                }
            }
        }

        sortedfiles.sort_by(|a, b| a.borrow().targetfilename().cmp(b.borrow().targetfilename()));

        let progress = ProgressMeter::new_shared(Arc::clone(sout), "Scanning: ", mttotalsize);

        // Start verifying the files
        for sourcefile in sortedfiles {
            // What filename does the file use
            let file = sourcefile.borrow().targetfilename().to_string();
            let name = DiskFile::split_relative_filename(&file, basepath);
            let target_pathname = DiskFile::get_canonical_pathname(&file);

            if self.noiselevel >= NoiseLevel::Debug {
                sout_line(
                    sout,
                    &format!(
                        "[DEBUG] VerifySourceFiles ----\n[DEBUG] file: {file}\n[DEBUG] name: {name}\n[DEBUG] targ: {target_pathname}"
                    ),
                );
            }

            // if the target file is in the list of extra files, we remove
            // it from the extra files.
            if let Some(position) = extrafiles
                .iter()
                .position(|e| e.as_str() == target_pathname.as_str())
            {
                extrafiles.remove(position);
            }

            let diskfile = self.add_diskfile();

            // Does the target file exist
            if !diskfile.borrow_mut().open(&file, serr) {
                // The file does not exist.
                if self.noiselevel > NoiseLevel::Silent {
                    sout_line(sout, &format!("Target: \"{name}\" - missing."));
                }

                continue;
            }

            // Remember that we have processed this file. Two source files
            // of the set may name the same one, and the insert is what
            // settles which one gets it.
            let claimed = self.try_map_insert(&diskfile);

            if !claimed {
                // The file has already been used!
                diskfile.borrow_mut().close();

                writeln!(serr, "Source file {name} is a duplicate.")
                    .expect("cannot write to serr");

                finalresult = false;

                continue;
            }

            // Yes. Record that fact.
            sourcefile.borrow_mut().set_target_exists(true);

            // Remember that the DiskFile is the target file
            sourcefile
                .borrow_mut()
                .set_target_file(Some(Rc::clone(&diskfile)));

            // Do the actual verification
            if !self.verify_data_file(
                sout,
                serr,
                &diskfile,
                Some(Rc::clone(&sourcefile)),
                basepath,
                &progress,
                false,
            ) {
                finalresult = false;
            }

            // We have finished with the file for now
            diskfile.borrow_mut().close();
        }

        // Find out how much data we have found
        self.update_verification_results();

        finalresult
    }

    // -------------------------------__VerifyExtraFiles__-------------------------------

    /// Scan any extra files specified on the command line
    ///
    /// C++ Par2Repairer::VerifyExtraFiles
    fn verify_extra_files(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        extrafiles: &[String],
        basepath: &str,
        renameonly: bool,
    ) -> bool {
        if self.noiselevel > NoiseLevel::Quiet {
            sout_line(sout, "\nScanning extra files:\n");
        }

        let recoverable_file_count = self
            .mainpacket
            .as_ref()
            .expect("verify_extra_files requires a main packet")
            .recoverable_file_count();

        if self.completefilecount < recoverable_file_count {
            // Total size of extra files for mt-progress line
            let mttotalextrasize: u64 = extrafiles
                .iter()
                .map(|filename| DiskFile::get_file_size(filename))
                .sum();

            let progress =
                ProgressMeter::new_shared(Arc::clone(sout), "Scanning: ", mttotalextrasize);

            for extrafile in extrafiles {
                let mut filename = extrafile.clone();

                // If the filename does not have a .par2 / .PAR2 / .Par2
                // extension we are interested in it.
                if !Self::is_par2_filename(&filename) {
                    filename = DiskFile::get_canonical_pathname(&filename);

                    // Has this file already been dealt with
                    if self.diskfilemap.find(&filename).is_none() {
                        let diskfile = self.add_diskfile();

                        // Does the file exist
                        if !diskfile.borrow_mut().open(&filename, serr) {
                            continue;
                        }

                        // Remember that we have processed this file.
                        // Another caller may be scanning the same one, and
                        // the insert is what settles which gets it.
                        let claimed = self.try_map_insert(&diskfile);

                        if !claimed {
                            diskfile.borrow_mut().close();
                            continue;
                        }

                        // Do the actual verification
                        // Ignore errors
                        let _ = self.verify_data_file(
                            sout,
                            serr,
                            &diskfile,
                            None,
                            basepath,
                            &progress,
                            renameonly,
                        );

                        // We have finished with the file for now
                        diskfile.borrow_mut().close();
                    }
                }
            }
        }

        // Find out how much data we have found
        self.update_verification_results();

        true
    }

    // -------------------------------__VerifyDataFile__---------------------------------

    /// Attempt to match the data in the DiskFile with the source file
    ///
    /// C++ Par2Repairer::VerifyDataFile
    #[allow(clippy::too_many_arguments)]
    fn verify_data_file(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        diskfile: &Rc<RefCell<DiskFile>>,
        mut sourcefile: Option<Rc<RefCell<Par2RepairerSourceFile>>>,
        basepath: &str,
        progress: &ProgressMeter,
        renameonly: bool,
    ) -> bool {
        // What type of match was made
        let mut matchtype = MatchType::NoMatch;
        // The MD5 Hash of the whole file
        let mut hashfull = Md5Hash::default();
        // The MD5 Hash of the first 16k of the file
        let mut hash16k = Md5Hash::default();

        // Are there any files that can be verified at the block level
        if self.blockverifiable {
            // Scan the file at the block level.
            if !self.scan_data_file(
                sout,
                serr,
                diskfile,
                &mut sourcefile,
                basepath,
                progress,
                renameonly,
                &mut matchtype,
                &mut hashfull,
                &mut hash16k,
            ) {
                return false;
            }

            match matchtype {
                // No data was found at all.
                MatchType::NoMatch => {}
                // We found some data.
                MatchType::PartialMatch => {
                    // Return them.
                    return true;
                }
                // We found a perfect match.
                MatchType::FullMatch => {
                    sourcefile
                        .expect("full match requires a source file")
                        .borrow_mut()
                        .set_complete_file(Some(Rc::clone(diskfile)));

                    // Return the match
                    return true;
                }
            }
        }

        // We did not find a match for any blocks of data within the file,
        // but if there are any files for which we did not have a
        // verification packet we can try a simple match of the hash for
        // the whole file.

        // Are there any files that cannot be verified at the block level
        if !self.unverifiablesourcefiles.is_empty() {
            // Would we have already computed the file hashes
            if !self.blockverifiable {
                let filesize = diskfile.borrow().file_size();

                let mut buffersize = 1024 * 1024usize;
                let limit = std::cmp::min(self.blocksize, filesize);
                if buffersize as u64 > limit {
                    buffersize = limit as usize;
                }

                let mut buffer = vec![0u8; buffersize];

                let mut offset = 0u64;

                let mut context = Md5Context::new();

                while offset < filesize {
                    let want = std::cmp::min(buffersize as u64, filesize - offset) as usize;

                    if !diskfile.borrow_mut().read(offset, &mut buffer[..want], serr) {
                        return false;
                    }

                    // Will the newly read data reach the 16k boundary
                    if offset < 16384 && offset + want as u64 >= 16384 {
                        context.update(&buffer[..(16384 - offset) as usize]);

                        // Compute the 16k hash
                        let mut temp = context.clone();
                        hash16k = temp.finalize();

                        // Is there more data
                        if offset + want as u64 > 16384 {
                            context.update(&buffer[(16384 - offset) as usize..want]);
                        }
                    } else {
                        context.update(&buffer[..want]);
                    }

                    offset += want as u64;
                }

                // Compute the file hash
                hashfull = context.finalize();

                // If we did not have 16k of data, then the 16k hash
                // is the same as the full hash
                if filesize < 16384 {
                    hash16k = hashfull;
                }
            }

            // Compare the hash values of each source file for a match
            let unverifiablesourcefiles = self.unverifiablesourcefiles.clone();
            for index in unverifiablesourcefiles {
                let sourcefile = self
                    .sourcefiles
                    .get(index)
                    .and_then(|sf| sf.as_ref())
                    .expect("unverifiable source file entry")
                    .clone();

                // Does the file match
                let descriptionpacket = sourcefile
                    .borrow()
                    .descriptionpacket()
                    .expect("unverifiable source file requires a description packet");
                let descriptionpacket = descriptionpacket.borrow();

                if sourcefile.borrow().get_complete_file().is_none()
                    && diskfile.borrow().file_size() == descriptionpacket.filesize()
                    && hash16k == descriptionpacket.hash16k_()
                    && hashfull == descriptionpacket.hashfull_()
                {
                    if self.noiselevel > NoiseLevel::Silent {
                        sout_line(
                            sout,
                            &format!(
                                "{} is a perfect match for {}",
                                diskfile.borrow().file_name(),
                                String::from_utf8_lossy(&descriptionpacket.filename())
                            ),
                        );
                    }

                    let filesize = descriptionpacket.filesize();
                    drop(descriptionpacket);

                    // Record that we have a perfect match for this source
                    // file
                    sourcefile
                        .borrow_mut()
                        .set_complete_file(Some(Rc::clone(diskfile)));

                    if self.blocksallocated {
                        // Allocate all of the DataBlocks for the source
                        // file to the DiskFile
                        let firstsourceblock = sourcefile.borrow().first_source_block();

                        let mut offset = 0u64;
                        let mut blockindex = 0usize;
                        while offset < filesize {
                            let datablock = &mut self.sourceblocks[firstsourceblock + blockindex];
                            datablock.set_location(Rc::clone(diskfile), offset);
                            datablock.set_length(std::cmp::min(self.blocksize, filesize - offset));

                            offset += self.blocksize;
                            blockindex += 1;
                        }
                    }

                    // Return the match
                    return true;
                }
            }
        }

        true
    }

    // -----------------------------__ScanDataFileAligned__-----------------------------

    /// Check every block of a source file at the offset where it is
    /// expected to be.
    ///
    /// C++ Par2Repairer::ScanDataFileAligned. The C++ hands batches of
    /// blocks to its worker pool while it reads ahead into shared buffers;
    /// this port reads and checks one block at a time on the calling
    /// thread and finds the same blocks.
    #[allow(clippy::too_many_arguments)]
    fn scan_data_file_aligned(
        &mut self,
        serr: &mut dyn Write,
        diskfile: &Rc<RefCell<DiskFile>>,
        progress: &ProgressMeter,
        sourcefile: &Option<Rc<RefCell<Par2RepairerSourceFile>>>,
        matched: &mut Vec<bool>,
        matchcount: &mut u32,
        hashfull: &mut Md5Hash,
        hash16k: &mut Md5Hash,
    ) -> bool {
        *matchcount = 0;

        // We must know which source file the data is supposed to belong to
        let Some(sourcefile) = sourcefile else {
            return false;
        };

        let verificationpacket = match sourcefile.borrow().verificationpacket() {
            Some(verificationpacket) => verificationpacket,
            None => return false,
        };

        // A source file which has already been matched must not claim its
        // blocks again
        if sourcefile.borrow().get_complete_file().is_some() {
            return false;
        }

        // Unless the file is exactly the right length it cannot be a
        // perfect match
        let filesize = sourcefile
            .borrow()
            .descriptionpacket()
            .expect("aligned scan requires a description packet")
            .borrow()
            .filesize();
        if diskfile.borrow().file_size() != filesize {
            return false;
        }

        let blockcount = verificationpacket.borrow().block_count();
        if blockcount == 0 {
            return false;
        }

        matched.clear();
        matched.resize(blockcount as usize, false);

        let blocksize = self.blocksize;

        // The hashes of the file are computed here, not by the block
        // hasher (the C++ FileHasher filehasher(fullhash)).
        let mut filehasher = FileHasher::new(self.fullhash);

        let mut hasher = self.backends.build_hasher();
        if !hasher.init(filesize, blocksize as usize, false) {
            return false;
        }

        let mut buffer = vec![0u8; blocksize as usize];
        let mut matchedblock = [false; 1];

        for block in 0..blockcount {
            let offset = u64::from(block) * blocksize;
            let length = std::cmp::min(blocksize, filesize - offset) as usize;

            if !diskfile
                .borrow_mut()
                .read(offset, &mut buffer[..length], serr)
            {
                return false;
            }

            filehasher.update(offset, &buffer[..length]);

            // Only the last block of a file can be short, and its entry
            // covers it padded out to the full block size with zeroes
            if length < blocksize as usize {
                buffer[length..].fill(0);
            }

            // A verification entry is the 20 bytes a block is expected to
            // hash to: 16 bytes of MD5 then the CRC32 little endian.
            let entry = verificationpacket.borrow().verification_entry(block);
            let mut expected = [0u8; FILEVERIFICATIONENTRY_SIZE];
            expected[..16].copy_from_slice(&entry.hash.hash);
            expected[16..].copy_from_slice(&entry.crc.to_le_bytes());

            hasher.check_blocks(&buffer, 1, 0, &expected, &mut matchedblock);

            if matchedblock[0] {
                matched[block as usize] = true;
                *matchcount += 1;

                if self.noiselevel > NoiseLevel::Quiet {
                    progress.add(length as u64);
                }
            }
        }

        let (full, short) = filehasher.get_hashes(filesize);
        if let Some(full) = full {
            *hashfull = full;
        }
        *hash16k = short;

        true
    }

    // --------------------------------__ScanDataFile__---------------------------------

    /// Perform a sliding window scan of the DiskFile looking for blocks of
    /// data that might belong to any of the source files (for which a
    /// verification packet was available). If a block of data might be from
    /// more than one source file, prefer the one specified by the
    /// `sourcefile` parameter. If the first data block found is for a
    /// different source file then `sourcefile` is changed accordingly.
    ///
    /// C++ Par2Repairer::ScanDataFile
    #[allow(clippy::too_many_arguments)]
    fn scan_data_file(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        diskfile: &Rc<RefCell<DiskFile>>,
        sourcefile: &mut Option<Rc<RefCell<Par2RepairerSourceFile>>>,
        basepath: &str,
        progress: &ProgressMeter,
        renameonly: bool,
        matchtype: &mut MatchType,
        hashfull: &mut Md5Hash,
        hash16k: &mut Md5Hash,
    ) -> bool {
        // How many matches have we had
        let mut count = 0u32;

        // Remember which file we wanted to match
        let originalsourcefile = sourcefile.clone();

        let filename = diskfile.borrow().file_name().to_string();
        let name = DiskFile::split_relative_filename(&filename, basepath);

        // Is the file empty
        if diskfile.borrow().file_size() == 0 {
            *matchtype = MatchType::NoMatch;

            // The hash of an empty file is needed to match against source
            // files which have no verification packet.
            if !self.unverifiablesourcefiles.is_empty() {
                let mut context = Md5Context::new();
                *hash16k = context.finalize();
                *hashfull = *hash16k;
            }

            // If the file is empty, then just return
            if self.noiselevel > NoiseLevel::Silent {
                if originalsourcefile.is_some() {
                    sout_line(sout, &format!("Target: \"{name}\" - empty."));
                } else {
                    sout_line(sout, &format!("File: \"{name}\" - empty."));
                }
            }

            return true;
        }

        let shortname = if name.chars().count() > 56 {
            let chars: Vec<char> = name.chars().collect();
            let head: String = chars[..28].iter().collect();
            let tail: String = chars[chars.len() - 28..].iter().collect();
            format!("{head}...{tail}")
        } else {
            name.clone()
        };

        if self.noiselevel > NoiseLevel::Quiet {
            sout_line(sout, &format!("Opening: \"{shortname}\""));
        }

        // Assume we will make a perfect match for the file
        *matchtype = MatchType::FullMatch;

        // How many duplicate blocks have been found. The C++ ignores
        // duplicates (`if (duplicate && false)`), so this stays 0.
        let duplicatecount = 0u32;

        // Have we found data blocks in this file that belong to more than
        // one target file
        let mut multipletargets = false;

        // Total number of bytes that were skipped whilst scanning
        let mut skippeddata = 0u64;

        let filesize = diskfile.borrow().file_size();

        let mut alignedmatch: Vec<bool> = Vec::new();
        let mut alignedcount = 0u32;
        let aligned = self.scan_data_file_aligned(
            serr,
            diskfile,
            progress,
            sourcefile,
            &mut alignedmatch,
            &mut alignedcount,
            hashfull,
            hash16k,
        );

        // The parts of the file which still have to be searched a byte at
        // a time
        let mut searchranges: Vec<(u64, u64)> = Vec::new();

        if aligned && alignedcount > 0 {
            let blockcount = alignedmatch.len();
            let firstsourceblock = sourcefile
                .as_ref()
                .expect("aligned scan requires a source file")
                .borrow()
                .first_source_block();

            // Record the blocks which were found where they were expected.
            // Their lengths were set when the source file was given its
            // data blocks.
            for (blocknumber, matched) in alignedmatch.iter().enumerate() {
                if *matched {
                    if self.blocksallocated {
                        self.sourceblocks[firstsourceblock + blocknumber]
                            .set_location(Rc::clone(diskfile), blocknumber as u64 * self.blocksize);
                    }

                    count += 1;
                }
            }

            if alignedcount < blockcount as u32 {
                *matchtype = MatchType::PartialMatch;

                // In rename-only mode, skip files that are not perfect
                // matches
                if renameonly {
                    return true;
                }

                // Search each run of blocks which was not where it was
                // expected. The blocks on either side of a run have been
                // claimed already, so nothing outside these ranges is left
                // to find.
                let mut blocknumber = 0usize;
                while blocknumber < blockcount {
                    if alignedmatch[blocknumber] {
                        blocknumber += 1;
                        continue;
                    }

                    let gapfirst = blocknumber;
                    while blocknumber < blockcount && !alignedmatch[blocknumber] {
                        blocknumber += 1;
                    }

                    searchranges.push((
                        gapfirst as u64 * self.blocksize,
                        std::cmp::min(filesize, blocknumber as u64 * self.blocksize),
                    ));
                }
            }
        } else {
            // Nothing is known about where the data is, so search all of it
            searchranges.push((0, filesize));
        }

        // Whichever scan read the whole of the file from its start
        // produced the 16k hash, and the whole file hash if that was asked
        // for
        let mut filehashes = aligned;

        if !searchranges.is_empty() {
            let wholefilesearched =
                searchranges.len() == 1 && searchranges[0].0 == 0 && searchranges[0].1 == filesize;

            // The MD5 hash of the whole file is only needed to match
            // against source files which have no verification packet, and
            // when it was asked for.
            let computefilehashes = ((self.fullhash && !aligned)
                || !self.unverifiablesourcefiles.is_empty())
                && wholefilesearched;

            filehashes = filehashes || wholefilesearched;

            // Create the checksummer for the file
            let mut filechecksummer = FileCheckSummer::new(
                Rc::clone(diskfile),
                self.blocksize,
                &self.windowtable,
                computefilehashes,
            );

            // How far will we scan the file (1 byte at a time) before
            // skipping ahead looking for the next block
            let scandistance = std::cmp::min(self.skipleaway << 1, self.blocksize);

            // Distance to skip forward if we don't find a block
            let scanskip = if self.skipdata {
                self.blocksize - scandistance
            } else {
                0
            };

            for (rangestart, rangeend) in searchranges {
                if !filechecksummer.start_at(rangestart, serr) {
                    return false;
                }

                // Which block do we expect to find first. Nothing is
                // suggested at the start of a range, just as nothing is
                // suggested after a block is not found.
                let mut nextentry: Option<usize> = None;

                // Assume we are half way through scanning
                let mut scanoffset = scandistance >> 1;

                // Offset of last data that was found
                let mut lastmatchoffset = rangestart;

                let mut oldoffset = rangestart;
                let mut printprogress = 0u64;

                // Whilst we have not reached the end of the range
                while filechecksummer.offset() < rangeend {
                    if self.noiselevel > NoiseLevel::Quiet {
                        // Update progress indicator
                        printprogress += filechecksummer.offset() - oldoffset;
                        if printprogress >= self.blocksize || filechecksummer.short_block() {
                            progress.add(printprogress);
                            printprogress = 0;
                        }
                        oldoffset = filechecksummer.offset();
                    }

                    // If we fail to find a match, it might be because it
                    // was a duplicate of a block that we have already
                    // found.
                    let mut duplicate = false;

                    // Look for a match
                    let currententry = self.verificationhashtable.find_match(
                        nextentry,
                        sourcefile.as_ref(),
                        &filechecksummer,
                        &mut duplicate,
                        &self.sourceblocks,
                    );

                    // Did we find a match
                    if let Some(currententry) = currententry {
                        if lastmatchoffset < filechecksummer.offset()
                            && self.noiselevel > NoiseLevel::Normal
                        {
                            progress.print_line(&format!(
                                "No data found between offset {} and {}",
                                lastmatchoffset,
                                filechecksummer.offset()
                            ));
                        }

                        // Is this the first match
                        if count == 0 {
                            // Which source file was it
                            *sourcefile = Some(Rc::clone(
                                self.verificationhashtable
                                    .entry(currententry)
                                    .sourcefile(),
                            ));

                            // If the first match found was not actually
                            // the first block for the source file, or it
                            // was not at the start of the data file: then
                            // this is a partial match.
                            if !self.verificationhashtable.entry(currententry).firstblock()
                                || filechecksummer.offset() != 0
                            {
                                *matchtype = MatchType::PartialMatch;

                                // In rename-only mode, skip files that are
                                // not perfect matches
                                if renameonly {
                                    return true;
                                }
                            }
                        } else {
                            // If the match found is not the one which was
                            // expected then this is a partial match
                            if Some(currententry) != nextentry {
                                *matchtype = MatchType::PartialMatch;

                                // In rename-only mode, skip files that are
                                // not perfect matches
                                if renameonly {
                                    return true;
                                }
                            }

                            // Is the match from a different source file
                            let same_sourcefile = sourcefile.as_ref().is_some_and(|sf| {
                                Rc::ptr_eq(
                                    sf,
                                    self.verificationhashtable
                                        .entry(currententry)
                                        .sourcefile(),
                                )
                            });
                            if !same_sourcefile {
                                multipletargets = true;
                            }
                        }

                        if self.blocksallocated {
                            // Record the match
                            let offset = filechecksummer.offset();
                            self.verificationhashtable.set_block(
                                currententry,
                                Rc::clone(diskfile),
                                offset,
                                &mut self.sourceblocks,
                            );
                        }

                        // Update the number of matches found
                        count += 1;

                        // What entry do we expect next
                        nextentry = self.verificationhashtable.entry(currententry).next();

                        // Advance to the next block
                        let blocklength = self.sourceblocks
                            [self.verificationhashtable.entry(currententry).datablock()]
                            .length();
                        if !filechecksummer.jump(blocklength, serr) {
                            return false;
                        }

                        // If the next match fails, assume we are half way
                        // through scanning for the next block
                        scanoffset = scandistance >> 1;

                        // Update offset of last match
                        lastmatchoffset = filechecksummer.offset();
                    } else {
                        // This cannot be a perfect match
                        *matchtype = MatchType::PartialMatch;

                        // In rename-only mode, skip files that are not
                        // perfect matches
                        if renameonly {
                            return true;
                        }

                        // What entry do we expect next
                        nextentry = None;

                        if !filechecksummer.step(serr) {
                            return false;
                        }

                        let skipfrom = filechecksummer.offset();

                        // Have we scanned too far without finding a block?
                        if scanskip > 0 {
                            scanoffset += 1;
                            if scanoffset >= scandistance && skipfrom < rangeend {
                                // Skip forwards to where we think we might
                                // find more data
                                if !filechecksummer.jump(scanskip, serr) {
                                    return false;
                                }

                                // Update the count of skipped data
                                skippeddata += filechecksummer.offset() - skipfrom;

                                // Reset scan offset to 0
                                scanoffset = 0;
                            }
                        }
                    }
                }

                if self.noiselevel > NoiseLevel::Quiet && filechecksummer.offset() >= rangeend {
                    progress.add(filechecksummer.offset() - oldoffset);
                }

                if lastmatchoffset < filechecksummer.offset()
                    && self.noiselevel > NoiseLevel::Normal
                {
                    progress.print_line(&format!(
                        "No data found between offset {} and {}",
                        lastmatchoffset,
                        filechecksummer.offset()
                    ));
                }
            }

            // Get the Full and 16k hash values of the file
            if wholefilesearched {
                let (full, short) = filechecksummer.get_file_hashes();
                if let Some(full) = full {
                    *hashfull = full;
                }
                *hash16k = short;
            }
        }

        if self.noiselevel >= NoiseLevel::Debug {
            let mut ss = String::new();
            if duplicatecount > 0 {
                let _ = writeln!(ss, "[DEBUG] duplicates: {duplicatecount}");
            }
            let _ = write!(
                ss,
                "[DEBUG] matchcount: {count}\n[DEBUG] ----------------------"
            );
            progress.print_line(&ss);
        }

        // Did we make any matches at all
        if count > 0 {
            let sourcefile = sourcefile
                .as_ref()
                .expect("a match requires a source file");
            let verificationpacket = sourcefile
                .borrow()
                .verificationpacket()
                .expect("a match requires a verification packet");
            let descriptionpacket = sourcefile
                .borrow()
                .descriptionpacket()
                .expect("a match requires a description packet");

            let blockcount = verificationpacket.borrow().block_count();

            // If this still might be a perfect match, check the file size
            // and number of blocks to confirm. A full match verifies the
            // file against the verification packet, and the description
            // packet's hashes are a separate claim, so the 16k hash is
            // checked as well, and the hash of the whole file when that was
            // asked for.
            let is_partial = {
                let descriptionpacket = descriptionpacket.borrow();
                *matchtype != MatchType::FullMatch
                    || count != blockcount
                    || diskfile.borrow().file_size() != descriptionpacket.filesize()
                    || (filehashes
                        && (*hash16k != descriptionpacket.hash16k_()
                            || (self.fullhash
                                && *hashfull != descriptionpacket.hashfull_())))
            };

            if is_partial {
                *matchtype = MatchType::PartialMatch;

                if self.noiselevel > NoiseLevel::Silent {
                    // Did we find data from multiple target files
                    if multipletargets {
                        // Were we scanning the target file or an extra file
                        if originalsourcefile.is_some() {
                            sout_line(
                                sout,
                                &format!("Target: \"{name}\" - damaged, found {count} data blocks from several target files."),
                            );
                        } else {
                            sout_line(
                                sout,
                                &format!("File: \"{name}\" - found {count} data blocks from several target files."),
                            );
                        }
                    } else {
                        // Did we find data blocks that belong to the target
                        // file
                        let same_target = originalsourcefile
                            .as_ref()
                            .is_some_and(|o| Rc::ptr_eq(o, sourcefile));
                        if same_target {
                            sout_line(
                                sout,
                                &format!("Target: \"{name}\" - damaged. Found {count} of {blockcount} data blocks."),
                            );
                        } else if originalsourcefile.is_some() {
                            // Were we scanning the target file or an extra
                            // file
                            let targetname = DiskFile::split_relative_filename(
                                sourcefile.borrow().targetfilename(),
                                basepath,
                            );

                            sout_line(
                                sout,
                                &format!("Target: \"{name}\" - damaged. Found {count} of {blockcount} data blocks from \"{targetname}\"."),
                            );
                        } else {
                            let targetname = DiskFile::split_relative_filename(
                                sourcefile.borrow().targetfilename(),
                                basepath,
                            );

                            sout_line(
                                sout,
                                &format!("File: \"{name}\" - found {count} of {blockcount} data blocks from \"{targetname}\"."),
                            );
                        }
                    }

                    if skippeddata > 0 {
                        sout_line(
                            sout,
                            &format!("{skippeddata} bytes of data were skipped whilst scanning.\nIf there are not enough blocks found to repair: try again with the -N option."),
                        );
                    }
                }
            } else if self.noiselevel > NoiseLevel::Silent {
                // Did we match the target file
                let same_target = originalsourcefile
                    .as_ref()
                    .is_some_and(|o| Rc::ptr_eq(o, sourcefile));
                if same_target {
                    sout_line(sout, &format!("Target: \"{name}\" - found."));
                } else {
                    let targetname = DiskFile::split_relative_filename(
                        sourcefile.borrow().targetfilename(),
                        basepath,
                    );
                    // Were we scanning the target file or an extra file
                    if originalsourcefile.is_some() {
                        sout_line(
                            sout,
                            &format!("Target: \"{name}\" - is a match for \"{targetname}\"."),
                        );
                    } else {
                        sout_line(
                            sout,
                            &format!("File: \"{name}\" - is a match for \"{targetname}\"."),
                        );
                    }
                }
            }
        } else {
            *matchtype = MatchType::NoMatch;

            if self.noiselevel > NoiseLevel::Silent {
                // We found not data, but did the file actually contain
                // blocks we had already found in other files.
                if duplicatecount > 0 {
                    sout_line(
                        sout,
                        &format!("File: \"{name}\" - found {duplicatecount} duplicate data blocks."),
                    );
                } else {
                    sout_line(sout, &format!("File: \"{name}\" - no data found."));
                }

                if skippeddata > 0 {
                    sout_line(
                        sout,
                        &format!("{skippeddata} bytes of data were skipped whilst scanning.\nIf there are not enough blocks found to repair: try again with the -N option."),
                    );
                }
            }
        }

        true
    }

    // -----------------------------__UpdateVerificationResults__-----------------------

    /// Find out how much data we have found
    ///
    /// C++ Par2Repairer::UpdateVerificationResults
    fn update_verification_results(&mut self) {
        self.availableblockcount = 0;
        self.missingblockcount = 0;

        self.completefilecount = 0;
        self.renamedfilecount = 0;
        self.damagedfilecount = 0;
        self.missingfilecount = 0;

        let totalfilecount = self
            .mainpacket
            .as_ref()
            .expect("update_verification_results requires a main packet")
            .total_file_count() as usize;

        // Check the recoverable files
        for sourcefile in self.sourcefiles.iter().take(totalfilecount) {
            if let Some(sourcefile) = sourcefile {
                let sourcefile = Rc::clone(sourcefile);
                let sourcefile = sourcefile.borrow();

                // Was a perfect match for the file found
                if let Some(completefile) = sourcefile.get_complete_file() {
                    // Is it the target file or a different one
                    match sourcefile.get_target_file() {
                        Some(targetfile) if Rc::ptr_eq(&completefile, &targetfile) => {
                            self.completefilecount += 1;
                        }
                        _ => {
                            self.renamedfilecount += 1;
                        }
                    }

                    self.availableblockcount += sourcefile.block_count();
                } else {
                    // Count the number of blocks that have been found
                    let firstsourceblock = sourcefile.first_source_block();
                    let blockcount = sourcefile.block_count() as usize;
                    for blocknumber in 0..blockcount {
                        if self.sourceblocks[firstsourceblock + blocknumber].is_set() {
                            self.availableblockcount += 1;
                        }
                    }

                    // Does the target file exist
                    if sourcefile.get_target_exists() {
                        self.damagedfilecount += 1;
                    } else {
                        self.missingfilecount += 1;
                    }
                }
            } else {
                self.missingfilecount += 1;
            }
        }

        self.missingblockcount = self.sourceblockcount - self.availableblockcount;
    }

    // -----------------------------__CheckVerificationResults__------------------------

    /// Check the verification results and report the results
    ///
    /// C++ Par2Repairer::CheckVerificationResults
    fn check_verification_results(&mut self, sout: &SharedSout) -> bool {
        let recoverablefilecount = self
            .mainpacket
            .as_ref()
            .expect("check_verification_results requires a main packet")
            .recoverable_file_count();
        let recoverypacketcount = self.recoverypacketmap.len() as u32;

        // Is repair needed
        if self.completefilecount < recoverablefilecount
            || self.renamedfilecount > 0
            || self.damagedfilecount > 0
            || self.missingfilecount > 0
        {
            if self.noiselevel > NoiseLevel::Silent {
                sout_line(sout, "Repair is required.");
            }
            if self.noiselevel > NoiseLevel::Quiet {
                if self.renamedfilecount > 0 {
                    sout_write(
                        sout,
                        &format!("{} file(s) have the wrong name.\n", self.renamedfilecount),
                    );
                }
                if self.missingfilecount > 0 {
                    sout_write(
                        sout,
                        &format!("{} file(s) are missing.\n", self.missingfilecount),
                    );
                }
                if self.damagedfilecount > 0 {
                    sout_write(
                        sout,
                        &format!(
                            "{} file(s) exist but are damaged.\n",
                            self.damagedfilecount
                        ),
                    );
                }
                if self.completefilecount > 0 {
                    sout_write(
                        sout,
                        &format!("{} file(s) are ok.\n", self.completefilecount),
                    );
                }

                sout_line(
                    sout,
                    &format!(
                        "You have {} out of {} data blocks available.",
                        self.availableblockcount, self.sourceblockcount
                    ),
                );
                if recoverypacketcount > 0 {
                    sout_line(
                        sout,
                        &format!("You have {recoverypacketcount} recovery blocks available."),
                    );
                }
            }

            // Is repair possible
            if recoverypacketcount >= self.missingblockcount {
                if self.noiselevel > NoiseLevel::Silent {
                    sout_line(sout, "Repair is possible.");
                }

                if self.noiselevel > NoiseLevel::Quiet {
                    if recoverypacketcount > self.missingblockcount {
                        sout_line(
                            sout,
                            &format!(
                                "You have an excess of {} recovery blocks.",
                                recoverypacketcount - self.missingblockcount
                            ),
                        );
                    }

                    if self.missingblockcount > 0 {
                        sout_line(
                            sout,
                            &format!(
                                "{} recovery blocks will be used to repair.",
                                self.missingblockcount
                            ),
                        );
                    } else if recoverypacketcount > 0 {
                        sout_line(
                            sout,
                            "None of the recovery blocks will be used for the repair.",
                        );
                    }
                }

                true
            } else {
                if self.noiselevel > NoiseLevel::Silent {
                    sout_line(
                        sout,
                        &format!(
                            "Repair is not possible.\nYou need {} more recovery blocks to be able to repair.",
                            self.missingblockcount - recoverypacketcount
                        ),
                    );
                }

                false
            }
        } else {
            if self.noiselevel > NoiseLevel::Silent {
                sout_line(sout, "All files are correct, repair is not required.");
            }

            true
        }
    }

    // -------------------------------__RenameTargetFiles__-----------------------------

    /// Rename any damaged or missnamed target files.
    ///
    /// C++ Par2Repairer::RenameTargetFiles
    fn rename_target_files(&mut self, serr: &mut dyn Write) -> bool {
        let totalfilecount = self
            .mainpacket
            .as_ref()
            .expect("rename_target_files requires a main packet")
            .total_file_count() as usize;

        // Rename any damaged target files. The list is cloned so the map can
        // be modified while the source files are being walked.
        let sourcefiles: Vec<Option<Rc<RefCell<Par2RepairerSourceFile>>>> =
            self.sourcefiles.iter().take(totalfilecount).cloned().collect();
        for sourcefile in sourcefiles.iter().flatten() {
            let sourcefile = Rc::clone(sourcefile);

            let targetfile = sourcefile.borrow().get_target_file();
            let completefile = sourcefile.borrow().get_complete_file();

            // If the target file exists but is not a complete version
            // of the file
            let is_complete = match (&targetfile, &completefile) {
                (Some(targetfile), Some(completefile)) => {
                    Rc::ptr_eq(targetfile, completefile)
                }
                (None, None) => true,
                _ => false,
            };
            if sourcefile.borrow().get_target_exists() && !is_complete {
                let targetfile = targetfile.expect("the target file exists");

                // Rename it
                self.map_remove(&targetfile);

                let renamed = targetfile.borrow_mut().rename_auto(serr);

                self.map_insert(&targetfile);

                if !renamed {
                    return false;
                }

                let index = self
                    .diskfiles
                    .iter()
                    .position(|d| Rc::ptr_eq(d, &targetfile))
                    .expect("disk file not owned by repairer");
                self.backuplist.push(index);

                // We no longer have a target file
                sourcefile.borrow_mut().set_target_exists(false);
                sourcefile.borrow_mut().set_target_file(None);
            }
        }

        // Rename any missnamed but complete versions of the files
        let sourcefiles: Vec<Option<Rc<RefCell<Par2RepairerSourceFile>>>> =
            self.sourcefiles.iter().take(totalfilecount).cloned().collect();
        for sourcefile in sourcefiles.iter().flatten() {
            let sourcefile = Rc::clone(sourcefile);

            let targetfile = sourcefile.borrow().get_target_file();

            // If there is no targetfile and there is a complete version
            let completefile = sourcefile.borrow().get_complete_file();
            if targetfile.is_none() {
                if let Some(targetfile) = completefile {
                    // Rename it
                    self.map_remove(&targetfile);

                    let newname = sourcefile.borrow().targetfilename().to_string();
                    let renamed = targetfile.borrow_mut().rename_to(&newname, serr);

                    self.map_insert(&targetfile);

                    if !renamed {
                        return false;
                    }

                    // This file is now the target file
                    sourcefile.borrow_mut().set_target_exists(true);
                    sourcefile
                        .borrow_mut()
                        .set_target_file(Some(Rc::clone(&targetfile)));

                    // We have one more complete file
                    self.completefilecount += 1;
                }
            }
        }

        true
    }

    // -------------------------------__CreateTargetFiles__-----------------------------

    /// Work out which files are being repaired, create them, and allocate
    /// target DataBlocks to them, and remember them for later verification.
    ///
    /// C++ Par2Repairer::CreateTargetFiles
    fn create_target_files(&mut self, serr: &mut dyn Write) -> bool {
        let totalfilecount = self
            .mainpacket
            .as_ref()
            .expect("create_target_files requires a main packet")
            .total_file_count() as usize;

        // Create any missing target files
        for filenumber in 0..totalfilecount {
            let sourcefile = match &self.sourcefiles[filenumber] {
                Some(sourcefile) => Rc::clone(sourcefile),
                None => continue,
            };

            // If the file does not exist
            if !sourcefile.borrow().get_target_exists() {
                let targetfile = self.add_diskfile();
                let filename = sourcefile.borrow().targetfilename().to_string();
                let filesize = sourcefile
                    .borrow()
                    .descriptionpacket()
                    .expect("create_target_files requires description packets")
                    .borrow()
                    .filesize();

                // Create the target file
                if !targetfile.borrow_mut().create(&filename, filesize, serr) {
                    return false;
                }

                // This file is now the target file
                sourcefile.borrow_mut().set_target_exists(true);
                sourcefile
                    .borrow_mut()
                    .set_target_file(Some(Rc::clone(&targetfile)));

                // Remember this file
                self.map_insert(&targetfile);

                // Allocate all of the target data blocks
                let firsttargetblock = sourcefile.borrow().first_target_block();
                let mut offset = 0u64;
                let mut blockindex = 0usize;
                while offset < filesize {
                    let datablock = &mut self.targetblocks[firsttargetblock + blockindex];
                    datablock.set_location(Rc::clone(&targetfile), offset);
                    datablock.set_length(std::cmp::min(self.blocksize, filesize - offset));

                    offset += self.blocksize;
                    blockindex += 1;
                }

                // Add the file to the list of those that will need to be
                // verified once the repair has completed.
                self.verifylist.push(filenumber);
            }
        }

        true
    }

    // -------------------------------__AllocateBuffers__------------------------------

    /// Allocate memory buffers for reading and writing data to disk.
    ///
    /// C++ Par2Repairer::AllocateBuffers
    fn allocate_buffers(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        memorylimit: usize,
    ) -> bool {
        // Would single pass processing use too much memory
        self.chunksize = if self.blocksize * self.missingblockcount as u64 > memorylimit as u64 {
            // Pick a size that is small enough
            (!(3usize) & (memorylimit / self.missingblockcount as usize)) as u64
        } else {
            self.blocksize
        };

        if MAX_CHUNK_SIZE != 0 && self.chunksize > MAX_CHUNK_SIZE {
            self.chunksize = MAX_CHUNK_SIZE;
        }

        // Allocate the two buffers
        self.transferbuffer = vec![0u8; self.chunksize as usize * NUM_TRANSFER_BUFFERS];
        self.outputbuffer = vec![0u8; self.chunksize as usize];

        let config = ProcessorConfig {
            numthreads: self.totalthreads,
            memorylimit,
        };
        // The processor multiplies with the stateless multiply_add alone, so
        // a snapshot of the matrix, which is only solved afterwards, serves
        // it just as the C++ reference processor's reference to it does.
        let mut processor = self
            .backends
            .build_processor(&config, Arc::new(self.rs.clone()));

        if !processor.init(self.chunksize as usize, self.missingblockcount) {
            writeln!(serr, "Could not allocate buffer memory.")
                .expect("cannot write to serr");
            return false;
        }

        if self.noiselevel >= NoiseLevel::Debug {
            sout_line(
                sout,
                &format!("[DEBUG] Process chunk size: {}", self.chunksize),
            );
        }

        self.processor = Some(processor);

        true
    }

    // -------------------------------__ComputeRSmatrix__------------------------------

    /// Work out which data blocks are available, which need to be copied
    /// directly to the output, and which need to be recreated, and compute
    /// the appropriate Reed Solomon matrix.
    ///
    /// C++ Par2Repairer::ComputeRSmatrix
    fn compute_rs_matrix(&mut self, sout: &SharedSout, serr: &mut dyn Write) -> bool {
        // The DataBlocks that will read from disk
        self.inputblocks = vec![DataBlock::new(); self.sourceblockcount as usize];
        // Those DataBlocks which need to be copied
        self.copyblocks = vec![DataBlock::new(); self.availableblockcount as usize];
        // Those DataBlocks that will be recalculated
        self.outputblocks = vec![DataBlock::new(); self.missingblockcount as usize];

        // Build an array listing which source data blocks are present and
        // which are missing
        let mut present = vec![false; self.sourceblockcount as usize];

        let mut inputblock = 0usize;
        let mut copyblock = 0usize;
        let mut outputblock = 0usize;

        // Iterate through all source blocks for all files
        for (index, was_found) in present.iter_mut().enumerate() {
            // Was this block found
            if self.sourceblocks[index].is_set() {
                // Record that the block was found
                *was_found = true;

                // Add the block to the list of those which will be read
                // as input (and which might also need to be copied).
                self.inputblocks[inputblock] = self.sourceblocks[index].clone();
                self.copyblocks[copyblock] = self.targetblocks[index].clone();

                inputblock += 1;
                copyblock += 1;
            } else {
                // Record that the block was missing
                *was_found = false;

                // Add the block to the list of those to be written
                self.outputblocks[outputblock] = self.targetblocks[index].clone();
                outputblock += 1;
            }
        }

        // Set the number of source blocks and which of them are present
        if let Err(msg) = self.rs.set_input_present(&present) {
            writeln!(serr, "{msg}").expect("cannot write to serr");
            return false;
        }

        // Start iterating through the available recovery packets
        let mut rp = self.recoverypacketmap.iter();

        // The exponents of those recovery blocks, kept for the processor
        let mut exponents: Vec<u16> = Vec::new();

        // Continue to fill the remaining list of data blocks to be read
        while inputblock < self.inputblocks.len() {
            // Get the next available recovery packet
            let (exponent, recoverypacket) = rp
                .next()
                .expect("enough recovery packets for the missing blocks");

            // Get the DataBlock from the recovery packet
            let recoveryblock = recoverypacket.borrow_mut().data_block().clone();

            // Add the recovery block to the list of blocks that will be read
            self.inputblocks[inputblock] = recoveryblock;

            // Record that the corresponding exponent value is the next one
            // to use in the RS matrix
            if !self.rs.set_output(true, *exponent as u16) {
                return false;
            }

            exponents.push(*exponent as u16);

            inputblock += 1;
        }

        // If we need to, compute and solve the RS matrix
        if self.missingblockcount == 0 {
            return true;
        }

        // Offer the erasure pattern, so that an implementation able to solve
        // it for itself is not made to wait for the matrix to be inverted
        // only to read columns out of it
        self.ownfactors = self
            .processor
            .as_mut()
            .expect("compute_rs_matrix requires a processor")
            .offer_erasures(&present, &exponents);

        if self.ownfactors {
            return true;
        }

        if let Err(msg) = self.rs.compute(self.noiselevel, sout) {
            writeln!(serr, "{msg}").expect("cannot write to serr");
            return false;
        }

        true
    }

    // --------------------------------__ProcessData__---------------------------------

    /// Read source data, process it through the RS matrix and write it to
    /// disk.
    ///
    /// C++ Par2Repairer::ProcessData
    fn process_data(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        blockoffset: u64,
        blocklength: usize,
        progress: &ProgressMeter,
    ) -> bool {
        let mut totalwritten: u64 = 0;

        // Index into inputblocks and copyblocks: the source blocks which
        // were found come first, then the recovery blocks in order.
        let mut inputblock = 0usize;
        let mut copyblock = 0usize;
        let mut inputindex = 0u32;

        let mut lastopenfile: Option<Rc<RefCell<DiskFile>>> = None;
        let mut failed = false;

        // Are there any blocks which need to be reconstructed
        if self.missingblockcount > 0 {
            let missingblockcount = self.missingblockcount as usize;
            {
                let processor = self
                    .processor
                    .as_mut()
                    .expect("process_data requires a processor");
                processor.set_chunk_length(blocklength);
                processor.reset_output();
            }

            // The matrix column for one input block, unused when the
            // processor has its own. There is one for each transfer buffer,
            // so that a column lasts as long as the block it belongs to.
            let mut factors: Vec<u16> = if self.ownfactors {
                Vec::new()
            } else {
                vec![0; missingblockcount * NUM_TRANSFER_BUFFERS]
            };

            // Every buffer starts free: the receiver of its last submission
            // while one is still in flight.
            let mut bufferfree: Vec<Option<Receiver<()>>> =
                (0..NUM_TRANSFER_BUFFERS).map(|_| None).collect();
            let mut bufferindex = 0usize;

            // For each input block
            while inputblock < self.inputblocks.len() {
                // Are we reading from a new file?
                {
                    let current = self.inputblocks[inputblock].diskfile();
                    let changed = match (&lastopenfile, &current) {
                        (Some(last), Some(current)) => !Rc::ptr_eq(last, current),
                        (None, Some(_)) => true,
                        _ => false,
                    };
                    if changed {
                        // Close the last file
                        if let Some(lastopenfile) = &lastopenfile {
                            lastopenfile.borrow_mut().close();
                        }

                        // Open the new file
                        lastopenfile = current;
                        if let Some(lastopenfile) = &lastopenfile {
                            let filename = lastopenfile.borrow().file_name().to_string();
                            if !lastopenfile.borrow_mut().open(&filename, serr) {
                                failed = true;
                                break;
                            }
                        }
                    }
                }

                // Wait for the next input buffer to come free. An Err only
                // says the implementation has failed and dropped its end
                // without finishing: the submission below reports that.
                if let Some(receiver) = bufferfree[bufferindex].take() {
                    let _ = receiver.recv();
                }

                let start = self.chunksize as usize * bufferindex;

                // Read data from the current input block
                if !self.inputblocks[inputblock].read_data(
                    blockoffset,
                    &mut self.transferbuffer[start..start + blocklength],
                    serr,
                ) {
                    failed = true;
                    break;
                }

                // Have we reached the last source data block
                if copyblock < self.copyblocks.len() {
                    // Does this block need to be copied to the target file
                    if self.copyblocks[copyblock].is_set() {
                        let mut wrote = 0usize;

                        // Write the block back to disk in the new target
                        // file
                        if !self.copyblocks[copyblock].write_data(
                            blockoffset,
                            &self.transferbuffer[start..start + blocklength],
                            &mut wrote,
                            serr,
                        ) {
                            failed = true;
                            break;
                        }

                        totalwritten += wrote as u64;
                    }
                    copyblock += 1;
                }

                // Look up the matrix column and process the data against
                // every output block
                let columnstart = bufferindex * missingblockcount;

                if !self.ownfactors {
                    for outputindex in 0..missingblockcount {
                        factors[columnstart + outputindex] =
                            self.rs.get_factor(inputindex, outputindex as u32);
                    }
                }

                {
                    let processor = self
                        .processor
                        .as_mut()
                        .expect("process_data requires a processor");

                    processor.wait_for_add();

                    let column: Option<&[u16]> = if self.ownfactors {
                        None
                    } else {
                        Some(&factors[columnstart..columnstart + missingblockcount])
                    };

                    bufferfree[bufferindex] = Some(processor.add_input(
                        &self.transferbuffer[start..start + blocklength],
                        inputindex,
                        column,
                    ));
                }
                bufferindex = (bufferindex + 1) % NUM_TRANSFER_BUFFERS;

                if self.noiselevel > NoiseLevel::Quiet {
                    progress.add(blocklength as u64);
                }

                inputblock += 1;
                inputindex += 1;
            }

            self.processor
                .as_mut()
                .expect("process_data requires a processor")
                .end_input();
        } else {
            // Reconstruction is not required, we are just copying blocks
            // between files

            // For each block that might need to be copied
            while copyblock < self.copyblocks.len() {
                // Does this block need to be copied
                if self.copyblocks[copyblock].is_set() {
                    // Are we reading from a new file?
                    {
                        let current = self.inputblocks[inputblock].diskfile();
                        let changed = match (&lastopenfile, &current) {
                            (Some(last), Some(current)) => !Rc::ptr_eq(last, current),
                            (None, Some(_)) => true,
                            _ => false,
                        };
                        if changed {
                            // Close the last file
                            if let Some(lastopenfile) = &lastopenfile {
                                lastopenfile.borrow_mut().close();
                            }

                            // Open the new file
                            lastopenfile = current;
                            if let Some(lastopenfile) = &lastopenfile {
                                let filename = lastopenfile.borrow().file_name().to_string();
                                if !lastopenfile.borrow_mut().open(&filename, serr) {
                                    return false;
                                }
                            }
                        }
                    }

                    // Read data from the current input block
                    if !self.inputblocks[inputblock]
                        .read_data(blockoffset, &mut self.transferbuffer[..blocklength], serr)
                    {
                        return false;
                    }

                    let mut wrote = 0usize;
                    if !self.copyblocks[copyblock].write_data(
                        blockoffset,
                        &self.transferbuffer[..blocklength],
                        &mut wrote,
                        serr,
                    ) {
                        return false;
                    }
                    totalwritten += wrote as u64;
                }

                if self.noiselevel > NoiseLevel::Quiet {
                    progress.add(blocklength as u64);
                }

                copyblock += 1;
                inputblock += 1;
            }
        }

        // Close the last file
        if let Some(lastopenfile) = &lastopenfile {
            lastopenfile.borrow_mut().close();
        }

        if failed {
            return false;
        }

        if self.noiselevel > NoiseLevel::Quiet {
            sout_write(sout, "Writing recovered data\r");
        }

        // For each output block that has been recomputed
        for outputindex in 0..self.missingblockcount as usize {
            // Take the accumulated output block from the processor
            let processor = self
                .processor
                .as_mut()
                .expect("process_data requires a processor");
            let outbuf: &[u8] = match processor.peek_output(outputindex as u32) {
                Some(buf) => buf,
                None => {
                    if !processor
                        .get_output(outputindex as u32, &mut self.outputbuffer[..blocklength])
                    {
                        writeln!(
                            serr,
                            "Could not read the repaired data back from the processor."
                        )
                        .expect("cannot write to serr");
                        return false;
                    }
                    &self.outputbuffer[..blocklength]
                }
            };

            // Write the data to the target file
            let mut wrote = 0usize;
            if !self.outputblocks[outputindex].write_data(blockoffset, outbuf, &mut wrote, serr) {
                return false;
            }
            totalwritten += wrote as u64;
        }

        if self.noiselevel > NoiseLevel::Quiet {
            sout_line(sout, &format!("Wrote {totalwritten} bytes to disk"));
        }

        true
    }

    // ------------------------------__VerifyTargetFiles__-----------------------------

    /// Verify that all of the reconstructed target files are now correct
    ///
    /// C++ Par2Repairer::VerifyTargetFiles
    fn verify_target_files(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        basepath: &str,
    ) -> bool {
        let mut finalresult = true;

        // Verify the target files in alphabetical order
        let mut sorted: Vec<(String, usize)> = self
            .verifylist
            .iter()
            .map(|&filenumber| {
                let targetfilename = self.sourcefiles[filenumber]
                    .as_ref()
                    .expect("verifylist holds existing source files")
                    .borrow()
                    .targetfilename()
                    .to_string();
                (targetfilename, filenumber)
            })
            .collect();
        sorted.sort_by(|low, high| low.0.cmp(&high.0));
        self.verifylist = sorted.into_iter().map(|(_, filenumber)| filenumber).collect();

        let mut mttotalsize = 0u64;
        for &filenumber in &self.verifylist {
            mttotalsize += self.sourcefiles[filenumber]
                .as_ref()
                .expect("verifylist holds existing source files")
                .borrow()
                .descriptionpacket()
                .expect("verify_target_files requires description packets")
                .borrow()
                .filesize();
        }
        let progress = ProgressMeter::new_shared(Arc::clone(sout), "Scanning: ", mttotalsize);

        // Iterate through each file in the verification list. C++ verifies
        // them with several threads at once; here one file is verified at a
        // time on the calling thread.
        let verifylist = self.verifylist.clone();
        for filenumber in verifylist {
            let sourcefile = Rc::clone(
                self.sourcefiles[filenumber]
                    .as_ref()
                    .expect("verifylist holds existing source files"),
            );

            let targetfile = sourcefile
                .borrow()
                .get_target_file()
                .expect("a file in the verification list has a target file");

            // Close the file
            if targetfile.borrow().is_open() {
                targetfile.borrow_mut().close();
            }

            // Mark all data blocks for the file as unknown
            let firstsourceblock = sourcefile.borrow().first_source_block();
            let blockcount = sourcefile.borrow().block_count() as usize;
            for datablock in &mut self.sourceblocks[firstsourceblock..firstsourceblock + blockcount]
            {
                datablock.clear_location();
            }

            // Say we don't have a complete version of the file
            sourcefile.borrow_mut().set_complete_file(None);

            // Re-open the target file
            let filename = targetfile.borrow().file_name().to_string();
            if !targetfile.borrow_mut().open(&filename, serr) {
                finalresult = false;
                continue;
            }

            // Verify the file again
            if !self.verify_data_file(
                sout,
                serr,
                &targetfile,
                Some(Rc::clone(&sourcefile)),
                basepath,
                &progress,
                false,
            ) {
                finalresult = false;
            }

            // Close the file again
            targetfile.borrow_mut().close();
        }

        // Find out how much data we have found
        self.update_verification_results();

        finalresult
    }

    // --------------------------__DeleteIncompleteTargetFiles__------------------------

    /// Delete all of the partly reconstructed files. C++ returns true, which
    /// no caller looks at.
    ///
    /// C++ Par2Repairer::DeleteIncompleteTargetFiles
    fn delete_incomplete_target_files(&mut self, serr: &mut dyn Write) {
        let verifylist = self.verifylist.clone();

        // Iterate through each file in the verification list
        for filenumber in verifylist {
            let sourcefile = Rc::clone(
                self.sourcefiles[filenumber]
                    .as_ref()
                    .expect("verifylist holds existing source files"),
            );

            if sourcefile.borrow().get_target_exists() {
                let targetfile = sourcefile
                    .borrow()
                    .get_target_file()
                    .expect("a file in the verification list has a target file");

                // Close and delete the file
                if targetfile.borrow().is_open() {
                    targetfile.borrow_mut().close();
                }
                let _ = targetfile.borrow_mut().delete(serr);

                // Forget the file
                self.map_remove(&targetfile);

                // There is no target file
                sourcefile.borrow_mut().set_target_exists(false);
                sourcefile.borrow_mut().set_target_file(None);
            }
        }
    }

    // ------------------------------__RemoveBackupFiles__-----------------------------

    /// C++ Par2Repairer::RemoveBackupFiles
    fn remove_backup_files(&mut self, sout: &SharedSout, serr: &mut dyn Write) {
        if self.noiselevel > NoiseLevel::Silent && !self.backuplist.is_empty() {
            sout_line(sout, "\nPurge backup files.");
        }

        // Iterate through each file in the backuplist
        for &index in &self.backuplist {
            if self.noiselevel > NoiseLevel::Silent {
                let (_, name) = DiskFile::split_filename(self.diskfiles[index].borrow().file_name());
                sout_line(sout, &format!("Remove \"{name}\"."));
            }

            let diskfile = &self.diskfiles[index];
            if diskfile.borrow().is_open() {
                diskfile.borrow_mut().close();
            }
            let _ = diskfile.borrow_mut().delete(serr);
        }
    }

    // -------------------------------__RemoveParFiles__-------------------------------

    /// C++ Par2Repairer::RemoveParFiles
    fn remove_par_files(&mut self, sout: &SharedSout, serr: &mut dyn Write) {
        if self.noiselevel > NoiseLevel::Silent && !self.par2list.is_empty() {
            sout_line(sout, "\nPurge par files.");
        }

        for s in &self.par2list {
            let mut diskfile = DiskFile::new();

            if diskfile.open(s, serr) {
                if self.noiselevel > NoiseLevel::Silent {
                    let (_, name) = DiskFile::split_filename(s);
                    sout_line(sout, &format!("Remove \"{name}\"."));
                }

                if diskfile.is_open() {
                    diskfile.close();
                }
                let _ = diskfile.delete(serr);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc::crc_update_block;
    use crate::par2fileformat;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    fn md5_of(data: &[u8]) -> Md5Hash {
        let mut context = Md5Context::new();
        context.update(data);
        context.finalize()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("par2rust_p2r_{tag}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn shared_sout() -> SharedSout {
        let writer: Box<dyn Write + Send> = Box::new(Vec::<u8>::new());
        Arc::new(Mutex::new(writer))
    }

    // Deterministic file content so the tests are reproducible.
    fn content(len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| {
                let i = i as u32;
                (((i + 1).wrapping_mul(1103515245).wrapping_add(12345)) >> 13) as u8 ^ (i >> 8) as u8
            })
            .collect()
    }

    // The block hash and block CRC conventions of the verification
    // packets: the MD5 of the block (zero padded to the block size for a
    // short block) and the complemented CRC32 of the same bytes.
    fn block_hash(data: &[u8]) -> Md5Hash {
        md5_of(data)
    }

    fn block_crc(data: &[u8]) -> u32 {
        !0u32 ^ crc_update_block(!0, data)
    }

    // One source file of the recovery set.
    struct SourceFile {
        name: String,
        data: Vec<u8>,
    }

    // A hand built PAR 2.0 recovery set: the packets a creator would
    // write, assembled in memory with correct hashes.
    struct Par2Set {
        blocksize: u64,
        exponents: Vec<u32>,
        files: Vec<SourceFile>,
    }

    // The description and verification packets of one source file.
    struct FilePackets {
        fileid: Md5Hash,
        description: DescriptionPacket,
        verification: VerificationPacket,
    }

    impl Par2Set {
        fn new(blocksize: u64, exponents: &[u32]) -> Par2Set {
            Par2Set {
                blocksize,
                exponents: exponents.to_vec(),
                files: Vec::new(),
            }
        }

        fn add_file(&mut self, name: &str, len: usize) {
            self.files.push(SourceFile {
                name: name.to_string(),
                data: content(len),
            });
        }

        // Write the source files into the directory.
        fn write_files(&self, dir: &Path) {
            for file in &self.files {
                std::fs::write(dir.join(&file.name), &file.data).unwrap();
            }
        }

        // The zero padded blocks of every file, in fileid order.
        fn source_blocks(&self) -> Vec<Vec<u8>> {
            let mut order: Vec<&SourceFile> = self.files.iter().collect();
            order.sort_by(|a, b| {
                md5_of(&file_id_input(a)).cmp(&md5_of(&file_id_input(b)))
            });
            let mut blocks = Vec::new();
            for file in order {
                for chunk in file.data.chunks(self.blocksize as usize) {
                    let mut block = chunk.to_vec();
                    block.resize(self.blocksize as usize, 0);
                    blocks.push(block);
                }
            }
            blocks
        }

        // Build the contents of the PAR 2.0 file.
        fn build(&self) -> Vec<u8> {
            // The description packets fix the file ids; the verification
            // packets hold the block hashes and CRCs.
            let mut infos: Vec<FilePackets> = Vec::new();
            for file in &self.files {
                let hashfull = md5_of(&file.data);
                let hash16k = md5_of(if file.data.len() > 16384 {
                    &file.data[..16384]
                } else {
                    &file.data
                });

                let mut description = DescriptionPacket::new();
                assert!(description.create(file.name.as_bytes(), file.data.len() as u64));
                description.hash16k(&hash16k);
                description.hashfull(&hashfull);
                description.compute_fileid();
                let fileid = description.fileid();

                let blockcount = (file.data.len() as u64).div_ceil(self.blocksize);
                let mut verification = VerificationPacket::new();
                assert!(verification.create(blockcount as u32));
                verification.set_fileid(&fileid);
                for (blocknumber, chunk) in
                    file.data.chunks(self.blocksize as usize).enumerate()
                {
                    let mut block = chunk.to_vec();
                    block.resize(self.blocksize as usize, 0);
                    verification.set_block_hash_and_crc(
                        blocknumber as u32,
                        &block_hash(&block),
                        block_crc(&block),
                    );
                }

                infos.push(FilePackets {
                    fileid,
                    description,
                    verification,
                });
            }
            infos.sort_by_key(|info| info.fileid);

            // The main packet fixes the set id.
            let mut fileids: Vec<Md5Hash> =
                infos.iter().map(|info| info.fileid).collect();
            let mut main = MainPacket::new();
            assert!(main.create(&mut fileids, self.blocksize));
            let setid = par2fileformat::packet_setid(main.critical().data());

            // Finish the description and verification packets.
            for info in infos.iter_mut() {
                info.description.critical_mut().finish_packet(&setid);
                info.verification.critical_mut().finish_packet(&setid);
            }

            // The recovery data: the creator's RS matrix with all the
            // source blocks present.
            let blocks = self.source_blocks();
            let mut rs = ReedSolomon::<Spec16>::new();
            rs.set_input_all(blocks.len() as u32).unwrap();
            for &exponent in &self.exponents {
                // The creator asks for the recovery blocks to be created,
                // so they are marked as missing from the matrix.
                assert!(rs.set_output(false, exponent as u16));
            }
            rs.compute(NoiseLevel::Quiet, &shared_sout()).unwrap();

            let mut recovery: Vec<Vec<u8>> = Vec::new();
            for output_index in 0..self.exponents.len() as u32 {
                let mut out = vec![0u8; self.blocksize as usize];
                for (input_index, block) in blocks.iter().enumerate() {
                    assert!(rs.process(
                        self.blocksize as usize,
                        input_index as u32,
                        block,
                        output_index,
                        &mut out,
                    ));
                }
                recovery.push(out);
            }

            // Assemble the file: the main packet, the descriptions and
            // verifications in fileid order, the creator packet, then the
            // recovery packets with their data following the header.
            let mut file = Vec::new();
            file.extend_from_slice(main.critical().data());
            for info in &infos {
                let description = info.description.critical();
                file.extend_from_slice(&description.data()[..description.packet_length()]);
                let verification = info.verification.critical();
                file.extend_from_slice(&verification.data()[..verification.packet_length()]);
            }
            let mut creator = CreatorPacket::new();
            assert!(creator.create(&setid));
            file.extend_from_slice(creator.critical().data());

            for (&exponent, data) in self.exponents.iter().zip(recovery.iter()) {
                let mut packet = vec![0u8; RECOVERYBLOCKPACKET_SIZE];
                packet[HDR_MAGIC_OFFSET..HDR_MAGIC_OFFSET + 8]
                    .copy_from_slice(&PACKET_MAGIC);
                par2fileformat::set_packet_length(
                    &mut packet,
                    RECOVERYBLOCKPACKET_SIZE as u64 + self.blocksize,
                );
                par2fileformat::set_packet_setid(&mut packet, &setid);
                packet[HDR_TYPE_OFFSET..HDR_TYPE_OFFSET + 16]
                    .copy_from_slice(&RECOVERYBLOCKPACKET_TYPE);
                packet[RECOVERYBLOCKPACKET_EXPONENT_OFFSET
                    ..RECOVERYBLOCKPACKET_EXPONENT_OFFSET + 4]
                    .copy_from_slice(&exponent.to_le_bytes());

                // The hash covers the packet body and the recovery data.
                let mut context = Md5Context::new();
                context.update(&packet[HDR_SETID_OFFSET..RECOVERYBLOCKPACKET_SIZE]);
                context.update(data);
                let hash = context.finalize();
                par2fileformat::set_packet_hash(&mut packet, &hash);

                file.extend_from_slice(&packet);
                file.extend_from_slice(data);
            }

            file
        }
    }

    // The bytes the file id is computed from: hash16k, length, name.
    fn file_id_input(file: &SourceFile) -> Vec<u8> {
        let hash16k = md5_of(if file.data.len() > 16384 {
            &file.data[..16384]
        } else {
            &file.data
        });
        let mut input = Vec::new();
        input.extend_from_slice(&hash16k.hash);
        input.extend_from_slice(&(file.data.len() as u64).to_le_bytes());
        input.extend_from_slice(file.name.as_bytes());
        input
    }

    // Run the repairer over the set in the directory.
    fn run(
        dir: &Path,
        dorepair: bool,
        purgefiles: bool,
        extrafiles: &[String],
    ) -> (Result, Par2Repairer) {
        let parfile = dir.join("test.par2");
        let mut basepath = dir.to_str().unwrap().to_string();
        basepath.push(std::path::MAIN_SEPARATOR);

        let mut repairer = Par2Repairer::new(NoiseLevel::Silent, Backends::default());
        let sout = shared_sout();
        let mut serr = Vec::new();
        let result = repairer.process(
            &sout,
            &mut serr,
            16 * 1024 * 1024,
            &basepath,
            1,
            1,
            parfile.to_str().unwrap().to_string(),
            extrafiles,
            dorepair,
            purgefiles,
            false,
            false,
            0,
            false,
        );
        (result, repairer)
    }

    #[test]
    fn all_files_correct() {
        let dir = temp_dir("ok");
        let mut set = Par2Set::new(1024, &[1, 2, 3]);
        set.add_file("alpha.bin", 3000);
        set.add_file("beta.bin", 1500);
        set.write_files(&dir);
        std::fs::write(dir.join("test.par2"), set.build()).unwrap();

        let (result, repairer) = run(&dir, true, false, &[]);
        assert_eq!(result, Result::Success);
        assert_eq!(repairer.blocksize, 1024);
        assert_eq!(repairer.sourceblockcount, 5);
        assert_eq!(repairer.availableblockcount, 5);
        assert_eq!(repairer.missingblockcount, 0);
        assert_eq!(repairer.completefilecount, 2);
        assert_eq!(repairer.renamedfilecount, 0);
        assert_eq!(repairer.damagedfilecount, 0);
        assert_eq!(repairer.missingfilecount, 0);
    }

    #[test]
    fn repair_missing_file() {
        let dir = temp_dir("missing");
        let mut set = Par2Set::new(1024, &[1, 2, 3]);
        set.add_file("alpha.bin", 3000);
        set.add_file("beta.bin", 1500);
        set.write_files(&dir);
        std::fs::write(dir.join("test.par2"), set.build()).unwrap();
        let beta = set.files[1].data.clone();
        std::fs::remove_file(dir.join("beta.bin")).unwrap();

        let (result, repairer) = run(&dir, true, false, &[]);
        assert_eq!(result, Result::Success);
        assert_eq!(std::fs::read(dir.join("beta.bin")).unwrap(), beta);
        // After the repair the verification results are recomputed, so
        // every file counts as complete again.
        assert_eq!(repairer.missingfilecount, 0);
        assert_eq!(repairer.completefilecount, 2);
        assert_eq!(repairer.missingblockcount, 0);
    }

    #[test]
    fn repair_damaged_file() {
        let dir = temp_dir("damaged");
        let mut set = Par2Set::new(1024, &[1, 2, 3]);
        set.add_file("alpha.bin", 3000);
        set.add_file("beta.bin", 1500);
        set.write_files(&dir);
        std::fs::write(dir.join("test.par2"), set.build()).unwrap();
        let alpha = set.files[0].data.clone();

        // Damage one byte inside the file's second block.
        let mut damaged = alpha.clone();
        damaged[1500] ^= 0xff;
        std::fs::write(dir.join("alpha.bin"), &damaged).unwrap();

        let (result, repairer) = run(&dir, true, false, &[]);
        assert_eq!(result, Result::Success);
        assert_eq!(std::fs::read(dir.join("alpha.bin")).unwrap(), alpha);
        // The counters are recomputed after the repair.
        assert_eq!(repairer.damagedfilecount, 0);
        assert_eq!(repairer.completefilecount, 2);
        assert_eq!(repairer.missingblockcount, 0);
    }

    #[test]
    fn repair_renamed_file() {
        let dir = temp_dir("renamed");
        let mut set = Par2Set::new(1024, &[1, 2, 3]);
        set.add_file("alpha.bin", 3000);
        set.add_file("beta.bin", 1500);
        set.write_files(&dir);
        std::fs::write(dir.join("test.par2"), set.build()).unwrap();

        // One file is complete but stored under the wrong name.
        std::fs::rename(dir.join("beta.bin"), dir.join("gamma.dat")).unwrap();
        let extra = vec![dir.join("gamma.dat").to_str().unwrap().to_string()];

        let (result, repairer) = run(&dir, true, false, &extra);
        assert_eq!(result, Result::Success);
        assert!(dir.join("beta.bin").exists());
        assert!(!dir.join("gamma.dat").exists());
        assert_eq!(repairer.renamedfilecount, 1);
    }

    #[test]
    fn repair_not_possible() {
        let dir = temp_dir("notenough");
        let mut set = Par2Set::new(1024, &[1, 2]);
        set.add_file("alpha.bin", 3000);
        set.add_file("beta.bin", 1500);
        set.write_files(&dir);
        std::fs::write(dir.join("test.par2"), set.build()).unwrap();
        std::fs::remove_file(dir.join("alpha.bin")).unwrap();
        std::fs::remove_file(dir.join("beta.bin")).unwrap();

        // Five source blocks are missing and only two recovery blocks
        // are available.
        let (result, repairer) = run(&dir, true, false, &[]);
        assert_eq!(result, Result::RepairNotPossible);
        assert_eq!(repairer.missingblockcount, 5);
    }

    #[test]
    fn missing_without_repair() {
        let dir = temp_dir("norepair");
        let mut set = Par2Set::new(1024, &[1, 2, 3]);
        set.add_file("alpha.bin", 3000);
        set.add_file("beta.bin", 1500);
        set.write_files(&dir);
        std::fs::write(dir.join("test.par2"), set.build()).unwrap();
        std::fs::remove_file(dir.join("beta.bin")).unwrap();

        let (result, _repairer) = run(&dir, false, false, &[]);
        assert_eq!(result, Result::RepairPossible);
        assert!(!dir.join("beta.bin").exists());
    }

    #[test]
    fn purge_removes_par_files() {
        let dir = temp_dir("purge");
        let mut set = Par2Set::new(1024, &[1, 2, 3]);
        set.add_file("alpha.bin", 3000);
        set.add_file("beta.bin", 1500);
        set.write_files(&dir);
        std::fs::write(dir.join("test.par2"), set.build()).unwrap();

        let (result, _repairer) = run(&dir, true, true, &[]);
        assert_eq!(result, Result::Success);
        assert!(!dir.join("test.par2").exists());
    }
}
