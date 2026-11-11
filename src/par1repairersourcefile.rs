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

//! Port of par1repairersourcefile.h / par1repairersourcefile.cpp.
//!
//! The `Par1RepairerSourceFile` object is used during verification and repair
//! to record details about a particular source file and the data blocks for
//! that file.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use crate::datablock::DataBlock;
use crate::descriptionpacket::DescriptionPacket;
use crate::diskfile::DiskFile;
use crate::md5::Md5Hash;
use crate::noiselevel::NoiseLevel;
use crate::par1fileformat::{
    fileentry_entrysize, fileentry_filesize, fileentry_hash16k, fileentry_hashfull,
    PAR1FILEENTRY_NAME_OFFSET,
};

/// The C++ original took a `PAR1FILEENTRY*` pointing at the whole entry
/// (fixed fields followed by the UTF-16 name). Here the entry is the bytes
/// of that entry, read with the accessors in [`crate::par1fileformat`].
pub struct Par1RepairerSourceFile {
    filename: String,
    filesize: u64,
    hashfull: Md5Hash,
    hash16k: Md5Hash,

    sourceblock: DataBlock,
    targetblock: DataBlock,

    targetexists: bool, // Whether the target file exists
    targetfile: Option<Rc<RefCell<DiskFile>>>, // The final version of the file
    completefile: Option<Rc<RefCell<DiskFile>>>, // A complete version of the file
}

impl Par1RepairerSourceFile {
    /// Construct the object from a PAR 1.0 file entry.
    ///
    /// `fileentry` must hold at least `entrysize` bytes: the fixed portion
    /// followed by the file name. The caller (the repairer) has already
    /// checked that the entry size is consistent with the file list.
    pub fn new(
        serr: &mut dyn Write,
        noiselevel: NoiseLevel,
        fileentry: &[u8],
        searchpath: &str,
    ) -> Par1RepairerSourceFile {
        let hashfull = fileentry_hashfull(fileentry);
        let hash16k = fileentry_hash16k(fileentry);

        let mut sourcefile = Par1RepairerSourceFile {
            filename: String::new(),
            filesize: fileentry_filesize(fileentry),
            hashfull,
            hash16k,
            sourceblock: DataBlock::new(),
            targetblock: DataBlock::new(),
            targetexists: false,
            targetfile: None,
            completefile: None,
        };

        let entrysize = fileentry_entrysize(fileentry) as usize;
        let namelen = (entrysize - PAR1FILEENTRY_NAME_OFFSET) / 2;

        // The name is stored as UTF-16 code units.
        let mut filename: Vec<u8> = Vec::new();
        for index in 0..namelen {
            // We can't deal with Unicode characters!
            let ch = u16::from_le_bytes(
                fileentry[PAR1FILEENTRY_NAME_OFFSET + index * 2
                    ..PAR1FILEENTRY_NAME_OFFSET + index * 2 + 2]
                    .try_into()
                    .unwrap(),
            );
            if ch >= 256 {
                // Convert the Unicode character to two characters
                filename.push((ch & 255) as u8);
                filename.push((ch >> 8) as u8);
            } else {
                filename.push((ch & 255) as u8);
            }
        }

        // Translate any characters the OS does not like;
        let mut filename =
            DescriptionPacket::translate_filename_from_par2_to_local(serr, noiselevel, &filename);

        // Strip the path from the filename. The C++ tries PATHSEP, then
        // ALTPATHSEP, then (on Windows only) ':' in that order, using the
        // last occurrence of the first one found.
        let where_ = path_separator_position(&filename);
        if let Some(where_) = where_ {
            filename = filename[where_ + 1..].to_vec();
        }

        // The C++ std::string held raw bytes; this port keeps local paths as
        // String throughout, so the translated bytes are decoded as UTF-8.
        sourcefile.filename = searchpath.to_string() + &String::from_utf8_lossy(&filename);

        sourcefile
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn filesize(&self) -> u64 {
        self.filesize
    }

    pub fn hashfull(&self) -> Md5Hash {
        self.hashfull
    }

    pub fn hash16k(&self) -> Md5Hash {
        self.hash16k
    }

    /// Set/Get which DiskFile will contain the final repaired version of the file.
    pub fn set_target_file(&mut self, diskfile: Option<Rc<RefCell<DiskFile>>>) {
        self.targetfile = diskfile;
    }

    pub fn get_target_file(&self) -> Option<Rc<RefCell<DiskFile>>> {
        self.targetfile.clone()
    }

    /// Set/Get whether or not the target file actually exists.
    pub fn set_target_exists(&mut self, exists: bool) {
        self.targetexists = exists;
    }

    pub fn get_target_exists(&self) -> bool {
        self.targetexists
    }

    /// Set/Get which DiskFile contains a full undamaged version of the source file.
    pub fn set_complete_file(&mut self, diskfile: Option<Rc<RefCell<DiskFile>>>) {
        self.completefile = diskfile.clone();

        match diskfile {
            Some(diskfile) => {
                let length = diskfile.borrow().file_size();
                self.sourceblock.set_location(diskfile, 0);
                self.sourceblock.set_length(length);
            }
            None => {
                self.sourceblock.clear_location();
                self.sourceblock.set_length(0);
            }
        }
    }

    pub fn get_complete_file(&self) -> Option<Rc<RefCell<DiskFile>>> {
        self.completefile.clone()
    }

    pub fn set_target_block(&mut self, diskfile: Rc<RefCell<DiskFile>>) {
        let length = diskfile.borrow().file_size();
        self.targetblock.set_location(diskfile, 0);
        self.targetblock.set_length(length);
    }

    pub fn sourceblock(&self) -> &DataBlock {
        &self.sourceblock
    }

    pub fn sourceblock_mut(&mut self) -> &mut DataBlock {
        &mut self.sourceblock
    }

    pub fn targetblock(&self) -> &DataBlock {
        &self.targetblock
    }

    pub fn targetblock_mut(&mut self) -> &mut DataBlock {
        &mut self.targetblock
    }
}

/// The position of the path separator that the C++ constructor would cut at:
/// the last PATHSEP, or failing that the last ALTPATHSEP, or failing that
/// (on Windows only) the last ':'.
fn path_separator_position(filename: &[u8]) -> Option<usize> {
    #[cfg(windows)]
    {
        filename
            .iter()
            .rposition(|&b| b == b'\\')
            .or_else(|| filename.iter().rposition(|&b| b == b'/'))
            .or_else(|| filename.iter().rposition(|&b| b == b':'))
    }
    #[cfg(not(windows))]
    {
        filename
            .iter()
            .rposition(|&b| b == b'/')
            .or_else(|| filename.iter().rposition(|&b| b == b'\\'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use crate::par1fileformat::{
        FILEENTRYSTATUS_INPARITYVOLUME, PAR1FILEENTRY_ENTRYSIZE_OFFSET,
        PAR1FILEENTRY_FILESIZE_OFFSET, PAR1FILEENTRY_HASH16K_OFFSET,
        PAR1FILEENTRY_HASHFULL_OFFSET, PAR1FILEENTRY_SIZE, PAR1FILEENTRY_STATUS_OFFSET,
    };

    // Build the bytes of a PAR1FILEENTRY holding the given UTF-16 name.
    fn make_entry(name: &[u16], filesize: u64, status: u64) -> Vec<u8> {
        let entrysize = PAR1FILEENTRY_SIZE + name.len() * 2;
        let mut data = vec![0u8; entrysize];
        data[PAR1FILEENTRY_ENTRYSIZE_OFFSET..PAR1FILEENTRY_ENTRYSIZE_OFFSET + 8]
            .copy_from_slice(&(entrysize as u64).to_le_bytes());
        data[PAR1FILEENTRY_STATUS_OFFSET..PAR1FILEENTRY_STATUS_OFFSET + 8]
            .copy_from_slice(&status.to_le_bytes());
        data[PAR1FILEENTRY_FILESIZE_OFFSET..PAR1FILEENTRY_FILESIZE_OFFSET + 8]
            .copy_from_slice(&filesize.to_le_bytes());
        data[PAR1FILEENTRY_HASHFULL_OFFSET..PAR1FILEENTRY_HASHFULL_OFFSET + 16]
            .copy_from_slice(&[0x11; 16]);
        data[PAR1FILEENTRY_HASH16K_OFFSET..PAR1FILEENTRY_HASH16K_OFFSET + 16]
            .copy_from_slice(&[0x22; 16]);
        for (i, unit) in name.iter().enumerate() {
            data[PAR1FILEENTRY_NAME_OFFSET + i * 2
                ..PAR1FILEENTRY_NAME_OFFSET + i * 2 + 2]
                .copy_from_slice(&unit.to_le_bytes());
        }
        data
    }

    fn ascii_utf16(s: &str) -> Vec<u16> {
        s.bytes().map(u16::from).collect()
    }

    #[test]
    fn test1_plain_name() {
        let entry = make_entry(&ascii_utf16("file.dat"), 12345, FILEENTRYSTATUS_INPARITYVOLUME);
        let sourcefile =
            Par1RepairerSourceFile::new(&mut std::io::sink(), NoiseLevel::Silent, &entry, "");
        assert_eq!(sourcefile.filename(), "file.dat");
        assert_eq!(sourcefile.filesize(), 12345);
        assert_eq!(sourcefile.hashfull(), Md5Hash { hash: [0x11; 16] });
        assert_eq!(sourcefile.hash16k(), Md5Hash { hash: [0x22; 16] });
        assert!(!sourcefile.get_target_exists());
        assert!(sourcefile.get_target_file().is_none());
        assert!(sourcefile.get_complete_file().is_none());
        assert!(!sourcefile.sourceblock().is_set());
        assert!(!sourcefile.targetblock().is_set());
    }

    #[test]
    fn test2_path_stripped_and_searchpath_prepended() {
        // The PAR 2.0 style name uses '/' which translate converts to the
        // local separator; either way the path is stripped.
        let entry = make_entry(
            &ascii_utf16("sub/dir/name.dat"),
            1,
            FILEENTRYSTATUS_INPARITYVOLUME,
        );
        let sourcefile =
            Par1RepairerSourceFile::new(&mut std::io::sink(), NoiseLevel::Silent, &entry, "base\\");
        assert_eq!(sourcefile.filename(), "base\\name.dat");
    }

    #[test]
    fn test3_wide_character_split() {
        // A code unit >= 256 becomes two bytes, low byte first.
        let entry = make_entry(&[u16::from(b'A'), 0x4142], 1, FILEENTRYSTATUS_INPARITYVOLUME);
        let sourcefile =
            Par1RepairerSourceFile::new(&mut std::io::sink(), NoiseLevel::Silent, &entry, "");
        assert_eq!(sourcefile.filename(), "ABA");
    }

    #[test]
    fn test4_complete_and_target_blocks() {
        let entry = make_entry(&ascii_utf16("file.dat"), 100, FILEENTRYSTATUS_INPARITYVOLUME);
        let mut sourcefile =
            Par1RepairerSourceFile::new(&mut std::io::sink(), NoiseLevel::Silent, &entry, "");

        // A complete file sets the source block to the whole disk file.
        let dir = std::env::temp_dir();
        let path = dir.join("par1repairersourcefile_test4.dat");
        let _ = fs::remove_file(&path);
        fs::write(&path, b"x").unwrap();
        let mut diskfile = DiskFile::new();
        assert!(diskfile.open_with_size(
            path.to_str().unwrap(),
            4096,
            &mut std::io::sink()
        ));
        let diskfile = Rc::new(RefCell::new(diskfile));
        sourcefile.set_complete_file(Some(Rc::clone(&diskfile)));
        assert!(sourcefile.sourceblock().is_set());
        assert_eq!(sourcefile.sourceblock().offset(), 0);
        assert_eq!(sourcefile.sourceblock().length(), 4096);
        assert!(Rc::ptr_eq(
            &sourcefile.get_complete_file().unwrap(),
            &diskfile
        ));

        // Losing the complete file clears the source block again.
        sourcefile.set_complete_file(None);
        assert!(!sourcefile.sourceblock().is_set());
        assert_eq!(sourcefile.sourceblock().length(), 0);
        assert!(sourcefile.get_complete_file().is_none());

        // The target block always names the whole target file.
        sourcefile.set_target_block(Rc::clone(&diskfile));
        assert!(sourcefile.targetblock().is_set());
        assert_eq!(sourcefile.targetblock().length(), 4096);

        sourcefile.set_target_file(Some(Rc::clone(&diskfile)));
        sourcefile.set_target_exists(true);
        assert!(sourcefile.get_target_exists());
        assert!(Rc::ptr_eq(&sourcefile.get_target_file().unwrap(), &diskfile));

        diskfile.borrow_mut().close();
        let _ = fs::remove_file(&path);
    }
}
