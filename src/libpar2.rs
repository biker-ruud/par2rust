//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2003 Peter Brian Clements
//  Copyright (c) 2026 Michael Nightingale
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

//! The public types of the library: the scheme for sizing the recovery
//! files, and the outcome a process reports.

use std::io::Write;

use crate::backends::Backends;
use crate::noiselevel::NoiseLevel;
use crate::par1repairer::{Par1Repairer, SharedSout};
use crate::par2creator::Par2Creator;
use crate::par2repairer::Par2Repairer;

/// What scheme will be used to select the sizes for the recovery files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Scheme {
    Unknown = 0,
    /// Each PAR2 file will have 2x as many blocks as previous
    Variable,
    /// Limit PAR2 file size
    Limited,
    /// All PAR2 files the same size
    Uniform,
}

/// Return type of par2cmdline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Result {
    Success = 0,

    /// Data files are damaged and there is enough recovery data available
    /// to repair them.
    RepairPossible = 1,

    /// Data files are damaged and there is insufficient recovery data
    /// available to be able to repair them.
    RepairNotPossible = 2,

    /// There was something wrong with the command line arguments
    InvalidCommandLineArguments = 3,

    /// The PAR2 files did not contain sufficient information about the
    /// data files to be able to verify them.
    InsufficientCriticalData = 4,

    /// Repair completed but the data files still appear to be damaged.
    RepairFailed = 5,

    /// An error occurred when accessing files
    FileIOError = 6,

    /// An internal error occurred
    LogicError = 7,

    /// Out of memory
    MemoryError = 8,
}

/// Determine how many recovery files to create.
///
/// The C++ original also takes a `sout` stream, but never writes to it, so
/// this port leaves it out. `recoveryfilecount` is the count requested on
/// the command line, and is overwritten with the count actually used.
pub fn compute_recovery_file_count(
    serr: &mut dyn Write,
    recoveryfilecount: &mut u32,
    recoveryfilescheme: Scheme,
    recoveryblockcount: u32,
    largestfilesize: u64,
    blocksize: u64,
) -> bool {
    // Are we computing any recovery blocks
    if recoveryblockcount == 0 {
        *recoveryfilecount = 0;
        return true;
    }

    match recoveryfilescheme {
        Scheme::Unknown => {
            let _ = writeln!(serr, "Scheme unspecified (create, verify, or repair).");
            false
        }
        Scheme::Variable | Scheme::Uniform => {
            if *recoveryfilecount == 0 {
                // If none specified then then filecount is roughly log2(blockcount)
                // This prevents you getting excessively large numbers of files
                // when the block count is high and also allows the files to have
                // sizes which vary exponentially.

                let mut blocks = recoveryblockcount;
                while blocks > 0 {
                    *recoveryfilecount += 1;
                    blocks >>= 1;
                }
            }

            if *recoveryfilecount > recoveryblockcount {
                // You cannot have more recovery files than there are recovery blocks
                // to put in them.
                let _ = writeln!(serr, "Too many recovery files specified.");
                return false;
            }

            true
        }
        Scheme::Limited => {
            // No recovery file will contain more recovery blocks than would
            // be required to reconstruct the largest source file if it
            // were missing. Other recovery files will have recovery blocks
            // distributed in an exponential scheme.

            let largest = largestfilesize.div_ceil(blocksize) as u32;
            let whole = (recoveryblockcount / largest).saturating_sub(1);

            let extra = recoveryblockcount - whole * largest;
            *recoveryfilecount = whole;
            let mut blocks = extra;
            while blocks > 0 {
                *recoveryfilecount += 1;
                blocks >>= 1;
            }

            true
        }
    }
}

/// Create the PAR 2.0 files. C++ par2create.
#[allow(clippy::too_many_arguments)]
pub fn par2create(
    sout: &SharedSout,
    serr: &mut dyn Write,
    noiselevel: NoiseLevel,
    memorylimit: usize,
    basepath: &str,
    nthreads: u32,
    filethreads: u32,
    parfilename: &str,
    extrafiles: &[String],
    blocksize: u64,
    firstblock: u32,
    recoveryfilescheme: Scheme,
    recoveryfilecount: u32,
    recoveryblockcount: u32,
    backends: Backends,
) -> Result {
    let mut creator = Par2Creator::new(noiselevel, backends);
    creator.process(
        sout,
        serr,
        memorylimit,
        basepath,
        nthreads,
        filethreads,
        parfilename,
        extrafiles,
        blocksize,
        firstblock,
        recoveryfilescheme,
        recoveryfilecount,
        recoveryblockcount,
    )
}

/// Verify or repair the PAR 2.0 files. C++ par2repair.
#[allow(clippy::too_many_arguments)]
pub fn par2repair(
    sout: &SharedSout,
    serr: &mut dyn Write,
    noiselevel: NoiseLevel,
    memorylimit: usize,
    basepath: &str,
    nthreads: u32,
    filethreads: u32,
    parfilename: String,
    extrafiles: &[String],
    dorepair: bool, // derived from operation
    purgefiles: bool,
    renameonly: bool,
    skipdata: bool,
    skipleaway: u64,
    fullhash: bool,
    backends: Backends,
) -> Result {
    let mut repairer = Par2Repairer::new(noiselevel, backends);
    repairer.process(
        sout,
        serr,
        memorylimit,
        basepath,
        nthreads,
        filethreads,
        parfilename,
        extrafiles,
        dorepair,
        purgefiles,
        renameonly,
        skipdata,
        skipleaway,
        fullhash,
    )
}

/// Verify or repair the PAR 1.0 files. C++ par1repair.
#[allow(clippy::too_many_arguments)]
pub fn par1repair(
    sout: &SharedSout,
    serr: &mut dyn Write,
    noiselevel: NoiseLevel,
    memorylimit: usize,
    nthreads: u32, // basepath is not used by Par1
    parfilename: String,
    extrafiles: &[String],
    dorepair: bool, // derived from operation
    purgefiles: bool, // filethreads/skipdata/skipleaway are not used by Par1
) -> Result {
    let mut repairer = Par1Repairer::new(noiselevel);
    repairer.process(
        sout,
        serr,
        memorylimit,
        nthreads,
        parfilename,
        extrafiles,
        dorepair,
        purgefiles,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_match_the_c_enums() {
        assert_eq!(Scheme::Unknown as u32, 0);
        assert_eq!(Scheme::Variable as u32, 1);
        assert_eq!(Scheme::Limited as u32, 2);
        assert_eq!(Scheme::Uniform as u32, 3);

        assert_eq!(Result::Success as u32, 0);
        assert_eq!(Result::RepairPossible as u32, 1);
        assert_eq!(Result::RepairNotPossible as u32, 2);
        assert_eq!(Result::InvalidCommandLineArguments as u32, 3);
        assert_eq!(Result::InsufficientCriticalData as u32, 4);
        assert_eq!(Result::RepairFailed as u32, 5);
        assert_eq!(Result::FileIOError as u32, 6);
        assert_eq!(Result::LogicError as u32, 7);
        assert_eq!(Result::MemoryError as u32, 8);
    }

    fn count(scheme: Scheme, filecount: u32, blockcount: u32, largest: u64, blocksize: u64) -> (bool, u32, String) {
        let mut serr: Vec<u8> = Vec::new();
        let mut filecount = filecount;
        let ok = compute_recovery_file_count(
            &mut serr,
            &mut filecount,
            scheme,
            blockcount,
            largest,
            blocksize,
        );
        (ok, filecount, String::from_utf8(serr).unwrap())
    }

    #[test]
    fn no_recovery_blocks_needs_no_files() {
        let (ok, filecount, err) = count(Scheme::Unknown, 7, 0, 1000, 4);
        assert!(ok);
        assert_eq!(filecount, 0);
        assert!(err.is_empty());
    }

    #[test]
    fn unknown_scheme_fails() {
        let (ok, _filecount, err) = count(Scheme::Unknown, 1, 4, 1000, 4);
        assert!(!ok);
        assert_eq!(err, "Scheme unspecified (create, verify, or repair).\n");
    }

    #[test]
    fn variable_scheme_counts_log2_blocks() {
        // 16 blocks: 16, 8, 4, 2, 1 -> 5 files
        let (ok, filecount, err) = count(Scheme::Variable, 0, 16, 1000, 4);
        assert!(ok);
        assert_eq!(filecount, 5);
        assert!(err.is_empty());

        // a requested count is kept
        let (ok, filecount, _) = count(Scheme::Uniform, 3, 16, 1000, 4);
        assert!(ok);
        assert_eq!(filecount, 3);
    }

    #[test]
    fn too_many_recovery_files_fails() {
        let (ok, _filecount, err) = count(Scheme::Variable, 5, 4, 1000, 4);
        assert!(!ok);
        assert_eq!(err, "Too many recovery files specified.\n");
    }

    #[test]
    fn limited_scheme_splits_whole_and_exponential() {
        // largest file is 10 blocks; 100 blocks -> 9 whole files of 10
        // blocks, 10 left over -> 4 more files (10, 5, 2, 1)
        let (ok, filecount, err) = count(Scheme::Limited, 0, 100, 40, 4);
        assert!(ok);
        assert_eq!(filecount, 13);
        assert!(err.is_empty());

        // fewer blocks than one largest file -> whole is 0, all exponential
        let (ok, filecount, _) = count(Scheme::Limited, 0, 8, 40, 4);
        assert!(ok);
        assert_eq!(filecount, 4);
    }
}
