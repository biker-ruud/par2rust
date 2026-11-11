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

//! Port of par1repairer.h / par1repairer.cpp.
//!
//! The `Par1Repairer` object controls the repair of files using the older
//! PAR 1.0 ("SmartPar") recovery files.  Each PAR 1.0 file holds a copy of
//! the file list (with the full and 16k hashes of every file in the set)
//! plus, when its volume number is non zero, one block of Reed Solomon
//! recovery data the size of the largest file in the set.
//!
//! Ownership notes for this port:
//! - The C++ class owned raw `DiskFile*`, `DataBlock*` and
//!   `Par1RepairerSourceFile*` pointers.  Here every `DiskFile` lives in
//!   [`Par1Repairer::diskfiles`] (shared through `Rc<RefCell<..>>` because
//!   the source files' data blocks and the recovery blocks all point at
//!   them), the recovery `DataBlock`s live in
//!   [`Par1Repairer::recoveryblocks`], and the source files live in
//!   [`Par1Repairer::sourcefiles`] / [`Par1Repairer::extrafiles`].
//! - `verifylist` holds indices into `sourcefiles` and `backuplist` holds
//!   the renamed backup `DiskFile`s; the C++ destructor's clean up is
//!   handled by Rust's ownership system.
//! - The C++ `sout` member stream is shared with the `ProgressMeter`
//!   objects the repairer creates, so it is passed around as a shared
//!   stream ([`SharedSout`]) while `serr` stays an explicit `&mut dyn
//!   Write` like in the rest of the crate.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::datablock::DataBlock;
use crate::diskfile::{DiskFile, DiskFileMap};
use crate::galois::Spec8;
use crate::libpar2::Result;
use crate::md5::Md5Context;
use crate::noiselevel::NoiseLevel;
use crate::par1fileformat::*;
use crate::par1repairersourcefile::Par1RepairerSourceFile;
use crate::progressmeter::ProgressMeter;
use crate::reedsolomon::ReedSolomon;

/// A command stream shared between the repairer and its progress meters.
pub type SharedSout = Arc<Mutex<Box<dyn Write + Send>>>;

/// Write one line to the shared command stream (the C++ `sout << ... <<
/// std::endl`).
pub(crate) fn sout_line(sout: &SharedSout, msg: &str) {
    if let Ok(mut sout) = sout.lock() {
        let _ = writeln!(sout, "{msg}");
    }
}

/// Write raw text to the shared command stream (the C++ `sout << ...`).
pub(crate) fn sout_write(sout: &SharedSout, msg: &str) {
    if let Ok(mut sout) = sout.lock() {
        let _ = write!(sout, "{msg}");
    }
}

/// The version number of SmartPar 1.1 par files (the C++ `static u32
/// smartpar11`).
const SMARTPAR11: u32 = 0x03000101;

/// C++ Par1Repairer
pub struct Par1Repairer {
    noiselevel: NoiseLevel,

    searchpath: String, // Where to look for the input files

    diskfiles: Vec<Rc<RefCell<DiskFile>>>, // All of the DiskFiles created by the repairer
    diskfilemap: DiskFileMap,              // Track which disk files are in use

    recoveryblocks: BTreeMap<u32, DataBlock>, // The recovery data (mapped by exponent)

    filelist: Option<Vec<u8>>, // The file list from the PAR file
    filelistsize: u32,         // The size of the file list

    blocksize: u64, // The size of recovery and data blocks
    chunksize: u64, // How much of a block can be processed.

    sourcefiles: Vec<Par1RepairerSourceFile>, // Files described in the PAR file
    extrafiles: Vec<Par1RepairerSourceFile>,  // Files described on the command line

    completefilecount: u32,
    renamedfilecount: u32,
    damagedfilecount: u32,
    missingfilecount: u32,

    verifylist: Vec<usize>,                    // Which files will be verified
    backuplist: Vec<Rc<RefCell<DiskFile>>>,    // Which files will be renamed
    parlist: Vec<String>,                      // Which PAR files will be renamed

    inputblocks: Vec<DataBlock>,  // Which DataBlocks will be read from disk
    outputblocks: Vec<DataBlock>, // Which DataBlocks have to calculated using RS

    rs: ReedSolomon<Spec8>, // The Reed Solomon matrix.

    inputbuffersize: usize,
    inputbuffer: Vec<u8>, // Buffer for reading DataBlocks (chunksize)

    outputbufferalignment: usize,
    outputbuffersize: usize,
    outputbuffer: Vec<u8>, // Buffer for writing DataBlocks (chunksize * missingblockcount)

    ignore16kfilehash: bool, // Ignore the 16k file hash (due to SmartPar 1.1)
}

impl Par1Repairer {
    pub fn new(noiselevel: NoiseLevel) -> Par1Repairer {
        Par1Repairer {
            noiselevel,

            searchpath: String::new(),

            diskfiles: Vec::new(),
            diskfilemap: DiskFileMap::new(),

            recoveryblocks: BTreeMap::new(),

            filelist: None,
            filelistsize: 0,

            blocksize: 0,
            chunksize: 0,

            sourcefiles: Vec::new(),
            extrafiles: Vec::new(),

            completefilecount: 0,
            renamedfilecount: 0,
            damagedfilecount: 0,
            missingfilecount: 0,

            verifylist: Vec::new(),
            backuplist: Vec::new(),
            parlist: Vec::new(),

            inputblocks: Vec::new(),
            outputblocks: Vec::new(),

            rs: ReedSolomon::new(),

            inputbuffersize: 0,
            inputbuffer: Vec::new(),

            outputbufferalignment: 0,
            outputbuffersize: 0,
            outputbuffer: Vec::new(),

            ignore16kfilehash: false,
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
        let index = self
            .diskfiles
            .iter()
            .position(|d| Rc::ptr_eq(d, diskfile))
            .expect("disk file not owned by repairer");
        let filename = diskfile.borrow().file_name().to_string();
        let success = self.diskfilemap.insert(&filename, index);
        debug_assert!(success);
    }

    // C++ `diskfilemap.Remove(diskfile)`.
    fn map_remove(&mut self, diskfile: &Rc<RefCell<DiskFile>>) {
        let filename = diskfile.borrow().file_name().to_string();
        self.diskfilemap.remove(&filename);
    }

    // --------------------------------__Process__-------------------------------------

    /// C++ Par1Repairer::Process
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        memorylimit: usize,
        _nthreads: u32,         // nthreads is not used by Par1
        parfilename: String,    // basepath is not used by Par1
        extrafiles: &[String],  // filethreads is not used by Par1
        dorepair: bool,         // derived from operation
        purgefiles: bool,       // skipdata/skipleaway are not used by Par1
    ) -> Result {
        // Determine the searchpath from the location of the main PAR file
        let (path, name) = DiskFile::split_filename(&parfilename);
        self.searchpath = path;

        // Load the main PAR file
        if !self.load_recovery_file(sout, serr, self.searchpath.clone() + &name) {
            return Result::LogicError;
        }

        // Load other PAR files related to the main PAR file
        if !self.load_other_recovery_files(sout, serr, parfilename) {
            return Result::LogicError;
        }

        // Load any extra PAR files specified on the command line
        if !self.load_extra_recovery_files(sout, serr, extrafiles) {
            return Result::LogicError;
        }

        if self.noiselevel > NoiseLevel::Quiet {
            sout_line(sout, "\nVerifying source files:\n");
        }

        // Check for the existence of and verify each of the source files
        if !self.verify_source_files(sout, serr) {
            return Result::FileIOError;
        }

        if self.completefilecount < self.sourcefiles.len() as u32 {
            if self.noiselevel > NoiseLevel::Quiet {
                sout_line(sout, "\nScanning extra files:\n");
            }

            // Check any other files specified on the command line to see if they are
            // actually copies of the source files that have the wrong filename
            if !self.verify_extra_files(sout, serr, extrafiles) {
                return Result::LogicError;
            }
        }

        // Find out how much data we have found
        self.update_verification_results();

        if self.noiselevel > NoiseLevel::Silent {
            sout_line(sout, "");
        }

        // Check the verification results and report the details
        if !self.check_verification_results(sout) {
            return Result::RepairNotPossible;
        }

        // Are any of the files incomplete
        if self.completefilecount < self.sourcefiles.len() as u32 {
            // Do we want to carry out a repair
            if dorepair {
                if self.noiselevel > NoiseLevel::Silent {
                    sout_line(sout, "");
                }

                // Rename any damaged or missnamed target files.
                if !self.rename_target_files(serr) {
                    return Result::FileIOError;
                }

                // Are we still missing any files
                if self.completefilecount < self.sourcefiles.len() as u32 {
                    // Work out which files are being repaired, create them, and allocate
                    // target DataBlocks to them, and remember them for later verification.
                    if !self.create_target_files(serr) {
                        return Result::FileIOError;
                    }

                    // Work out which data blocks are available, which need to be recreated,
                    // and compute the appropriate Reed Solomon matrix.
                    if !self.compute_rs_matrix(sout, serr) {
                        // Delete all of the partly reconstructed files
                        self.delete_incomplete_target_files(serr);
                        return Result::FileIOError;
                    }

                    // Allocate memory buffers for reading and writing data to disk.
                    if !self.allocate_buffers(memorylimit) {
                        // Delete all of the partly reconstructed files
                        self.delete_incomplete_target_files(serr);
                        return Result::MemoryError;
                    }
                    if self.noiselevel > NoiseLevel::Silent {
                        sout_line(sout, "");
                    }

                    // Set the total amount of data to be processed.
                    let progress = ProgressMeter::new_shared(
                        sout.clone(),
                        "Repairing: ",
                        self.blocksize
                            * self.sourcefiles.len() as u64
                            * self.verifylist.len() as u64,
                    );

                    // Start at an offset of 0 within a block.
                    let mut blockoffset = 0u64;
                    while blockoffset < self.blocksize // Continue until the end of the block.
                    {
                        // Work out how much data to process this time.
                        let blocklength = std::cmp::min(
                            self.chunksize,
                            self.blocksize - blockoffset,
                        ) as usize;

                        // Read source data, process it through the RS matrix and write it to disk.
                        if !self.process_data(blockoffset, blocklength, &progress, sout, serr)
                        {
                            // Delete all of the partly reconstructed files
                            self.delete_incomplete_target_files(serr);
                            return Result::FileIOError;
                        }

                        // Advance to the need offset within each block
                        blockoffset += blocklength as u64;
                    }

                    if self.noiselevel > NoiseLevel::Silent {
                        sout_line(sout, "\nVerifying repaired files:\n");
                    }

                    // Verify that all of the reconstructed target files are now correct
                    if !self.verify_target_files(sout, serr) {
                        // Delete all of the partly reconstructed files
                        self.delete_incomplete_target_files(serr);
                        return Result::FileIOError;
                    }
                }

                // Are all of the target files now complete?
                if self.completefilecount < self.sourcefiles.len() as u32 {
                    let _ = writeln!(serr, "Repair Failed.");
                    return Result::RepairFailed;
                } else {
                    if self.noiselevel > NoiseLevel::Silent {
                        sout_line(sout, "\nRepair complete.");
                    }
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

    // --------------------------------__Process__-------------------------------------

    /// Test whether filename has a .PAR / .par / .pNN extension.
    ///
    /// C++ Par1Repairer::IsParFilename
    pub fn is_par_filename(filename: &str) -> bool {
        let where_ = match filename.rfind('.') {
            Some(w) => w,
            None => return false,
        };

        let tail = &filename[where_ + 1..];
        if tail.len() < 3 {
            return false;
        }

        let tail = tail.as_bytes();
        if tail[0] != b'P' && tail[0] != b'p' {
            return false;
        }

        ((tail[1] == b'A' || tail[1] == b'a') && (tail[2] == b'R' || tail[2] == b'r'))
            || (tail[1].is_ascii_digit() && tail[2].is_ascii_digit())
    }
    // --------------------------------__LoadRecoveryFile__-------------------------------------

    /// C++ Par1Repairer::LoadRecoveryFile
    fn load_recovery_file(
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

        self.parlist.push(filename.clone());

        let mut havevolume = false;
        let mut volumenumber = 0u32;

        // How big is the file
        let filesize = diskfile.borrow().file_size();
        if filesize >= PAR1FILEHEADER_SIZE as u64 {
            // Allocate a buffer to read data into
            let buffersize = std::cmp::min(1048576u64, filesize) as usize;
            let mut buffer = vec![0u8; buffersize];

            // The C++ `do { ... } while (false)` becomes a labelled block.
            'load: {
                let mut fileheader = [0u8; PAR1FILEHEADER_SIZE];
                if !diskfile.borrow_mut().read(0, &mut fileheader, serr) {
                    break 'load;
                }

                // Is this really a PAR file?
                if fileheader_magic(&fileheader) != PAR1_MAGIC {
                    break 'load;
                }

                // Is the version number correct?
                if fileheader_fileversion(&fileheader) != 0x00010000 {
                    break 'load;
                }

                self.ignore16kfilehash =
                    fileheader_programversion(&fileheader) == SMARTPAR11;

                // Prepare to carry out MD5 Hash check of the Control Hash
                let mut context = Md5Context::new();
                let mut offset = PAR1FILEHEADER_SETHASH_OFFSET as u64;

                // Process until the end of the file is reached
                while offset < filesize {
                    // How much data should we read?
                    let want = std::cmp::min(buffersize as u64, filesize - offset) as usize;
                    if !diskfile.borrow_mut().read(offset, &mut buffer[..want], serr) {
                        break 'load;
                    }

                    context.update(&buffer[..want]);

                    offset += want as u64;
                }

                // Did we read the whole file
                if offset < filesize {
                    break 'load;
                }

                // Compute the hash value
                let hash = context.finalize();

                // Is it correct?
                if hash != fileheader_controlhash(&fileheader) {
                    break 'load;
                }

                // Check that the volume number is ok
                let header_volumenumber = fileheader_volumenumber(&fileheader);
                if header_volumenumber >= 256 {
                    break 'load;
                }

                let numberoffiles = fileheader_numberoffiles(&fileheader);
                let filelistoffset = fileheader_filelistoffset(&fileheader);
                let filelistsize = fileheader_filelistsize(&fileheader);
                let dataoffset = fileheader_dataoffset(&fileheader);
                let datasize = fileheader_datasize(&fileheader);

                // Are there any files?
                if numberoffiles == 0
                    || filelistoffset < PAR1FILEHEADER_SIZE as u64
                    || filelistsize == 0
                {
                    break 'load;
                }

                // Verify that the file list and data offsets are ok
                if (filelistoffset + filelistsize > filesize)
                    || (datasize != 0
                        && (dataoffset < PAR1FILEHEADER_SIZE as u64
                            || dataoffset + datasize > filesize))
                    || (datasize != 0
                        && ((filelistoffset <= dataoffset
                            && dataoffset < filelistoffset + filelistsize)
                            || (dataoffset <= filelistoffset
                                && filelistoffset < dataoffset + datasize)))
                {
                    break 'load;
                }

                // Check the size of the file list
                if filelistsize > 200000 {
                    break 'load;
                }

                // If we already have a copy of the file list, make sure this
                // one has the same size
                if self.filelist.is_some() && self.filelistsize != filelistsize as u32 {
                    break 'load;
                }

                // Read the file list into the buffer
                let mut temp = vec![0u8; filelistsize as usize];
                if !diskfile.borrow_mut().read(filelistoffset, &mut temp, serr) {
                    break 'load;
                }

                // If we already have a copy of the file list, make sure this
                // copy is identical
                if let Some(filelist) = &self.filelist {
                    if filelist != &temp {
                        break 'load;
                    }
                } else {
                    // Prepare to scan the file list
                    let mut remaining = filelistsize as usize;
                    let mut current = 0usize;
                    let mut fileindex = 0u64;
                    let searchpath = self.searchpath.clone();

                    // Process until we run out of files or data
                    while remaining > 0 && fileindex < numberoffiles {
                        // Check that there is enough data remaining to read
                        // the fixed portion
                        if remaining < PAR1FILEENTRY_SIZE {
                            break;
                        }

                        let entrysize = fileentry_entrysize(&temp[current..]) as usize;

                        // Is the entry's declared size consistent with the
                        // remaining data
                        if remaining < entrysize {
                            break;
                        }

                        // Check the length of the filename
                        if entrysize <= PAR1FILEENTRY_SIZE {
                            break;
                        }

                        let entry = &temp[current..current + entrysize];

                        // Check the file size
                        if self.blocksize < fileentry_filesize(entry) {
                            self.blocksize = fileentry_filesize(entry);
                        }

                        // Create source file and add it to the appropriate list
                        let sourcefile = Par1RepairerSourceFile::new(
                            serr,
                            self.noiselevel,
                            entry,
                            &searchpath,
                        );
                        if fileentry_status(entry) & FILEENTRYSTATUS_INPARITYVOLUME != 0 {
                            self.sourcefiles.push(sourcefile);
                        } else {
                            self.extrafiles.push(sourcefile);
                        }

                        remaining -= entrysize;
                        current += entrysize;

                        fileindex += 1;
                    }

                    // Did we find the correct number of files
                    if fileindex < numberoffiles {
                        self.sourcefiles.clear();
                        self.extrafiles.clear();
                        break 'load;
                    }

                    self.filelistsize = filelistsize as u32;
                    self.filelist = Some(temp);
                }

                // Is this a recovery volume?
                if header_volumenumber > 0 {
                    // Make sure there is data and that it is the correct size
                    if dataoffset == 0 || datasize != self.blocksize {
                        break 'load;
                    }

                    // What volume number is this?
                    volumenumber = (header_volumenumber - 1) as u32;

                    // Do we already have this volume?
                    if !self.recoveryblocks.contains_key(&volumenumber) {
                        // Create a data block
                        let mut datablock = DataBlock::new();
                        datablock.set_length(self.blocksize);
                        datablock.set_location(diskfile.clone(), dataoffset);

                        // Store it in the map
                        self.recoveryblocks.insert(volumenumber, datablock);

                        havevolume = true;
                    }
                }
            }
        }

        // We have finished with the file for now
        diskfile.borrow_mut().close();

        if self.noiselevel > NoiseLevel::Quiet {
            if havevolume {
                sout_line(sout, &format!("Loaded recovery volume {volumenumber}"));
            } else {
                sout_line(sout, "No new recovery volumes found");
            }
        }

        // Remember that the file was processed
        self.map_insert(&diskfile);

        true
    }

    // --------------------------------__LoadRecoveryFile__-------------------------------------

    // --------------------------------__LoadOtherRecoveryFiles__-------------------------------------

    /// C++ Par1Repairer::LoadOtherRecoveryFiles
    fn load_other_recovery_files(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        filename: String,
    ) -> bool {
        // Split the original PAR filename into path and name parts
        let (path, name) = DiskFile::split_filename(&filename);
        let mut name = name;

        // Find the file extension
        if let Some(where_) = name.rfind('.') {
            // remove it
            name = name[..where_].to_string();
        }

        // Search for additional PAR files
        let wildcard = name + ".???";
        let files = DiskFile::find_files(&path, &wildcard, false);

        for filename in files {
            // Find the file extension
            if filename.rfind('.').is_some() && Self::is_par_filename(&filename) {
                self.load_recovery_file(sout, serr, filename);
            }
        }

        true
    }

    /// Load packets from any other PAR files whose names are given on the
    /// command line.
    ///
    /// C++ Par1Repairer::LoadExtraRecoveryFiles
    fn load_extra_recovery_files(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        extrafiles: &[String],
    ) -> bool {
        for filename in extrafiles {
            // Find the file extension
            if filename.rfind('.').is_some() && Self::is_par_filename(filename) {
                self.load_recovery_file(sout, serr, filename.clone());
            }
        }

        true
    }
    // --------------------------------__VerifyDataFile__-------------------------------------

    /// Attempt to verify all of the source files.
    ///
    /// C++ Par1Repairer::VerifySourceFiles
    fn verify_source_files(&mut self, sout: &SharedSout, serr: &mut dyn Write) -> bool {
        let mut finalresult = true;

        for index in 0..self.sourcefiles.len() {
            let filename = self.sourcefiles[index].filename().to_string();

            // Check to see if we have already used this file
            if self.diskfilemap.find(&filename).is_some() {
                let name = DiskFile::split_relative_filename(&filename, "");

                // The file has already been used!
                let _ = writeln!(serr, "Source file {name} is a duplicate.");

                finalresult = false;
            } else {
                let diskfile = self.add_diskfile();

                // Does the target file exist
                if diskfile.borrow_mut().open(&filename, serr) {
                    // Yes. Record that fact.
                    self.sourcefiles[index].set_target_exists(true);

                    // Remember that the DiskFile is the target file
                    self.sourcefiles[index].set_target_file(Some(diskfile.clone()));

                    // Remember that we have processed this file
                    self.map_insert(&diskfile);

                    // Do the actual verification
                    if !self.verify_data_file(sout, serr, &diskfile, Some(index)) {
                        finalresult = false;
                    }

                    // We have finished with the file for now
                    diskfile.borrow_mut().close();

                    // Find out how much data we have found
                    self.update_verification_results();
                } else if self.noiselevel > NoiseLevel::Silent {
                    // The file does not exist.
                    let (_, name) = DiskFile::split_filename(&filename);

                    sout_line(sout, &format!("Target: \"{name}\" - missing."));
                }
            }
        }

        finalresult
    }

    /// Scan any extra files specified on the command line.
    ///
    /// C++ Par1Repairer::VerifyExtraFiles
    fn verify_extra_files(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        extrafiles: &[String],
    ) -> bool {
        let mut index = 0usize;
        while index < extrafiles.len()
            && self.completefilecount < self.sourcefiles.len() as u32
        {
            let mut filename = extrafiles[index].clone();
            index += 1;

            let mut skip = false;

            // Find the file extension
            if filename.rfind('.').is_some() && Self::is_par_filename(&filename) {
                skip = true;
            }

            if !skip {
                filename = DiskFile::get_canonical_pathname(&filename);

                // Has this file already been dealt with
                if self.diskfilemap.find(&filename).is_none() {
                    let diskfile = self.add_diskfile();

                    // Does the file exist
                    if !diskfile.borrow_mut().open(&filename, serr) {
                        continue;
                    }

                    // Remember that we have processed this file
                    self.map_insert(&diskfile);

                    // Do the actual verification
                    // Ignore errors
                    self.verify_data_file(sout, serr, &diskfile, None);

                    // We have finished with the file for now
                    diskfile.borrow_mut().close();

                    // Find out how much data we have found
                    self.update_verification_results();
                }
            }
        }

        true
    }

    /// Check the file held in `diskfile` against all of the source files
    /// and record a match.  `sourcefile` is the index of the source file
    /// that the disk file was expected to be (if any).
    ///
    /// C++ Par1Repairer::VerifyDataFile
    fn verify_data_file(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        diskfile: &Rc<RefCell<DiskFile>>,
        sourcefile: Option<usize>,
    ) -> bool {
        let mut matched: Option<usize> = None;

        let name = {
            let filename = diskfile.borrow().file_name().to_string();
            DiskFile::split_filename(&filename).1
        };

        // How big is the file we are checking
        let filesize = diskfile.borrow().file_size();

        if filesize == 0 {
            if self.noiselevel > NoiseLevel::Silent {
                sout_line(sout, &format!("Target: \"{name}\" - empty."));
            }
            return true;
        }

        // Search for the first file that is the correct size
        let mut sourceiterator = self
            .sourcefiles
            .iter()
            .position(|sf| filesize == sf.filesize());

        // Are there any files that are the correct size?
        if sourceiterator.is_some() {
            // Allocate a buffer to compute the file hash
            let buffersize = std::cmp::min(1048576u64, filesize) as usize;
            let mut buffer = vec![0u8; buffersize];

            // Read the first 16k of the file
            let want = std::cmp::min(16384u64, filesize) as usize;
            if !diskfile.borrow_mut().read(0, &mut buffer[..want], serr) {
                return false;
            }

            // Compute the MD5 hash of the first 16k
            let mut contextfull = Md5Context::new();
            contextfull.update(&buffer[..want]);
            let mut context16k = contextfull.clone();
            let hash16k = context16k.finalize();

            if !self.ignore16kfilehash {
                // Search for the first file that has the correct 16k hash
                sourceiterator = self.sourcefiles.iter().position(|sf| {
                    filesize == sf.filesize() && hash16k == sf.hash16k()
                });
            }

            // Are there any files with the correct 16k hash?
            if sourceiterator.is_some() {
                // Compute the MD5 hash of the whole file
                if filesize > 16384 {
                    let mut offset = 16384u64;
                    let message = format!("Scanning: \"{name}\": ");
                    let progress =
                        ProgressMeter::new_shared(sout.clone(), &message, filesize);
                    while offset < filesize {
                        let want =
                            std::cmp::min(buffersize as u64, filesize - offset) as usize;

                        if !diskfile.borrow_mut().read(offset, &mut buffer[..want], serr) {
                            return false;
                        }

                        contextfull.update(&buffer[..want]);

                        offset += want as u64;

                        if self.noiselevel > NoiseLevel::Quiet {
                            progress.update(offset);
                        }
                    }
                }

                let hashfull = contextfull.finalize();

                // Search for the first file that has the correct full hash
                sourceiterator = self.sourcefiles.iter().position(|sf| {
                    filesize == sf.filesize()
                        && (self.ignore16kfilehash || hash16k == sf.hash16k())
                        && hashfull == sf.hashfull()
                });

                // Are there any files with the correct full hash?
                if sourceiterator.is_some() {
                    // If a source file was originally specified, check to see
                    // if it is a match
                    let specified_match = sourcefile.filter(|&sf| {
                        let sf = &self.sourcefiles[sf];
                        sf.filesize() == filesize
                            && (self.ignore16kfilehash || sf.hash16k() == hash16k)
                            && sf.hashfull() == hashfull
                    });
                    if specified_match.is_some() {
                        matched = specified_match;
                    } else {
                        // Search for a file which matches and has not already
                        // been matched
                        matched = self.sourcefiles.iter().position(|sf| {
                            filesize == sf.filesize()
                                && (self.ignore16kfilehash || hash16k == sf.hash16k())
                                && hashfull == sf.hashfull()
                                && sf.get_complete_file().is_none()
                        });
                    }
                }
            }
        }

        // Did we find a match
        if let Some(matched) = matched {
            self.sourcefiles[matched].set_complete_file(Some(diskfile.clone()));

            if self.noiselevel > NoiseLevel::Silent {
                // Was the match the file we were originally looking for
                if Some(matched) == sourcefile {
                    sout_line(sout, &format!("Target: \"{name}\" - found."));
                }
                // Were we looking for a specific file
                else if let Some(sourcefile) = sourcefile {
                    let filename = self.sourcefiles[sourcefile].filename().to_string();
                    let targetname = DiskFile::split_filename(&filename).1;

                    sout_line(
                        sout,
                        &format!(
                            "Target: \"{name}\" - is a match for \"{targetname}\"."
                        ),
                    );
                }
            }
        } else if self.noiselevel > NoiseLevel::Silent {
            sout_line(sout, &format!("File: \"{name}\" - no data found."));
        }

        true
    }
    // --------------------------------__UpdateVerificationResults__-------------------------------------

    /// C++ Par1Repairer::UpdateVerificationResults
    fn update_verification_results(&mut self) {
        self.completefilecount = 0;
        self.renamedfilecount = 0;
        self.damagedfilecount = 0;
        self.missingfilecount = 0;

        // Check the recoverable files
        for sourcefile in &self.sourcefiles {
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
            } else if sourcefile.get_target_exists() {
                // Does the target file exist
                self.damagedfilecount += 1;
            } else {
                self.missingfilecount += 1;
            }
        }
    }

    /// C++ Par1Repairer::CheckVerificationResults
    fn check_verification_results(&self, sout: &SharedSout) -> bool {
        let sourcefilecount = self.sourcefiles.len() as u32;

        // Is repair needed
        if self.completefilecount < sourcefilecount
            || self.renamedfilecount > 0
            || self.damagedfilecount > 0
            || self.missingfilecount > 0
        {
            if self.noiselevel > NoiseLevel::Silent {
                sout_line(sout, "Repair is required.");
            }
            if self.noiselevel > NoiseLevel::Quiet {
                if self.renamedfilecount > 0 {
                    sout_line(
                        sout,
                        &format!(
                            "{} file(s) have the wrong name.",
                            self.renamedfilecount
                        ),
                    );
                }
                if self.missingfilecount > 0 {
                    sout_line(
                        sout,
                        &format!("{} file(s) are missing.", self.missingfilecount),
                    );
                }
                if self.damagedfilecount > 0 {
                    sout_line(
                        sout,
                        &format!(
                            "{} file(s) exist but are damaged.",
                            self.damagedfilecount
                        ),
                    );
                }
                if self.completefilecount > 0 {
                    sout_line(
                        sout,
                        &format!("{} file(s) are ok.", self.completefilecount),
                    );
                }
            }

            // Is repair possible
            let recoveryblockcount = self.recoveryblocks.len() as u32;
            let needed = self.damagedfilecount + self.missingfilecount;
            if recoveryblockcount >= needed {
                if self.noiselevel > NoiseLevel::Silent {
                    sout_line(sout, "Repair is possible.");
                }

                if self.noiselevel > NoiseLevel::Quiet {
                    if recoveryblockcount > needed {
                        sout_line(
                            sout,
                            &format!(
                                "You have an excess of {} recovery files.",
                                recoveryblockcount - needed
                            ),
                        );
                    }

                    if needed > 0 {
                        sout_line(
                            sout,
                            &format!("{needed} recovery files will be used to repair."),
                        );
                    } else if recoveryblockcount > 0 {
                        sout_line(
                            sout,
                            "None of the recovery files will be used for the repair.",
                        );
                    }
                }

                true
            } else {
                if self.noiselevel > NoiseLevel::Silent {
                    sout_line(
                        sout,
                        &format!(
                            "Repair is not possible.\nYou need {} more recovery files to be able to repair.",
                            needed - recoveryblockcount
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

    // --------------------------------__UpdateVerificationResults__-------------------------------------

    // --------------------------------__RenameTargetFiles__-------------------------------------

    /// Rename any damaged or missnamed target files.
    ///
    /// C++ Par1Repairer::RenameTargetFiles
    fn rename_target_files(&mut self, serr: &mut dyn Write) -> bool {
        // Rename any damaged target files
        for index in 0..self.sourcefiles.len() {
            let (targetexists, targetfile, completefile) = {
                let sourcefile = &self.sourcefiles[index];
                (
                    sourcefile.get_target_exists(),
                    sourcefile.get_target_file(),
                    sourcefile.get_complete_file(),
                )
            };

            // If the target file exists but is not a complete version of the file
            let same = match (&targetfile, &completefile) {
                (Some(targetfile), Some(completefile)) => {
                    Rc::ptr_eq(targetfile, completefile)
                }
                _ => false,
            };
            if targetexists && !same {
                let targetfile = targetfile.unwrap();

                // Rename it
                self.map_remove(&targetfile);

                if !targetfile.borrow_mut().rename_auto(serr) {
                    return false;
                }

                self.backuplist.push(targetfile.clone());

                self.map_insert(&targetfile);

                // We no longer have a target file
                let sourcefile = &mut self.sourcefiles[index];
                sourcefile.set_target_exists(false);
                sourcefile.set_target_file(None);
            }
        }

        // Rename any missnamed but complete versions of the files
        for index in 0..self.sourcefiles.len() {
            let (targetfile, completefile) = {
                let sourcefile = &self.sourcefiles[index];
                (sourcefile.get_target_file(), sourcefile.get_complete_file())
            };

            // If there is no targetfile and there is a complete version
            if targetfile.is_none() {
                if let Some(targetfile) = completefile {
                    // Rename it
                    self.map_remove(&targetfile);

                    let filename = self.sourcefiles[index].filename().to_string();
                    if !targetfile.borrow_mut().rename_to(&filename, serr) {
                        return false;
                    }
                    self.map_insert(&targetfile);

                    // This file is now the target file
                    let sourcefile = &mut self.sourcefiles[index];
                    sourcefile.set_target_exists(true);
                    sourcefile.set_target_file(Some(targetfile));

                    // We have one more complete file
                    self.completefilecount += 1;
                }
            }
        }

        true
    }

    // --------------------------------__RenameTargetFiles__-------------------------------------

    // --------------------------------__CreateTargetFiles__-------------------------------------

    /// Work out which files are being repaired, create them, and allocate
    /// target DataBlocks to them, and remember them for later verification.
    ///
    /// C++ Par1Repairer::CreateTargetFiles
    fn create_target_files(&mut self, serr: &mut dyn Write) -> bool {
        // Create any missing target files
        for index in 0..self.sourcefiles.len() {
            // If the file does not exist
            if !self.sourcefiles[index].get_target_exists() {
                let filename = self.sourcefiles[index].filename().to_string();
                let filesize = self.sourcefiles[index].filesize();

                let targetfile = self.add_diskfile();

                // Create the target file
                if !targetfile.borrow_mut().create(&filename, filesize, serr) {
                    return false;
                }

                // This file is now the target file
                let sourcefile = &mut self.sourcefiles[index];
                sourcefile.set_target_exists(true);
                sourcefile.set_target_file(Some(targetfile.clone()));

                // Remember this file
                self.map_insert(&targetfile);

                let sourcefile = &mut self.sourcefiles[index];
                sourcefile.set_target_block(targetfile);

                // Add the file to the list of those that will need to be
                // verified once the repair has completed.
                self.verifylist.push(index);
            }
        }

        true
    }
    // --------------------------------__ComputeRSmatrix__-------------------------------------

    /// Work out which data blocks are available, which need to be recreated,
    /// and compute the appropriate Reed Solomon matrix.
    ///
    /// C++ Par1Repairer::ComputeRSmatrix
    fn compute_rs_matrix(&mut self, sout: &SharedSout, serr: &mut dyn Write) -> bool {
        self.inputblocks.clear(); // The DataBlocks that will read from disk
        self.outputblocks.clear(); // Those DataBlocks that will be recalculated

        // Build an array listing which source data blocks are present and
        // which are missing
        let mut present: Vec<bool> = Vec::new();

        // Iterate through all source files
        for index in 0..self.sourcefiles.len() {
            // Was this block found
            if self.sourcefiles[index].sourceblock().is_set() {
                // Open the file the block was found in.
                if !self.sourcefiles[index].sourceblock_mut().open(serr) {
                    return false;
                }

                // Record that the block was found
                present.push(true);

                // Add the block to the list of those which will be read
                // as input (and which might also need to be copied).
                let block = self.sourcefiles[index].sourceblock().clone();
                self.inputblocks.push(block);
            } else {
                // Record that the block was missing
                present.push(false);

                // Add the block to the list of those to be written
                let block = self.sourcefiles[index].targetblock().clone();
                self.outputblocks.push(block);
            }
        }

        // Set the number of source blocks and which of them are present
        if let Err(err) = self.rs.set_input_present(&present) {
            let _ = writeln!(serr, "{err}");
            return false;
        }

        // Start iterating through the available recovery packets
        let recoveryblocks: Vec<(u32, DataBlock)> = self
            .recoveryblocks
            .iter()
            .map(|(exponent, block)| (*exponent, block.clone()))
            .collect();
        let mut recoveryiterator = recoveryblocks.into_iter();

        // Continue to fill the remaining list of data blocks to be read
        while self.inputblocks.len() < self.sourcefiles.len() {
            // Get the next available recovery block
            let Some((exponent, mut recoveryblock)) = recoveryiterator.next() else {
                // The C++ would dereference the end of the map here; there
                // are not enough recovery blocks.
                return false;
            };

            // Make sure the file is open
            if !recoveryblock.open(serr) {
                return false;
            }

            // Add the recovery block to the list of blocks that will be read
            self.inputblocks.push(recoveryblock);

            // Record that the corresponding exponent value is the next one
            // to use in the RS matrix
            if !self.rs.set_output(true, exponent as u16) {
                return false;
            }
        }

        // If we need to, compute and solve the RS matrix
        if self.verifylist.is_empty() {
            return true;
        }

        match self.rs.compute(self.noiselevel, sout) {
            Ok(()) => true,
            Err(err) => {
                let _ = writeln!(serr, "{err}");
                false
            }
        }
    }

    /// Allocate memory buffers for reading and writing data to disk.
    ///
    /// C++ Par1Repairer::AllocateBuffers
    fn allocate_buffers(&mut self, memorylimit: usize) -> bool {
        let verifycount = self.verifylist.len();

        // Would single pass processing use too much memory
        if self.blocksize * verifycount as u64 > memorylimit as u64 {
            // Pick a size that is small enough
            self.chunksize = (!(3usize) & (memorylimit / verifycount)) as u64;
        } else {
            self.chunksize = self.blocksize;
        }

        // Allocate the two buffers (the C++ checked for allocation failure;
        // Rust's Vec panics instead)
        self.inputbuffersize = self.chunksize as usize;
        self.inputbuffer = vec![0u8; self.inputbuffersize];
        self.outputbufferalignment = (self.inputbuffersize + std::mem::size_of::<u32>() - 1)
            & !(std::mem::size_of::<u32>() - 1);
        self.outputbuffersize = self.outputbufferalignment * verifycount;
        self.outputbuffer = vec![0u8; self.outputbuffersize];

        true
    }

    // --------------------------------__ComputeRSmatrix__-------------------------------------

    // --------------------------------__ProcessData__-------------------------------------

    /// Read source data, process it through the RS matrix and write it to
    /// disk.
    ///
    /// C++ Par1Repairer::ProcessData
    fn process_data(
        &mut self,
        blockoffset: u64,
        blocklength: usize,
        progress: &ProgressMeter,
        sout: &SharedSout,
        serr: &mut dyn Write,
    ) -> bool {
        let mut totalwritten = 0u64;

        // Clear the output buffer
        self.outputbuffer.fill(0);

        // Are there any blocks which need to be reconstructed
        if !self.verifylist.is_empty() {
            // For each input block
            for inputindex in 0..self.inputblocks.len() {
                // Read data from the current input block
                if !self.inputblocks[inputindex]
                    .read_data(blockoffset, &mut self.inputbuffer[..blocklength], serr)
                {
                    return false;
                }

                // For each output block
                for outputindex in 0..self.verifylist.len() {
                    // Select the appropriate part of the output buffer
                    let start = outputindex * self.outputbufferalignment;

                    // Process the data
                    let _ = self.rs.process(
                        blocklength,
                        inputindex as u32,
                        &self.inputbuffer[..blocklength],
                        outputindex as u32,
                        &mut self.outputbuffer[start..start + blocklength],
                    );

                    if self.noiselevel > NoiseLevel::Quiet {
                        progress.add(blocklength as u64);
                    }
                }
            }
        }

        if self.noiselevel > NoiseLevel::Quiet {
            sout_write(sout, "Writing recovered data\r");
        }

        // For each output block that has been recomputed
        for outputindex in 0..self.verifylist.len() {
            // Select the appropriate part of the output buffer
            let start = outputindex * self.outputbufferalignment;

            // Write the data to the target file
            let mut wrote = 0usize;
            if !self.outputblocks[outputindex].write_data(
                blockoffset,
                &self.outputbuffer[start..start + blocklength],
                &mut wrote,
                serr,
            ) {
                return false;
            }
            totalwritten += wrote as u64;
        }

        if self.noiselevel > NoiseLevel::Quiet {
            sout_line(sout, &format!("Wrote {totalwritten} bytes to disk"));
        }

        true
    }

    // --------------------------------__ProcessData__-------------------------------------

    // --------------------------------__VerifyTargetFiles__-------------------------------------

    /// Verify that all of the reconstructed target files are now correct.
    ///
    /// C++ Par1Repairer::VerifyTargetFiles
    fn verify_target_files(&mut self, sout: &SharedSout, serr: &mut dyn Write) -> bool {
        let mut finalresult = true;

        // Iterate through each file in the verification list
        let verifylist = self.verifylist.clone();
        for index in verifylist {
            let targetfile = self.sourcefiles[index].get_target_file().unwrap();

            // Close the file
            if targetfile.borrow().is_open() {
                targetfile.borrow_mut().close();
            }

            // Say we don't have a complete version of the file
            self.sourcefiles[index].set_complete_file(None);

            // Re-open the target file
            if !targetfile.borrow_mut().reopen(serr) {
                finalresult = false;
                continue;
            }

            // Verify the file again
            if !self.verify_data_file(sout, serr, &targetfile, Some(index)) {
                finalresult = false;
            }

            // Close the file again
            targetfile.borrow_mut().close();

            // Find out how much data we have found
            self.update_verification_results();
        }

        finalresult
    }

    /// Delete all of the partly reconstructed files.
    ///
    /// C++ Par1Repairer::DeleteIncompleteTargetFiles
    fn delete_incomplete_target_files(&mut self, serr: &mut dyn Write) -> bool {
        // Iterate through each file in the verification list
        let verifylist = self.verifylist.clone();
        for index in verifylist {
            if self.sourcefiles[index].get_target_exists() {
                let targetfile = self.sourcefiles[index].get_target_file().unwrap();

                // Close and delete the file
                if targetfile.borrow().is_open() {
                    targetfile.borrow_mut().close();
                }
                targetfile.borrow_mut().delete(serr);

                // Forget the file
                self.map_remove(&targetfile);

                // There is no target file
                let sourcefile = &mut self.sourcefiles[index];
                sourcefile.set_target_exists(false);
                sourcefile.set_target_file(None);
            }
        }

        true
    }

    /// C++ Par1Repairer::RemoveBackupFiles
    fn remove_backup_files(&self, sout: &SharedSout, serr: &mut dyn Write) -> bool {
        if self.noiselevel > NoiseLevel::Silent && !self.backuplist.is_empty() {
            sout_line(sout, "\nPurge backup files.");
        }

        // Iterate through each file in the backuplist
        for backupfile in &self.backuplist {
            if self.noiselevel > NoiseLevel::Silent {
                let filename = backupfile.borrow().file_name().to_string();
                let name = DiskFile::split_filename(&filename).1;
                sout_line(sout, &format!("Remove \"{name}\"."));
            }

            if backupfile.borrow().is_open() {
                backupfile.borrow_mut().close();
            }
            backupfile.borrow_mut().delete(serr);
        }

        true
    }

    /// C++ Par1Repairer::RemoveParFiles
    fn remove_par_files(&self, sout: &SharedSout, serr: &mut dyn Write) -> bool {
        if self.noiselevel > NoiseLevel::Silent && !self.parlist.is_empty() {
            sout_line(sout, "\nPurge par files.");
        }

        for filename in &self.parlist {
            let diskfile = Rc::new(RefCell::new(DiskFile::new()));

            if diskfile.borrow_mut().open(filename, serr) {
                if self.noiselevel > NoiseLevel::Silent {
                    let name = DiskFile::split_filename(filename).1;
                    sout_line(sout, &format!("Remove \"{name}\"."));
                }

                if diskfile.borrow().is_open() {
                    diskfile.borrow_mut().close();
                }
                diskfile.borrow_mut().delete(serr);
            }
        }

        true
    }
    // --------------------------------__VerifyTargetFiles__-------------------------------------
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md5::Md5Hash;
    use std::path::PathBuf;

    fn md5_of(data: &[u8]) -> Md5Hash {
        let mut context = Md5Context::new();
        context.update(data);
        context.finalize()
    }

    // Build the bytes of a PAR1FILEENTRY holding the given (ASCII) name.
    fn build_entry(
        name: &str,
        filesize: u64,
        hashfull: Md5Hash,
        hash16k: Md5Hash,
        status: u64,
    ) -> Vec<u8> {
        // ASCII as UTF-16 little endian: every byte followed by a zero byte.
        let name_units: Vec<u8> = name.bytes().flat_map(|b| [b, 0]).collect();
        let entrysize = PAR1FILEENTRY_SIZE + name_units.len();
        let mut data = vec![0u8; entrysize];
        data[PAR1FILEENTRY_ENTRYSIZE_OFFSET..PAR1FILEENTRY_ENTRYSIZE_OFFSET + 8]
            .copy_from_slice(&(entrysize as u64).to_le_bytes());
        data[PAR1FILEENTRY_STATUS_OFFSET..PAR1FILEENTRY_STATUS_OFFSET + 8]
            .copy_from_slice(&status.to_le_bytes());
        data[PAR1FILEENTRY_FILESIZE_OFFSET..PAR1FILEENTRY_FILESIZE_OFFSET + 8]
            .copy_from_slice(&filesize.to_le_bytes());
        data[PAR1FILEENTRY_HASHFULL_OFFSET..PAR1FILEENTRY_HASHFULL_OFFSET + 16]
            .copy_from_slice(&hashfull.hash);
        data[PAR1FILEENTRY_HASH16K_OFFSET..PAR1FILEENTRY_HASH16K_OFFSET + 16]
            .copy_from_slice(&hash16k.hash);
        data[PAR1FILEENTRY_NAME_OFFSET..].copy_from_slice(&name_units);
        data
    }

    // Build a complete PAR 1.0 file: header, file list and optional
    // recovery data, with a correct control hash.
    fn build_par1(volumenumber: u64, entries: &[Vec<u8>], data: Option<&[u8]>) -> Vec<u8> {
        let filelist: Vec<u8> = entries.concat();
        let filelistsize = filelist.len() as u64;
        let dataoffset = if data.is_some() {
            (PAR1FILEHEADER_SIZE + filelist.len()) as u64
        } else {
            0
        };
        let datasize = data.map_or(0, |d| d.len() as u64);

        let mut file = vec![0u8; PAR1FILEHEADER_SIZE];
        file[PAR1FILEHEADER_MAGIC_OFFSET..PAR1FILEHEADER_MAGIC_OFFSET + 8]
            .copy_from_slice(&PAR1_MAGIC);
        file[PAR1FILEHEADER_FILEVERSION_OFFSET..PAR1FILEHEADER_FILEVERSION_OFFSET + 4]
            .copy_from_slice(&0x00010000u32.to_le_bytes());
        file[PAR1FILEHEADER_PROGRAMVERSION_OFFSET..PAR1FILEHEADER_PROGRAMVERSION_OFFSET + 4]
            .copy_from_slice(&0x00010002u32.to_le_bytes());
        file[PAR1FILEHEADER_VOLUMENUMBER_OFFSET..PAR1FILEHEADER_VOLUMENUMBER_OFFSET + 8]
            .copy_from_slice(&volumenumber.to_le_bytes());
        file[PAR1FILEHEADER_NUMBEROFFILES_OFFSET..PAR1FILEHEADER_NUMBEROFFILES_OFFSET + 8]
            .copy_from_slice(&(entries.len() as u64).to_le_bytes());
        file[PAR1FILEHEADER_FILELISTOFFSET_OFFSET..PAR1FILEHEADER_FILELISTOFFSET_OFFSET + 8]
            .copy_from_slice(&(PAR1FILEHEADER_SIZE as u64).to_le_bytes());
        file[PAR1FILEHEADER_FILELISTSIZE_OFFSET..PAR1FILEHEADER_FILELISTSIZE_OFFSET + 8]
            .copy_from_slice(&filelistsize.to_le_bytes());
        file[PAR1FILEHEADER_DATAOFFSET_OFFSET..PAR1FILEHEADER_DATAOFFSET_OFFSET + 8]
            .copy_from_slice(&dataoffset.to_le_bytes());
        file[PAR1FILEHEADER_DATASIZE_OFFSET..PAR1FILEHEADER_DATASIZE_OFFSET + 8]
            .copy_from_slice(&datasize.to_le_bytes());
        file.extend_from_slice(&filelist);
        if let Some(d) = data {
            file.extend_from_slice(d);
        }

        // The control hash covers everything from the set hash onwards.
        let hash = md5_of(&file[PAR1FILEHEADER_SETHASH_OFFSET..]);
        file[PAR1FILEHEADER_CONTROLHASH_OFFSET..PAR1FILEHEADER_CONTROLHASH_OFFSET + 16]
            .copy_from_slice(&hash.hash);
        file
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("par2rust_p1r_{tag}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn shared_sout() -> SharedSout {
        let writer: Box<dyn Write + Send> = Box::new(Vec::<u8>::new());
        Arc::new(Mutex::new(writer))
    }

    #[test]
    fn test1_is_par_filename() {
        assert!(Par1Repairer::is_par_filename("test.PAR"));
        assert!(Par1Repairer::is_par_filename("test.par"));
        assert!(Par1Repairer::is_par_filename("test.pAr"));
        assert!(Par1Repairer::is_par_filename("test.PAR1"));
        assert!(Par1Repairer::is_par_filename("test.P01"));
        assert!(Par1Repairer::is_par_filename("test.p99"));
        assert!(!Par1Repairer::is_par_filename("no_extension"));
        assert!(!Par1Repairer::is_par_filename("test.P"));
        assert!(!Par1Repairer::is_par_filename("test.PN"));
        assert!(!Par1Repairer::is_par_filename("test.txt"));
        assert!(!Par1Repairer::is_par_filename("test.PRR"));
    }

    #[test]
    fn test2_verify_complete_files() {
        let dir = temp_dir("t2");
        let content = b"Hello, PAR 1.0 world!";
        std::fs::write(dir.join("test.txt"), content).unwrap();

        let hashfull = md5_of(content);
        let hash16k = md5_of(&content[..content.len().min(16384)]);
        let entry = build_entry(
            "test.txt",
            content.len() as u64,
            hashfull,
            hash16k,
            FILEENTRYSTATUS_INPARITYVOLUME,
        );
        let parfile = dir.join("test.PAR");
        std::fs::write(&parfile, build_par1(0, &[entry], None)).unwrap();

        let mut repairer = Par1Repairer::new(NoiseLevel::Silent);
        let sout = shared_sout();
        let mut serr = Vec::new();
        let result = repairer.process(
            &sout,
            &mut serr,
            1024 * 1024,
            1,
            parfile.to_str().unwrap().to_string(),
            &[],
            false,
            false,
        );

        assert_eq!(result, Result::Success);
        assert_eq!(repairer.sourcefiles.len(), 1);
        assert_eq!(repairer.blocksize, content.len() as u64);
        assert_eq!(repairer.completefilecount, 1);
        assert_eq!(repairer.damagedfilecount, 0);
        assert_eq!(repairer.missingfilecount, 0);
    }

    #[test]
    fn test3_repair_missing_file() {
        let dir = temp_dir("t3");
        let content: Vec<u8> = (0..1000u32).map(|i| (i * 7 + 3) as u8).collect();

        let hashfull = md5_of(&content);
        let hash16k = md5_of(&content[..content.len().min(16384)]);
        let entry = build_entry(
            "test.txt",
            content.len() as u64,
            hashfull,
            hash16k,
            FILEENTRYSTATUS_INPARITYVOLUME,
        );

        // Volume 0 holds only the file list.
        std::fs::write(
            dir.join("test.PAR"),
            build_par1(0, std::slice::from_ref(&entry), None),
        )
        .unwrap();
        // Volume 1 holds the recovery data.  With a single data block the
        // Reed Solomon factor for exponent 0 is 1, so the recovery block
        // is a copy of the data.
        std::fs::write(
            dir.join("test.P01"),
            build_par1(1, &[entry], Some(&content)),
        )
        .unwrap();
        // The data file itself is missing.

        let mut repairer = Par1Repairer::new(NoiseLevel::Silent);
        let sout = shared_sout();
        let mut serr = Vec::new();
        let result = repairer.process(
            &sout,
            &mut serr,
            1024 * 1024,
            1,
            dir.join("test.PAR").to_str().unwrap().to_string(),
            &[],
            true,
            false,
        );

        assert_eq!(result, Result::Success);
        assert_eq!(repairer.completefilecount, 1);
        assert_eq!(repairer.missingfilecount, 0);
        assert_eq!(std::fs::read(dir.join("test.txt")).unwrap(), content);
    }

    #[test]
    fn test4_damaged_without_recovery() {
        let dir = temp_dir("t4");
        let content = b"the original contents!!"; // 23 bytes
        std::fs::write(dir.join("test.txt"), b"the corrupted contents!").unwrap();

        let hashfull = md5_of(content);
        let hash16k = md5_of(content);
        let entry = build_entry(
            "test.txt",
            content.len() as u64,
            hashfull,
            hash16k,
            FILEENTRYSTATUS_INPARITYVOLUME,
        );
        std::fs::write(dir.join("test.PAR"), build_par1(0, &[entry], None)).unwrap();

        let mut repairer = Par1Repairer::new(NoiseLevel::Silent);
        let sout = shared_sout();
        let mut serr = Vec::new();
        let result = repairer.process(
            &sout,
            &mut serr,
            1024 * 1024,
            1,
            dir.join("test.PAR").to_str().unwrap().to_string(),
            &[],
            true,
            false,
        );

        // The file is damaged and there are no recovery blocks.
        assert_eq!(result, Result::RepairNotPossible);
        assert_eq!(repairer.damagedfilecount, 1);
    }

    #[test]
    fn test5_corrupt_par_file_ignored() {
        let dir = temp_dir("t5");
        let content = b"some data";

        let hashfull = md5_of(content);
        let entry = build_entry(
            "test.txt",
            content.len() as u64,
            hashfull,
            hashfull,
            FILEENTRYSTATUS_INPARITYVOLUME,
        );
        let mut par = build_par1(0, &[entry], None);
        // Corrupt the file list so the control hash no longer matches.
        let mid = PAR1FILEHEADER_SIZE + 8;
        par[mid] ^= 0xff;
        std::fs::write(dir.join("test.PAR"), par).unwrap();

        let mut repairer = Par1Repairer::new(NoiseLevel::Silent);
        let sout = shared_sout();
        let mut serr = Vec::new();
        let result = repairer.process(
            &sout,
            &mut serr,
            1024 * 1024,
            1,
            dir.join("test.PAR").to_str().unwrap().to_string(),
            &[],
            false,
            false,
        );

        // The corrupt PAR file is skipped, leaving nothing to verify.
        assert_eq!(result, Result::Success);
        assert!(repairer.sourcefiles.is_empty());
    }
}

