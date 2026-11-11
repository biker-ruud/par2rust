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

//! This module defines the format of a PAR 1.0 file.
//!
//! This is the layout of the older PAR 1.0 ("SmartPar") recovery files that
//! `Par1Repairer` reads. A PAR 1.0 file starts with a fixed file header,
//! followed by a list of file entries, followed by the recovery data:
//!
//!   PAR1FILEHEADER:
//!     PAR1MAGIC  magic;           // = {'P', 'A', 'R', 0, 0, 0, 0, 0}
//!     leu32      fileversion;     // Must be 0x00010000
//!     leu32      programversion;
//!     MD5Hash    controlhash;     // Hash of the file from sethash onwards
//!     MD5Hash    sethash;
//!     leu64      volumenumber;
//!     leu64      numberoffiles;
//!     leu64      filelistoffset;
//!     leu64      filelistsize;
//!     leu64      dataoffset;
//!     leu64      datasize;
//!
//!   PAR1FILEENTRY:
//!     leu64      entrysize;       // Length of this entry including the name
//!     leu64      status;          // FILEENTRYSTATUS flags
//!     leu64      filesize;
//!     MD5Hash    hashfull;        // Hash of the whole file
//!     MD5Hash    hash16k;         // Hash of the first 16k of the file
//!     leu16      name[];          // File name in UTF-16 (see Par1RepairerSourceFile)
//!
//! The C++ original overlays packed structs on the file bytes (see
//! par1fileformat.h). Here the data stays as byte buffers and the fields are
//! read at the offsets those structs define, so the two agree byte for byte.
//!
//! All numeric fields in the file format are in LITTLE ENDIAN format.

use crate::md5::Md5Hash;
use crate::par2fileformat::Magic;

// The magic value at the start of every PAR 1.0 file.
//
// In C++ this is the global `par1_magic` defined in par1fileformat.cpp.
pub const PAR1_MAGIC: Magic = *b"PAR\0\0\0\0\0";

// The file header that starts every PAR 1.0 file.

pub const PAR1FILEHEADER_SIZE: usize = 96;

pub const PAR1FILEHEADER_MAGIC_OFFSET: usize = 0;
pub const PAR1FILEHEADER_FILEVERSION_OFFSET: usize = 8;
pub const PAR1FILEHEADER_PROGRAMVERSION_OFFSET: usize = 12;
pub const PAR1FILEHEADER_CONTROLHASH_OFFSET: usize = 16;
// The control hash covers the file from this offset to the end of the file
// (the C++ code uses offsetof(PAR1FILEHEADER, sethash) for this).
pub const PAR1FILEHEADER_SETHASH_OFFSET: usize = 32;
pub const PAR1FILEHEADER_VOLUMENUMBER_OFFSET: usize = 48;
pub const PAR1FILEHEADER_NUMBEROFFILES_OFFSET: usize = 56;
pub const PAR1FILEHEADER_FILELISTOFFSET_OFFSET: usize = 64;
pub const PAR1FILEHEADER_FILELISTSIZE_OFFSET: usize = 72;
pub const PAR1FILEHEADER_DATAOFFSET_OFFSET: usize = 80;
pub const PAR1FILEHEADER_DATASIZE_OFFSET: usize = 88;

// A file entry in the file list. The fixed portion is followed by the file
// name stored as UTF-16 code units, so an entry is larger than this size and
// its total length is held in the entrysize field.

pub const PAR1FILEENTRY_SIZE: usize = 56;

pub const PAR1FILEENTRY_ENTRYSIZE_OFFSET: usize = 0;
pub const PAR1FILEENTRY_STATUS_OFFSET: usize = 8;
pub const PAR1FILEENTRY_FILESIZE_OFFSET: usize = 16;
pub const PAR1FILEENTRY_HASHFULL_OFFSET: usize = 24;
pub const PAR1FILEENTRY_HASH16K_OFFSET: usize = 40;
pub const PAR1FILEENTRY_NAME_OFFSET: usize = 56;

// The values of the status field of a file entry
// (the C++ enum FILEENTRYSTATUS).

pub const FILEENTRYSTATUS_INPARITYVOLUME: u64 = 1;
pub const FILEENTRYSTATUS_CHECKED: u64 = 2;

// Field access on a PAR 1.0 header or entry held as bytes.

fn le_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn le_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn md5(data: &[u8], offset: usize) -> Md5Hash {
    let mut hash = [0u8; 16];
    hash.copy_from_slice(&data[offset..offset + 16]);
    Md5Hash { hash }
}

pub fn fileheader_magic(data: &[u8]) -> Magic {
    let mut magic = [0u8; 8];
    magic.copy_from_slice(&data[PAR1FILEHEADER_MAGIC_OFFSET..PAR1FILEHEADER_MAGIC_OFFSET + 8]);
    magic
}

pub fn fileheader_fileversion(data: &[u8]) -> u32 {
    le_u32(data, PAR1FILEHEADER_FILEVERSION_OFFSET)
}

pub fn fileheader_programversion(data: &[u8]) -> u32 {
    le_u32(data, PAR1FILEHEADER_PROGRAMVERSION_OFFSET)
}

pub fn fileheader_controlhash(data: &[u8]) -> Md5Hash {
    md5(data, PAR1FILEHEADER_CONTROLHASH_OFFSET)
}

pub fn fileheader_sethash(data: &[u8]) -> Md5Hash {
    md5(data, PAR1FILEHEADER_SETHASH_OFFSET)
}

pub fn fileheader_volumenumber(data: &[u8]) -> u64 {
    le_u64(data, PAR1FILEHEADER_VOLUMENUMBER_OFFSET)
}

pub fn fileheader_numberoffiles(data: &[u8]) -> u64 {
    le_u64(data, PAR1FILEHEADER_NUMBEROFFILES_OFFSET)
}

pub fn fileheader_filelistoffset(data: &[u8]) -> u64 {
    le_u64(data, PAR1FILEHEADER_FILELISTOFFSET_OFFSET)
}

pub fn fileheader_filelistsize(data: &[u8]) -> u64 {
    le_u64(data, PAR1FILEHEADER_FILELISTSIZE_OFFSET)
}

pub fn fileheader_dataoffset(data: &[u8]) -> u64 {
    le_u64(data, PAR1FILEHEADER_DATAOFFSET_OFFSET)
}

pub fn fileheader_datasize(data: &[u8]) -> u64 {
    le_u64(data, PAR1FILEHEADER_DATASIZE_OFFSET)
}

pub fn fileentry_entrysize(data: &[u8]) -> u64 {
    le_u64(data, PAR1FILEENTRY_ENTRYSIZE_OFFSET)
}

pub fn fileentry_status(data: &[u8]) -> u64 {
    le_u64(data, PAR1FILEENTRY_STATUS_OFFSET)
}

pub fn fileentry_filesize(data: &[u8]) -> u64 {
    le_u64(data, PAR1FILEENTRY_FILESIZE_OFFSET)
}

pub fn fileentry_hashfull(data: &[u8]) -> Md5Hash {
    md5(data, PAR1FILEENTRY_HASHFULL_OFFSET)
}

pub fn fileentry_hash16k(data: &[u8]) -> Md5Hash {
    md5(data, PAR1FILEENTRY_HASH16K_OFFSET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test1_par1_magic() {
        // The bytes of the C++ global par1_magic.
        assert_eq!(PAR1_MAGIC, [b'P', b'A', b'R', 0, 0, 0, 0, 0]);
    }

    #[test]
    fn test2_header_layout() {
        // The packed struct sizes of the C++ original.
        assert_eq!(PAR1FILEHEADER_FILEVERSION_OFFSET, 8);
        assert_eq!(PAR1FILEHEADER_CONTROLHASH_OFFSET, 16);
        assert_eq!(PAR1FILEHEADER_SETHASH_OFFSET, 32);
        assert_eq!(PAR1FILEHEADER_VOLUMENUMBER_OFFSET, 48);
        assert_eq!(PAR1FILEHEADER_DATASIZE_OFFSET + 8, PAR1FILEHEADER_SIZE);
        assert_eq!(PAR1FILEENTRY_HASH16K_OFFSET + 16, PAR1FILEENTRY_NAME_OFFSET);
        assert_eq!(PAR1FILEENTRY_NAME_OFFSET, PAR1FILEENTRY_SIZE);
    }

    // Build a header buffer the way a PAR 1.0 file would hold it.
    fn make_header() -> Vec<u8> {
        let mut data = vec![0u8; PAR1FILEHEADER_SIZE];
        data[PAR1FILEHEADER_MAGIC_OFFSET..PAR1FILEHEADER_MAGIC_OFFSET + 8]
            .copy_from_slice(&PAR1_MAGIC);
        data[PAR1FILEHEADER_FILEVERSION_OFFSET..PAR1FILEHEADER_FILEVERSION_OFFSET + 4]
            .copy_from_slice(&0x00010000u32.to_le_bytes());
        data[PAR1FILEHEADER_PROGRAMVERSION_OFFSET..PAR1FILEHEADER_PROGRAMVERSION_OFFSET + 4]
            .copy_from_slice(&0x03000101u32.to_le_bytes());
        data[PAR1FILEHEADER_CONTROLHASH_OFFSET..PAR1FILEHEADER_CONTROLHASH_OFFSET + 16]
            .copy_from_slice(&[0x11; 16]);
        data[PAR1FILEHEADER_SETHASH_OFFSET..PAR1FILEHEADER_SETHASH_OFFSET + 16]
            .copy_from_slice(&[0x22; 16]);
        data[PAR1FILEHEADER_VOLUMENUMBER_OFFSET..PAR1FILEHEADER_VOLUMENUMBER_OFFSET + 8]
            .copy_from_slice(&3u64.to_le_bytes());
        data[PAR1FILEHEADER_NUMBEROFFILES_OFFSET..PAR1FILEHEADER_NUMBEROFFILES_OFFSET + 8]
            .copy_from_slice(&2u64.to_le_bytes());
        data[PAR1FILEHEADER_FILELISTOFFSET_OFFSET..PAR1FILEHEADER_FILELISTOFFSET_OFFSET + 8]
            .copy_from_slice(&96u64.to_le_bytes());
        data[PAR1FILEHEADER_FILELISTSIZE_OFFSET..PAR1FILEHEADER_FILELISTSIZE_OFFSET + 8]
            .copy_from_slice(&200u64.to_le_bytes());
        data[PAR1FILEHEADER_DATAOFFSET_OFFSET..PAR1FILEHEADER_DATAOFFSET_OFFSET + 8]
            .copy_from_slice(&296u64.to_le_bytes());
        data[PAR1FILEHEADER_DATASIZE_OFFSET..PAR1FILEHEADER_DATASIZE_OFFSET + 8]
            .copy_from_slice(&1024u64.to_le_bytes());
        data
    }

    #[test]
    fn test3_header_fields() {
        let data = make_header();
        assert_eq!(fileheader_magic(&data), PAR1_MAGIC);
        assert_eq!(fileheader_fileversion(&data), 0x00010000);
        assert_eq!(fileheader_programversion(&data), 0x03000101);
        assert_eq!(fileheader_controlhash(&data), Md5Hash { hash: [0x11; 16] });
        assert_eq!(fileheader_sethash(&data), Md5Hash { hash: [0x22; 16] });
        assert_eq!(fileheader_volumenumber(&data), 3);
        assert_eq!(fileheader_numberoffiles(&data), 2);
        assert_eq!(fileheader_filelistoffset(&data), 96);
        assert_eq!(fileheader_filelistsize(&data), 200);
        assert_eq!(fileheader_dataoffset(&data), 296);
        assert_eq!(fileheader_datasize(&data), 1024);

        // Little endian on the wire.
        assert_eq!(
            &data[PAR1FILEHEADER_VOLUMENUMBER_OFFSET..PAR1FILEHEADER_VOLUMENUMBER_OFFSET + 8],
            &[3, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn test4_entry_fields() {
        let mut data = vec![0u8; PAR1FILEENTRY_SIZE + 8];
        data[PAR1FILEENTRY_ENTRYSIZE_OFFSET..PAR1FILEENTRY_ENTRYSIZE_OFFSET + 8]
            .copy_from_slice(&72u64.to_le_bytes());
        data[PAR1FILEENTRY_STATUS_OFFSET..PAR1FILEENTRY_STATUS_OFFSET + 8]
            .copy_from_slice(&FILEENTRYSTATUS_INPARITYVOLUME.to_le_bytes());
        data[PAR1FILEENTRY_FILESIZE_OFFSET..PAR1FILEENTRY_FILESIZE_OFFSET + 8]
            .copy_from_slice(&12345u64.to_le_bytes());
        data[PAR1FILEENTRY_HASHFULL_OFFSET..PAR1FILEENTRY_HASHFULL_OFFSET + 16]
            .copy_from_slice(&[0xab; 16]);
        data[PAR1FILEENTRY_HASH16K_OFFSET..PAR1FILEENTRY_HASH16K_OFFSET + 16]
            .copy_from_slice(&[0xcd; 16]);

        assert_eq!(fileentry_entrysize(&data), 72);
        assert_eq!(
            fileentry_status(&data),
            FILEENTRYSTATUS_INPARITYVOLUME
        );
        assert_eq!(fileentry_filesize(&data), 12345);
        assert_eq!(fileentry_hashfull(&data), Md5Hash { hash: [0xab; 16] });
        assert_eq!(fileentry_hash16k(&data), Md5Hash { hash: [0xcd; 16] });

        // The status flags match the C++ enum FILEENTRYSTATUS.
        assert_eq!(FILEENTRYSTATUS_INPARITYVOLUME, 1);
        assert_eq!(FILEENTRYSTATUS_CHECKED, 2);
        assert_ne!(FILEENTRYSTATUS_INPARITYVOLUME, FILEENTRYSTATUS_CHECKED);
    }
}
