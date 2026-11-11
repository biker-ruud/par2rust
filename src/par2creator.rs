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

//! Port of par2cmdline's par2creator.h / par2creator.cpp.
//!
//! The Par2Creator creates a set of PAR 2.0 files for a set of data
//! files. It opens the source files and records their description and
//! verification packets, builds the main packet (which fixes the set id)
//! and the creator packet, allocates the critical and recovery packets
//! across the recovery files, computes the Reed Solomon recovery data
//! through a processor, and writes everything to disk.

use std::cell::RefCell;
use std::cmp::min;
use std::cmp::Ordering;
use std::io::Write;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::backends::{Backends, ProcessorConfig};
use crate::creatorpacket::CreatorPacket;
use crate::criticalpacket::CriticalPacket;
use crate::datablock::DataBlock;
use crate::descriptionpacket::DescriptionPacket;
use crate::diskfile::{DiskFile, MAX_PATH_LEN};
use crate::galois::Spec16;
use crate::libpar2::{compute_recovery_file_count, Result, Scheme};
use crate::mainpacket::MainPacket;
use crate::md5::Md5Hash;
use crate::noiselevel::NoiseLevel;
use crate::par1repairer::{sout_line, sout_write, SharedSout};
use crate::par2creatorsourcefile::{CriticalPacketRef, Par2CreatorSourceFile};
use crate::par2repairer::{FILE_THREADS, MAX_CHUNK_SIZE, NUM_TRANSFER_BUFFERS};
use crate::processor::Processor;
use crate::progressmeter::ProgressMeter;
use crate::recoverypacket::RecoveryPacket;
use crate::reedsolomon::ReedSolomon;
use crate::taskpool::{default_threads, resolve_threads};
use crate::verificationpacket::VerificationPacket;

/// A critical packet owned by the creator or by one of the source files.
///
/// The C++ original keeps raw pointers in the critical packet list; the
/// Rust port holds shared references, and this enum is the tagged version
/// of that needed to treat all four packet kinds alike.
#[derive(Clone)]
enum CriticalPacketHandle {
    Description(Rc<RefCell<DescriptionPacket>>),
    Verification(Rc<RefCell<VerificationPacket>>),
    Main(Rc<RefCell<MainPacket>>),
    Creator(Rc<RefCell<CreatorPacket>>),
}

impl From<CriticalPacketRef> for CriticalPacketHandle {
    fn from(packet: CriticalPacketRef) -> CriticalPacketHandle {
        match packet {
            CriticalPacketRef::Description(packet) => CriticalPacketHandle::Description(packet),
            CriticalPacketRef::Verification(packet) => CriticalPacketHandle::Verification(packet),
        }
    }
}

impl CriticalPacketHandle {
    /// Run a function over the base critical packet of the handle.
    fn with_critical<R>(&self, f: impl FnOnce(&CriticalPacket) -> R) -> R {
        match self {
            CriticalPacketHandle::Description(packet) => f(packet.borrow().critical()),
            CriticalPacketHandle::Verification(packet) => f(packet.borrow().critical()),
            CriticalPacketHandle::Main(packet) => f(packet.borrow().critical()),
            CriticalPacketHandle::Creator(packet) => f(packet.borrow().critical()),
        }
    }

    /// The length of the packet data.
    fn packet_length(&self) -> u64 {
        self.with_critical(|packet| packet.packet_length() as u64)
    }

    /// Store the set id in the packet and compute its hash.
    fn finish_packet(&self, setid: &Md5Hash) {
        match self {
            CriticalPacketHandle::Description(packet) => {
                packet.borrow_mut().critical_mut().finish_packet(setid)
            }
            CriticalPacketHandle::Verification(packet) => {
                packet.borrow_mut().critical_mut().finish_packet(setid)
            }
            CriticalPacketHandle::Main(packet) => {
                packet.borrow_mut().critical_mut().finish_packet(setid)
            }
            CriticalPacketHandle::Creator(packet) => {
                packet.borrow_mut().critical_mut().finish_packet(setid)
            }
        }
    }

    /// Write the packet to the specified file at the specified offset.
    fn write_packet(&self, diskfile: &mut DiskFile, offset: u64, serr: &mut dyn Write) -> bool {
        self.with_critical(|packet| packet.write_packet(diskfile, offset, serr))
    }

    /// Sort critical packets by type and fileid, as C++ CompareLess does.
    fn compare_less(&self, other: &CriticalPacketHandle) -> bool {
        self.with_critical(|left| {
            other.with_critical(|right| CriticalPacket::compare_less(left, right))
        })
    }
}

/// A critical packet allocated to a position in an output file. The C++
/// CriticalPacketEntry holds pointers to the file and to the packet; the
/// Rust port holds shared references instead.
struct CriticalPacketEntry {
    diskfile: Rc<RefCell<DiskFile>>, // The file the packet is stored in
    offset: u64,                     // The offset at which it is stored
    packet: CriticalPacketHandle,    // The packet itself
}

/// The C++ FileAllocation: the name chosen for one recovery file and
/// which recovery blocks it will hold.
#[derive(Clone, Default)]
struct FileAllocation {
    filename: String,
    exponent: u32,
    count: u32,
}

/// C++ Par2Creator
pub struct Par2Creator {
    noiselevel: NoiseLevel,

    /// The implementations the application supplied
    backends: Backends,

    totalthreads: u32, // Number of threads the whole create may use
    filethreads: u32,  // Number of threads used to read files

    blocksize: u64, // The size of a block (par2 block size)
    chunksize: u64, // The amount of a block processed at a time

    transferbuffer: Vec<u8>, // Buffer for reading source data
    outputbuffer: Vec<u8>,   // Buffer for reading recovery data back

    sourcefilecount: u32,  // Number of source files
    sourceblockcount: u32, // Number of source blocks

    largestfilesize: u64, // Size of the largest source file

    recoveryfilescheme: Scheme, // Which scheme determines the contents of each recovery file
    recoveryfilecount: u32,     // Number of recovery files to create
    recoveryblockcount: u32,    // Number of recovery blocks to create
    firstrecoveryblock: u32,    // The exponent value of the first recovery block

    mainpacket: Option<Rc<RefCell<MainPacket>>>,    // The main packet for the set
    creatorpacket: Option<Rc<RefCell<CreatorPacket>>>, // The creator packet for the set

    sourcefiles: Vec<Par2CreatorSourceFile>, // Details of each source file
    sourceblocks: Vec<DataBlock>,            // Data block for each source block

    recoveryfiles: Vec<Rc<RefCell<DiskFile>>>, // The recovery files (one more than recoveryfilecount, the last being the main .par2 file)
    recoverypackets: Vec<RecoveryPacket>,      // The recovery packets

    criticalpackets: Vec<CriticalPacketHandle>,    // Critical packets in sort order
    criticalpacketentries: Vec<CriticalPacketEntry>, // Critical packets allocated to output files

    rs: ReedSolomon<Spec16>, // The Reed Solomon codec

    processor: Option<Box<dyn Processor>>, // Accumulates the recovery data

    deferhashcomputation: bool, // Should computation of file hashes be postponed
}

impl Par2Creator {
    pub fn new(noiselevel: NoiseLevel, backends: Backends) -> Par2Creator {
        Par2Creator {
            noiselevel,
            backends,
            totalthreads: default_threads(),
            filethreads: FILE_THREADS,
            blocksize: 0,
            chunksize: 0,
            transferbuffer: Vec::new(),
            outputbuffer: Vec::new(),
            sourcefilecount: 0,
            sourceblockcount: 0,
            largestfilesize: 0,
            recoveryfilescheme: Scheme::Unknown,
            recoveryfilecount: 0,
            recoveryblockcount: 0,
            firstrecoveryblock: 0,
            mainpacket: None,
            creatorpacket: None,
            sourcefiles: Vec::new(),
            sourceblocks: Vec::new(),
            recoveryfiles: Vec::new(),
            recoverypackets: Vec::new(),
            criticalpackets: Vec::new(),
            criticalpacketentries: Vec::new(),
            rs: ReedSolomon::new(),
            processor: None,
            deferhashcomputation: false,
        }
    }

    /// The number of files read at once (the C++ GetFileThreads). This
    /// port scans the files one at a time on the calling thread, so the
    /// value is only recorded.
    pub fn filethreads(&self) -> u32 {
        self.filethreads
    }

    /// Create the PAR 2.0 files. C++ Par2Creator::Process.
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        memorylimit: usize,
        basepath: &str,
        nthreads: u32,
        filethreads: u32,
        parfilename: &str,
        extrafiles: &[String],
        blocksize: u64,
        firstblock: u32,
        recoveryfilescheme: Scheme,
        recoveryfilecount: u32,
        recoveryblockcount: u32,
    ) -> Result {
        self.filethreads = filethreads;

        if !self.check_basepath(parfilename, serr) {
            return Result::FileIOError;
        }

        // Get information from commandline
        self.blocksize = blocksize;
        self.sourcefilecount = extrafiles.len() as u32;
        self.recoveryblockcount = recoveryblockcount;
        self.recoveryfilecount = recoveryfilecount;
        self.firstrecoveryblock = firstblock;
        self.recoveryfilescheme = recoveryfilescheme;

        // Set the number of threads
        self.totalthreads = resolve_threads(nthreads);

        // Compute block size from block count or vice versa depending on
        // which was specified on the command line
        if !self.compute_block_count(extrafiles, serr) {
            return Result::InvalidCommandLineArguments;
        }

        // Determine how many recovery files to create.
        if !compute_recovery_file_count(
            serr,
            &mut self.recoveryfilecount,
            self.recoveryfilescheme,
            self.recoveryblockcount,
            self.largestfilesize,
            self.blocksize,
        ) {
            return Result::InvalidCommandLineArguments;
        }

        // Determine how much recovery data can be computed on one pass
        if !self.calculate_process_block_size(memorylimit) {
            return Result::LogicError;
        }

        if self.recoveryblockcount > 0 && self.noiselevel >= NoiseLevel::Debug {
            sout_line(
                sout,
                &format!("[DEBUG] Process chunk size: {}", self.chunksize),
            );
        }

        if self.noiselevel > NoiseLevel::Quiet {
            // Display information.
            sout_line(
                sout,
                &format!(
                    "Block size: {}\nSource file count: {}\nSource block count: {}\nRecovery block count: {}\nRecovery file count: {}\n",
                    self.blocksize,
                    self.sourcefilecount,
                    self.sourceblockcount,
                    self.recoveryblockcount,
                    self.recoveryfilecount
                ),
            );
        }

        // Open all of the source files, compute the Hashes and CRC values,
        // and store the results in the file verification and file
        // description packets.
        if !self.open_source_files(sout, serr, extrafiles, basepath) {
            return Result::FileIOError;
        }

        // Create the main packet and determine the setid to use with all
        // packets
        if !self.create_main_packet() {
            return Result::LogicError;
        }

        // Create the creator packet.
        if !self.create_creator_packet() {
            return Result::LogicError;
        }

        // Initialise all of the source blocks ready to start reading data
        // from the source files.
        if !self.create_source_blocks() {
            return Result::LogicError;
        }

        // Create all of the output files and allocate all packets to
        // appropriate file offsets.
        if !self.initialise_output_files(parfilename, serr) {
            return Result::FileIOError;
        }

        if self.recoveryblockcount > 0 {
            // Allocate memory buffers for reading and writing data to disk.
            if !self.allocate_buffers(memorylimit, serr) {
                return Result::MemoryError;
            }

            // Compute the Reed Solomon matrix
            if !self.compute_rs_matrix(sout, serr) {
                return Result::LogicError;
            }

            // Set the total amount of data to be processed.
            let progress = ProgressMeter::new_shared(
                Arc::clone(sout),
                "Processing: ",
                self.blocksize * self.sourceblockcount as u64,
            );

            // Start at an offset of 0 within a block.
            let mut blockoffset = 0u64;
            while blockoffset < self.blocksize {
                // Continue until the end of the block. Work out how much
                // data to process this time.
                let blocklength = min(self.chunksize, self.blocksize - blockoffset) as usize;

                // Read source data, process it through the RS matrix and
                // write it to disk.
                if !self.process_data(sout, serr, blockoffset, blocklength, &progress) {
                    return Result::FileIOError;
                }

                blockoffset += blocklength as u64;
            }

            if self.noiselevel > NoiseLevel::Quiet {
                sout_line(sout, "Writing recovery packets");
            }

            // Finish computation of the recovery packets and write the
            // headers to disk.
            if !self.write_recovery_packet_headers(serr) {
                return Result::FileIOError;
            }

            // Finish computing the full file hash values of the source files
            if !self.finish_file_hash_computation() {
                return Result::LogicError;
            }
        }

        // Fill in all remaining details in the critical packets.
        if !self.finish_critical_packets() {
            return Result::LogicError;
        }

        if self.noiselevel > NoiseLevel::Quiet {
            sout_line(sout, "Writing verification packets");
        }

        // Write all other critical packets to disk.
        if !self.write_critical_packets(serr) {
            return Result::FileIOError;
        }

        // Close all files.
        if !self.close_files() {
            return Result::FileIOError;
        }

        if self.noiselevel > NoiseLevel::Silent {
            sout_line(sout, "Done");
        }

        Result::Success
    }

    // ---------------------------------CheckBasepath---------------------------------

    /// Check basepath permission by creating and deleting a dummy file.
    /// C++ Par2Creator::CheckBasepath.
    fn check_basepath(&self, parfilename: &str, serr: &mut dyn Write) -> bool {
        let checkfilename = format!("{parfilename}.check.par2");
        let mut diskfile = DiskFile::new();
        let dummysize = 4096u64;

        if !diskfile.create(&checkfilename, dummysize, serr) {
            return false;
        }

        diskfile.close();

        if !diskfile.delete(serr) {
            return false;
        }

        true
    }

    // ---------------------------------ComputeBlockCount---------------------------------

    /// Compute block size from block count or vice versa depending on
    /// which was specified on the command line. C++
    /// Par2Creator::ComputeBlockCount.
    fn compute_block_count(&mut self, extrafiles: &[String], serr: &mut dyn Write) -> bool {
        // The C++ original caches the file sizes; keep them likewise.
        let mut filesizes: Vec<u64> = Vec::with_capacity(extrafiles.len());
        let mut largestfilesize = 0u64;
        for extrafile in extrafiles {
            let filesize = DiskFile::get_file_size(extrafile);
            if largestfilesize < filesize {
                largestfilesize = filesize;
            }
            filesizes.push(filesize);
        }
        self.largestfilesize = largestfilesize;

        if self.blocksize == 0 {
            writeln!(serr, "ERROR: Block size was zero!").expect("cannot write to serr");
            return false;
        }

        if self.blocksize % 4 != 0 {
            writeln!(serr, "ERROR: Block size was not a multiple of 4 bytes!")
                .expect("cannot write to serr");
            return false;
        }

        let mut count = 0u64;
        for filesize in &filesizes {
            count += filesize.div_ceil(self.blocksize);
        }

        if count > 32768 {
            writeln!(
                serr,
                "Block size is too small. It would require {count}blocks."
            )
            .expect("cannot write to serr");
            return false;
        }

        self.sourceblockcount = count as u32;

        true
    }

    // ------------------------------CalculateProcessBlockSize------------------------------

    /// Determine how much recovery data can be computed on one pass. C++
    /// Par2Creator::CalculateProcessBlockSize.
    fn calculate_process_block_size(&mut self, memorylimit: usize) -> bool {
        // Are we computing any recovery blocks
        if self.recoveryblockcount == 0 {
            self.chunksize = 0;

            self.deferhashcomputation = false;
        } else {
            // Would single pass processing use too much memory
            if self.blocksize * self.recoveryblockcount as u64 > memorylimit as u64 {
                // Pick a size that is small enough
                self.chunksize =
                    (!(3usize) & (memorylimit / self.recoveryblockcount as usize)) as u64;

                self.deferhashcomputation = false;
            } else {
                self.chunksize = self.blocksize;

                self.deferhashcomputation = true;
            }

            if MAX_CHUNK_SIZE != 0 && self.chunksize > MAX_CHUNK_SIZE {
                self.chunksize = MAX_CHUNK_SIZE;
                self.deferhashcomputation = false;
            }
        }

        true
    }

    // --------------------------------OpenSourceFiles--------------------------------

    /// Open all of the source files, compute the Hashes and CRC values,
    /// and store the results in the file verification and file description
    /// packets. C++ Par2Creator::OpenSourceFiles. The C++ original reads
    /// the files in parallel across `filethreads` threads; this port
    /// scans one file at a time on the calling thread.
    fn open_source_files(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        extrafiles: &[String],
        basepath: &str,
    ) -> bool {
        // Total size of files for the progress line
        let mttotalsize: u64 = extrafiles
            .iter()
            .map(|extrafile| DiskFile::get_file_size(extrafile))
            .sum();

        let progress = ProgressMeter::new_shared(Arc::clone(sout), "", mttotalsize);

        let mut criticalrefs: Vec<CriticalPacketRef> = Vec::new();
        for extrafile in extrafiles {
            let name = DiskFile::split_relative_filename(extrafile, basepath);

            if self.noiselevel > NoiseLevel::Silent {
                sout_line(sout, &format!("Opening: {name}"));
            }

            let mut sourcefile = Par2CreatorSourceFile::new();

            // Open the source file and compute its Hashes and CRCs.
            if !sourcefile.open(
                self.noiselevel,
                serr,
                extrafile,
                self.blocksize,
                self.deferhashcomputation,
                basepath,
                &progress,
                &self.backends,
            ) {
                return false;
            }

            // Record the file verification and file description packets
            // in the critical packet list.
            sourcefile.record_critical_packets(&mut criticalrefs);

            // Close the source file until its needed
            sourcefile.close();

            self.sourcefiles.push(sourcefile);
        }

        self.criticalpackets
            .extend(criticalrefs.into_iter().map(CriticalPacketHandle::from));

        true
    }
    // --------------------------------CreateMainPacket--------------------------------

    /// Create the main packet and determine the setid to use with all
    /// packets. C++ Par2Creator::CreateMainPacket.
    fn create_main_packet(&mut self) -> bool {
        // Construct the main packet from the list of source files and the
        // block size.
        let mainpacket = Rc::new(RefCell::new(MainPacket::new()));

        // Add the main packet to the list of critical packets.
        self.criticalpackets
            .push(CriticalPacketHandle::Main(Rc::clone(&mainpacket)));

        // Create the packet (sourcefiles will get sorted into FileId
        // order, which the C++ MainPacket::Create does in place).
        self.sourcefiles
            .sort_by_key(Par2CreatorSourceFile::fileid);
        let mut fileids: Vec<Md5Hash> = self
            .sourcefiles
            .iter()
            .map(Par2CreatorSourceFile::fileid)
            .collect();
        let created = mainpacket.borrow_mut().create(&mut fileids, self.blocksize);

        self.mainpacket = Some(mainpacket);

        created
    }

    // -------------------------------CreateCreatorPacket-------------------------------

    /// Create the creator packet. C++ Par2Creator::CreateCreatorPacket.
    fn create_creator_packet(&mut self) -> bool {
        // Construct the creator packet
        let setid = self
            .mainpacket
            .as_ref()
            .expect("create_main_packet first")
            .borrow()
            .setid();

        let mut creatorpacket = CreatorPacket::new();
        let created = creatorpacket.create(&setid);

        self.creatorpacket = Some(Rc::new(RefCell::new(creatorpacket)));

        created
    }

    // -------------------------------CreateSourceBlocks-------------------------------

    /// Initialise all of the source blocks ready to start reading data
    /// from the source files. C++ Par2Creator::CreateSourceBlocks.
    fn create_source_blocks(&mut self) -> bool {
        // Allocate the array of source blocks
        self.sourceblocks = vec![DataBlock::new(); self.sourceblockcount as usize];

        let mut start = 0usize;
        for sourcefile in &self.sourcefiles {
            // Allocate the appropriate number of source blocks to each
            // source file.
            let count = sourcefile.block_count() as usize;
            sourcefile.initialise_source_blocks(
                &mut self.sourceblocks[start..start + count],
                self.blocksize,
            );
            start += count;
        }

        true
    }

    // ------------------------------InitialiseOutputFiles------------------------------

    /// Create all of the output files and allocate all packets to
    /// appropriate file offsets. C++ Par2Creator::InitialiseOutputFiles.
    fn initialise_output_files(&mut self, parfilename: &str, serr: &mut dyn Write) -> bool {
        // Allocate the recovery packets
        self.recoverypackets = (0..self.recoveryblockcount)
            .map(|_| RecoveryPacket::new())
            .collect();

        // Choose filenames and decide which recovery blocks to place in
        // each file
        let mut fileallocations: Vec<FileAllocation> =
            vec![FileAllocation::default(); self.recoveryfilecount as usize + 1]; // One extra file with no recovery blocks
        {
            let recoveryfilecount = self.recoveryfilecount;
            let recoveryblockcount = self.recoveryblockcount;

            // Decide how many recovery blocks to place in each file
            let mut exponent = self.firstrecoveryblock;
            // A non zero count, as the C++ guard below establishes.
            if let Some(nonzero) = std::num::NonZeroU32::new(recoveryfilecount) {
                let recoveryfilecount = nonzero.get();
                match self.recoveryfilescheme {
                    Scheme::Unknown => {
                        // The C++ original asserts here; the command line
                        // parser guarantees a scheme was chosen.
                        return false;
                    }
                    Scheme::Uniform => {
                        // Files will have roughly the same number of
                        // recovery blocks each.

                        let base = recoveryblockcount / recoveryfilecount;
                        let remainder = recoveryblockcount % recoveryfilecount;

                        for (filenumber, fileallocation) in fileallocations
                            .iter_mut()
                            .enumerate()
                            .take(recoveryfilecount as usize)
                        {
                            fileallocation.exponent = exponent;
                            fileallocation.count = if (filenumber as u32) < remainder {
                                base + 1
                            } else {
                                base
                            };
                            exponent += fileallocation.count;
                        }
                    }

                    Scheme::Variable => {
                        // Files will have recovery blocks allocated in an
                        // exponential fashion.

                        // Work out how many blocks to place in the
                        // smallest file. The C++ original shifts 32 bit
                        // values; saturating 64 bit arithmetic gives the
                        // same result without ever wrapping.
                        let mut lowblockcount = 1u64;
                        let mut maxrecoveryblocks = (1u64 << recoveryfilecount.min(63)) - 1;
                        while maxrecoveryblocks < recoveryblockcount as u64 {
                            lowblockcount = lowblockcount.saturating_mul(2);
                            maxrecoveryblocks = maxrecoveryblocks.saturating_mul(2);
                        }

                        // Allocate the blocks.
                        let mut blocks = recoveryblockcount as u64;
                        for fileallocation in fileallocations
                            .iter_mut()
                            .take(recoveryfilecount as usize)
                        {
                            let number = min(lowblockcount, blocks) as u32;
                            fileallocation.exponent = exponent;
                            fileallocation.count = number;
                            exponent += number;
                            blocks -= number as u64;
                            lowblockcount = lowblockcount.saturating_mul(2);
                        }
                    }

                    Scheme::Limited => {
                        // Files will be allocated in an exponential
                        // fashion but the Maximum file size will be
                        // limited.

                        let largest = self.largestfilesize.div_ceil(self.blocksize) as u32;
                        let mut filenumber = recoveryfilecount;
                        let mut blocks = recoveryblockcount;

                        exponent = self.firstrecoveryblock + recoveryblockcount;

                        // Allocate uniformly at the top
                        while blocks >= 2 * largest && filenumber > 0 {
                            filenumber -= 1;
                            exponent -= largest;
                            blocks -= largest;

                            fileallocations[filenumber as usize].exponent = exponent;
                            fileallocations[filenumber as usize].count = largest;
                        }
                        debug_assert!(blocks > 0 && filenumber > 0);

                        exponent = self.firstrecoveryblock;
                        let mut count = 1u32;
                        let files = filenumber;

                        // Allocate exponentially at the bottom
                        let mut filenumber = 0u32;
                        while filenumber < files {
                            let number = min(count, blocks);
                            fileallocations[filenumber as usize].exponent = exponent;
                            fileallocations[filenumber as usize].count = number;

                            exponent += number;
                            blocks -= number;
                            count = count.saturating_mul(2);
                            filenumber += 1;
                        }
                    }
                }
            }

            // There will be an extra file with no recovery blocks.
            fileallocations[recoveryfilecount as usize].exponent = exponent;
            fileallocations[recoveryfilecount as usize].count = 0;

            // Determine digit widths for recovery filenames
            let mut digitslow = 1usize;
            let mut digitscount = 1usize;
            {
                let mut limitlow = 0u32;
                let mut limitcount = 0u32;
                for fileallocation in fileallocations.iter() {
                    if limitlow < fileallocation.exponent {
                        limitlow = fileallocation.exponent;
                    }
                    if limitcount < fileallocation.count {
                        limitcount = fileallocation.count;
                    }
                }

                let mut t = limitlow;
                while t >= 10 {
                    digitslow += 1;
                    t /= 10;
                }

                let mut t = limitcount;
                while t >= 10 {
                    digitscount += 1;
                    t /= 10;
                }
            }

            // Set the filenames
            for fileallocation in fileallocations
                .iter_mut()
                .take(recoveryfilecount as usize)
            {
                let filename = format!(
                    "{parfilename}.vol{:0digitslow$}+{:0digitscount$}.par2",
                    fileallocation.exponent, fileallocation.count
                );

                if filename.len() > MAX_PATH_LEN {
                    writeln!(
                        serr,
                        "{filename} pathlength is more than {MAX_PATH_LEN}."
                    )
                    .expect("cannot write to serr");
                    return false;
                }
                fileallocation.filename = filename;
            }

            let mainpar = format!("{parfilename}.par2");
            if mainpar.len() > MAX_PATH_LEN {
                writeln!(serr, "{mainpar} pathlength is more than {MAX_PATH_LEN}.")
                    .expect("cannot write to serr");
                return false;
            }
            fileallocations[recoveryfilecount as usize].filename = mainpar;
        }

        // Allocate the recovery files
        {
            self.recoveryfiles = (0..=self.recoveryfilecount)
                .map(|_| Rc::new(RefCell::new(DiskFile::new())))
                .collect();

            // Sort critical packets, so we get consistency.
            self.criticalpackets.sort_by(|left, right| {
                match (left.compare_less(right), right.compare_less(left)) {
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => Ordering::Equal,
                }
            });

            // Allocate packets to the output files
            {
                let setid = self
                    .mainpacket
                    .as_ref()
                    .expect("create_main_packet first")
                    .borrow()
                    .setid();
                let creatorpacket = Rc::clone(
                    self.creatorpacket
                        .as_ref()
                        .expect("create_creator_packet first"),
                );
                let creatorpacketlength =
                    creatorpacket.borrow().critical().packet_length() as u64;

                // The next recovery packet to create
                let mut packet = 0usize;

                // For each recovery file:
                for (fileindex, fileallocation) in fileallocations.iter().enumerate() {
                    // How many recovery blocks in this file
                    let count = fileallocation.count;

                    let recoveryfile = Rc::clone(&self.recoveryfiles[fileindex]);

                    // start at the beginning of the recovery file
                    let mut offset = 0u64;

                    if count == 0 {
                        // Write one set of critical packets
                        for criticalpacket in &self.criticalpackets {
                            let packetlength = criticalpacket.packet_length();
                            self.criticalpacketentries.push(CriticalPacketEntry {
                                diskfile: Rc::clone(&recoveryfile),
                                offset,
                                packet: criticalpacket.clone(),
                            });
                            offset += packetlength;
                        }
                    } else {
                        // How many copies of each critical packet
                        let mut copies = 0u32;
                        let mut t = count;
                        while t > 0 {
                            copies += 1;
                            t >>= 1;
                        }

                        // Get ready to iterate through the critical
                        // packets. None stands for the end of the list.
                        let mut packetcount = 0u64;
                        let mut nextcriticalpacket: Option<usize> = None;

                        // What is the first exponent
                        let mut exponent = fileallocation.exponent;

                        // Start allocating the recovery packets
                        let limit = exponent + count;
                        while exponent < limit {
                            // Add the next recovery packet
                            self.recoverypackets[packet].create(
                                Rc::clone(&recoveryfile),
                                offset,
                                self.blocksize,
                                exponent,
                                &setid,
                            );

                            offset += self.recoverypackets[packet].packet_length();
                            packet += 1;
                            exponent += 1;

                            // Add some critical packets
                            packetcount += copies as u64 * self.criticalpackets.len() as u64;
                            while packetcount >= count as u64 {
                                // If the list ran out, start again at the
                                // beginning.
                                let index = nextcriticalpacket.unwrap_or(0);
                                let criticalpacket = self.criticalpackets[index].clone();
                                let packetlength = criticalpacket.packet_length();
                                self.criticalpacketentries.push(CriticalPacketEntry {
                                    diskfile: Rc::clone(&recoveryfile),
                                    offset,
                                    packet: criticalpacket,
                                });
                                offset += packetlength;

                                nextcriticalpacket =
                                    if index + 1 < self.criticalpackets.len() {
                                        Some(index + 1)
                                    } else {
                                        None
                                    };

                                packetcount -= count as u64;
                            }
                        }
                    }

                    // Add one copy of the creator packet
                    self.criticalpacketentries.push(CriticalPacketEntry {
                        diskfile: Rc::clone(&recoveryfile),
                        offset,
                        packet: CriticalPacketHandle::Creator(Rc::clone(&creatorpacket)),
                    });
                    offset += creatorpacketlength;

                    // Create the file on disk and make it the required size
                    if !recoveryfile
                        .borrow_mut()
                        .create(&fileallocation.filename, offset, serr)
                    {
                        return false;
                    }
                }
            }
        }

        true
    }
    // --------------------------------AllocateBuffers--------------------------------

    /// Allocate memory buffers for reading and writing data to disk. C++
    /// Par2Creator::AllocateBuffers.
    fn allocate_buffers(&mut self, memorylimit: usize, serr: &mut dyn Write) -> bool {
        self.transferbuffer = vec![0u8; self.chunksize as usize * NUM_TRANSFER_BUFFERS];
        self.outputbuffer = vec![0u8; self.chunksize as usize];

        let config = ProcessorConfig {
            numthreads: self.totalthreads,
            memorylimit,
        };

        // The processor multiplies with the stateless multiply_add alone,
        // so a snapshot of the matrix, which is only solved afterwards,
        // serves it just as the C++ reference processor's reference to it
        // does.
        let mut processor = self
            .backends
            .build_processor(&config, Arc::new(self.rs.clone()));

        if !processor.init(self.chunksize as usize, self.recoveryblockcount) {
            writeln!(serr, "Could not allocate buffer memory.").expect("cannot write to serr");
            return false;
        }

        self.processor = Some(processor);

        true
    }

    // --------------------------------ComputeRSMatrix--------------------------------

    /// Compute the Reed Solomon matrix. C++ Par2Creator::ComputeRSMatrix.
    fn compute_rs_matrix(&mut self, sout: &SharedSout, serr: &mut dyn Write) -> bool {
        // Set the number of input blocks
        if let Err(msg) = self.rs.set_input_all(self.sourceblockcount) {
            writeln!(serr, "{msg}").expect("cannot write to serr");
            return false;
        }

        // Set the number of output blocks to be created. They are all
        // marked missing: they are what this run computes.
        if !self.rs.set_output_range(
            false,
            self.firstrecoveryblock as u16,
            (self.firstrecoveryblock + self.recoveryblockcount - 1) as u16,
        ) {
            return false;
        }

        // Compute the RS matrix
        if let Err(msg) = self.rs.compute(self.noiselevel, sout) {
            writeln!(serr, "{msg}").expect("cannot write to serr");
            return false;
        }

        true
    }

    // ---------------------------------ProcessData---------------------------------

    /// Read source data, process it through the RS matrix and write it to
    /// disk. C++ Par2Creator::ProcessData.
    fn process_data(
        &mut self,
        sout: &SharedSout,
        serr: &mut dyn Write,
        blockoffset: u64,
        blocklength: usize,
        progress: &ProgressMeter,
    ) -> bool {
        let recoveryblockcount = self.recoveryblockcount;

        {
            let processor = self
                .processor
                .as_mut()
                .expect("process_data requires a processor");
            processor.set_chunk_length(blocklength);
            processor.reset_output();
        }

        // Offer the exponents, so that an implementation able to work out
        // its own coefficients is not made to read a column out of the
        // matrix.
        let exponents: Vec<u16> = (0..recoveryblockcount)
            .map(|recoveryblock| (self.firstrecoveryblock + recoveryblock) as u16)
            .collect();
        let ownfactors = {
            let processor = self
                .processor
                .as_mut()
                .expect("process_data requires a processor");
            processor.offer_recovery_exponents(self.sourceblocks.len() as u32, &exponents)
        };

        // The matrix column for one input block, unused when the
        // processor has its own. There is one for each transfer buffer,
        // so that a column lasts as long as the block it belongs to.
        let mut factors: Vec<u16> = if ownfactors {
            Vec::new()
        } else {
            vec![0; recoveryblockcount as usize * NUM_TRANSFER_BUFFERS]
        };

        // Every buffer starts free: the receiver of its last submission
        // while one is still in flight.
        let mut bufferfree: Vec<Option<Receiver<()>>> =
            (0..NUM_TRANSFER_BUFFERS).map(|_| None).collect();
        let mut bufferindex = 0usize;

        // If we have deferred computation of the file hash and block crc
        // and hashes sourcefile and sourceindex will be used to update
        // them during the main recovery block computation
        let mut sourcefile = 0usize;
        let mut sourceindex = 0u32;

        let mut lastopenfile: Option<Rc<RefCell<DiskFile>>> = None;
        let mut failed = false;

        // For each input block
        for inputblock in 0..self.sourceblocks.len() {
            // Are we reading from a new file?
            {
                let current = self.sourceblocks[inputblock].diskfile();
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
            if !self.sourceblocks[inputblock]
                .read_data(blockoffset, &mut self.transferbuffer[start..start + blocklength], serr)
            {
                failed = true;
                break;
            }

            if self.deferhashcomputation {
                debug_assert!(blockoffset == 0 && blocklength as u64 == self.blocksize);
                debug_assert!(sourcefile < self.sourcefiles.len());

                self.sourcefiles[sourcefile].update_hashes(
                    sourceindex,
                    &self.transferbuffer[start..start + blocklength],
                    blocklength,
                );
            }

            // Look up the matrix column and process the data against
            // every output block
            let columnstart = bufferindex * recoveryblockcount as usize;

            if !ownfactors {
                for outputblock in 0..recoveryblockcount as usize {
                    factors[columnstart + outputblock] =
                        self.rs.get_factor(inputblock as u32, outputblock as u32);
                }
            }

            {
                let processor = self
                    .processor
                    .as_mut()
                    .expect("process_data requires a processor");

                processor.wait_for_add();

                let column: Option<&[u16]> = if ownfactors {
                    None
                } else {
                    Some(&factors[columnstart..columnstart + recoveryblockcount as usize])
                };

                bufferfree[bufferindex] = Some(processor.add_input(
                    &self.transferbuffer[start..start + blocklength],
                    inputblock as u32,
                    column,
                ));
            }
            bufferindex = (bufferindex + 1) % NUM_TRANSFER_BUFFERS;

            if self.noiselevel > NoiseLevel::Quiet {
                progress.add(blocklength as u64);
            }

            // Work out which source file the next block belongs to
            if sourceindex + 1 >= self.sourcefiles[sourcefile].block_count() {
                sourceindex = 0;
                sourcefile += 1;
            } else {
                sourceindex += 1;
            }
        }

        self.processor
            .as_mut()
            .expect("process_data requires a processor")
            .end_input();

        // Close the last file
        if let Some(lastopenfile) = &lastopenfile {
            lastopenfile.borrow_mut().close();
        }

        if failed {
            return false;
        }

        if self.noiselevel > NoiseLevel::Quiet {
            sout_write(sout, "Writing recovery packets\r");
        }

        // For each output block
        for outputblock in 0..recoveryblockcount as usize {
            // Take the accumulated output block from the processor
            let processor = self
                .processor
                .as_mut()
                .expect("process_data requires a processor");
            let outbuf: &[u8] = match processor.peek_output(outputblock as u32) {
                Some(buf) => buf,
                None => {
                    if !processor
                        .get_output(outputblock as u32, &mut self.outputbuffer[..blocklength])
                    {
                        writeln!(
                            serr,
                            "Could not read the recovery data back from the processor."
                        )
                        .expect("cannot write to serr");
                        return false;
                    }
                    &self.outputbuffer[..blocklength]
                }
            };

            // Write the data to the recovery packet
            if !self.recoverypackets[outputblock].write_data(blockoffset, outbuf, serr) {
                return false;
            }
        }

        if self.noiselevel > NoiseLevel::Quiet {
            sout_line(
                sout,
                &format!(
                    "Wrote {} bytes to disk",
                    recoveryblockcount as u64 * blocklength as u64
                ),
            );
        }

        true
    }

    // -----------------------------WriteRecoveryPacketHeaders-----------------------------

    /// Finish computation of the recovery packets and write the headers to
    /// disk. C++ Par2Creator::WriteRecoveryPacketHeaders.
    fn write_recovery_packet_headers(&mut self, serr: &mut dyn Write) -> bool {
        // For each recovery packet
        for recoverypacket in &mut self.recoverypackets {
            // Finish the packet header and write it to disk
            if !recoverypacket.write_header(serr) {
                return false;
            }
        }

        true
    }

    // -----------------------------FinishFileHashComputation-----------------------------

    /// Finish computing the full file hash values of the source files.
    /// C++ Par2Creator::FinishFileHashComputation.
    fn finish_file_hash_computation(&mut self) -> bool {
        // If we deferred the computation of the full file hash, then we
        // finish it now
        if self.deferhashcomputation {
            // For each source file
            for sourcefile in &mut self.sourcefiles {
                sourcefile.finish_hashes();
            }
        }

        true
    }

    // -------------------------------FinishCriticalPackets-------------------------------

    /// Fill in all remaining details in the critical packets. C++
    /// Par2Creator::FinishCriticalPackets.
    fn finish_critical_packets(&mut self) -> bool {
        // Get the setid from the main packet
        let setid = self
            .mainpacket
            .as_ref()
            .expect("create_main_packet first")
            .borrow()
            .setid();

        // Store the setid in each of the critical packets and compute the
        // packet_hash of each one.
        for criticalpacket in &self.criticalpackets {
            criticalpacket.finish_packet(&setid);
        }

        true
    }

    // --------------------------------WriteCriticalPackets--------------------------------

    /// Write all other critical packets to disk. C++
    /// Par2Creator::WriteCriticalPackets.
    fn write_critical_packets(&self, serr: &mut dyn Write) -> bool {
        // For each critical packet
        for packetentry in &self.criticalpacketentries {
            // Write it to disk
            if !packetentry.packet.write_packet(
                &mut packetentry.diskfile.borrow_mut(),
                packetentry.offset,
                serr,
            ) {
                return false;
            }
        }

        true
    }

    // -----------------------------------CloseFiles-----------------------------------

    /// Close all files. C++ Par2Creator::CloseFiles.
    fn close_files(&self) -> bool {
        // Close each recovery file.
        for recoveryfile in &self.recoveryfiles {
            recoveryfile.borrow_mut().close();
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::par2repairer::Par2Repairer;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU32;
    use std::sync::Mutex;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "par2rust_creator_{tag}_{}_{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn shared_sout() -> SharedSout {
        let writer: Box<dyn Write + Send> = Box::new(Vec::<u8>::new());
        Arc::new(Mutex::new(writer))
    }

    // Deterministic pseudo random file content, reproducible across runs.
    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        let mut x = seed.wrapping_mul(31).wrapping_add(7);
        (0..len)
            .map(|i| {
                x = x
                    .wrapping_mul(13)
                    .wrapping_add(i as u8)
                    .wrapping_add(3);
                x
            })
            .collect()
    }

    // The parameters a create run is given.
    struct Create {
        blocksize: u64,
        firstblock: u32,
        scheme: Scheme,
        filecount: u32,
        blockcount: u32,
        memorylimit: usize,
    }

    impl Create {
        fn new(blocksize: u64, blockcount: u32) -> Create {
            Create {
                blocksize,
                firstblock: 0,
                scheme: Scheme::Variable,
                filecount: 0,
                blockcount,
                memorylimit: 16 * 1024 * 1024,
            }
        }
    }

    // Write the source files and create a PAR 2.0 set for them. Returns
    // the base path (with trailing separator) used for both runs.
    fn create(dir: &Path, files: &[(&str, &[u8])], params: &Create) -> String {
        let mut basepath = dir.to_str().unwrap().to_string();
        basepath.push(std::path::MAIN_SEPARATOR);

        let extrafiles: Vec<String> = files
            .iter()
            .map(|(name, data)| {
                let path = dir.join(name);
                std::fs::write(&path, data).unwrap();
                path.to_str().unwrap().to_string()
            })
            .collect();

        let parfilename = dir.join("test").to_str().unwrap().to_string();
        let mut creator = Par2Creator::new(NoiseLevel::Silent, Backends::default());
        let sout = shared_sout();
        let mut serr = Vec::new();
        let result = creator.process(
            &sout,
            &mut serr,
            params.memorylimit,
            &basepath,
            1,
            1,
            &parfilename,
            &extrafiles,
            params.blocksize,
            params.firstblock,
            params.scheme,
            params.filecount,
            params.blockcount,
        );
        assert_eq!(
            result,
            Result::Success,
            "create failed: {}",
            String::from_utf8_lossy(&serr)
        );

        basepath
    }

    // Run the repairer over the set in the directory.
    fn repair(dir: &Path, basepath: &str, dorepair: bool) -> Result {
        let parfile = dir.join("test.par2");
        let mut repairer = Par2Repairer::new(NoiseLevel::Silent, Backends::default());
        let sout = shared_sout();
        let mut serr = Vec::new();
        let result = repairer.process(
            &sout,
            &mut serr,
            16 * 1024 * 1024,
            basepath,
            1,
            1,
            parfile.to_str().unwrap().to_string(),
            &[],
            dorepair,
            false,
            false,
            false,
            0,
            false,
        );
        assert!(
            result == Result::Success || result == Result::RepairPossible,
            "repair failed: {}",
            String::from_utf8_lossy(&serr)
        );
        result
    }

    // The names of the files of the set, sorted.
    fn par_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_string())
            .filter(|name| name.ends_with(".par2"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn round_trip_all_correct() {
        let dir = temp_dir("ok");
        let alpha = pattern(3000, 1);
        let beta = pattern(1500, 2);
        let basepath = create(
            &dir,
            &[("alpha.bin", &alpha), ("beta.bin", &beta)],
            &Create::new(1024, 3),
        );

        assert!(dir.join("test.par2").exists());
        assert!(par_files(&dir).len() > 1);

        assert_eq!(repair(&dir, &basepath, false), Result::Success);
    }

    #[test]
    fn round_trip_repair_missing_file() {
        let dir = temp_dir("missing");
        let alpha = pattern(3000, 3);
        let beta = pattern(1500, 4);
        let basepath = create(
            &dir,
            &[("alpha.bin", &alpha), ("beta.bin", &beta)],
            &Create::new(1024, 3),
        );

        std::fs::remove_file(dir.join("beta.bin")).unwrap();

        assert_eq!(repair(&dir, &basepath, true), Result::Success);
        assert_eq!(std::fs::read(dir.join("beta.bin")).unwrap(), beta);
    }

    #[test]
    fn round_trip_repair_damaged_file() {
        let dir = temp_dir("damaged");
        let alpha = pattern(3000, 5);
        let beta = pattern(1500, 6);
        let basepath = create(
            &dir,
            &[("alpha.bin", &alpha), ("beta.bin", &beta)],
            &Create::new(1024, 3),
        );

        // Damage one byte inside the file's second block.
        let mut damaged = alpha.clone();
        damaged[1500] ^= 0xff;
        std::fs::write(dir.join("alpha.bin"), &damaged).unwrap();

        assert_eq!(repair(&dir, &basepath, true), Result::Success);
        assert_eq!(std::fs::read(dir.join("alpha.bin")).unwrap(), alpha);
    }

    #[test]
    fn uniform_scheme_filenames() {
        let dir = temp_dir("uniform");
        let alpha = pattern(5000, 7);
        let mut params = Create::new(1024, 10);
        params.scheme = Scheme::Uniform;
        params.filecount = 3;
        let basepath = create(&dir, &[("alpha.bin", &alpha)], &params);

        // Ten blocks over three files: 4 + 3 + 3, starting at exponent
        // 0. The exponent is written with two digits because the main
        // file would follow at exponent 10.
        assert_eq!(
            par_files(&dir),
            vec![
                "test.par2".to_string(),
                "test.vol00+4.par2".to_string(),
                "test.vol04+3.par2".to_string(),
                "test.vol07+3.par2".to_string(),
            ]
        );

        assert_eq!(repair(&dir, &basepath, false), Result::Success);
    }

    #[test]
    fn limited_scheme_repair() {
        let dir = temp_dir("limited");
        let alpha = pattern(5000, 8);
        let beta = pattern(2500, 9);
        let mut params = Create::new(1024, 10);
        params.scheme = Scheme::Limited;
        params.filecount = 4;
        let basepath = create(&dir, &[("alpha.bin", &alpha), ("beta.bin", &beta)], &params);

        std::fs::remove_file(dir.join("alpha.bin")).unwrap();

        assert_eq!(repair(&dir, &basepath, true), Result::Success);
        assert_eq!(std::fs::read(dir.join("alpha.bin")).unwrap(), alpha);
    }

    #[test]
    fn no_recovery_blocks() {
        let dir = temp_dir("norecovery");
        let alpha = pattern(3000, 10);
        let basepath = create(&dir, &[("alpha.bin", &alpha)], &Create::new(1024, 0));

        // Only the main file, holding the critical packets.
        assert_eq!(par_files(&dir), vec!["test.par2".to_string()]);

        assert_eq!(repair(&dir, &basepath, false), Result::Success);
    }

    #[test]
    fn multi_pass_small_memory() {
        let dir = temp_dir("multipass");
        let alpha = pattern(4096, 11);
        let beta = pattern(4096, 12);

        // A memory limit that forces the block to be processed in
        // several chunks, so that the file hashes cannot be computed
        // while the recovery data is accumulated.
        let mut params = Create::new(4096, 4);
        params.memorylimit = 8192;
        let basepath = create(&dir, &[("alpha.bin", &alpha), ("beta.bin", &beta)], &params);

        // Damage a byte in the second half of each file.
        let mut damaged = alpha.clone();
        damaged[3000] ^= 0xff;
        std::fs::write(dir.join("alpha.bin"), &damaged).unwrap();
        let mut damaged = beta.clone();
        damaged[100] ^= 0xff;
        std::fs::write(dir.join("beta.bin"), &damaged).unwrap();

        assert_eq!(repair(&dir, &basepath, true), Result::Success);
        assert_eq!(std::fs::read(dir.join("alpha.bin")).unwrap(), alpha);
        assert_eq!(std::fs::read(dir.join("beta.bin")).unwrap(), beta);
    }

    #[test]
    fn first_recovery_block() {
        let dir = temp_dir("firstblock");
        let alpha = pattern(3000, 13);
        let mut params = Create::new(1024, 3);
        params.firstblock = 16;
        let basepath = create(&dir, &[("alpha.bin", &alpha)], &params);

        // The exponents start at 16, and the file names show them.
        assert!(par_files(&dir)
            .iter()
            .any(|name| name.starts_with("test.vol16+")));

        std::fs::remove_file(dir.join("alpha.bin")).unwrap();

        assert_eq!(repair(&dir, &basepath, true), Result::Success);
        assert_eq!(std::fs::read(dir.join("alpha.bin")).unwrap(), alpha);
    }

    #[test]
    fn short_last_block() {
        let dir = temp_dir("shortblock");
        // A file whose last block is only a few bytes long.
        let alpha = pattern(1025, 14);
        let basepath = create(&dir, &[("alpha.bin", &alpha)], &Create::new(1024, 2));

        std::fs::remove_file(dir.join("alpha.bin")).unwrap();

        assert_eq!(repair(&dir, &basepath, true), Result::Success);
        assert_eq!(std::fs::read(dir.join("alpha.bin")).unwrap(), alpha);
    }

    #[test]
    fn empty_source_file() {
        let dir = temp_dir("empty");
        let alpha = pattern(2000, 15);
        let empty: Vec<u8> = Vec::new();
        let basepath = create(&dir, &[("alpha.bin", &alpha), ("empty.bin", &empty)], &Create::new(1024, 2));

        std::fs::remove_file(dir.join("empty.bin")).unwrap();

        assert_eq!(repair(&dir, &basepath, true), Result::Success);
        assert_eq!(std::fs::read(dir.join("empty.bin")).unwrap(), empty);
    }
}
