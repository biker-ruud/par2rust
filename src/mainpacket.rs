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

//! Port of par2cmdline's mainpacket.h / mainpacket.cpp.
//!
//! The main packet ties all other critical packets together.
//! It specifies the block size to use for both verification of
//! files and for the Reed Solomon computation.
//! It also specifies how many of the source files are repairable
//! and in what order they should be processed.
//!
//! C++ `Create` takes the creator's source file list and sorts it in place
//! by fileid; this port takes the fileids themselves and sorts them in place
//! (`Par2CreatorSourceFile::CompareLess` compares nothing but the fileid).

use std::io::Write;

use crate::criticalpacket::CriticalPacket;
use crate::diskfile::DiskFile;
use crate::md5::{Md5Context, Md5Hash};
use crate::par2fileformat::{
    self, HDR_MAGIC_OFFSET, HDR_SETID_OFFSET, HDR_TYPE_OFFSET, MAINPACKET_BLOCKSIZE_OFFSET,
    MAINPACKET_FILEID_OFFSET, MAINPACKET_RECOVERABLEFILECOUNT_OFFSET, MAINPACKET_SIZE,
    MAINPACKET_TYPE, MD5HASH_SIZE, PACKET_HEADER_SIZE, PACKET_MAGIC,
};

pub struct MainPacket {
    critical: CriticalPacket,

    blocksize: u64,
    totalfilecount: u32,
    recoverablefilecount: u32,
}

impl MainPacket {
    pub fn new() -> MainPacket {
        MainPacket {
            critical: CriticalPacket::new(),
            blocksize: 0,
            totalfilecount: 0,
            recoverablefilecount: 0,
        }
    }

    pub fn critical(&self) -> &CriticalPacket {
        &self.critical
    }

    pub fn critical_mut(&mut self) -> &mut CriticalPacket {
        &mut self.critical
    }

    /// Construct the main packet from the source file list and block size.
    /// "fileids" will be sorted based on their Md5Hash value.
    pub fn create(&mut self, fileids: &mut [Md5Hash], blocksize: u64) -> bool {
        self.recoverablefilecount = fileids.len() as u32;
        self.totalfilecount = self.recoverablefilecount;
        self.blocksize = blocksize;

        // Allocate memory for the main packet with enough fileid entries
        let packetlength = MAINPACKET_SIZE + fileids.len() * MD5HASH_SIZE;
        self.critical.allocate_packet(packetlength, 0);

        // Record the details we already know in the packet
        {
            let data = self.critical.data_mut();
            data[HDR_MAGIC_OFFSET..HDR_MAGIC_OFFSET + 8].copy_from_slice(&PACKET_MAGIC);
            par2fileformat::set_packet_length(data, packetlength as u64);
            data[HDR_TYPE_OFFSET..HDR_TYPE_OFFSET + 16].copy_from_slice(&MAINPACKET_TYPE);

            data[MAINPACKET_BLOCKSIZE_OFFSET..MAINPACKET_BLOCKSIZE_OFFSET + 8]
                .copy_from_slice(&blocksize.to_le_bytes());
            data[MAINPACKET_RECOVERABLEFILECOUNT_OFFSET
                ..MAINPACKET_RECOVERABLEFILECOUNT_OFFSET + 4]
                .copy_from_slice(&self.totalfilecount.to_le_bytes());
        }

        // Sort the source files according to their fileid values
        if self.totalfilecount > 1 {
            fileids.sort();
        }

        // Store the fileid values in the main packet
        {
            let data = self.critical.data_mut();
            for (i, fileid) in fileids.iter().enumerate() {
                let offset = MAINPACKET_FILEID_OFFSET + i * MD5HASH_SIZE;
                data[offset..offset + MD5HASH_SIZE].copy_from_slice(&fileid.hash);
            }
        }

        // Compute the set_id_hash
        let data = self.critical.data();
        let mut setidcontext = Md5Context::new();
        setidcontext.update(&data[MAINPACKET_BLOCKSIZE_OFFSET..packetlength]);
        let setid = setidcontext.finalize();
        par2fileformat::set_packet_setid(self.critical.data_mut(), &setid);

        // Compute the packet_hash (over the packet including the set_id_hash)
        let data = self.critical.data();
        let mut packetcontext = Md5Context::new();
        packetcontext.update(&data[HDR_SETID_OFFSET..packetlength]);
        let hash = packetcontext.finalize();

        let data = self.critical.data_mut();
        par2fileformat::set_packet_hash(data, &hash);

        true
    }

    /// Load a main packet from a specified file
    pub fn load(
        &mut self,
        diskfile: &mut DiskFile,
        offset: u64,
        header: &[u8],
        serr: &mut dyn Write,
    ) -> bool {
        let length = par2fileformat::packet_length(header) as usize;

        // Is the packet large enough
        if length < MAINPACKET_SIZE {
            return false;
        }

        // Is there a whole number of fileid values
        if 0 < (length - MAINPACKET_SIZE) % MD5HASH_SIZE {
            return false;
        }

        // Is the packet too large
        if length > MAINPACKET_SIZE + 32768 * MD5HASH_SIZE {
            return false;
        }

        // Compute the total number of entries in the fileid array
        self.totalfilecount = ((length - MAINPACKET_SIZE) / MD5HASH_SIZE) as u32;

        self.critical.allocate_packet(length, 0);

        // Copy the header into the packet
        self.critical.data_mut()[..PACKET_HEADER_SIZE]
            .copy_from_slice(&header[..PACKET_HEADER_SIZE]);

        // Read the rest of the packet from disk
        if !diskfile.read(
            offset + PACKET_HEADER_SIZE as u64,
            &mut self.critical.data_mut()[PACKET_HEADER_SIZE..length],
            serr,
        ) {
            return false;
        }

        // Does the packet have enough fileid values
        self.recoverablefilecount = u32::from_le_bytes(
            self.critical.data()[MAINPACKET_RECOVERABLEFILECOUNT_OFFSET
                ..MAINPACKET_RECOVERABLEFILECOUNT_OFFSET + 4]
                .try_into()
                .unwrap(),
        );
        if self.recoverablefilecount > self.totalfilecount {
            return false;
        }

        // Is the block size valid
        self.blocksize = u64::from_le_bytes(
            self.critical.data()[MAINPACKET_BLOCKSIZE_OFFSET..MAINPACKET_BLOCKSIZE_OFFSET + 8]
                .try_into()
                .unwrap(),
        );
        if self.blocksize == 0 || self.blocksize & 3 != 0 {
            return false;
        }

        true
    }

    /// Get the set id.
    pub fn setid(&self) -> Md5Hash {
        par2fileformat::packet_setid(self.critical.data())
    }

    /// Get the block size.
    pub fn block_size(&self) -> u64 {
        debug_assert!(!self.critical.data().is_empty());

        self.blocksize
    }

    /// Get the file counts.
    pub fn recoverable_file_count(&self) -> u32 {
        debug_assert!(!self.critical.data().is_empty());

        self.recoverablefilecount
    }

    pub fn total_file_count(&self) -> u32 {
        debug_assert!(!self.critical.data().is_empty());

        self.totalfilecount
    }

    /// Get the fileid hash of one of the files
    pub fn fileid(&self, filenumber: u32) -> Md5Hash {
        debug_assert!(!self.critical.data().is_empty());
        debug_assert!(filenumber < self.totalfilecount);

        let offset = MAINPACKET_FILEID_OFFSET + filenumber as usize * MD5HASH_SIZE;
        let mut hash = [0u8; 16];
        hash.copy_from_slice(&self.critical.data()[offset..offset + MD5HASH_SIZE]);
        Md5Hash { hash }
    }
}

impl Default for MainPacket {
    fn default() -> Self {
        MainPacket::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::par2fileformat::{packet_hash, packet_length, packet_magic, packet_type};
    use std::fs;

    fn fileid(n: u8) -> Md5Hash {
        Md5Hash { hash: [n; 16] }
    }

    #[test]
    fn test1_create() {
        // Unsorted input must come out sorted, with the hashes filled in.
        let mut fileids = vec![fileid(3), fileid(1), fileid(2)];
        let mut packet = MainPacket::new();
        assert!(packet.create(&mut fileids, 4096));

        assert_eq!(packet.total_file_count(), 3);
        assert_eq!(packet.recoverable_file_count(), 3);
        assert_eq!(packet.block_size(), 4096);
        assert_eq!(packet.fileid(0), fileid(1));
        assert_eq!(packet.fileid(1), fileid(2));
        assert_eq!(packet.fileid(2), fileid(3));

        let data = packet.critical().data();
        let packetlength = packet.critical().packet_length();
        assert_eq!(packetlength, MAINPACKET_SIZE + 3 * MD5HASH_SIZE);
        assert_eq!(packet_length(data), packetlength as u64);
        assert_eq!(packet_magic(data), PACKET_MAGIC);
        assert_eq!(packet_type(data), MAINPACKET_TYPE);

        // set_id_hash = MD5 of everything from blocksize onwards.
        let mut setidcontext = Md5Context::new();
        setidcontext.update(&data[MAINPACKET_BLOCKSIZE_OFFSET..packetlength]);
        assert_eq!(packet.setid(), setidcontext.finalize());

        // packet_hash = MD5 of everything from setid onwards.
        let mut packetcontext = Md5Context::new();
        packetcontext.update(&data[HDR_SETID_OFFSET..packetlength]);
        assert_eq!(packet_hash(data), packetcontext.finalize());
    }

    #[test]
    fn test2_create_empty() {
        let mut fileids: Vec<Md5Hash> = Vec::new();
        let mut packet = MainPacket::new();
        assert!(packet.create(&mut fileids, 1024));
        assert_eq!(packet.total_file_count(), 0);
        assert_eq!(packet.critical().packet_length(), MAINPACKET_SIZE);
    }

    #[test]
    fn test3_load_round_trip() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_mainpacket_{}.par2", std::process::id()));
        let _ = fs::remove_file(&path);

        let mut fileids = vec![fileid(7), fileid(9)];
        let mut packet = MainPacket::new();
        assert!(packet.create(&mut fileids, 4096));

        let mut diskfile = DiskFile::new();
        assert!(diskfile.create(path.to_str().unwrap(), 0, &mut serr));
        assert!(packet
            .critical()
            .write_packet(&mut diskfile, 0, &mut serr));
        diskfile.close();

        let raw = fs::read(&path).unwrap();
        let mut diskfile = DiskFile::new();
        assert!(diskfile.open(path.to_str().unwrap(), &mut serr));

        let mut loaded = MainPacket::new();
        assert!(loaded.load(&mut diskfile, 0, &raw, &mut serr));
        diskfile.close();

        assert_eq!(loaded.block_size(), 4096);
        assert_eq!(loaded.total_file_count(), 2);
        assert_eq!(loaded.recoverable_file_count(), 2);
        assert_eq!(loaded.fileid(0), fileid(7));
        assert_eq!(loaded.fileid(1), fileid(9));
        assert_eq!(loaded.setid(), packet.setid());

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test4_load_rejects_bad_packets() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_mainpacket_bad_{}.par2", std::process::id()));
        let _ = fs::remove_file(&path);

        // A packet body long enough for any of the rejections below.
        let body_len = MAINPACKET_SIZE + 32769 * MD5HASH_SIZE;
        let mut raw = vec![0u8; body_len];
        raw[..8].copy_from_slice(&PACKET_MAGIC);

        let mut write_and_load = |length: u64, blocksize: u64, recoverable: u32| -> bool {
            raw[8..16].copy_from_slice(&length.to_le_bytes());
            raw[MAINPACKET_BLOCKSIZE_OFFSET..MAINPACKET_BLOCKSIZE_OFFSET + 8]
                .copy_from_slice(&blocksize.to_le_bytes());
            raw[MAINPACKET_RECOVERABLEFILECOUNT_OFFSET
                ..MAINPACKET_RECOVERABLEFILECOUNT_OFFSET + 4]
                .copy_from_slice(&recoverable.to_le_bytes());
            fs::write(&path, &raw).unwrap();

            let mut diskfile = DiskFile::new();
            assert!(diskfile.open(path.to_str().unwrap(), &mut serr));
            let mut packet = MainPacket::new();
            let ok = packet.load(&mut diskfile, 0, &raw[..PACKET_HEADER_SIZE], &mut serr);
            diskfile.close();
            ok
        };

        // Too small.
        assert!(!write_and_load(MAINPACKET_SIZE as u64 - 1, 4096, 0));
        // Not a whole number of fileids.
        assert!(!write_and_load(MAINPACKET_SIZE as u64 + 1, 4096, 0));
        // Too large.
        assert!(!write_and_load(
            MAINPACKET_SIZE as u64 + 32769 * MD5HASH_SIZE as u64,
            4096,
            0
        ));
        // More recoverable files than fileids.
        assert!(!write_and_load(MAINPACKET_SIZE as u64 + 16, 4096, 2));
        // Zero block size.
        assert!(!write_and_load(MAINPACKET_SIZE as u64, 0, 0));
        // Block size not a multiple of 4.
        assert!(!write_and_load(MAINPACKET_SIZE as u64, 4097, 0));
        // A valid one.
        assert!(write_and_load(MAINPACKET_SIZE as u64 + 16, 4096, 1));

        fs::remove_file(&path).unwrap();
    }
}
