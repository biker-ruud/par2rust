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

//! Port of par2cmdline's recoverypacket.h / recoverypacket.cpp.
//!
//! The RecoveryPacket object is used to access a specific recovery
//! packet within a recovery file (for when creating them, or using
//! them during a repair operation).
//!
//! Because the actual recovery data for the packet may be large, the
//! RecoveryPacket object only contains a copy of packet header and
//! exponent value for the block and uses a DataBlock object for
//! all accesses to the actual recovery data in the packet.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use crate::datablock::DataBlock;
use crate::diskfile::DiskFile;
use crate::md5::{Md5Context, Md5Hash};
use crate::par2fileformat::{
    packet_length, set_packet_hash, set_packet_length, set_packet_setid, HDR_MAGIC_OFFSET,
    HDR_SETID_OFFSET, HDR_TYPE_OFFSET, PACKET_HEADER_SIZE, PACKET_MAGIC,
    RECOVERYBLOCKPACKET_EXPONENT_OFFSET, RECOVERYBLOCKPACKET_SIZE, RECOVERYBLOCKPACKET_TYPE,
};

pub struct RecoveryPacket {
    diskfile: Option<Rc<RefCell<DiskFile>>>, // The specific file that this packet is stored in
    offset: u64, // The offset at which the packet is stored

    // The packet (excluding the actual recovery data): the 64 byte
    // packet header followed by the 4 byte exponent value.
    packet: [u8; RECOVERYBLOCKPACKET_SIZE],

    packetcontext: Option<Md5Context>, // MD5 Context used to compute the packet hash

    datablock: DataBlock, // The recovery data block.
}

impl RecoveryPacket {
    pub fn new() -> RecoveryPacket {
        RecoveryPacket {
            diskfile: None,
            offset: 0,
            packet: [0u8; RECOVERYBLOCKPACKET_SIZE],
            packetcontext: None,
            datablock: DataBlock::new(),
        }
    }

    /// Create a recovery packet for a specified file.
    ///
    /// The packet header can be almost completely filled in using the
    /// supplied information and computation of the hash of the packet
    /// started immediately. The hash will be updated as new data is
    /// written to the packet.
    pub fn create(
        &mut self,
        diskfile: Rc<RefCell<DiskFile>>,
        offset: u64,
        blocksize: u64,
        exponent: u32,
        setid: &Md5Hash,
    ) {
        self.diskfile = Some(Rc::clone(&diskfile));
        self.offset = offset;

        // Record everything we know in the packet.
        let packet = &mut self.packet;
        packet[..].fill(0);
        packet[HDR_MAGIC_OFFSET..HDR_MAGIC_OFFSET + 8].copy_from_slice(&PACKET_MAGIC);
        set_packet_length(packet, (RECOVERYBLOCKPACKET_SIZE as u64) + blocksize);
        //packet.header.hash;  // Not known yet.
        set_packet_setid(packet, setid);
        packet[HDR_TYPE_OFFSET..HDR_TYPE_OFFSET + 16].copy_from_slice(&RECOVERYBLOCKPACKET_TYPE);
        // The C++ leu32 exponent is stored little endian on disk.
        packet[RECOVERYBLOCKPACKET_EXPONENT_OFFSET..RECOVERYBLOCKPACKET_EXPONENT_OFFSET + 4]
            .copy_from_slice(&exponent.to_le_bytes());

        // Start computation of the packet hash
        let mut packetcontext = Md5Context::new();
        packetcontext.update(
            &self.packet[HDR_SETID_OFFSET..RECOVERYBLOCKPACKET_SIZE],
        );
        self.packetcontext = Some(packetcontext);

        // Set the data block to immediately follow the header on disk
        self.datablock
            .set_location(diskfile, offset + RECOVERYBLOCKPACKET_SIZE as u64);
        self.datablock.set_length(blocksize);
    }

    /// Write some data to the recovery data block and update the recovery packet.
    pub fn write_data(
        &mut self,
        position: u64,
        buffer: &[u8],
        serr: &mut dyn Write,
    ) -> bool {
        // Update the packet hash
        self.packetcontext.as_mut().unwrap().update(buffer);

        // Write the data to the data block
        let mut wrote = 0usize;
        self.datablock.write_data(position, buffer, &mut wrote, serr)
    }

    /// Finish computing the hash of the recovery packet and write the header to disk.
    pub fn write_header(&mut self, serr: &mut dyn Write) -> bool {
        // Finish computing the packet hash
        let hash = self.packetcontext.as_mut().unwrap().finalize();
        set_packet_hash(&mut self.packet, &hash);

        // Write the header to disk
        self.diskfile
            .as_ref()
            .unwrap()
            .borrow_mut()
            .write(self.offset, &self.packet, serr)
    }

    /// Load a recovery packet from a specified file.
    ///
    /// The header of the packet will already have been read from disk. The
    /// only thing that actually needs to be read is the exponent value.
    /// The recovery data is not read from disk at this point. Its location
    /// is recovered in the DataBlock object.
    pub fn load(
        &mut self,
        diskfile: Rc<RefCell<DiskFile>>,
        offset: u64,
        header: &[u8],
        serr: &mut dyn Write,
    ) -> bool {
        self.diskfile = Some(Rc::clone(&diskfile));
        self.offset = offset;

        // Is the packet actually large enough
        if packet_length(header) <= RECOVERYBLOCKPACKET_SIZE as u64 {
            return false;
        }

        // Save the fixed header
        self.packet[..PACKET_HEADER_SIZE].copy_from_slice(&header[..PACKET_HEADER_SIZE]);

        // Set the data block to immediately follow the header on disk
        self.datablock
            .set_location(Rc::clone(&diskfile), offset + RECOVERYBLOCKPACKET_SIZE as u64);
        self.datablock
            .set_length(packet_length(header) - RECOVERYBLOCKPACKET_SIZE as u64);

        // Read the rest of the packet header
        diskfile.borrow_mut().read(
            offset + PACKET_HEADER_SIZE as u64,
            &mut self.packet[PACKET_HEADER_SIZE..RECOVERYBLOCKPACKET_SIZE],
            serr,
        )
    }

    /// Get the length of the packet.
    pub fn packet_length(&self) -> u64 {
        packet_length(&self.packet)
    }

    /// The exponent of the packet (a little endian leu32 on disk).
    pub fn exponent(&self) -> u32 {
        u32::from_le_bytes(
            self.packet[RECOVERYBLOCKPACKET_EXPONENT_OFFSET
                ..RECOVERYBLOCKPACKET_EXPONENT_OFFSET + 4]
                .try_into()
                .unwrap(),
        )
    }

    /// The length of the recovery data.
    pub fn block_size(&self) -> u64 {
        packet_length(&self.packet) - RECOVERYBLOCKPACKET_SIZE as u64
    }

    /// The data block.
    pub fn data_block(&mut self) -> &mut DataBlock {
        &mut self.datablock
    }
}

impl Default for RecoveryPacket {
    fn default() -> Self {
        RecoveryPacket::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::par2fileformat::{
        packet_hash, packet_magic, packet_setid, packet_type, HDR_HASH_OFFSET,
    };
    use std::fs;

    fn test_setid() -> Md5Hash {
        Md5Hash { hash: [3u8; 16] }
    }

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "par2rust_recoverypacket_{}_{}.bin",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        path
    }

    #[test]
    fn test1_create() {
        let mut serr = std::io::sink();
        let path = temp_path("create");

        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .create(path.to_str().unwrap(), 1000, &mut serr));

        let mut packet = RecoveryPacket::new();
        packet.create(Rc::clone(&diskfile), 0, 100, 5, &test_setid());

        assert_eq!(packet.packet_length(), 68 + 100);
        assert_eq!(packet.exponent(), 5);
        assert_eq!(packet.block_size(), 100);
        assert_eq!(packet_magic(&packet.packet), PACKET_MAGIC);
        assert_eq!(packet_type(&packet.packet), RECOVERYBLOCKPACKET_TYPE);
        assert_eq!(packet_setid(&packet.packet), test_setid());
        assert_eq!(packet.data_block().length(), 100);
        assert_eq!(packet.data_block().offset(), 68);

        diskfile.borrow_mut().close();
        diskfile.borrow_mut().delete(&mut serr);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test2_write_and_header_hash() {
        let mut serr = std::io::sink();
        let path = temp_path("hash");

        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .create(path.to_str().unwrap(), 1000, &mut serr));

        let mut packet = RecoveryPacket::new();
        packet.create(Rc::clone(&diskfile), 0, 100, 5, &test_setid());

        let data: Vec<u8> = (0u8..100).collect();
        assert!(packet.write_data(0, &data, &mut serr));
        assert!(packet.write_header(&mut serr));

        // The header hash must cover [setid .. packet size) plus all the
        // recovery data, in order.
        let mut ctx = Md5Context::new();
        let mut hdr = packet.packet;
        hdr[HDR_HASH_OFFSET..HDR_HASH_OFFSET + 16].fill(0); // hash unknown at context start
        ctx.update(&hdr[HDR_SETID_OFFSET..RECOVERYBLOCKPACKET_SIZE]);
        ctx.update(&data);
        let expected = ctx.finalize();
        assert_eq!(packet_hash(&packet.packet), expected);

        // The header must be on disk.
        let mut header = [0u8; RECOVERYBLOCKPACKET_SIZE];
        assert!(diskfile.borrow_mut().read(0, &mut header, &mut serr));
        assert_eq!(&header, &packet.packet);

        // And the data block must be on disk right after it.
        let mut block = [0u8; 100];
        assert!(diskfile.borrow_mut().read(68, &mut block, &mut serr));
        assert_eq!(block, data.as_slice());

        diskfile.borrow_mut().close();
        diskfile.borrow_mut().delete(&mut serr);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test3_load_round_trip() {
        let mut serr = std::io::sink();
        let path = temp_path("load");

        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .create(path.to_str().unwrap(), 1000, &mut serr));

        let mut packet = RecoveryPacket::new();
        packet.create(Rc::clone(&diskfile), 0, 100, 7, &test_setid());
        let data: Vec<u8> = (0u8..100).collect();
        assert!(packet.write_data(0, &data, &mut serr));
        assert!(packet.write_header(&mut serr));

        // Load it back.
        let mut header = [0u8; PACKET_HEADER_SIZE];
        assert!(diskfile.borrow_mut().read(0, &mut header, &mut serr));

        let mut loaded = RecoveryPacket::new();
        assert!(loaded.load(Rc::clone(&diskfile), 0, &header, &mut serr));
        assert_eq!(loaded.packet_length(), 168);
        assert_eq!(loaded.exponent(), 7);
        assert_eq!(loaded.block_size(), 100);
        assert_eq!(loaded.data_block().offset(), 68);
        assert_eq!(loaded.data_block().length(), 100);

        // The data block reads the recovery data.
        let mut block = [0u8; 100];
        assert!(loaded.data_block().read_data(0, &mut block, &mut serr));
        assert_eq!(block, data.as_slice());

        diskfile.borrow_mut().close();
        diskfile.borrow_mut().delete(&mut serr);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test4_load_too_small() {
        let mut serr = std::io::sink();
        let path = temp_path("toosmall");

        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .create(path.to_str().unwrap(), 1000, &mut serr));

        let mut header = [0u8; PACKET_HEADER_SIZE];
        header[..8].copy_from_slice(&PACKET_MAGIC);
        set_packet_length(&mut header, RECOVERYBLOCKPACKET_SIZE as u64);

        let mut packet = RecoveryPacket::new();
        assert!(!packet.load(Rc::clone(&diskfile), 0, &header, &mut serr));

        set_packet_length(&mut header, RECOVERYBLOCKPACKET_SIZE as u64 + 1);
        assert!(packet.load(Rc::clone(&diskfile), 0, &header, &mut serr));
        assert_eq!(packet.block_size(), 1);

        diskfile.borrow_mut().close();
        diskfile.borrow_mut().delete(&mut serr);
        let _ = fs::remove_file(&path);
    }
}
