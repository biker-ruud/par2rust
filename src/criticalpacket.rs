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

//! Port of par2cmdline's criticalpacket.h / criticalpacket.cpp.
//!
//! Base class for main packet, file verification packet, file description
//! packet and creator packet.
//!
//! These packets are all small and are held in memory in their entirety.
//!
//! The C++ class owns a raw `new[]` buffer; here the buffer is a `Vec<u8>`
//! and subclasses reach the bytes through [`CriticalPacket::data`] /
//! [`CriticalPacket::data_mut`].
//!
//! C++ `CriticalPacketEntry` stores raw `DiskFile*` / `CriticalPacket*`
//! pointers. Rust has no safe equivalent for a list of entries that share
//! those objects, so the entry stores the index of the caller-owned
//! `DiskFile` and of the caller-owned packet instead, and the methods take
//! the owning slices.

use std::io::Write;

use crate::diskfile::DiskFile;
use crate::md5::{Md5Context, Md5Hash};
use crate::par2fileformat::{
    self, FILEDESCRIPTIONPACKET_FILEID_OFFSET, FILEVERIFICATIONPACKET_FILEID_OFFSET,
    HDR_SETID_OFFSET, PACKET_HEADER_SIZE, RECOVERYBLOCKPACKET_EXPONENT_OFFSET,
};

pub struct CriticalPacket {
    packetdata: Vec<u8>,
    packetlength: usize,
}

impl CriticalPacket {
    pub fn new() -> CriticalPacket {
        // There is no data initially
        CriticalPacket {
            packetdata: Vec::new(),
            packetlength: 0,
        }
    }

    /// Obtain the length of the packet.
    pub fn packet_length(&self) -> usize {
        self.packetlength
    }

    /// The allocated buffer, including any extra padding.
    pub fn data(&self) -> &[u8] {
        &self.packetdata
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.packetdata
    }

    /// Allocate some memory for the packet (plus some extra padding).
    /// C++ returns the pointer; here the caller reaches the buffer through
    /// [`CriticalPacket::data_mut`].
    pub fn allocate_packet(&mut self, length: usize, extra: usize) {
        // Hey! We can't allocate the packet twice
        debug_assert!(self.packetlength == 0 && self.packetdata.is_empty());

        // Remember the requested packet length
        self.packetlength = length;

        // Allocate and clear the requested packet length plus the extra.
        self.packetdata = vec![0u8; length + extra];
    }

    /// Write a copy of the packet to the specified file at the specified
    /// offset.
    pub fn write_packet(
        &self,
        diskfile: &mut DiskFile,
        fileoffset: u64,
        serr: &mut dyn Write,
    ) -> bool {
        debug_assert!(!self.packetdata.is_empty() && self.packetlength != 0);

        diskfile.write(fileoffset, &self.packetdata[..self.packetlength], serr)
    }

    /// Finish a packet (by storing the set_id_hash and then computing the
    /// packet_hash).
    pub fn finish_packet(&mut self, set_id_hash: &Md5Hash) {
        debug_assert!(
            !self.packetdata.is_empty() && self.packetlength >= PACKET_HEADER_SIZE
        );

        par2fileformat::set_packet_setid(&mut self.packetdata, set_id_hash);

        let mut packetcontext = Md5Context::new();
        packetcontext.update(&self.packetdata[HDR_SETID_OFFSET..self.packetlength]);
        let hash = packetcontext.finalize();
        par2fileformat::set_packet_hash(&mut self.packetdata, &hash);
    }

    /// Sort critical packets by type and FileId.
    ///
    /// Order is:
    /// Main Packet
    /// File Description Packet (sorted by FileID)
    /// File Verification Packet  (sorted by FileID)
    /// Creator Packet
    /// Recovery Packet (sorted by exponent)
    pub fn compare_less(left: &CriticalPacket, right: &CriticalPacket) -> bool {
        let left_value = type_value(left.packetdata[par2fileformat::HDR_TYPE_OFFSET + 8]);
        let right_value = type_value(right.packetdata[par2fileformat::HDR_TYPE_OFFSET + 8]);

        if left_value < right_value {
            return true;
        }
        if left_value > right_value {
            return false;
        }

        if left_value == 1 {
            // file description packets
            fileid(&left.packetdata, FILEDESCRIPTIONPACKET_FILEID_OFFSET)
                < fileid(&right.packetdata, FILEDESCRIPTIONPACKET_FILEID_OFFSET)
        } else if left_value == 2 {
            // file verification packets
            fileid(&left.packetdata, FILEVERIFICATIONPACKET_FILEID_OFFSET)
                < fileid(&right.packetdata, FILEVERIFICATIONPACKET_FILEID_OFFSET)
        } else if left_value == 4 {
            // recovery packet
            u32::from_le_bytes(
                left.packetdata
                    [RECOVERYBLOCKPACKET_EXPONENT_OFFSET..RECOVERYBLOCKPACKET_EXPONENT_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            ) < u32::from_le_bytes(
                right.packetdata
                    [RECOVERYBLOCKPACKET_EXPONENT_OFFSET
                        ..RECOVERYBLOCKPACKET_EXPONENT_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            )
        } else {
            // they're equal.
            false
        }
    }
}

impl Default for CriticalPacket {
    fn default() -> Self {
        CriticalPacket::new()
    }
}

fn type_value(type8: u8) -> i32 {
    match type8 {
        b'M' => 0,
        b'F' => 1,
        b'I' => 2,
        b'C' => 3,
        b'R' => 4,
        _ => 5,
    }
}

fn fileid(data: &[u8], offset: usize) -> Md5Hash {
    let mut hash = [0u8; 16];
    hash.copy_from_slice(&data[offset..offset + 16]);
    Md5Hash { hash }
}

/// Class used to record the fact that a copy of a particular critical packet
/// will be written to a particular file at a specific offset.
///
/// The C++ entry holds raw pointers; this port holds the indices of the
/// caller-owned `DiskFile` and `CriticalPacket`.
pub struct CriticalPacketEntry {
    pub diskfile: usize,
    pub offset: u64,
    pub packet: usize,
}

impl CriticalPacketEntry {
    pub fn new(diskfile: usize, offset: u64, packet: usize) -> CriticalPacketEntry {
        CriticalPacketEntry {
            diskfile,
            offset,
            packet,
        }
    }

    /// Write the packet to disk.
    pub fn write_packet(
        &self,
        diskfiles: &mut [DiskFile],
        packets: &[CriticalPacket],
        serr: &mut dyn Write,
    ) -> bool {
        debug_assert!(packets[self.packet].packet_length() != 0);

        // Tell the packet to write itself to disk
        packets[self.packet].write_packet(&mut diskfiles[self.diskfile], self.offset, serr)
    }

    /// Obtain the length of the packet.
    pub fn packet_length(&self, packets: &[CriticalPacket]) -> u64 {
        // Ask the packet how big it is.
        packets[self.packet].packet_length() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::par2fileformat::{
        set_packet_length, CREATORPACKET_SIZE, FILEDESCRIPTIONPACKET_SIZE,
        FILEVERIFICATIONPACKET_SIZE, MAINPACKET_SIZE, PACKET_HEADER_SIZE, PACKET_MAGIC,
        RECOVERYBLOCKPACKET_SIZE,
    };
    use std::fs;

    fn make_packet(type8: u8) -> CriticalPacket {
        make_packet_sized(type8, PACKET_HEADER_SIZE)
    }

    // F/I/R packets carry a fileid or exponent after the header.
    fn make_packet_sized(type8: u8, length: usize) -> CriticalPacket {
        let mut packet = CriticalPacket::new();
        packet.allocate_packet(length, 0);
        packet.data_mut()[0..8].copy_from_slice(&PACKET_MAGIC);
        set_packet_length(packet.data_mut(), length as u64);
        packet.data_mut()[par2fileformat::HDR_TYPE_OFFSET + 8] = type8;
        packet
    }

    #[test]
    fn test1_finish_packet() {
        let mut packet = make_packet(b'M');
        let setid = Md5Hash { hash: [0xab; 16] };
        packet.finish_packet(&setid);

        assert_eq!(
            par2fileformat::packet_setid(packet.data()),
            setid
        );

        // The hash covers the packet from the setid field to the end.
        let mut ctx = Md5Context::new();
        ctx.update(&packet.data()[HDR_SETID_OFFSET..PACKET_HEADER_SIZE]);
        let expected = ctx.finalize();
        assert_eq!(par2fileformat::packet_hash(packet.data()), expected);
    }

    #[test]
    fn test2_write_packet() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "par2rust_criticalpacket_{}.pkt",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        let packet = make_packet(b'M');
        let mut diskfile = DiskFile::new();
        assert!(diskfile.create(path.to_str().unwrap(), 0, &mut serr));
        assert!(packet.write_packet(&mut diskfile, 0, &mut serr));
        diskfile.close();

        assert_eq!(fs::read(&path).unwrap(), packet.data());

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test3_compare_less_type_order() {
        let main = make_packet(b'M');
        let filedesc = make_packet(b'F');
        let ifsc = make_packet(b'I');
        let creator = make_packet(b'C');
        let recovery = make_packet(b'R');
        let other = make_packet(b'X');

        assert!(CriticalPacket::compare_less(&main, &filedesc));
        assert!(CriticalPacket::compare_less(&filedesc, &ifsc));
        assert!(CriticalPacket::compare_less(&ifsc, &creator));
        assert!(CriticalPacket::compare_less(&creator, &recovery));
        assert!(CriticalPacket::compare_less(&recovery, &other));
        assert!(!CriticalPacket::compare_less(&filedesc, &main));
        assert!(!CriticalPacket::compare_less(&main, &main));
    }

    #[test]
    fn test4_compare_less_fileid_and_exponent() {
        // Md5Hash orders as a big number: the last byte is most significant.
        let mut low = make_packet_sized(b'F', 80);
        low.data_mut()[FILEDESCRIPTIONPACKET_FILEID_OFFSET + 15] = 1;
        let mut high = make_packet_sized(b'F', 80);
        high.data_mut()[FILEDESCRIPTIONPACKET_FILEID_OFFSET + 15] = 2;
        assert!(CriticalPacket::compare_less(&low, &high));
        assert!(!CriticalPacket::compare_less(&high, &low));
        assert!(!CriticalPacket::compare_less(&low, &low));

        let mut low = make_packet_sized(b'I', 80);
        low.data_mut()[FILEVERIFICATIONPACKET_FILEID_OFFSET + 15] = 1;
        let mut high = make_packet_sized(b'I', 80);
        high.data_mut()[FILEVERIFICATIONPACKET_FILEID_OFFSET + 15] = 2;
        assert!(CriticalPacket::compare_less(&low, &high));

        let mut low = make_packet_sized(b'R', 68);
        low.data_mut()[RECOVERYBLOCKPACKET_EXPONENT_OFFSET..RECOVERYBLOCKPACKET_EXPONENT_OFFSET + 4]
            .copy_from_slice(&7u32.to_le_bytes());
        let mut high = make_packet_sized(b'R', 68);
        high.data_mut()
            [RECOVERYBLOCKPACKET_EXPONENT_OFFSET..RECOVERYBLOCKPACKET_EXPONENT_OFFSET + 4]
            .copy_from_slice(&9u32.to_le_bytes());
        assert!(CriticalPacket::compare_less(&low, &high));
        assert!(!CriticalPacket::compare_less(&high, &low));
    }

    #[test]
    fn test5_packet_entry() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "par2rust_criticalpacket_entry_{}.pkt",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        let packets = vec![make_packet(b'M'), make_packet(b'C')];

        let mut diskfiles = vec![DiskFile::new()];
        assert!(diskfiles[0].create(path.to_str().unwrap(), 0, &mut serr));

        let entry0 = CriticalPacketEntry::new(0, 0, 0);
        let entry1 = CriticalPacketEntry::new(0, PACKET_HEADER_SIZE as u64, 1);
        assert_eq!(entry0.packet_length(&packets), PACKET_HEADER_SIZE as u64);
        assert!(entry0.write_packet(&mut diskfiles, &packets, &mut serr));
        assert!(entry1.write_packet(&mut diskfiles, &packets, &mut serr));
        diskfiles[0].close();

        let written = fs::read(&path).unwrap();
        assert_eq!(&written[..PACKET_HEADER_SIZE], packets[0].data());
        assert_eq!(&written[PACKET_HEADER_SIZE..], packets[1].data());

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test6_compare_less_matrix() {
        // Ported from upstream criticalpacket_test.cpp: eleven packets that
        // must come out strictly ordered under compare_less.
        let mut packets: Vec<CriticalPacket> = Vec::new();

        packets.push(make_packet_sized(b'M', MAINPACKET_SIZE));

        let p = make_packet_sized(b'F', FILEDESCRIPTIONPACKET_SIZE);
        packets.push(p); // fileid all zero
        let mut p = make_packet_sized(b'F', FILEDESCRIPTIONPACKET_SIZE);
        p.data_mut()[FILEDESCRIPTIONPACKET_FILEID_OFFSET] = 1;
        packets.push(p);
        let mut p = make_packet_sized(b'F', FILEDESCRIPTIONPACKET_SIZE);
        p.data_mut()[FILEDESCRIPTIONPACKET_FILEID_OFFSET + 1] = 1;
        packets.push(p);

        let p = make_packet_sized(b'I', FILEVERIFICATIONPACKET_SIZE);
        packets.push(p); // fileid all zero
        let mut p = make_packet_sized(b'I', FILEVERIFICATIONPACKET_SIZE);
        p.data_mut()[FILEVERIFICATIONPACKET_FILEID_OFFSET] = 1;
        packets.push(p);
        let mut p = make_packet_sized(b'I', FILEVERIFICATIONPACKET_SIZE);
        p.data_mut()[FILEVERIFICATIONPACKET_FILEID_OFFSET + 1] = 1;
        packets.push(p);

        packets.push(make_packet_sized(b'C', CREATORPACKET_SIZE));

        let mut p = make_packet_sized(b'R', RECOVERYBLOCKPACKET_SIZE);
        p.data_mut()[RECOVERYBLOCKPACKET_EXPONENT_OFFSET..RECOVERYBLOCKPACKET_EXPONENT_OFFSET + 4]
            .copy_from_slice(&0u32.to_le_bytes());
        packets.push(p);
        let mut p = make_packet_sized(b'R', RECOVERYBLOCKPACKET_SIZE);
        p.data_mut()[RECOVERYBLOCKPACKET_EXPONENT_OFFSET..RECOVERYBLOCKPACKET_EXPONENT_OFFSET + 4]
            .copy_from_slice(&1u32.to_le_bytes());
        packets.push(p);

        // Unknown type sorts last.
        packets.push(make_packet_sized(b'\0', PACKET_HEADER_SIZE));

        for i in 0..packets.len() {
            for j in 0..packets.len() {
                assert_eq!(
                    CriticalPacket::compare_less(&packets[i], &packets[j]),
                    i < j,
                    "compare_less failed for {i} {j}"
                );
            }
        }
    }
}
