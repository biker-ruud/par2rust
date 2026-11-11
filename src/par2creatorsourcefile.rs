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

//! The Par2CreatorSourceFile contains the file verification and file
//! description packet for one source file.

use std::cell::RefCell;
use std::cmp::min;
use std::io::Write;
use std::rc::Rc;

use crate::backends::Backends;
use crate::datablock::DataBlock;
use crate::descriptionpacket::DescriptionPacket;
use crate::diskfile::DiskFile;
use crate::hasher::Hasher;
use crate::md5::Md5Context;
use crate::noiselevel::NoiseLevel;
use crate::progressmeter::ProgressMeter;
use crate::verificationpacket::VerificationPacket;

use crate::crc::crc_update_block;
use crate::crc::crc_update_block_zeros;

/// A critical packet owned by a source file, for the critical packet list.
#[derive(Clone)]
pub enum CriticalPacketRef {
    Description(Rc<RefCell<DescriptionPacket>>),
    Verification(Rc<RefCell<VerificationPacket>>),
}

pub struct Par2CreatorSourceFile {
    descriptionpacket: Option<Rc<RefCell<DescriptionPacket>>>, // The file description packet.
    verificationpacket: Option<Rc<RefCell<VerificationPacket>>>, // The file verification packet.
    diskfile: Option<Rc<RefCell<DiskFile>>>, // The source file

    filesize: u64,      // The size of the source file.
    diskfilename: String, // The filename of the source file on disk.
    parfilename: Vec<u8>, // The filename that will be recorded in the file description packet.

    blockcount: u32, // How many blocks the file will be divided into.

    hasher: Option<Box<dyn Hasher>>, // Hashes the blocks of the file, and the file itself
}

impl Par2CreatorSourceFile {
    pub fn new() -> Par2CreatorSourceFile {
        Par2CreatorSourceFile {
            descriptionpacket: None,
            verificationpacket: None,
            diskfile: None,
            filesize: 0,
            diskfilename: String::new(),
            parfilename: Vec::new(),
            blockcount: 0,
            hasher: None,
        }
    }

    // Open the source file, compute the MD5 Hash of the whole file and the
    // first 16k of the file, and then compute the FileId and store the
    // results in a file description packet and a file verification packet.

    // The C++ original takes the same nine parameters, so they are kept
    // rather than bundled.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &mut self,
        noiselevel: NoiseLevel,
        serr: &mut dyn Write,
        extrafile: &str,
        blocksize: u64,
        deferhashcomputation: bool,
        basepath: &str,
        progress: &ProgressMeter,
        backends: &Backends,
    ) -> bool {
        // Get the filename and filesize
        self.diskfilename = extrafile.to_string();
        self.filesize = DiskFile::get_file_size(extrafile);

        // Work out how many blocks the file will be sliced into
        self.blockcount = self.filesize.div_ceil(blocksize) as u32;

        // Determine what filename to record in the PAR2 files
        self.parfilename =
            DescriptionPacket::translate_filename_from_local_to_par2(
                serr,
                noiselevel,
                extrafile.get(basepath.len()..).unwrap_or("").as_bytes(),
            );

        // Create the Description and Verification packets
        let descriptionpacket = Rc::new(RefCell::new(DescriptionPacket::new()));
        descriptionpacket
            .borrow_mut()
            .create(&self.parfilename, self.filesize);
        self.descriptionpacket = Some(Rc::clone(&descriptionpacket));

        let verificationpacket = Rc::new(RefCell::new(VerificationPacket::new()));
        verificationpacket.borrow_mut().create(self.blockcount);
        self.verificationpacket = Some(Rc::clone(&verificationpacket));

        // Create the diskfile object
        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        self.diskfile = Some(Rc::clone(&diskfile));

        // Open the source file
        if !diskfile
            .borrow_mut()
            .open_with_size(extrafile, self.filesize, serr)
        {
            return false;
        }

        // Do we want to defer the computation of the full file hash, and
        // the block crc and hashes. This is only permitted if there
        // is sufficient memory available to create all recovery blocks
        // in one pass of the source files (i.e. chunksize == blocksize)
        if deferhashcomputation {
            // Initialise a buffer to read the first 16k of the source file
            let mut buffersize = 16 * 1024;
            if buffersize as u64 > self.filesize {
                buffersize = self.filesize as usize;
            }
            let mut buffer = vec![0u8; buffersize];

            // Read the data from the file
            if !diskfile.borrow_mut().read(0, &mut buffer, serr) {
                diskfile.borrow_mut().close();
                return false;
            }

            // Compute the hash of the data read from the file
            let mut context = Md5Context::new();
            context.update(&buffer);
            let hash = context.finalize();

            // Store the hash in the descriptionpacket and compute the file id
            descriptionpacket.borrow_mut().hash16k(&hash);

            // Compute the fileid and store it in the verification packet.
            descriptionpacket.borrow_mut().compute_fileid();
            let fileid = descriptionpacket.borrow().fileid();
            verificationpacket.borrow_mut().set_fileid(&fileid);

            // Allocate a hasher for the block and file hashes computed
            // during the recovery data generation phase
            let mut hasher = backends.build_hasher();

            if !hasher.init(self.filesize, blocksize as usize, true) {
                diskfile.borrow_mut().close();
                return false;
            }
            self.hasher = Some(hasher);
        } else {
            // Initialise a buffer to read the source file
            let mut buffersize = 1024 * 1024;
            if buffersize as u64 > min(blocksize, self.filesize) {
                buffersize = min(blocksize, self.filesize) as usize;
            }
            let mut buffer = vec![0u8; buffersize];

            // Get ready to start reading source file to compute the
            // hashes and crcs
            let mut offset: u64 = 0;
            let mut blocknumber: u32 = 0;
            let mut need = blocksize;

            let mut filecontext = Md5Context::new();
            let mut blockcontext = Md5Context::new();
            let mut blockcrc: u32 = 0;

            // Whilst we have not reached the end of the file
            while offset < self.filesize {
                // Work out how much we can read
                let want = min(self.filesize - offset, buffersize as u64) as usize;

                // Read some data from the file into the buffer
                if !diskfile.borrow_mut().read(offset, &mut buffer[..want], serr) {
                    diskfile.borrow_mut().close();
                    return false;
                }

                // If the new data passes the 16k boundary, compute the 16k
                // hash for the file
                if offset < 16384 && offset + want as u64 >= 16384 {
                    filecontext.update(&buffer[..(16384 - offset) as usize]);

                    let mut temp = filecontext.clone();
                    let hash = temp.finalize();

                    // Store the 16k hash in the file description packet
                    descriptionpacket.borrow_mut().hash16k(&hash);

                    if offset + want as u64 > 16384 {
                        let start = (16384 - offset) as usize;
                        let len = (offset + want as u64) as usize - 16384;
                        filecontext.update(&buffer[start..start + len]);
                    }
                } else {
                    filecontext.update(&buffer[..want]);
                }

                // Get ready to update block hashes and crcs
                let mut used = 0usize;

                // Whilst we have not used all of the data we just read
                while used < want {
                    // How much of it can we use for the current block
                    let use_len = min(need, (want - used) as u64) as usize;

                    blockcrc = !0
                        ^ crc_update_block(!0 ^ blockcrc, &buffer[used..used + use_len]);
                    blockcontext.update(&buffer[used..used + use_len]);

                    used += use_len;
                    need -= use_len as u64;

                    // Have we finished the current block
                    if need == 0 {
                        let blockhash = blockcontext.finalize();

                        // Store the block hash and block crc in the file
                        // verification packet.
                        verificationpacket.borrow_mut().set_block_hash_and_crc(
                            blocknumber,
                            &blockhash,
                            blockcrc,
                        );

                        blocknumber += 1;

                        // More blocks
                        if blocknumber < self.blockcount {
                            need = blocksize;

                            blockcontext.reset();
                            blockcrc = 0;
                        }
                    }
                }

                if noiselevel > NoiseLevel::Quiet {
                    progress.add(want as u64);
                }

                offset += want as u64;
            }

            // Did we finish the last block
            if need > 0 {
                blockcrc = !0 ^ crc_update_block_zeros(!0 ^ blockcrc, need);
                blockcontext.update_zeros(need as usize);

                let blockhash = blockcontext.finalize();

                // Store the block hash and block crc in the file verification
                // packet.
                verificationpacket.borrow_mut().set_block_hash_and_crc(
                    blocknumber,
                    &blockhash,
                    blockcrc,
                );
            }

            // Finish computing the file hash.
            let filehash = filecontext.finalize();

            // Store the file hash in the file description packet.
            descriptionpacket.borrow_mut().hashfull(&filehash);

            // Did we compute the 16k hash.
            if offset < 16384 {
                // Store the 16k hash in the file description packet.
                descriptionpacket.borrow_mut().hash16k(&filehash);
            }

            // Compute the fileid and store it in the verification packet.
            descriptionpacket.borrow_mut().compute_fileid();
            let fileid = descriptionpacket.borrow().fileid();
            verificationpacket.borrow_mut().set_fileid(&fileid);
        }

        true
    }

    pub fn close(&mut self) {
        if let Some(diskfile) = &self.diskfile {
            diskfile.borrow_mut().close();
        }
    }

    pub fn record_critical_packets(&self, criticalpackets: &mut Vec<CriticalPacketRef>) {
        // Add the file description packet and file verification packet to
        // the critical packet list.
        criticalpackets.push(CriticalPacketRef::Description(
            self.descriptionpacket.clone().unwrap(),
        ));
        criticalpackets.push(CriticalPacketRef::Verification(
            self.verificationpacket.clone().unwrap(),
        ));
    }

    pub fn compare_less(left: &Par2CreatorSourceFile, right: &Par2CreatorSourceFile) -> bool {
        // Sort source files based on fileid
        left.descriptionpacket().borrow().fileid()
            < right.descriptionpacket().borrow().fileid()
    }

    pub fn fileid(&self) -> crate::md5::Md5Hash {
        // Get the file id hash
        self.descriptionpacket().borrow().fileid()
    }

    // Allocate the appropriate number of source blocks to the source file
    pub fn initialise_source_blocks(&self, sourceblocks: &mut [DataBlock], blocksize: u64) {
        let diskfile = Rc::clone(self.diskfile());

        for (blocknum, sourceblock) in sourceblocks.iter_mut().enumerate() {
            // Configure each source block to an appropriate offset and
            // length within the source file.
            sourceblock.set_location(Rc::clone(&diskfile), blocknum as u64 * blocksize);
            sourceblock.set_length(min(
                blocksize,
                self.filesize - blocknum as u64 * blocksize,
            ));
            sourceblock.set_filesize(self.filesize);
        }
    }

    // Update the file hash and the block crc and hashes
    pub fn update_hashes(&mut self, blocknumber: u32, buffer: &[u8], length: usize) {
        let hasher = self.hasher.as_mut().unwrap();

        // The block is hashed padded out to its full length, the file only
        // as far as its end
        let len = self.filesize - blocknumber as u64 * length as u64;
        let filelength = if length as u64 > len { len as usize } else { length };

        hasher.submit_blocks(buffer, 1, filelength);

        let mut result = [0u8; 20];
        hasher.collect_blocks(&mut result, 1);

        let blockhash = crate::md5::Md5Hash {
            hash: result[..16].try_into().unwrap(),
        };

        let blockcrc = u32::from_le_bytes(result[16..20].try_into().unwrap());

        // Store the results in the verification packet
        self.verificationpacket
            .as_ref()
            .unwrap()
            .borrow_mut()
            .set_block_hash_and_crc(blocknumber, &blockhash, blockcrc);
    }

    // Finish computation of the full file hash. The 16k hash was stored when
    // the file was opened
    pub fn finish_hashes(&mut self) {
        let hasher = self.hasher.as_mut().unwrap();

        let mut hashfull = [0u8; 16];
        let mut hash16k = [0u8; 16];
        hasher.end_file(&mut hashfull, &mut hash16k);

        let hash = crate::md5::Md5Hash { hash: hashfull };

        // Store it in the description packet
        self.descriptionpacket
            .as_ref()
            .unwrap()
            .borrow_mut()
            .hashfull(&hash);
    }

    // How many blocks does this source file use
    pub fn block_count(&self) -> u32 {
        self.blockcount
    }

    pub fn descriptionpacket(&self) -> &Rc<RefCell<DescriptionPacket>> {
        self.descriptionpacket.as_ref().unwrap()
    }

    pub fn verificationpacket(&self) -> &Rc<RefCell<VerificationPacket>> {
        self.verificationpacket.as_ref().unwrap()
    }

    pub fn diskfile(&self) -> &Rc<RefCell<DiskFile>> {
        self.diskfile.as_ref().unwrap()
    }

    pub fn filesize(&self) -> u64 {
        self.filesize
    }

    pub fn diskfilename(&self) -> &str {
        &self.diskfilename
    }

    pub fn parfilename(&self) -> &[u8] {
        &self.parfilename
    }
}

impl Default for Par2CreatorSourceFile {
    fn default() -> Par2CreatorSourceFile {
        Par2CreatorSourceFile::new()
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

    fn tempname(tag: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "par2rust_pcsf_{}_{}",
            tag,
            std::process::id()
        ));
        path.to_string_lossy().into_owned()
    }

    fn padded(content: &[u8], blocknum: u32, blocksize: usize) -> Vec<u8> {
        let start = blocknum as usize * blocksize;
        let mut block = vec![0u8; blocksize];
        let end = min(start + blocksize, content.len());
        if start < content.len() {
            block[..end - start].copy_from_slice(&content[start..end]);
        }
        block
    }

    fn check_packets(sourcefile: &Par2CreatorSourceFile, content: &[u8], blocksize: usize) {
        let desc = sourcefile.descriptionpacket().borrow();
        assert_eq!(desc.hashfull_().hash, md5(content).hash);
        let first16k = min(content.len(), 16384);
        assert_eq!(desc.hash16k_().hash, md5(&content[..first16k]).hash);

        let ver = sourcefile.verificationpacket().borrow();
        assert_eq!(ver.block_count(), sourcefile.block_count());
        assert_eq!(ver.fileid().hash, desc.fileid().hash);

        for blocknum in 0..sourcefile.block_count() {
            let block = padded(content, blocknum, blocksize);
            let entry = ver.verification_entry(blocknum);
            assert_eq!(entry.hash.hash, md5(&block).hash, "block {blocknum} hash");
            assert_eq!(entry.crc, crc(&block), "block {blocknum} crc");
        }
    }

    #[test]
    fn open_computes_hashes_across_16k_boundary() {
        let name = tempname("boundary");
        let content: Vec<u8> = (0..25000u32).map(|i| (i * 31 + 7) as u8).collect();
        std::fs::write(&name, &content).unwrap();

        let mut serr = std::io::sink();
        let progress = ProgressMeter::new(Box::new(std::io::sink()), "", 25000);
        let backends = Backends::default();

        let mut sourcefile = Par2CreatorSourceFile::new();
        assert!(sourcefile.open(
            NoiseLevel::Normal,
            &mut serr,
            &name,
            10000,
            false,
            "",
            &progress,
            &backends,
        ));

        assert_eq!(sourcefile.block_count(), 3);
        assert_eq!(sourcefile.filesize(), 25000);
        check_packets(&sourcefile, &content, 10000);

        sourcefile.close();
        let _ = std::fs::remove_file(&name);
    }

    #[test]
    fn open_small_file_uses_full_hash_as_16k() {
        let name = tempname("small");
        let content: Vec<u8> = (0..500u32).map(|i| (i * 3 + 1) as u8).collect();
        std::fs::write(&name, &content).unwrap();

        let mut serr = std::io::sink();
        let progress = ProgressMeter::new(Box::new(std::io::sink()), "", 500);
        let backends = Backends::default();

        let mut sourcefile = Par2CreatorSourceFile::new();
        assert!(sourcefile.open(
            NoiseLevel::Quiet,
            &mut serr,
            &name,
            300,
            false,
            "",
            &progress,
            &backends,
        ));

        assert_eq!(sourcefile.block_count(), 2);
        check_packets(&sourcefile, &content, 300);

        {
            let desc = sourcefile.descriptionpacket().borrow();
            assert_eq!(desc.hash16k_().hash, desc.hashfull_().hash);
        }

        sourcefile.close();
        let _ = std::fs::remove_file(&name);
    }

    #[test]
    fn deferred_hashes_match_immediate() {
        let name = tempname("deferred");
        let content: Vec<u8> = (0..25000u32).map(|i| (i * 17 + 11) as u8).collect();
        std::fs::write(&name, &content).unwrap();

        let mut serr = std::io::sink();
        let progress = ProgressMeter::new(Box::new(std::io::sink()), "", 25000);
        let backends = Backends::default();

        let mut sourcefile = Par2CreatorSourceFile::new();
        assert!(sourcefile.open(
            NoiseLevel::Quiet,
            &mut serr,
            &name,
            10000,
            true,
            "",
            &progress,
            &backends,
        ));

        // The 16k hash and fileid are already stored
        let first16k = sourcefile.descriptionpacket().borrow().hash16k_();
        assert_eq!(first16k.hash, md5(&content[..16384]).hash);

        for blocknum in 0..sourcefile.block_count() {
            let block = padded(&content, blocknum, 10000);
            sourcefile.update_hashes(blocknum, &block, 10000);
        }
        sourcefile.finish_hashes();

        check_packets(&sourcefile, &content, 10000);

        sourcefile.close();
        let _ = std::fs::remove_file(&name);
    }

    #[test]
    fn parfilename_strips_basepath() {
        let dir = std::env::temp_dir();
        let name = dir.join("par2rust_pcsf_base").join("sub.bin");
        std::fs::create_dir_all(dir.join("par2rust_pcsf_base")).unwrap();
        std::fs::write(&name, b"hello").unwrap();
        let name = name.to_string_lossy().into_owned();
        let basepath = dir.join("par2rust_pcsf_base").to_string_lossy().into_owned();

        let mut serr = std::io::sink();
        let progress = ProgressMeter::new(Box::new(std::io::sink()), "", 5);
        let backends = Backends::default();

        let mut sourcefile = Par2CreatorSourceFile::new();
        assert!(sourcefile.open(
            NoiseLevel::Quiet,
            &mut serr,
            &name,
            4096,
            false,
            &basepath,
            &progress,
            &backends,
        ));

        let expected = DescriptionPacket::translate_filename_from_local_to_par2(
            &mut serr,
            NoiseLevel::Quiet,
            std::path::MAIN_SEPARATOR_STR.as_bytes(),
        );
        let mut want = expected.clone();
        want.extend_from_slice(b"sub.bin");
        assert_eq!(sourcefile.parfilename(), want.as_slice());

        sourcefile.close();
        let _ = std::fs::remove_file(&name);
        let _ = std::fs::remove_dir(dir.join("par2rust_pcsf_base"));
    }

    #[test]
    fn record_and_compare_and_blocks() {
        let name = tempname("record");
        let content: Vec<u8> = (0..1500u32).map(|i| (i * 5 + 2) as u8).collect();
        std::fs::write(&name, &content).unwrap();

        let mut serr = std::io::sink();
        let progress = ProgressMeter::new(Box::new(std::io::sink()), "", 1500);
        let backends = Backends::default();

        let mut sourcefile = Par2CreatorSourceFile::new();
        assert!(sourcefile.open(
            NoiseLevel::Quiet,
            &mut serr,
            &name,
            1000,
            false,
            "",
            &progress,
            &backends,
        ));

        let mut criticals = Vec::new();
        sourcefile.record_critical_packets(&mut criticals);
        assert_eq!(criticals.len(), 2);
        match &criticals[0] {
            CriticalPacketRef::Description(desc) => {
                assert_eq!(desc.borrow().fileid().hash, sourcefile.fileid().hash);
            }
            _ => panic!("description packet expected first"),
        }

        let mut blocks: Vec<DataBlock> = (0..2).map(|_| DataBlock::new()).collect();
        sourcefile.initialise_source_blocks(&mut blocks, 1000);
        assert_eq!(blocks[0].offset(), 0);
        assert_eq!(blocks[0].length(), 1000);
        assert_eq!(blocks[1].offset(), 1000);
        assert_eq!(blocks[1].length(), 500);
        assert!(blocks[0].diskfile().is_some());

        assert!(!Par2CreatorSourceFile::compare_less(&sourcefile, &sourcefile));

        sourcefile.close();
        let _ = std::fs::remove_file(&name);
    }
}
