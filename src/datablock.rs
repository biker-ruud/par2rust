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

//! Port of par2cmdline's datablock.h / datablock.cpp.
//!
//! A Data Block is a block of data of a specific length at a specific
//! offset in a specific file.
//!
//! It may be either a block of data in a source file from which recovery
//! data is being computed, a block of recovery data in a recovery file, or
//! a block in a target file that is being reconstructed.
//!
//! C++ holds a shared `DiskFile*`, so the Rust port uses
//! `Rc<RefCell<DiskFile>>`. The C++ DiskFile methods used a global error
//! stream; here `serr` is passed explicitly.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use crate::diskfile::DiskFile;

// Cloning a DataBlock copies the location descriptor while both copies refer
// to the same underlying DiskFile, exactly like copying a C++ `DataBlock*`.
#[derive(Clone)]
pub struct DataBlock {
    diskfile: Option<Rc<RefCell<DiskFile>>>, // Which disk file is the block associated with
    offset: u64,    // What is the file offset
    length: u64,    // How large is the block
    filesize: u64,  // How large was the original file
}

impl DataBlock {
    pub fn new() -> DataBlock {
        DataBlock {
            diskfile: None,
            offset: 0,
            length: 0,
            filesize: 0,
        }
    }

    /// Set the length of the block.
    pub fn set_length(&mut self, length: u64) {
        self.length = length;
    }

    pub fn set_filesize(&mut self, filesize: u64) {
        self.filesize = filesize;
    }

    /// Set the location of the block.
    pub fn set_location(&mut self, diskfile: Rc<RefCell<DiskFile>>, offset: u64) {
        self.diskfile = Some(diskfile);
        self.offset = offset;
    }

    /// Clear the location of the block.
    pub fn clear_location(&mut self) {
        self.diskfile = None;
        self.offset = 0;
    }

    /// Check to see if the location of the block has been set.
    pub fn is_set(&self) -> bool {
        if self.filesize > 0 {
            if let Some(diskfile) = &self.diskfile {
                let file_size = diskfile.borrow().file_size();
                return !(self.offset + self.length > file_size && self.filesize > file_size);
            }
        }
        self.diskfile.is_some()
    }

    /// Which disk file is this data block in.
    pub fn diskfile(&self) -> Option<Rc<RefCell<DiskFile>>> {
        self.diskfile.clone()
    }

    /// What offset is the block located at.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// What is the length of this block.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// How large was the original file.
    pub fn filesize(&self) -> u64 {
        self.filesize
    }

    /// Open the disk file if it is not already open (so that it can be read).
    pub fn open(&mut self, serr: &mut dyn Write) -> bool {
        let Some(diskfile) = &self.diskfile else {
            return false;
        };
        if diskfile.borrow().is_open() {
            return true;
        }
        let filename = diskfile.borrow().file_name().to_string();
        diskfile.borrow_mut().open(&filename, serr)
    }

    /// Read some of the data from disk into memory.
    pub fn read_data(
        &mut self,
        position: u64,
        buffer: &mut [u8],
        serr: &mut dyn Write,
    ) -> bool {
        debug_assert!(self.diskfile.is_some());
        let diskfile = self.diskfile.as_ref().unwrap();

        let size = buffer.len();

        // Check to see if the position from which data is to be read
        // is within the bounds of the data block
        if self.length > position {
            // Compute the file offset and how much data to physically read from disk
            let fileoffset = self.offset + position;
            let want = std::cmp::min(
                std::cmp::min(size as u64, self.length - position),
                diskfile.borrow().file_size().saturating_sub(fileoffset),
            ) as usize;

            // Read the data from the file into the buffer
            if !diskfile.borrow_mut().read(fileoffset, &mut buffer[..want], serr) {
                return false;
            }

            // If the read extends beyond the end of the data block,
            // then the rest of the buffer is zeroed.
            if want < size {
                buffer[want..].fill(0);
            }
        } else {
            // Zero the whole buffer
            buffer.fill(0);
        }

        true
    }

    /// Write some of the data from memory to disk.
    pub fn write_data(
        &mut self,
        position: u64,
        buffer: &[u8],
        wrote: &mut usize,
        serr: &mut dyn Write,
    ) -> bool {
        debug_assert!(self.diskfile.is_some());
        let diskfile = self.diskfile.as_ref().unwrap();

        let size = buffer.len();
        *wrote = 0;

        // Check to see if the position from which data is to be written
        // is within the bounds of the data block
        if self.length > position {
            // Compute the file offset and how much data to physically write to disk
            let fileoffset = self.offset + position;
            let have = std::cmp::min(size as u64, self.length - position) as usize;

            // Write the data from the buffer to disk
            if !diskfile.borrow_mut().write(fileoffset, &buffer[..have], serr) {
                return false;
            }

            *wrote = have;
        }

        true
    }
}

impl Default for DataBlock {
    fn default() -> Self {
        DataBlock::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test1_unset() {
        let block = DataBlock::new();
        assert!(!block.is_set());
        assert_eq!(block.offset(), 0);
        assert_eq!(block.length(), 0);
        assert!(block.diskfile().is_none());
    }

    #[test]
    fn test2_write_read_round_trip() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_datablock_{}.bin", std::process::id()));
        let _ = fs::remove_file(&path);

        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .create(path.to_str().unwrap(), 100, &mut serr));

        let mut block = DataBlock::new();
        block.set_location(Rc::clone(&diskfile), 10);
        block.set_length(50);
        assert!(block.is_set());
        assert_eq!(block.offset(), 10);
        assert_eq!(block.length(), 50);

        // Write 20 bytes at position 5 of the block.
        let data: Vec<u8> = (0u8..20).collect();
        let mut wrote = 0usize;
        assert!(block.write_data(5, &data, &mut wrote, &mut serr));
        assert_eq!(wrote, 20);

        // Read them back.
        let mut buffer = [0xffu8; 20];
        assert!(block.read_data(5, &mut buffer, &mut serr));
        assert_eq!(buffer, data.as_slice());

        // Reading beyond the block end zero-fills the tail.
        let mut wrote = 0usize;
        assert!(block.write_data(30, &data, &mut wrote, &mut serr));
        assert_eq!(wrote, 20);
        let mut buffer = [0xffu8; 10];
        assert!(block.read_data(45, &mut buffer, &mut serr));
        assert_eq!(&buffer[..5], &data[15..20]);
        assert_eq!(&buffer[5..], &[0u8; 5]);

        // Reading entirely beyond the block end zeroes everything.
        let mut buffer = [0xffu8; 4];
        assert!(block.read_data(50, &mut buffer, &mut serr));
        assert_eq!(buffer, [0u8; 4]);

        // Writing entirely beyond the block end writes nothing but succeeds.
        let mut wrote = 0usize;
        assert!(block.write_data(50, &data, &mut wrote, &mut serr));
        assert_eq!(wrote, 0);

        diskfile.borrow_mut().close();
        drop(block);
        diskfile.borrow_mut().delete(&mut serr);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test3_write_truncated_to_block() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_datablock_trunc_{}.bin", std::process::id()));
        let _ = fs::remove_file(&path);

        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .create(path.to_str().unwrap(), 100, &mut serr));

        let mut block = DataBlock::new();
        block.set_location(Rc::clone(&diskfile), 0);
        block.set_length(10);

        let data = [7u8; 16];
        let mut wrote = 0usize;
        assert!(block.write_data(0, &data, &mut wrote, &mut serr));
        assert_eq!(wrote, 10);

        diskfile.borrow_mut().close();
        drop(block);
        diskfile.borrow_mut().delete(&mut serr);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test4_is_set_filesize_check() {
        let mut serr = std::io::sink();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("par2rust_datablock_isset_{}.bin", std::process::id()));
        let _ = fs::remove_file(&path);

        let diskfile = Rc::new(RefCell::new(DiskFile::new()));
        assert!(diskfile
            .borrow_mut()
            .create(path.to_str().unwrap(), 100, &mut serr));

        let mut block = DataBlock::new();
        block.set_location(Rc::clone(&diskfile), 90);
        block.set_length(20); // extends past the 100 byte file
        assert!(block.is_set()); // filesize == 0, so plain check

        block.set_filesize(200); // original file was bigger than the disk file
        assert!(!block.is_set());

        block.set_filesize(50); // smaller than the disk file
        assert!(block.is_set());

        block.clear_location();
        assert!(!block.is_set());

        diskfile.borrow_mut().close();
        diskfile.borrow_mut().delete(&mut serr);
        let _ = fs::remove_file(&path);
    }
}
