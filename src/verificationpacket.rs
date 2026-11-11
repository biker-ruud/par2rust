//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2003 Peter Brian Clements
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

//! Port of par2cmdline's verificationpacket.h / verificationpacket.cpp.
//!
//! The file verification packet stores details that allow individual blocks
//! of valid data within a damaged file to be identified.

use std::io::Write;

use crate::criticalpacket::CriticalPacket;
use crate::diskfile::DiskFile;
use crate::md5::Md5Hash;
use crate::par2fileformat::{
    self, FILEVERIFICATIONENTRY_SIZE, FILEVERIFICATIONPACKET_ENTRIES_OFFSET,
    FILEVERIFICATIONPACKET_FILEID_OFFSET, FILEVERIFICATIONPACKET_SIZE,
    FILEVERIFICATIONPACKET_TYPE, HDR_MAGIC_OFFSET, HDR_TYPE_OFFSET, PACKET_HEADER_SIZE,
    PACKET_MAGIC,
};

/// One verification record: the MD5 hash and CRC of one data block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileVerificationEntry {
    pub hash: Md5Hash,
    pub crc: u32,
}

pub struct VerificationPacket {
    critical: CriticalPacket,
    blockcount: u32,
}

impl VerificationPacket {
    pub fn new() -> VerificationPacket {
        VerificationPacket {
            critical: CriticalPacket::new(),
            blockcount: 0,
        }
    }

    pub fn critical(&self) -> &CriticalPacket {
        &self.critical
    }

    pub fn critical_mut(&mut self) -> &mut CriticalPacket {
        &mut self.critical
    }

    /// Create a packet large enough for the specified number of blocks.
    pub fn create(&mut self, blockcount: u32) -> bool {
        self.blockcount = blockcount;

        // Allocate a packet large enough to hold the required number of
        // verification entries.
        let packetlength =
            FILEVERIFICATIONPACKET_SIZE + blockcount as usize * FILEVERIFICATIONENTRY_SIZE;
        self.critical.allocate_packet(packetlength, 0);

        // Record everything we know in the packet.
        {
            let data = self.critical.data_mut();
            data[HDR_MAGIC_OFFSET..HDR_MAGIC_OFFSET + 8].copy_from_slice(&PACKET_MAGIC);
            par2fileformat::set_packet_length(data, packetlength as u64);
            // packet->header.hash;  // Not known yet
            // packet->header.setid; // Not known yet
            data[HDR_TYPE_OFFSET..HDR_TYPE_OFFSET + 16]
                .copy_from_slice(&FILEVERIFICATIONPACKET_TYPE);
        }
        // packet->fileid;       // Not known yet
        // packet->entries;      // Not known yet

        true
    }

    /// Set the fileid (computed from the file description packet).
    pub fn set_fileid(&mut self, fileid: &Md5Hash) {
        debug_assert!(!self.critical.data().is_empty());

        // Store the fileid in the packet.
        self.critical.data_mut()[FILEVERIFICATIONPACKET_FILEID_OFFSET
            ..FILEVERIFICATIONPACKET_FILEID_OFFSET + 16]
            .copy_from_slice(&fileid.hash);
    }

    /// Set the block hash and block crc for a specific data block.
    pub fn set_block_hash_and_crc(&mut self, blocknumber: u32, hash: &Md5Hash, crc: u32) {
        debug_assert!(!self.critical.data().is_empty());
        debug_assert!(blocknumber < self.blockcount);

        // Store the block hash and block crc in the packet.
        let offset = FILEVERIFICATIONPACKET_ENTRIES_OFFSET
            + blocknumber as usize * FILEVERIFICATIONENTRY_SIZE;
        {
            let data = self.critical.data_mut();
            data[offset..offset + 16].copy_from_slice(&hash.hash);
            data[offset + 16..offset + 20].copy_from_slice(&crc.to_le_bytes());
        }
    }

    /// Load a verification packet from a specified file.
    pub fn load(
        &mut self,
        diskfile: &mut DiskFile,
        offset: u64,
        header: &[u8],
        _serr: &mut dyn Write,
    ) -> bool {
        let length = par2fileformat::packet_length(header) as usize;

        // Is the packet large enough
        if length <= FILEVERIFICATIONPACKET_SIZE {
            return false;
        }

        // Does the packet have a whole number of verification records
        if 0 < (length - FILEVERIFICATIONPACKET_SIZE) % FILEVERIFICATIONENTRY_SIZE {
            return false;
        }

        // Is the packet too large
        if length > FILEVERIFICATIONPACKET_SIZE + 32768 * FILEVERIFICATIONENTRY_SIZE {
            return false;
        }

        // Allocate the packet
        self.critical.allocate_packet(length, 0);

        // Copy the header into the packet
        self.critical.data_mut()[..PACKET_HEADER_SIZE]
            .copy_from_slice(&header[..PACKET_HEADER_SIZE]);

        // How many blocks are there
        self.blockcount =
            ((length - FILEVERIFICATIONPACKET_SIZE) / FILEVERIFICATIONENTRY_SIZE) as u32;

        // Read the rest of the packet
        diskfile.read(
            offset + PACKET_HEADER_SIZE as u64,
            &mut self.critical.data_mut()[PACKET_HEADER_SIZE..length],
            _serr,
        )
    }

    /// Get the FileId.
    pub fn fileid(&self) -> Md5Hash {
        debug_assert!(!self.critical.data().is_empty());

        let mut hash = [0u8; 16];
        hash.copy_from_slice(
            &self.critical.data()[FILEVERIFICATIONPACKET_FILEID_OFFSET
                ..FILEVERIFICATIONPACKET_FILEID_OFFSET + 16],
        );
        Md5Hash { hash }
    }

    /// Get the block count.
    pub fn block_count(&self) -> u32 {
        debug_assert!(!self.critical.data().is_empty());

        self.blockcount
    }

    /// Get a specific verification entry.
    pub fn verification_entry(&self, blocknumber: u32) -> FileVerificationEntry {
        debug_assert!(!self.critical.data().is_empty());

        let offset = FILEVERIFICATIONPACKET_ENTRIES_OFFSET
            + blocknumber as usize * FILEVERIFICATIONENTRY_SIZE;
        let data = self.critical.data();
        let mut hash = [0u8; 16];
        hash.copy_from_slice(&data[offset..offset + 16]);
        let crc = u32::from_le_bytes(data[offset + 16..offset + 20].try_into().unwrap());
        FileVerificationEntry {
            hash: Md5Hash { hash },
            crc,
        }
    }
}

impl Default for VerificationPacket {
    fn default() -> Self {
        VerificationPacket::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::par2fileformat::{packet_length, packet_magic, packet_type};
    use std::fs;

    #[test]
    fn test1_create() {
        let mut packet = VerificationPacket::new();
        assert!(packet.create(3));

        let data = packet.critical().data();
        let packetlength = packet.critical().packet_length();
        assert_eq!(packetlength, FILEVERIFICATIONPACKET_SIZE + 3 * FILEVERIFICATIONENTRY_SIZE);
        assert_eq!(packet_length(data), packetlength as u64);
        assert_eq!(packet_magic(data), PACKET_MAGIC);
        assert_eq!(packet_type(data), FILEVERIFICATIONPACKET_TYPE);
        assert_eq!(packet.block_count(), 3);
    }

    #[test]
    fn test2_entries() {
        let mut packet = VerificationPacket::new();
        assert!(packet.create(2));
        let fileid = Md5Hash { hash: [1; 16] };
        packet.set_fileid(&fileid);
        packet.set_block_hash_and_crc(0, &Md5Hash { hash: [2; 16] }, 0x12345678);
        packet.set_block_hash_and_crc(1, &Md5Hash { hash: [3; 16] }, 0xdeadbeef);

        assert_eq!(packet.fileid(), fileid);
        assert_eq!(
            packet.verification_entry(0),
            FileVerificationEntry {
                hash: Md5Hash { hash: [2; 16] },
                crc: 0x12345678
            }
        );
        assert_eq!(
            packet.verification_entry(1),
            FileVerificationEntry {
                hash: Md5Hash { hash: [3; 16] },
                crc: 0xdeadbeef
            }
        );
    }

    #[test]
    fn test3_load_round_trip() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_verificationpacket_{}.par2", std::process::id()));
        let _ = fs::remove_file(&path);

        let mut packet = VerificationPacket::new();
        assert!(packet.create(2));
        packet.set_fileid(&Md5Hash { hash: [1; 16] });
        packet.set_block_hash_and_crc(0, &Md5Hash { hash: [2; 16] }, 0x11111111);
        packet.set_block_hash_and_crc(1, &Md5Hash { hash: [3; 16] }, 0x22222222);

        let packetlength = packet.critical().packet_length();
        let mut diskfile = DiskFile::new();
        assert!(diskfile.create(path.to_str().unwrap(), 0, &mut serr));
        assert!(packet
            .critical()
            .write_packet(&mut diskfile, 0, &mut serr));
        diskfile.close();

        let raw = fs::read(&path).unwrap();
        assert_eq!(raw.len(), packetlength);
        let mut diskfile = DiskFile::new();
        assert!(diskfile.open(path.to_str().unwrap(), &mut serr));

        let mut loaded = VerificationPacket::new();
        assert!(loaded.load(&mut diskfile, 0, &raw, &mut serr));
        diskfile.close();

        assert_eq!(loaded.block_count(), 2);
        assert_eq!(loaded.fileid(), packet.fileid());
        assert_eq!(loaded.verification_entry(0), packet.verification_entry(0));
        assert_eq!(loaded.verification_entry(1), packet.verification_entry(1));

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test4_load_rejects_bad_packets() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "par2rust_verificationpacket_bad_{}.par2",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        let body_len = FILEVERIFICATIONPACKET_SIZE + 32768 * FILEVERIFICATIONENTRY_SIZE + 20;
        let mut raw = vec![0u8; body_len];
        raw[..8].copy_from_slice(&PACKET_MAGIC);

        let mut write_and_load = |length: u64| -> bool {
            raw[8..16].copy_from_slice(&length.to_le_bytes());
            fs::write(&path, &raw).unwrap();

            let mut diskfile = DiskFile::new();
            assert!(diskfile.open(path.to_str().unwrap(), &mut serr));
            let mut packet = VerificationPacket::new();
            let ok = packet.load(&mut diskfile, 0, &raw[..PACKET_HEADER_SIZE], &mut serr);
            diskfile.close();
            ok
        };

        // Too small.
        assert!(!write_and_load(FILEVERIFICATIONPACKET_SIZE as u64));
        // One whole entry is fine.
        assert!(write_and_load(
            FILEVERIFICATIONPACKET_SIZE as u64 + FILEVERIFICATIONENTRY_SIZE as u64
        ));
        // Not a whole number of entries.
        assert!(!write_and_load(
            FILEVERIFICATIONPACKET_SIZE as u64 + FILEVERIFICATIONENTRY_SIZE as u64 + 1
        ));
        // Too large.
        assert!(!write_and_load(
            FILEVERIFICATIONPACKET_SIZE as u64
                + 32768 * FILEVERIFICATIONENTRY_SIZE as u64
                + FILEVERIFICATIONENTRY_SIZE as u64
        ));

        fs::remove_file(&path).unwrap();
    }
}
