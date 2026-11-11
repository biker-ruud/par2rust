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

//! Port of par2cmdline's creatorpacket.h / creatorpacket.cpp.
//!
//! The creator packet records details as to which PAR2 client
//! created a particular recovery file.
//!
//! The PAR 2.0 specification requires the presence of a
//! creator packet, but it is not actually needed for the
//! verification or recovery of damaged files.

use std::io::Write;

use crate::criticalpacket::CriticalPacket;
use crate::diskfile::DiskFile;
use crate::md5::{Md5Context, Md5Hash};
use crate::par2fileformat::{
    self, CREATORPACKET_CLIENT_OFFSET, CREATORPACKET_SIZE, CREATORPACKET_TYPE, HDR_MAGIC_OFFSET,
    HDR_SETID_OFFSET, HDR_TYPE_OFFSET, PACKET_HEADER_SIZE, PACKET_MAGIC,
};

// C++ builds this string from the PACKAGE and VERSION macros; the values
// come from configure.ac (AC_INIT([par2cmdline], [1.4.0], ...)).
const CREATOR_STRING: &str = "Created by par2cmdline version 1.4.0.";

pub struct CreatorPacket {
    critical: CriticalPacket,
}

impl CreatorPacket {
    pub fn new() -> CreatorPacket {
        CreatorPacket {
            critical: CriticalPacket::new(),
        }
    }

    pub fn critical(&self) -> &CriticalPacket {
        &self.critical
    }

    pub fn critical_mut(&mut self) -> &mut CriticalPacket {
        &mut self.critical
    }

    /// Create a creator packet for a specified set id hash value.
    pub fn create(&mut self, setid: &Md5Hash) -> bool {
        // Allocate a packet just large enough for creator name
        let creator = CREATOR_STRING.as_bytes();
        let packetlength = CREATORPACKET_SIZE + ((3 + creator.len()) & !3);
        self.critical.allocate_packet(packetlength, 0);

        // Fill in the details we know
        {
            let data = self.critical.data_mut();
            data[HDR_MAGIC_OFFSET..HDR_MAGIC_OFFSET + 8].copy_from_slice(&PACKET_MAGIC);
            par2fileformat::set_packet_length(data, packetlength as u64);
            par2fileformat::set_packet_setid(data, setid);
            data[HDR_TYPE_OFFSET..HDR_TYPE_OFFSET + 16].copy_from_slice(&CREATORPACKET_TYPE);

            // Copy the creator description into the packet
            data[CREATORPACKET_CLIENT_OFFSET..CREATORPACKET_CLIENT_OFFSET + creator.len()]
                .copy_from_slice(creator);
        }

        // Compute the packet hash
        let data = self.critical.data();
        let mut packetcontext = Md5Context::new();
        packetcontext.update(&data[HDR_SETID_OFFSET..packetlength]);
        let hash = packetcontext.finalize();
        par2fileformat::set_packet_hash(self.critical.data_mut(), &hash);

        true
    }

    /// Load a creator packet from a specified file.
    pub fn load(
        &mut self,
        diskfile: &mut DiskFile,
        offset: u64,
        header: &[u8],
        serr: &mut dyn Write,
    ) -> bool {
        let length = par2fileformat::packet_length(header) as usize;

        // Is the packet long enough
        if length <= CREATORPACKET_SIZE {
            return false;
        }

        // Is the packet too large (what is the longest reasonable creator description)
        if length - CREATORPACKET_SIZE > 100000 {
            return false;
        }

        // Allocate the packet (with a little extra so we will have NULLs after the description)
        self.critical.allocate_packet(length, 4);

        // Copy the header into the packet
        self.critical.data_mut()[..PACKET_HEADER_SIZE]
            .copy_from_slice(&header[..PACKET_HEADER_SIZE]);

        // Load the rest of the packet from disk
        diskfile.read(
            offset + PACKET_HEADER_SIZE as u64,
            &mut self.critical.data_mut()[PACKET_HEADER_SIZE..length],
            serr,
        )
    }
}

impl Default for CreatorPacket {
    fn default() -> Self {
        CreatorPacket::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::par2fileformat::{packet_hash, packet_length, packet_magic, packet_type};
    use std::fs;

    #[test]
    fn test1_create() {
        let setid = Md5Hash { hash: [0xa5; 16] };
        let mut packet = CreatorPacket::new();
        assert!(packet.create(&setid));

        let creator = CREATOR_STRING.as_bytes();
        let packetlength = packet.critical().packet_length();
        assert_eq!(packetlength, CREATORPACKET_SIZE + ((3 + creator.len()) & !3));

        let data = packet.critical().data();
        assert_eq!(packet_length(data), packetlength as u64);
        assert_eq!(packet_magic(data), PACKET_MAGIC);
        assert_eq!(packet_type(data), CREATORPACKET_TYPE);
        assert_eq!(par2fileformat::packet_setid(data), setid);
        assert_eq!(
            &data[CREATORPACKET_CLIENT_OFFSET..CREATORPACKET_CLIENT_OFFSET + creator.len()],
            creator
        );

        // packet_hash = MD5 of everything from setid onwards.
        let mut packetcontext = Md5Context::new();
        packetcontext.update(&data[HDR_SETID_OFFSET..packetlength]);
        assert_eq!(packet_hash(data), packetcontext.finalize());
    }

    #[test]
    fn test2_load_round_trip() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_creatorpacket_{}.par2", std::process::id()));
        let _ = fs::remove_file(&path);

        let setid = Md5Hash { hash: [0x5a; 16] };
        let mut packet = CreatorPacket::new();
        assert!(packet.create(&setid));

        let mut diskfile = DiskFile::new();
        assert!(diskfile.create(path.to_str().unwrap(), 0, &mut serr));
        assert!(packet
            .critical()
            .write_packet(&mut diskfile, 0, &mut serr));
        diskfile.close();

        let raw = fs::read(&path).unwrap();
        let mut diskfile = DiskFile::new();
        assert!(diskfile.open(path.to_str().unwrap(), &mut serr));

        let mut loaded = CreatorPacket::new();
        assert!(loaded.load(&mut diskfile, 0, &raw, &mut serr));
        diskfile.close();

        assert_eq!(loaded.critical().packet_length(), packet.critical().packet_length());
        // The loaded packet carries 4 bytes of extra padding, so compare
        // only the packet bytes themselves.
        let len = packet.critical().packet_length();
        assert_eq!(&loaded.critical().data()[..len], &packet.critical().data()[..len]);

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test3_load_rejects_bad_packets() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_creatorpacket_bad_{}.par2", std::process::id()));
        let _ = fs::remove_file(&path);

        let body_len = CREATORPACKET_SIZE + 100001;
        let mut raw = vec![0u8; body_len];
        raw[..8].copy_from_slice(&PACKET_MAGIC);

        let mut write_and_load = |length: u64| -> bool {
            raw[8..16].copy_from_slice(&length.to_le_bytes());
            fs::write(&path, &raw).unwrap();

            let mut diskfile = DiskFile::new();
            assert!(diskfile.open(path.to_str().unwrap(), &mut serr));
            let mut packet = CreatorPacket::new();
            let ok = packet.load(&mut diskfile, 0, &raw[..PACKET_HEADER_SIZE], &mut serr);
            diskfile.close();
            ok
        };

        // Exactly header-sized (or smaller) is too small.
        assert!(!write_and_load(CREATORPACKET_SIZE as u64));
        // One byte of description is fine.
        assert!(write_and_load(CREATORPACKET_SIZE as u64 + 1));
        // More than 100000 bytes of description is too large.
        assert!(!write_and_load(CREATORPACKET_SIZE as u64 + 100001));

        fs::remove_file(&path).unwrap();
    }
}
