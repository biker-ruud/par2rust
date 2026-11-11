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

//! Port of par2cmdline's descriptionpacket.h / descriptionpacket.cpp.
//!
//! The description packet records details about a file (including its name,
//! size, and the Hash of both the whole file and the first 16k of the file).
//!
//! C++ `std::string` is byte oriented, so filenames are carried as `Vec<u8>`
//! here (the C++ `sout` stream parameter was never used and is dropped).

use std::io::Write;

use crate::criticalpacket::CriticalPacket;
use crate::diskfile::DiskFile;
use crate::md5::{Md5Context, Md5Hash};
use crate::noiselevel::NoiseLevel;
use crate::par2fileformat::{
    self, FILEDESCRIPTIONPACKET_FILEID_OFFSET, FILEDESCRIPTIONPACKET_HASH16K_OFFSET,
    FILEDESCRIPTIONPACKET_HASHFULL_OFFSET, FILEDESCRIPTIONPACKET_LENGTH_OFFSET,
    FILEDESCRIPTIONPACKET_NAME_OFFSET, FILEDESCRIPTIONPACKET_SIZE,
    FILEDESCRIPTIONPACKET_TYPE, HDR_MAGIC_OFFSET, HDR_TYPE_OFFSET, PACKET_HEADER_SIZE,
    PACKET_MAGIC,
};

pub struct DescriptionPacket {
    critical: CriticalPacket,
}

impl DescriptionPacket {
    pub fn new() -> DescriptionPacket {
        DescriptionPacket {
            critical: CriticalPacket::new(),
        }
    }

    pub fn critical(&self) -> &CriticalPacket {
        &self.critical
    }

    pub fn critical_mut(&mut self) -> &mut CriticalPacket {
        &mut self.critical
    }

    /// Construct the packet and store the filename and size.
    pub fn create(&mut self, filename: &[u8], filesize: u64) -> bool {
        // Allocate some extra bytes for the packet in memory so that a
        // strlen()-style scan can be used on the filename. The extra bytes
        // do not get written to disk.
        let packetlength = FILEDESCRIPTIONPACKET_SIZE + ((3 + filename.len()) & !3);
        self.critical.allocate_packet(packetlength, 4);

        // Store everything that is currently known in the packet.
        {
            let data = self.critical.data_mut();
            data[HDR_MAGIC_OFFSET..HDR_MAGIC_OFFSET + 8].copy_from_slice(&PACKET_MAGIC);
            par2fileformat::set_packet_length(data, packetlength as u64);
            // packet->header.hash;  // Not known yet
            // packet->header.setid; // Not known yet
            data[HDR_TYPE_OFFSET..HDR_TYPE_OFFSET + 16]
                .copy_from_slice(&FILEDESCRIPTIONPACKET_TYPE);

            // packet->fileid;   // Not known yet
            // packet->hashfull; // Not known yet
            // packet->hash16k;  // Not known yet
            data[FILEDESCRIPTIONPACKET_LENGTH_OFFSET..FILEDESCRIPTIONPACKET_LENGTH_OFFSET + 8]
                .copy_from_slice(&filesize.to_le_bytes());

            data[FILEDESCRIPTIONPACKET_NAME_OFFSET..FILEDESCRIPTIONPACKET_NAME_OFFSET
                + filename.len()]
                .copy_from_slice(filename);
        }

        true
    }

    /// Store the computed Hash values in the packet.
    pub fn hash16k(&mut self, hash: &Md5Hash) {
        self.critical.data_mut()[FILEDESCRIPTIONPACKET_HASH16K_OFFSET
            ..FILEDESCRIPTIONPACKET_HASH16K_OFFSET + 16]
            .copy_from_slice(&hash.hash);
    }

    pub fn hashfull(&mut self, hash: &Md5Hash) {
        self.critical.data_mut()[FILEDESCRIPTIONPACKET_HASHFULL_OFFSET
            ..FILEDESCRIPTIONPACKET_HASHFULL_OFFSET + 16]
            .copy_from_slice(&hash.hash);
    }

    /// Compute and store the file id hash from information in the packet.
    pub fn compute_fileid(&mut self) {
        // Compute the fileid from the hash, length, and name fields in the
        // packet.
        let namelen = strlen(self.critical.data(), FILEDESCRIPTIONPACKET_NAME_OFFSET);
        let start = FILEDESCRIPTIONPACKET_HASH16K_OFFSET;
        let end = FILEDESCRIPTIONPACKET_SIZE + namelen;
        let mut context = Md5Context::new();
        context.update(&self.critical.data()[start..end]);
        let fileid = context.finalize();

        self.critical.data_mut()[FILEDESCRIPTIONPACKET_FILEID_OFFSET
            ..FILEDESCRIPTIONPACKET_FILEID_OFFSET + 16]
            .copy_from_slice(&fileid.hash);
    }

    /// Return the size of the file.
    pub fn filesize(&self) -> u64 {
        debug_assert!(!self.critical.data().is_empty());

        u64::from_le_bytes(
            self.critical.data()[FILEDESCRIPTIONPACKET_LENGTH_OFFSET
                ..FILEDESCRIPTIONPACKET_LENGTH_OFFSET + 8]
                .try_into()
                .unwrap(),
        )
    }

    /// Load a description packet from a specified file.
    pub fn load(
        &mut self,
        diskfile: &mut DiskFile,
        offset: u64,
        header: &[u8],
        serr: &mut dyn Write,
    ) -> bool {
        let length = par2fileformat::packet_length(header) as usize;

        // Is the packet big enough
        if length <= FILEDESCRIPTIONPACKET_SIZE {
            return false;
        }

        // Is the packet too large (what is the longest permissible filename)
        if length - FILEDESCRIPTIONPACKET_SIZE > 100000 {
            return false;
        }

        // Allocate the packet (with a little extra so we will have NULLs
        // after the filename)
        self.critical.allocate_packet(length, 4);

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

        // Are the file and 16k hashes consistent
        let filesize = self.filesize();
        let hash16k_offset = FILEDESCRIPTIONPACKET_HASH16K_OFFSET;
        let hashfull_offset = FILEDESCRIPTIONPACKET_HASHFULL_OFFSET;
        if filesize <= 16384
            && self.critical.data()[hash16k_offset..hash16k_offset + 16]
                != self.critical.data()[hashfull_offset..hashfull_offset + 16]
        {
            return false;
        }

        true
    }

    /// Return the name of the file.
    ///
    /// NB whilst the file format does not guarantee that the name will have
    /// a NULL termination character, par2cmdline always allocates a little
    /// extra data and fills it with NULLs to allow the filename to be
    /// directly read out of the packet.
    pub fn filename(&self) -> Vec<u8> {
        debug_assert!(!self.critical.data().is_empty());

        let start = FILEDESCRIPTIONPACKET_NAME_OFFSET;
        let end = start + strlen(self.critical.data(), start);
        self.critical.data()[start..end].to_vec()
    }

    /// Get the Hash values from the packet.
    pub fn hashfull_(&self) -> Md5Hash {
        debug_assert!(!self.critical.data().is_empty());

        let mut hash = [0u8; 16];
        hash.copy_from_slice(
            &self.critical.data()[FILEDESCRIPTIONPACKET_HASHFULL_OFFSET
                ..FILEDESCRIPTIONPACKET_HASHFULL_OFFSET + 16],
        );
        Md5Hash { hash }
    }

    pub fn hash16k_(&self) -> Md5Hash {
        debug_assert!(!self.critical.data().is_empty());

        let mut hash = [0u8; 16];
        hash.copy_from_slice(
            &self.critical.data()[FILEDESCRIPTIONPACKET_HASH16K_OFFSET
                ..FILEDESCRIPTIONPACKET_HASH16K_OFFSET + 16],
        );
        Md5Hash { hash }
    }

    pub fn fileid(&self) -> Md5Hash {
        debug_assert!(!self.critical.data().is_empty());

        let mut hash = [0u8; 16];
        hash.copy_from_slice(
            &self.critical.data()[FILEDESCRIPTIONPACKET_FILEID_OFFSET
                ..FILEDESCRIPTIONPACKET_FILEID_OFFSET + 16],
        );
        Md5Hash { hash }
    }

    /// Returns the URL-style encoding of a character:
    /// "%HH" where H is a hex-digit.
    pub fn url_encode_char(c: u8) -> String {
        let high_bits = (c >> 4) & 0xf;
        let low_bits = c & 0xf;
        let hi = if high_bits < 10 {
            (b'0' + high_bits) as char
        } else {
            (b'A' + (high_bits - 10)) as char
        };
        let lo = if low_bits < 10 {
            (b'0' + low_bits) as char
        } else {
            (b'A' + (low_bits - 10)) as char
        };
        format!("%{}{}", hi, lo)
    }

    /// Converts the filename from that on disk to the version
    /// in the Par file.  Par uses HTML-style slashes ('/' or
    /// UNIX-style slashes) to denote directories.  This
    /// function also prints a warning if a character may be
    /// illegal on another machine.
    ///
    /// NOTE: I decided to warn users, not change the files.
    /// If a user is just backing up files on their own system
    /// and not sending them to users on another operating
    /// system, we don't want to change the filenames.
    pub fn translate_filename_from_local_to_par2(
        serr: &mut dyn Write,
        noiselevel: NoiseLevel,
        local_filename: &[u8],
    ) -> Vec<u8> {
        let mut par2_encoded_filename: Vec<u8> = Vec::new();

        for &byte in local_filename {
            let mut ch = byte;

            let mut ok = true;
            if ch < 32 {
                ok = false;
            } else {
                match ch {
                    b'"' | b'*' | b':' | b'<' | b'>' | b'?' | b'|' => ok = false,
                    _ => {}
                }
            }

            if !ok && noiselevel >= NoiseLevel::Normal {
                let _ = writeln!(
                    serr,
                    "WARNING: A filename contains the character '{}' which some systems do not allow in filenames.",
                    ch as char
                );
            }

            #[cfg(windows)]
            {
                // replace Windows-slash with HTML-slash
                if ch == b'\\' {
                    ch = b'/';
                }
            }
            #[cfg(not(windows))]
            {
                if ch == b'\\' && noiselevel >= NoiseLevel::Normal {
                    let _ = writeln!(
                        serr,
                        "WARNING: Found Windows-style slash '\\' in filename.  Windows systems may have trouble with it."
                    );
                }
            }

            par2_encoded_filename.push(ch);
        }

        // Par files should never contain an absolute path.  On Windows,
        // These start "C:\...", etc.  An attacker could put an absolute
        // path into a Par file and overwrite system files.
        if par2_encoded_filename.len() > 1 && par2_encoded_filename[1] == b':'
            && noiselevel >= NoiseLevel::Normal
        {
            let _ = writeln!(
                serr,
                "WARNING: The second character in the filename \"{}\" is a colon (':').",
                String::from_utf8_lossy(&par2_encoded_filename)
            );
            let _ = writeln!(
                serr,
                "       This may be interpreted by Windows systems as an absolute path."
            );
            let _ = writeln!(
                serr,
                "       This file may be ignored by Par clients because absolute paths"
            );
            let _ = writeln!(
                serr,
                "        are a way for an attacker to overwrite system files."
            );
        }
        if !par2_encoded_filename.is_empty() && par2_encoded_filename[0] == b'/'
            && noiselevel >= NoiseLevel::Normal
        {
            let _ = writeln!(
                serr,
                "WARNING: The first character in the filename \"{}\" is an HTML-slash ('/').",
                String::from_utf8_lossy(&par2_encoded_filename)
            );
            let _ = writeln!(
                serr,
                "       This may be interpreted by UNIX systems as an absolute path."
            );
            let _ = writeln!(
                serr,
                "       This file may be ignored by Par clients because absolute paths"
            );
            let _ = writeln!(
                serr,
                "        are a way for an attacker to overwrite system files."
            );
        }
        if contains_subsequence(&par2_encoded_filename, b"../")
            && noiselevel >= NoiseLevel::Quiet
        {
            let _ = writeln!(
                serr,
                "WARNING: The filename \"{}\" contains \"..\".",
                String::from_utf8_lossy(&par2_encoded_filename)
            );
            let _ = writeln!(
                serr,
                "       This is a parent directory. This file may be ignored"
            );
            let _ = writeln!(
                serr,
                "       by Par clients because parent directories are a way"
            );
            let _ = writeln!(
                serr,
                "       for an attacker to overwrite system files."
            );
        }
        if par2_encoded_filename.len() > 255 && noiselevel >= NoiseLevel::Normal {
            let _ = writeln!(
                serr,
                "WARNING: A filename is over 255 characters.  That may be too long"
            );
            let _ = writeln!(
                serr,
                "         for Windows systems to handle."
            );
        }

        par2_encoded_filename
    }

    /// Take a filename that matches the PAR2 standard ('/' slashes, etc.)
    /// and convert it to a legal filename on the local system.
    /// While at it, try to fix things: illegal characters, attempts
    /// to write an absolute path, directories named "..", etc.
    ///
    /// This implementation changes any illegal char into the URL-style
    /// encoding of %HH where H is a hex-digit.
    ///
    /// NOTE: Windows limits path names to 255 characters.  I'm not
    /// sure that anything can be done here for that.
    pub fn translate_filename_from_par2_to_local(
        serr: &mut dyn Write,
        noiselevel: NoiseLevel,
        par2_encoded_filename: &[u8],
    ) -> Vec<u8> {
        let mut local_filename: Vec<u8> = Vec::new();

        for &byte in par2_encoded_filename {
            let mut ch = byte;

            let mut ok = true;
            #[cfg(windows)]
            {
                if ch < 32 {
                    ok = false;
                } else {
                    match ch {
                        b'"' | b'*' | b':' | b'<' | b'>' | b'?' | b'|' => ok = false,
                        _ => {}
                    }
                }
            }
            #[cfg(not(windows))]
            {
                // other UNIXes allow anything.
                if ch < 32 {
                    ok = false;
                }
            }

            // replace unix / to windows \ or windows \ to unix /
            #[cfg(windows)]
            {
                if ch == b'/' {
                    ch = b'\\';
                }
            }
            #[cfg(not(windows))]
            {
                if ch == b'\\' {
                    // This is a legal Par2 character, but assume someone screwed up.
                    if noiselevel >= NoiseLevel::Quiet {
                        let _ = writeln!(
                            serr,
                            "INFO: Found Windows-style slash in filename.  Changing to UNIX-style slash."
                        );
                    }
                    ch = b'/';
                }
            }

            if ok {
                local_filename.push(ch);
            } else {
                if noiselevel >= NoiseLevel::Quiet {
                    let _ = writeln!(
                        serr,
                        "INFO: Found illegal character '{}' in filename.  Changed it to \"{}\"",
                        ch as char,
                        DescriptionPacket::url_encode_char(ch)
                    );
                }
                // convert problem characters to hex
                local_filename.extend(DescriptionPacket::url_encode_char(ch).as_bytes());
            }
        }

        #[cfg(windows)]
        {
            // Par files should never contain an absolute path.  On Windows,
            // These start "C:\...", etc.  An attacker could put an absolute
            // path into a Par file and overwrite system files.  For Windows
            // systems, we've already changed ':' to "%3A", so any absolute
            // path should have become relative.

            // Replace any references to ".." which could also be used by
            // an attacker.
            let dots = DescriptionPacket::url_encode_char(b'.')
                + DescriptionPacket::url_encode_char(b'.').as_str();
            while let Some(index) = find_subsequence(&local_filename, b"..\\") {
                if noiselevel >= NoiseLevel::Quiet {
                    let _ = writeln!(
                        serr,
                        "INFO: Found attempt to write parent directory.  Changing \"..\" to \"{}\"",
                        dots
                    );
                }

                local_filename.splice(index..index + 2, dots.as_bytes().iter().copied());
            }
        }
        #[cfg(not(windows))]
        {
            // On UNIX systems, we don't want to allow filename to start with a slash,
            // because someone could be sneakily trying to overwrite a system file.
            if !local_filename.is_empty() && local_filename[0] == b'/' {
                if noiselevel >= NoiseLevel::Quiet {
                    let _ = writeln!(
                        serr,
                        "INFO: Found attempt to write absolute path.  Changing '/' at start of filename to \"{}\"",
                        DescriptionPacket::url_encode_char(b'/')
                    );
                }

                local_filename.splice(
                    0..1,
                    DescriptionPacket::url_encode_char(b'/').as_bytes().iter().copied(),
                );
            }

            // Replace any references to ".." which could be sneaking
            let dots = DescriptionPacket::url_encode_char(b'.')
                + DescriptionPacket::url_encode_char(b'.').as_str();
            while let Some(index) = find_subsequence(&local_filename, b"../") {
                if noiselevel >= NoiseLevel::Quiet {
                    let _ = writeln!(
                        serr,
                        "INFO: Found attempt to write parent directory.  Changing \"..\" to \"{}\"",
                        dots
                    );
                }
                local_filename.splice(index..index + 2, dots.as_bytes().iter().copied());
            }
        }

        local_filename
    }
}

impl Default for DescriptionPacket {
    fn default() -> Self {
        DescriptionPacket::new()
    }
}

/// strlen() equivalent: index of the first NUL at or after `offset`.
fn strlen(data: &[u8], offset: usize) -> usize {
    let mut i = offset;
    while data[i] != 0 {
        i += 1;
    }
    i - offset
}

fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    find_subsequence(haystack, needle).is_some()
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::par2fileformat::{packet_length, packet_magic, packet_type};
    use std::fs;

    #[test]
    fn test1_create() {
        let mut packet = DescriptionPacket::new();
        assert!(packet.create(b"sub\\dir\\file.dat", 123456));

        let data = packet.critical().data();
        let packetlength = packet.critical().packet_length();
        assert_eq!(packetlength, FILEDESCRIPTIONPACKET_SIZE + 16);
        assert_eq!(packet_length(data), packetlength as u64);
        assert_eq!(packet_magic(data), PACKET_MAGIC);
        assert_eq!(packet_type(data), FILEDESCRIPTIONPACKET_TYPE);
        assert_eq!(packet.filesize(), 123456);
        assert_eq!(packet.filename(), b"sub\\dir\\file.dat");
    }

    #[test]
    fn test2_compute_fileid() {
        let mut packet = DescriptionPacket::new();
        assert!(packet.create(b"hello.txt", 10));
        let hashfull = Md5Hash { hash: [1; 16] };
        let hash16k = Md5Hash { hash: [2; 16] };
        packet.hashfull(&hashfull);
        packet.hash16k(&hash16k);
        packet.compute_fileid();

        // fileid = MD5 of hash16k, length, name (no NUL).
        let mut context = Md5Context::new();
        context.update(&[2u8; 16]);
        context.update(&10u64.to_le_bytes());
        context.update(b"hello.txt");
        assert_eq!(packet.fileid(), context.finalize());
        assert_eq!(packet.hashfull_(), hashfull);
        assert_eq!(packet.hash16k_(), hash16k);
    }

    #[test]
    fn test3_load_round_trip() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_descriptionpacket_{}.par2", std::process::id()));
        let _ = fs::remove_file(&path);

        let mut packet = DescriptionPacket::new();
        assert!(packet.create(b"round_trip.bin", 99));
        let hash = Md5Hash { hash: [7; 16] };
        packet.hashfull(&hash);
        packet.hash16k(&hash); // filesize <= 16384, so hashes must match
        packet.compute_fileid();
        par2fileformat::set_packet_setid(
            packet.critical_mut().data_mut(),
            &Md5Hash { hash: [8; 16] },
        );

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

        let mut loaded = DescriptionPacket::new();
        assert!(loaded.load(&mut diskfile, 0, &raw, &mut serr));
        diskfile.close();

        assert_eq!(loaded.filesize(), 99);
        assert_eq!(loaded.filename(), b"round_trip.bin");
        assert_eq!(loaded.fileid(), packet.fileid());
        assert_eq!(loaded.hash16k_(), hash);

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test4_load_rejects_bad_packets() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_descriptionpacket_bad_{}.par2", std::process::id()));
        let _ = fs::remove_file(&path);

        let body_len = FILEDESCRIPTIONPACKET_SIZE + 100001;
        let mut raw = vec![0u8; body_len];
        raw[..8].copy_from_slice(&PACKET_MAGIC);

        let mut write_and_load = |length: u64, filesize: u64, mismatch_hashes: bool| -> bool {
            raw[8..16].copy_from_slice(&length.to_le_bytes());
            raw[FILEDESCRIPTIONPACKET_LENGTH_OFFSET..FILEDESCRIPTIONPACKET_LENGTH_OFFSET + 8]
                .copy_from_slice(&filesize.to_le_bytes());
            raw[FILEDESCRIPTIONPACKET_HASH16K_OFFSET] = 0;
            raw[FILEDESCRIPTIONPACKET_HASHFULL_OFFSET] = if mismatch_hashes { 1 } else { 0 };
            fs::write(&path, &raw).unwrap();

            let mut diskfile = DiskFile::new();
            assert!(diskfile.open(path.to_str().unwrap(), &mut serr));
            let mut packet = DescriptionPacket::new();
            let ok = packet.load(&mut diskfile, 0, &raw[..PACKET_HEADER_SIZE], &mut serr);
            diskfile.close();
            ok
        };

        // Too small.
        assert!(!write_and_load(FILEDESCRIPTIONPACKET_SIZE as u64, 1, false));
        // One byte of name is fine.
        assert!(write_and_load(
            FILEDESCRIPTIONPACKET_SIZE as u64 + 1,
            20000,
            false
        ));
        // Too large.
        assert!(!write_and_load(
            FILEDESCRIPTIONPACKET_SIZE as u64 + 100001,
            1,
            false
        ));
        // Small file with mismatched hashes.
        assert!(!write_and_load(
            FILEDESCRIPTIONPACKET_SIZE as u64 + 1,
            16384,
            true
        ));
        // Large file with mismatched hashes is fine.
        assert!(write_and_load(
            FILEDESCRIPTIONPACKET_SIZE as u64 + 1,
            16385,
            true
        ));

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test5_url_encode_char() {
        assert_eq!(DescriptionPacket::url_encode_char(b'a'), "%61");
        assert_eq!(DescriptionPacket::url_encode_char(0), "%00");
        assert_eq!(DescriptionPacket::url_encode_char(0xff), "%FF");
        assert_eq!(DescriptionPacket::url_encode_char(b'/'), "%2F");
        assert_eq!(DescriptionPacket::url_encode_char(b'.'), "%2E");
    }

    #[test]
    fn test6_translate_local_to_par2() {
        let mut serr = std::io::sink();
        let out = DescriptionPacket::translate_filename_from_local_to_par2(
            &mut serr,
            NoiseLevel::Silent,
            b"sub\\dir:file",
        );
        #[cfg(windows)]
        assert_eq!(out, b"sub/dir:file");
        #[cfg(not(windows))]
        assert_eq!(out, b"sub\\dir:file");

        // Warnings do not change the result.
        let mut serr = std::io::sink();
        let out = DescriptionPacket::translate_filename_from_local_to_par2(
            &mut serr,
            NoiseLevel::Normal,
            b"../evil",
        );
        assert_eq!(out, b"../evil");
    }

    #[test]
    fn test7_translate_par2_to_local() {
        let mut serr = std::io::sink();
        let out = DescriptionPacket::translate_filename_from_par2_to_local(
            &mut serr,
            NoiseLevel::Silent,
            b"sub/dir:file",
        );
        #[cfg(windows)]
        assert_eq!(out, b"sub\\dir%3Afile");
        #[cfg(not(windows))]
        assert_eq!(out, b"sub/dir:file");

        // Parent directory attempt is neutralised on Windows.
        let mut serr = std::io::sink();
        let out = DescriptionPacket::translate_filename_from_par2_to_local(
            &mut serr,
            NoiseLevel::Quiet,
            b"a/..\\b",
        );
        #[cfg(windows)]
        assert_eq!(out, b"a\\%2E%2E\\b");
        #[cfg(not(windows))]
        assert_eq!(out, b"a/..\\b");
    }
}
