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

//! This module defines the format of a PAR 2.0 file.
//!
//! PAR 2.0 files consist of one or more "packets" that contain information
//! that is required to be able to verify and repair damaged data files.
//!
//! All packets start with a short "header" which contains information
//! used to describe what sort of data is stored in the rest of the packet
//! and also to allow that data to be verified.
//!
//! This module details the format for the following packet types described
//! in the PAR 2.0 specification:
//!
//!  Main Packet                        `MAINPACKET`
//!  File Description Packet            `FILEDESCRIPTIONPACKET`
//!  Input File Slice Checksum Packet   `FILEVERIFICATIONPACKET`
//!  Recovery Slice Packet              `RECOVERYBLOCKPACKET`
//!  Creator Packet                     `CREATORPACKET`
//!
//! The C++ original overlays packed structs on the packet bytes. Here the
//! packets stay as byte buffers and the fields are read and written at the
//! offsets the structs define, so the two agree byte for byte.
//!
//! All numeric fields in the file format are in LITTLE ENDIAN format.

use crate::md5::Md5Hash;

// Two simple types used in the packet header.
pub type Magic = [u8; 8];
pub type PacketType = [u8; 16];

// An MD5Hash is 16 bytes on disk.
pub const MD5HASH_SIZE: usize = 16;

// Every packet starts with a packet header:
//
//   MAGIC      magic;  // = {'P', 'A', 'R', '2', '\0', 'P', 'K', 'T'}
//   leu64      length; // Length of entire packet including header
//   MD5Hash    hash;   // Hash of entire packet excepting the first 3 fields
//   MD5Hash    setid;  // Normally computed as the Hash of body of "Main Packet"
//   PACKETTYPE type;   // Used to specify the meaning of the rest of the packet

pub const PACKET_HEADER_SIZE: usize = 64;

pub const HDR_MAGIC_OFFSET: usize = 0;
pub const HDR_LENGTH_OFFSET: usize = 8;
pub const HDR_HASH_OFFSET: usize = 16;
pub const HDR_SETID_OFFSET: usize = 32;
pub const HDR_TYPE_OFFSET: usize = 48;

// The file verification packet is used to determine whether or not any
// parts of a damaged file are useable.
// It contains a FileId used to pair it with a corresponding file description
// packet, followed by an array of hash and crc values. The number of entries in
// the array can be determined from the packet_length.
//
//   FILEVERIFICATIONENTRY:
//     MD5Hash  hash;
//     leu32    crc;
pub const FILEVERIFICATIONENTRY_SIZE: usize = 20;

//   FILEVERIFICATIONPACKET:
//     PACKET_HEADER       header;
//     MD5Hash             fileid;  // MD5hash of file_hash_16k, file_length, file_name
//     FILEVERIFICATIONENTRY entries[];
pub const FILEVERIFICATIONPACKET_FILEID_OFFSET: usize = PACKET_HEADER_SIZE;
pub const FILEVERIFICATIONPACKET_ENTRIES_OFFSET: usize = PACKET_HEADER_SIZE + 16;
pub const FILEVERIFICATIONPACKET_SIZE: usize = PACKET_HEADER_SIZE + 16;

// The file description packet is used to record the name of the file,
// its size, and the Hash of both the whole file and the first 16k of
// the file.
// If the name of the file is an exact multiple of 4 characters in length
// then it may not have a NULL termination. If the name of the file is not
// an exact multiple of 4, then it will be padded with 0 bytes at the
// end to make it up to a multiple of 4.
//
//   FILEDESCRIPTIONPACKET:
//     PACKET_HEADER  header;
//     MD5Hash        fileid;    // MD5hash of [hash16k, length, name]
//     MD5Hash        hashfull;  // MD5 Hash of the whole file
//     MD5Hash        hash16k;   // MD5 Hash of the first 16k of the file
//     leu64          length;    // Length of the file
//     u8             name[];    // Name of the file, padded with 1 to 3 zero
//                               // bytes to reach a multiple of 4 bytes.
pub const FILEDESCRIPTIONPACKET_FILEID_OFFSET: usize = PACKET_HEADER_SIZE;
pub const FILEDESCRIPTIONPACKET_HASHFULL_OFFSET: usize = PACKET_HEADER_SIZE + 16;
pub const FILEDESCRIPTIONPACKET_HASH16K_OFFSET: usize = PACKET_HEADER_SIZE + 32;
pub const FILEDESCRIPTIONPACKET_LENGTH_OFFSET: usize = PACKET_HEADER_SIZE + 48;
pub const FILEDESCRIPTIONPACKET_NAME_OFFSET: usize = PACKET_HEADER_SIZE + 56;
pub const FILEDESCRIPTIONPACKET_SIZE: usize = PACKET_HEADER_SIZE + 56;

// The main packet is used to tie together the other packets in a recovery file.
// It specifies the block size used to virtually slice the source files, a count
// of the number of source files, and an array of Hash values used to specify
// in what order the source files are processed.
// Each entry in the fileid array corresponds with the fileid value
// in a file description packet and a file verification packet.
// The fileid array may contain more entries than the count of the number
// of recoverable files. The extra entries correspond to files that were not
// used during the creation of the recovery files and which may not therefore
// be repaired if they are found to be damaged.
//
//   MAINPACKET:
//     PACKET_HEADER  header;
//     leu64          blocksize;
//     leu32          recoverablefilecount;
//     MD5Hash        fileid[];
pub const MAINPACKET_BLOCKSIZE_OFFSET: usize = PACKET_HEADER_SIZE;
pub const MAINPACKET_RECOVERABLEFILECOUNT_OFFSET: usize = PACKET_HEADER_SIZE + 8;
pub const MAINPACKET_FILEID_OFFSET: usize = PACKET_HEADER_SIZE + 12;
pub const MAINPACKET_SIZE: usize = PACKET_HEADER_SIZE + 12;

// The creator packet is used to identify which program created a particular
// recovery file. It is not required for verification or recovery of damaged
// files.
//
//   CREATORPACKET:
//     PACKET_HEADER  header;
//     u8             client[];
pub const CREATORPACKET_CLIENT_OFFSET: usize = PACKET_HEADER_SIZE;
pub const CREATORPACKET_SIZE: usize = PACKET_HEADER_SIZE;

// The recovery block packet contains a single block of recovery data along
// with the exponent value used during the computation of that block.
//
//   RECOVERYBLOCKPACKET:
//     PACKET_HEADER  header;
//     leu32          exponent;
//     (data follows)
pub const RECOVERYBLOCKPACKET_EXPONENT_OFFSET: usize = PACKET_HEADER_SIZE;
pub const RECOVERYBLOCKPACKET_DATA_OFFSET: usize = PACKET_HEADER_SIZE + 4;
pub const RECOVERYBLOCKPACKET_SIZE: usize = PACKET_HEADER_SIZE + 4;

pub const PACKET_MAGIC: Magic = *b"PAR2\0PKT";

pub const FILEVERIFICATIONPACKET_TYPE: PacketType = *b"PAR 2.0\0IFSC\0\0\0\0";
pub const FILEDESCRIPTIONPACKET_TYPE: PacketType = *b"PAR 2.0\0FileDesc";
pub const MAINPACKET_TYPE: PacketType = *b"PAR 2.0\0Main\0\0\0\0";
pub const RECOVERYBLOCKPACKET_TYPE: PacketType = *b"PAR 2.0\0RecvSlic";
pub const CREATORPACKET_TYPE: PacketType = *b"PAR 2.0\0Creator\0";

// Field access on a packet held as bytes.

pub fn packet_magic(data: &[u8]) -> Magic {
    let mut magic = [0u8; 8];
    magic.copy_from_slice(&data[HDR_MAGIC_OFFSET..HDR_MAGIC_OFFSET + 8]);
    magic
}

pub fn packet_length(data: &[u8]) -> u64 {
    u64::from_le_bytes(
        data[HDR_LENGTH_OFFSET..HDR_LENGTH_OFFSET + 8]
            .try_into()
            .unwrap(),
    )
}

pub fn set_packet_length(data: &mut [u8], length: u64) {
    data[HDR_LENGTH_OFFSET..HDR_LENGTH_OFFSET + 8].copy_from_slice(&length.to_le_bytes());
}

pub fn packet_hash(data: &[u8]) -> Md5Hash {
    let mut hash = [0u8; 16];
    hash.copy_from_slice(&data[HDR_HASH_OFFSET..HDR_HASH_OFFSET + 16]);
    Md5Hash { hash }
}

pub fn set_packet_hash(data: &mut [u8], hash: &Md5Hash) {
    data[HDR_HASH_OFFSET..HDR_HASH_OFFSET + 16].copy_from_slice(&hash.hash);
}

pub fn packet_setid(data: &[u8]) -> Md5Hash {
    let mut hash = [0u8; 16];
    hash.copy_from_slice(&data[HDR_SETID_OFFSET..HDR_SETID_OFFSET + 16]);
    Md5Hash { hash }
}

pub fn set_packet_setid(data: &mut [u8], hash: &Md5Hash) {
    data[HDR_SETID_OFFSET..HDR_SETID_OFFSET + 16].copy_from_slice(&hash.hash);
}

pub fn packet_type(data: &[u8]) -> PacketType {
    let mut packettype = [0u8; 16];
    packettype.copy_from_slice(&data[HDR_TYPE_OFFSET..HDR_TYPE_OFFSET + 16]);
    packettype
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test1_packet_magic() {
        assert_eq!(PACKET_MAGIC, *b"PAR2\0PKT");
    }

    #[test]
    fn test2_packet_types() {
        assert_eq!(FILEVERIFICATIONPACKET_TYPE.len(), 16);
        assert_eq!(&FILEVERIFICATIONPACKET_TYPE[0..8], b"PAR 2.0\0");
        assert_eq!(&FILEVERIFICATIONPACKET_TYPE[8..12], b"IFSC");
        assert_eq!(&FILEDESCRIPTIONPACKET_TYPE[8..16], b"FileDesc");
        assert_eq!(&MAINPACKET_TYPE[8..12], b"Main");
        assert_eq!(&RECOVERYBLOCKPACKET_TYPE[8..16], b"RecvSlic");
        assert_eq!(&CREATORPACKET_TYPE[8..15], b"Creator");
    }

    #[test]
    fn test3_header_field_round_trip() {
        let mut data = vec![0u8; PACKET_HEADER_SIZE];
        data[HDR_MAGIC_OFFSET..HDR_MAGIC_OFFSET + 8].copy_from_slice(&PACKET_MAGIC);
        data[HDR_TYPE_OFFSET..HDR_TYPE_OFFSET + 16].copy_from_slice(&MAINPACKET_TYPE);
        set_packet_length(&mut data, 0x0102030405060708);
        let hash = Md5Hash { hash: [0xab; 16] };
        set_packet_hash(&mut data, &hash);
        let setid = Md5Hash { hash: [0xcd; 16] };
        set_packet_setid(&mut data, &setid);

        assert_eq!(packet_magic(&data), PACKET_MAGIC);
        assert_eq!(packet_type(&data), MAINPACKET_TYPE);
        assert_eq!(packet_length(&data), 0x0102030405060708);
        assert_eq!(packet_hash(&data), hash);
        assert_eq!(packet_setid(&data), setid);

        // Little endian on the wire.
        assert_eq!(&data[HDR_LENGTH_OFFSET..HDR_LENGTH_OFFSET + 8], &[8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn test4_layout_offsets() {
        assert_eq!(HDR_TYPE_OFFSET + 16, PACKET_HEADER_SIZE);
        assert_eq!(FILEVERIFICATIONPACKET_ENTRIES_OFFSET, 80);
        assert_eq!(FILEDESCRIPTIONPACKET_NAME_OFFSET, 120);
        assert_eq!(MAINPACKET_FILEID_OFFSET, 76);
        assert_eq!(RECOVERYBLOCKPACKET_DATA_OFFSET, 68);
        assert_eq!(FILEVERIFICATIONENTRY_SIZE, 20);
    }
}
