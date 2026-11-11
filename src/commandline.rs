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

//! Port of par2cmdline's commandline.cpp.
//!
//! The CommandLine object is responsible for understanding the format
//! of the command line parameters and parsing the command line to
//! extract details as to what the user wants to do.

use std::io::Write;

use crate::diskfile::{DiskFile, FileSizeCache, PATH_SEP};
use crate::libpar2::{compute_recovery_file_count, Scheme};
use crate::noiselevel::NoiseLevel;
use crate::par2repairer::FILE_THREADS;
use crate::taskpool::{default_threads, MAX_THREAD_COUNT};

/// The name and version reported by `showversion`. The C++ original takes
/// these from the autoconf PACKAGE and VERSION macros.
const PACKAGE: &str = "par2cmdline";
const VERSION: &str = "1.4.0";

/// Case insensitive comparison of byte strings, like C's stricmp. Unlike
/// `eq_ignore_ascii_case` this compares byte by byte exactly as the C
/// original does.
#[allow(clippy::manual_ignore_case_cmp)]
fn stricmp_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
}

/// Case insensitive comparison, like C's stricmp.
fn stricmp(a: &str, b: &str) -> bool {
    stricmp_bytes(a.as_bytes(), b.as_bytes())
}

/// Byte at index, or 0 (the C '\0') past the end of the string.
fn byte_at(s: &str, i: usize) -> u8 {
    s.as_bytes().get(i).copied().unwrap_or(0)
}

/// What operation will we be carrying out. C++ CommandLine::Operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    /// No operation was selected (help or version was displayed).
    None = 0,
    /// Create new PAR2 recovery volumes.
    Create,
    /// Verify but don't repair damaged data files.
    Verify,
    /// Verify and if possible repair damaged data files.
    Repair,
}

/// Which version of PAR files will be processed. C++ CommandLine::Version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Version {
    /// Version not yet determined.
    Unknown = 0,
    /// Processing PAR 1.0 files.
    Par1,
    /// Processing PAR 2.0 files.
    Par2,
}

/// The CommandLine object is responsible for understanding the format
/// of the command line parameters are parsing the command line to
/// extract details as to what the user wants to do.
pub struct CommandLine {
    /// Caches the size of each file, to prevent multiple calls to OS.
    pub filesize_cache: FileSizeCache,

    // options for all operations
    /// What version files will be processed.
    pub version: Version,
    /// How much display output should there be.
    pub noiselevel: NoiseLevel,
    /// How much memory is permitted to be used for the output buffer when
    /// creating or repairing.
    pub memorylimit: usize,
    /// The path par2 is run from.
    pub basepath: String,
    /// Number of threads for main processing (0 means use default number).
    pub nthreads: u32,
    /// Number of threads for file processing.
    pub filethreads: u32,

    /// The name of the PAR2 file to create, or the name of the first PAR2
    /// file to read when verifying or repairing.
    pub parfilename: String,

    /// The filenames on command-line (after expanding wildcards like '*').
    pub rawfilenames: Vec<String>,

    /// The filenames that will be used by Par. These have been verified to
    /// exist, have a path-name relative to the basepath, etc. When creating,
    /// these will be the source files, and when verifying or repairing, this
    /// will be additional PAR2 files or data files to be examined.
    pub extrafiles: Vec<String>,

    // which operation
    /// The operation to be carried out.
    pub operation: Operation,

    // options for verify/repair operation
    /// Purge backup and par files on successful recovery.
    pub purgefiles: bool,
    /// Only attempt to repair via rename, skip files that are not perfect
    /// matches.
    pub renameonly: bool,
    /// Whether we should assume that all good data blocks are within +/-
    /// bytes of where we expect to find them and should skip data that is
    /// too far away.
    pub skipdata: bool,
    /// The maximum leaway +/- that we will allow when searching for blocks.
    pub skipleaway: u64,
    /// Whether to check the hash of the whole of each file as well as the
    /// hash of each of its blocks.
    pub fullhash: bool,

    // options for creating par files
    /// How many blocks the source files should be virtually split into.
    pub blockcount: u32,
    /// What virtual block size to use.
    pub blocksize: u64,

    /// What the exponent value for the first recovery block will be.
    pub firstblock: u32,

    /// How the size of the recovery files should be calculated.
    pub recoveryfilescheme: Scheme,

    /// How many recovery files should be created.
    pub recoveryfilecount: u32,

    /// How many recovery blocks should be created.
    pub recoveryblockcount: u32,
    /// Set if the recoveryblockcount has been specified.
    pub recoveryblockcountset: bool,

    /// What percentage of recovery data should be created.
    pub redundancy: u32,
    /// Target filesize of recovery files.
    pub redundancysize: u64,

    /// Set if the redundancy has been specified.
    pub redundancyset: bool,

    /// Recurse into subdirectories.
    pub recursive: bool,
    /// Follow symlinks to files when scanning for source files.
    pub followlinks: bool,
}

impl Default for CommandLine {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandLine {
    /// C++ CommandLine::CommandLine.
    pub fn new() -> Self {
        CommandLine {
            filesize_cache: FileSizeCache::new(),
            version: Version::Unknown,
            noiselevel: NoiseLevel::Unknown,
            memorylimit: 0,
            basepath: String::new(),
            nthreads: 0, // 0 means use default number
            filethreads: FILE_THREADS, // default from header file
            parfilename: String::new(),
            rawfilenames: Vec::new(),
            extrafiles: Vec::new(),
            operation: Operation::None,
            purgefiles: false,
            renameonly: false,
            skipdata: false,
            skipleaway: 0,
            fullhash: false,
            blockcount: 0,
            blocksize: 0,
            firstblock: 0,
            recoveryfilescheme: Scheme::Unknown,
            recoveryfilecount: 0,
            recoveryblockcount: 0,
            recoveryblockcountset: false,
            redundancy: 0,
            redundancysize: 0,
            redundancyset: false,
            recursive: false,
            followlinks: false,
        }
    }

    /// C++ CommandLine::showversion.
    pub fn showversion(sout: &mut dyn Write) {
        let version = PACKAGE.to_string() + " version " + VERSION;
        let _ = writeln!(sout, "{version}");
    }

    /// C++ CommandLine::banner.
    pub fn banner(sout: &mut dyn Write) {
        let _ = write!(
            sout,
            "Copyright (C) 2003-2015 Peter Brian Clements.\n\
Copyright (C) 2011-2012 Marcel Partap.\n\
Copyright (C) 2012-2026 Ike Devolder.\n\
Copyright (C) 2014-2017 Jussi Kansanen.\n\
Copyright (C) 2019 Michael Nahas.\n\
\n\
par2cmdline comes with ABSOLUTELY NO WARRANTY.\n\
\n\
This is free software, and you are welcome to redistribute it and/or modify\n\
it under the terms of the GNU General Public License as published by the\n\
Free Software Foundation; either version 2 of the License, or (at your\n\
option) any later version. See COPYING for details.\n\n"
        );
    }

    /// Display details of the correct format for command line parameters.
    /// C++ CommandLine::usage.
    pub fn usage(sout: &mut dyn Write) {
        let _ = write!(
            sout,
            "Usage:\n\
  par2 -h  : show this help\n\
  par2 -V  : show version\n\
  par2 -VV : show version and copyright\n\
\n\
  par2 c(reate) [options] <PAR2 file> [files] : Create PAR2 files\n\
  par2 v(erify) [options] <PAR2 file> [files] : Verify files using PAR2 file\n\
  par2 r(epair) [options] <PAR2 file> [files] : Repair files using PAR2 files\n\
\n\
You may also leave out the \"c\", \"v\", and \"r\" commands by using \"par2create\",\n\
\"par2verify\", or \"par2repair\" instead.\n\
\n\
Options: (all uses)\n\
  -a<file> : Set the main PAR2 archive name\n\
  -B<path> : Set the basepath to use as reference for the datafiles\n\
  -v [-v]  : Be more verbose\n\
  -q [-q]  : Be more quiet (-q -q gives silence)\n\
  -m<n>    : Memory (in MB) to use (default is half of total physical memory)\n"
        );
        let _ = write!(
            sout,
            "  -t<n>    : Number of threads used for main processing ({} detected)\n\
  -T<n>    : Number of files hashed in parallel\n\
             ({FILE_THREADS} are the default)\n",
            default_threads()
        );
        let _ = write!(
            sout,
            "  --       : Treat all following arguments as filenames\n\
Options: (verify or repair)\n\
  -p       : Purge backup files and par files on successful recovery or\n\
             when no recovery is needed\n\
  -O       : Rename-only mode (skip files that are not perfect matches,\n\
             useful for quickly fixing renamed files)\n\
  -N       : Data skipping (find badly mispositioned data blocks)\n\
  -S<n>    : Skip leaway (distance +/- from expected block position, default 64)\n\
  --full-hash : Also check the hash of the whole of each file\n\
Options: (create)\n\
  -b<n>    : Set the Block-Count (default 2000)\n\
  -s<n>    : Set the Block-Size (don't use both -b and -s)\n\
  -r<n>    : Level of redundancy (%, default 5%)\n\
  -r<c><n> : Redundancy target size, <c>=g(iga),m(ega),k(ilo) bytes\n\
  -c<n>    : Recovery Block-Count (don't use both -r and -c)\n\
  -f<n>    : First Recovery-Block-Number (default 0)\n\
  -u       : Uniform recovery file sizes (default is variable)\n\
  -l       : Limit size of recovery files (don't use both -u and -l)\n\
  -n<n>    : Number of recovery files (max 31) (don't use both -n and -l)\n\
  -R       : Recurse into subdirectories\n\
             (Be aware of wildcard shell expansion)\n\
  -L       : Follow symlinks to files when scanning for source files\n\
             (Use at your own risk: symlink targets may lie outside the\n\
             basepath. Symlinks to directories are not followed.\n\
             POSIX systems only)\n\
   @       : Process a listing of files specified in text (file) input \n\
             (eg. @filelist.txt, or bare @ to read from stdin) \n\
\n"
        );
        let _ = write!(
            sout,
            "Example:\n\
   par2 repair *.par2\n\
\n"
        );
    }

    /// Parse the supplied command line arguments. C++ CommandLine::Parse.
    pub fn parse(&mut self, sout: &mut dyn Write, serr: &mut dyn Write, argv: &[&str]) -> bool {
        if !self.read_args(sout, serr, argv) {
            return false;
        }

        if self.operation != Operation::None {
            // user didn't do "par --help", etc.
            if !self.check_values_and_set_defaults(sout, serr) {
                return false;
            }
        }

        if self.operation == Operation::Create {
            if !self.compute_block_size(serr) {
                return false;
            }

            let mut sourceblockcount: u32 = 0;
            let mut largestfilesize: u64 = 0;
            for i in &self.extrafiles {
                let filesize = self.filesize_cache.get(i);
                sourceblockcount += filesize.div_ceil(self.blocksize) as u32;
                if filesize > largestfilesize {
                    largestfilesize = filesize;
                }
            }

            if sourceblockcount > 32768 {
                let _ = writeln!(
                    serr,
                    "Too many source blocks ({sourceblockcount} > 32768)."
                );
                return false;
            }

            if !Self::compute_recovery_block_count(
                serr,
                &mut self.recoveryblockcount,
                sourceblockcount,
                self.blocksize,
                self.firstblock,
                self.recoveryfilescheme,
                self.recoveryfilecount,
                self.recoveryblockcountset,
                self.redundancy,
                self.redundancysize,
                largestfilesize,
            ) {
                return false;
            }
        }

        true
    }

    /// Read the text of arguments into the class's variables.
    /// C++ CommandLine::ReadArgs.
    fn read_args(&mut self, sout: &mut dyn Write, serr: &mut dyn Write, argv: &[&str]) -> bool {
        if argv.is_empty() {
            return false;
        }

        // Split the program name into path and filename
        let (path, name) = DiskFile::split_filename(argv[0]);
        let mut args = &argv[1..];

        if !args.is_empty() && args[0].starts_with('-') {
            if matches!(args[0], "-h" | "--help") {
                Self::usage(sout);
                return true;
            } else if matches!(args[0], "-V" | "--version") {
                Self::showversion(sout);
                return true;
            } else if args[0] == "-VV" {
                Self::showversion(sout);
                let _ = writeln!(sout);
                Self::banner(sout);
                return true;
            }
        }

        let mut name = name;
        // Strip ".exe" from the end
        if name.len() > 4
            && stricmp_bytes(&name.as_bytes()[name.len() - 4..], b".exe")
        {
            name.truncate(name.len() - 4);
        }

        // Check the resulting program name
        if stricmp(&name, "par2create") {
            self.operation = Operation::Create;
        } else if stricmp(&name, "par2verify") {
            self.operation = Operation::Verify;
        } else if stricmp(&name, "par2repair") {
            self.operation = Operation::Repair;
        }

        // Have we determined what operation we want?
        if self.operation == Operation::None {
            if args.len() < 2 {
                let _ = writeln!(serr, "Not enough command line arguments.");
                return false;
            }

            match args[0].as_bytes().first().map(u8::to_ascii_lowercase) {
                Some(b'c') if args[0].len() == 1 || stricmp(args[0], "create") => {
                    self.operation = Operation::Create;
                }
                Some(b'v') if args[0].len() == 1 || stricmp(args[0], "verify") => {
                    self.operation = Operation::Verify;
                }
                Some(b'r') if args[0].len() == 1 || stricmp(args[0], "repair") => {
                    self.operation = Operation::Repair;
                }
                _ => {}
            }

            if self.operation == Operation::None {
                let _ = writeln!(serr, "Invalid operation specified: {}", args[0]);
                return false;
            }
            args = &args[1..];
        }

        let mut options = true;
        self.basepath = String::new();

        while !args.is_empty() {
            let arg = args[0];
            if !arg.is_empty() {
                // MODIFIED FOR @FILELIST FUNCTIONALITY
                if options && !arg.starts_with('-') && !arg.starts_with('@') {
                    options = false;
                }
                // MODIFIED FOR @FILELIST FUNCTIONALITY
                if options && arg.starts_with('-') {
                    match byte_at(arg, 1) {
                        b'a' => {
                            let setparfile = if arg == "-a" {
                                if args.len() > 1 {
                                    let next = args[1];
                                    let setparfile = self.set_par_filename(serr, next);
                                    args = &args[1..];
                                    setparfile
                                } else {
                                    // C++ reads argv[1] past the end here.
                                    false
                                }
                            } else {
                                self.set_par_filename(serr, &arg[2..])
                            };

                            if !setparfile {
                                let _ = writeln!(serr, "failed to set the main par file");
                                return false;
                            }
                        }
                        b'b' => {
                            // Set the block count
                            if self.operation != Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify block count unless creating."
                                );
                                return false;
                            }
                            if self.blockcount > 0 {
                                let _ = writeln!(serr, "Cannot specify block count twice.");
                                return false;
                            } else if self.blocksize > 0 {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify both block count and block size."
                                );
                                return false;
                            }

                            let mut i = 2usize;
                            while self.blockcount <= 3276 && byte_at(arg, i).is_ascii_digit() {
                                self.blockcount = self.blockcount * 10
                                    + u32::from(byte_at(arg, i) - b'0');
                                i += 1;
                            }
                            if self.blockcount == 0
                                || self.blockcount > 32768
                                || byte_at(arg, i) != 0
                            {
                                let _ =
                                    writeln!(serr, "Invalid block count option: {arg}");
                                return false;
                            }
                        }
                        b's' => {
                            // Set the block size
                            if self.operation != Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify block size unless creating."
                                );
                                return false;
                            }
                            if self.blocksize > 0 {
                                let _ = writeln!(serr, "Cannot specify block size twice.");
                                return false;
                            } else if self.blockcount > 0 {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify both block count and block size."
                                );
                                return false;
                            }

                            let mut i = 2usize;
                            while self.blocksize <= 429496729 && byte_at(arg, i).is_ascii_digit()
                            {
                                self.blocksize = self.blocksize * 10
                                    + u64::from(byte_at(arg, i) - b'0');
                                i += 1;
                            }
                            if byte_at(arg, i) != 0 || self.blocksize == 0 {
                                let _ =
                                    writeln!(serr, "Invalid block size option: {arg}");
                                return false;
                            }
                            if self.blocksize & 3 != 0 {
                                let _ =
                                    writeln!(serr, "Block size must be a multiple of 4.");
                                return false;
                            }
                        }
                        b't' => {
                            // Set amount of threads
                            self.nthreads = 0;
                            let mut i = 2usize;
                            while byte_at(arg, i).is_ascii_digit() {
                                self.nthreads = self.nthreads.wrapping_mul(10)
                                    .wrapping_add(u32::from(byte_at(arg, i) - b'0'));
                                i += 1;
                            }

                            if self.nthreads == 0 {
                                let _ = writeln!(serr, "Invalid thread option: {arg}");
                                return false;
                            }

                            if self.nthreads > MAX_THREAD_COUNT {
                                self.nthreads = MAX_THREAD_COUNT;
                            }
                        }
                        b'T' => {
                            // Set amount of file threads
                            self.filethreads = 0;
                            let mut i = 2usize;
                            while byte_at(arg, i).is_ascii_digit() {
                                self.filethreads = self.filethreads.wrapping_mul(10)
                                    .wrapping_add(u32::from(byte_at(arg, i) - b'0'));
                                i += 1;
                            }

                            if self.filethreads == 0 {
                                let _ = writeln!(serr, "Invalid file-thread option: {arg}");
                                return false;
                            }
                        }
                        b'r' => {
                            // Set the amount of redundancy required
                            if self.operation != Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify redundancy unless creating."
                                );
                                return false;
                            }
                            if self.redundancyset {
                                let _ = writeln!(serr, "Cannot specify redundancy twice.");
                                return false;
                            } else if self.recoveryblockcountset {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify both redundancy and recovery block count."
                                );
                                return false;
                            }

                            let c = byte_at(arg, 2);
                            if c == b'k' || c == b'm' || c == b'g' {
                                let mut i = 3usize;
                                while byte_at(arg, i).is_ascii_digit() {
                                    self.redundancysize = self
                                        .redundancysize
                                        .wrapping_mul(10)
                                        .wrapping_add(u64::from(byte_at(arg, i) - b'0'));
                                    i += 1;
                                }
                                // The C++ switch falls through: g multiplies
                                // by 1024 three times, m twice, k once.
                                match c {
                                    b'g' => {
                                        self.redundancysize = self
                                            .redundancysize
                                            .wrapping_mul(1024)
                                            .wrapping_mul(1024)
                                            .wrapping_mul(1024);
                                    }
                                    b'm' => {
                                        self.redundancysize = self
                                            .redundancysize
                                            .wrapping_mul(1024)
                                            .wrapping_mul(1024);
                                    }
                                    _ => {
                                        self.redundancysize =
                                            self.redundancysize.wrapping_mul(1024);
                                    }
                                }
                            } else {
                                let mut i = 2usize;
                                while byte_at(arg, i).is_ascii_digit() {
                                    self.redundancy = self.redundancy.wrapping_mul(10)
                                        .wrapping_add(u32::from(byte_at(arg, i) - b'0'));
                                    i += 1;
                                }
                                if byte_at(arg, i) != 0 {
                                    let _ = writeln!(
                                        serr,
                                        "Invalid redundancy option: {arg}"
                                    );
                                    return false;
                                }
                                if self.redundancy == 0 && self.recoveryfilecount > 0 {
                                    let _ = writeln!(
                                        serr,
                                        "Cannot set redundancy to 0 and file count > 0"
                                    );
                                    return false;
                                }
                                if self.redundancy > 100 {
                                    let _ = writeln!(
                                        serr,
                                        "WARNING: Creating recovery file(s) with {} redundancy.",
                                        self.redundancy
                                    );
                                }
                            }
                            self.redundancyset = true;
                        }
                        b'c' => {
                            // Set the number of recovery blocks to create
                            if self.operation != Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify recovery block count unless creating."
                                );
                                return false;
                            }
                            if self.recoveryblockcountset {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify recovery block count twice."
                                );
                                return false;
                            } else if self.redundancyset {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify both recovery block count and redundancy."
                                );
                                return false;
                            }

                            let mut i = 2usize;
                            while self.recoveryblockcount <= 32768
                                && byte_at(arg, i).is_ascii_digit()
                            {
                                self.recoveryblockcount = self.recoveryblockcount * 10
                                    + u32::from(byte_at(arg, i) - b'0');
                                i += 1;
                            }
                            if self.recoveryblockcount > 32768 || byte_at(arg, i) != 0 {
                                let _ = writeln!(
                                    serr,
                                    "Invalid recoveryblockcount option: {arg}"
                                );
                                return false;
                            }
                            if self.recoveryblockcount == 0 && self.recoveryfilecount > 0 {
                                let _ = writeln!(
                                    serr,
                                    "Cannot set recoveryblockcount to 0 and file count > 0"
                                );
                                return false;
                            }
                            self.recoveryblockcountset = true;
                        }
                        b'f' => {
                            // Specify the First block recovery number
                            if self.operation != Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify first block number unless creating."
                                );
                                return false;
                            }
                            if self.firstblock > 0 {
                                let _ = writeln!(serr, "Cannot specify first block twice.");
                                return false;
                            }

                            let mut i = 2usize;
                            while self.firstblock <= 3276 && byte_at(arg, i).is_ascii_digit() {
                                self.firstblock = self.firstblock * 10
                                    + u32::from(byte_at(arg, i) - b'0');
                                i += 1;
                            }
                            if self.firstblock > 32768 || byte_at(arg, i) != 0 {
                                let _ =
                                    writeln!(serr, "Invalid first block option: {arg}");
                                return false;
                            }
                        }
                        b'u' => {
                            // Specify uniformly sized recovery files
                            if self.operation != Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify uniform files unless creating."
                                );
                                return false;
                            }
                            if byte_at(arg, 2) != 0 {
                                let _ = writeln!(serr, "Invalid option: {arg}");
                                return false;
                            }
                            if self.recoveryfilescheme != Scheme::Unknown
                                && self.recoveryfilescheme != Scheme::Uniform
                            {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify two recovery file size schemes."
                                );
                                return false;
                            }

                            self.recoveryfilescheme = Scheme::Uniform;
                        }
                        b'l' => {
                            // Limit the size of the recovery files
                            if self.operation != Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify limit files unless creating."
                                );
                                return false;
                            }
                            if byte_at(arg, 2) != 0 {
                                let _ = writeln!(serr, "Invalid option: {arg}");
                                return false;
                            }
                            if self.recoveryfilescheme != Scheme::Unknown {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify two recovery file size schemes."
                                );
                                return false;
                            }
                            if self.recoveryfilecount > 0 {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify limited size and number of files at the same time."
                                );
                                return false;
                            }

                            self.recoveryfilescheme = Scheme::Limited;
                        }
                        b'n' => {
                            // Specify the number of recovery files
                            if self.operation != Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify recovery file count unless creating."
                                );
                                return false;
                            }
                            if self.recoveryfilecount > 0 {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify recovery file count twice."
                                );
                                return false;
                            }
                            // (Removed "Cannot set file count when redundancy is set to 0.")
                            if self.recoveryblockcountset && self.recoveryblockcount == 0 {
                                let _ = writeln!(
                                    serr,
                                    "Cannot set file count when recovery block count is set to 0."
                                );
                                return false;
                            }
                            if self.recoveryfilescheme == Scheme::Limited {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify limited size and number of files at the same time."
                                );
                                return false;
                            }

                            let mut i = 2usize;
                            while byte_at(arg, i).is_ascii_digit() {
                                self.recoveryfilecount = self.recoveryfilecount.wrapping_mul(10)
                                    .wrapping_add(u32::from(byte_at(arg, i) - b'0'));
                                i += 1;
                            }
                            if self.recoveryfilecount == 0 || byte_at(arg, i) != 0 {
                                let _ = writeln!(
                                    serr,
                                    "Invalid recovery file count option: {arg}"
                                );
                                return false;
                            }

                            // cap the maximum number of recovery files
                            // because the number of recovery blocks will be
                            // (2 ^ recoveryfilecount) - 1
                            // the number 32 will overflow the u32 resulting in 1
                            if self.recoveryfilecount > 31 {
                                let _ = writeln!(
                                    serr,
                                    "Invalid recovery file count option: {}",
                                    self.recoveryfilecount
                                );
                                let _ = writeln!(
                                    serr,
                                    "  the maximum allowed recovery file count is 31"
                                );
                                return false;
                            }

                            // When number of recovery files is used, set the
                            // recoveryfilescheme to uniform, since variable
                            // will not always be able to fill all the files
                            self.recoveryfilescheme = Scheme::Uniform;
                        }
                        b'm' => {
                            // Specify how much memory to use for output buffers
                            if self.memorylimit > 0 {
                                let _ =
                                    writeln!(serr, "Cannot specify memory limit twice.");
                                return false;
                            }

                            let mut i = 2usize;
                            while byte_at(arg, i).is_ascii_digit() {
                                self.memorylimit = self.memorylimit.wrapping_mul(10)
                                    .wrapping_add(usize::from(byte_at(arg, i) - b'0'));
                                i += 1;
                            }
                            if self.memorylimit == 0 || byte_at(arg, i) != 0 {
                                let _ =
                                    writeln!(serr, "Invalid memory limit option: {arg}");
                                return false;
                            }
                        }
                        b'v' => {
                            match self.noiselevel {
                                NoiseLevel::Unknown => {
                                    if byte_at(arg, 2) == b'v' {
                                        self.noiselevel = NoiseLevel::Debug;
                                    } else {
                                        self.noiselevel = NoiseLevel::Noisy;
                                    }
                                }
                                NoiseLevel::Noisy | NoiseLevel::Debug => {
                                    self.noiselevel = NoiseLevel::Debug;
                                }
                                _ => {
                                    let _ =
                                        writeln!(serr, "Cannot use both -v and -q.");
                                    return false;
                                }
                            }
                        }
                        b'q' => {
                            match self.noiselevel {
                                NoiseLevel::Unknown => {
                                    if byte_at(arg, 2) == b'q' {
                                        self.noiselevel = NoiseLevel::Silent;
                                    } else {
                                        self.noiselevel = NoiseLevel::Quiet;
                                    }
                                }
                                NoiseLevel::Quiet | NoiseLevel::Silent => {
                                    self.noiselevel = NoiseLevel::Silent;
                                }
                                _ => {
                                    let _ =
                                        writeln!(serr, "Cannot use both -v and -q.");
                                    return false;
                                }
                            }
                        }
                        b'p' => {
                            if self.operation != Operation::Repair
                                && self.operation != Operation::Verify
                            {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify purge unless repairing or verifying."
                                );
                                return false;
                            }
                            self.purgefiles = true;
                        }
                        b'O' => {
                            if self.operation != Operation::Repair
                                && self.operation != Operation::Verify
                            {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify rename-only unless repairing or verifying."
                                );
                                return false;
                            }
                            self.renameonly = true;
                        }
                        b'h' => {
                            Self::usage(sout);
                            return false;
                        }
                        // "break;" not needed.
                        b'R' => {
                            if self.operation == Operation::Create {
                                self.recursive = true;
                            } else {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specific Recursive unless creating."
                                );
                                return false;
                            }
                        }
                        b'L' => {
                            if self.operation == Operation::Create {
                                self.followlinks = true;
                            } else {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify -L (follow symlinks) unless creating."
                                );
                                return false;
                            }
                        }
                        b'N' => {
                            if self.operation == Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify Data Skipping unless reparing or verifying."
                                );
                                return false;
                            }
                            self.skipdata = true;
                        }
                        b'S' => {
                            // Set the skip leaway
                            if self.operation == Operation::Create {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify skip leaway when creating."
                                );
                                return false;
                            }
                            if !self.skipdata {
                                let _ = writeln!(
                                    serr,
                                    "Cannot specify skip leaway and no skipping."
                                );
                                return false;
                            }

                            let mut i = 2usize;
                            while self.skipleaway <= 429496729
                                && byte_at(arg, i).is_ascii_digit()
                            {
                                self.skipleaway = self.skipleaway * 10
                                    + u64::from(byte_at(arg, i) - b'0');
                                i += 1;
                            }
                            if byte_at(arg, i) != 0 || self.skipleaway == 0 {
                                let _ =
                                    writeln!(serr, "Invalid skipleaway option: {arg}");
                                return false;
                            }
                        }
                        b'B' => {
                            // Set the basepath manually
                            if arg == "-B" {
                                if args.len() > 1 {
                                    let next = args[1];
                                    self.basepath = DiskFile::get_canonical_pathname(next);
                                    args = &args[1..];
                                } else {
                                    // C++ reads argv[1] past the end here.
                                    let _ = writeln!(serr, "failed to set the main par file");
                                    return false;
                                }
                            } else {
                                self.basepath =
                                    DiskFile::get_canonical_pathname(&arg[2..]);
                            }
                        }
                        b'-' => {
                            if arg == "--full-hash" {
                                if self.operation == Operation::Create {
                                    let _ = writeln!(
                                        serr,
                                        "Cannot specify a full hash check unless repairing or verifying."
                                    );
                                    return false;
                                }
                                self.fullhash = true;
                            } else if arg != "--" {
                                let _ = writeln!(serr, "Unknown option: {arg}");
                                let _ = writeln!(
                                    serr,
                                    "  (Options must appear after create, repair or verify.)"
                                );
                                let _ = writeln!(
                                    serr,
                                    "  (Run \"{path}{name} --help\" for supported options.)"
                                );
                                return false;
                            } else {
                                // "--": treat all following arguments as filenames
                                args = &args[1..];
                                options = false;
                                continue;
                            }
                        }
                        _ => {
                            let _ = writeln!(serr, "Invalid option specified: {arg}");
                            return false;
                        }
                    }
                } else if let Some(listname) = arg.strip_prefix('@') {
                    // Handle list files

                    // 1. Check if the '@' is followed by a filename
                    let lines: Box<dyn Iterator<Item = String>> = if listname.is_empty() {
                        // Bare '@' - read filelist from stdin
                        use std::io::BufRead;
                        Box::new(
                            std::io::stdin()
                                .lock()
                                .lines()
                                .map(|line| line.unwrap_or_default()),
                        )
                    } else {
                        match std::fs::File::open(listname) {
                            Ok(listfile) => {
                                use std::io::BufRead;
                                Box::new(
                                    std::io::BufReader::new(listfile)
                                        .lines()
                                        .map(|line| line.unwrap_or_default()),
                                )
                            }
                            Err(_) => {
                                let _ = writeln!(
                                    serr,
                                    "Could not open list file: {listname}"
                                );
                                return false;
                            }
                        }
                    };

                    for line in lines {
                        // 2. Trim whitespace or skip whitespace-only lines.
                        // This finds the first non-whitespace character.
                        if !line
                            .chars()
                            .any(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'))
                        {
                            continue; // Line is empty or only whitespace
                        }

                        // 3. Removed the 'parfilename.length() == 0' block.
                        // All files in the list are now treated as source
                        // files.

                        let (lpath, lname) = DiskFile::split_filename(&line);
                        for fn_ in DiskFile::find_files_follow(
                            &lpath,
                            &lname,
                            self.recursive,
                            self.followlinks,
                        ) {
                            self.rawfilenames
                                .push(DiskFile::get_canonical_pathname(&fn_));
                        }
                    }
                } else if self.parfilename.is_empty() {
                    if !self.set_par_filename(serr, arg) {
                        let _ = writeln!(serr, "failed to set the main par file");
                        return false;
                    }
                } else {
                    let (path, name) = DiskFile::split_filename(arg);
                    for fn_ in
                        DiskFile::find_files_follow(&path, &name, self.recursive, self.followlinks)
                    {
                        self.rawfilenames
                            .push(DiskFile::get_canonical_pathname(&fn_));
                    }
                }
            }

            args = &args[1..];
        }

        true
    }
}

impl CommandLine {
    /// C++ CommandLine::GetTotalPhysicalMemory.
    #[cfg(windows)]
    fn get_total_physical_memory() -> u64 {
        #[repr(C)]
        struct MemoryStatusEx {
            dw_length: u32,
            dw_memory_load: u32,
            ull_total_phys: u64,
            ull_avail_phys: u64,
            ull_total_page_file: u64,
            ull_avail_page_file: u64,
            ull_total_virtual: u64,
            ull_avail_virtual: u64,
            ull_avail_extended_virtual: u64,
        }
        extern "system" {
            fn GlobalMemoryStatusEx(lpbuffer: *mut MemoryStatusEx) -> i32;
        }

        let mut total_physical_memory = 0u64;

        let mut mse = MemoryStatusEx {
            dw_length: 0,
            dw_memory_load: 0,
            ull_total_phys: 0,
            ull_avail_phys: 0,
            ull_total_page_file: 0,
            ull_avail_page_file: 0,
            ull_total_virtual: 0,
            ull_avail_virtual: 0,
            ull_avail_extended_virtual: 0,
        };
        mse.dw_length = std::mem::size_of::<MemoryStatusEx>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut mse) } != 0 {
            total_physical_memory = mse.ull_total_phys;
        }

        if total_physical_memory == 0 {
            #[repr(C)]
            struct MemoryStatus {
                dw_length: u32,
                dw_memory_load: u32,
                dw_total_phys: u32,
                dw_avail_phys: u32,
                dw_total_page_file: u32,
                dw_avail_page_file: u32,
                dw_total_virtual: u32,
                dw_avail_virtual: u32,
            }
            extern "system" {
                fn GlobalMemoryStatus(lpbuffer: *mut MemoryStatus);
            }

            let mut ms = MemoryStatus {
                dw_length: 0,
                dw_memory_load: 0,
                dw_total_phys: 0,
                dw_avail_phys: 0,
                dw_total_page_file: 0,
                dw_avail_page_file: 0,
                dw_total_virtual: 0,
                dw_avail_virtual: 0,
            };
            unsafe { GlobalMemoryStatus(&mut ms) };
            total_physical_memory = u64::from(ms.dw_total_phys);
        }

        total_physical_memory
    }

    /// C++ CommandLine::GetTotalPhysicalMemory (the sysconf version reads
    /// _SC_PHYS_PAGES * _SC_PAGESIZE; without a libc dependency this port
    /// reads MemTotal from /proc/meminfo where that file exists).
    #[cfg(not(windows))]
    fn get_total_physical_memory() -> u64 {
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    if let Some(kb) = rest.trim().split_whitespace().next() {
                        if let Ok(kb) = kb.parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
        0
    }

    /// Check values that were set during read_args. If values went unset, set
    /// them with default values. C++ CommandLine::CheckValuesAndSetDefaults.
    fn check_values_and_set_defaults(&mut self, sout: &mut dyn Write, serr: &mut dyn Write) -> bool {
        if self.parfilename.is_empty() {
            let _ = writeln!(serr, "You must specify a Recovery file.");
            return false;
        }

        // Default noise level
        if self.noiselevel == NoiseLevel::Unknown {
            self.noiselevel = NoiseLevel::Normal;
        }

        // Default memorylimit of 256MB
        if self.memorylimit == 0 {
            let total_physical_memory = Self::get_total_physical_memory();

            if total_physical_memory == 0 {
                if self.noiselevel >= NoiseLevel::Debug {
                    let _ = writeln!(sout, "[DEBUG] could not detect physical memory");
                }

                // Default/error case:
                self.memorylimit = 256;
            } else {
                if self.noiselevel >= NoiseLevel::Debug {
                    let _ = writeln!(
                        sout,
                        "[DEBUG] detected physical memory: {total_physical_memory} bytes"
                    );
                }

                // 1/8th of total physical memory or floor to 256MiB if lower:
                self.memorylimit = (total_physical_memory / 1048576 / 8) as usize;
                if self.memorylimit < 256 {
                    self.memorylimit = 256;
                }
            }
        }

        // limit to 1GB on 32-bit platforms to avoid exhausing the addressable memory space
        if cfg!(target_pointer_width = "32") && self.memorylimit > 1024 {
            self.memorylimit = 1024;
        }

        // convert to megabytes
        self.memorylimit *= 1048576;

        if self.noiselevel >= NoiseLevel::Debug {
            let _ = writeln!(sout, "[DEBUG] memorylimit: {} bytes", self.memorylimit);
        }

        // Default basepath (uses parfilename)
        if self.basepath.is_empty() {
            if self.noiselevel >= NoiseLevel::Debug {
                let _ = writeln!(sout, "[DEBUG] parfilename: {}", self.parfilename);
            }

            let (path, _dummy) = DiskFile::split_filename(&self.parfilename);
            self.basepath = DiskFile::get_canonical_pathname(&path);

            // fallback
            if self.basepath.is_empty() {
                self.basepath = DiskFile::get_canonical_pathname("./");
            }
        }

        let lastchar = self.basepath.as_bytes().last().copied().unwrap_or(0);
        if lastchar != b'/' && lastchar != b'\\' {
            self.basepath.push(PATH_SEP);
        }

        if self.noiselevel >= NoiseLevel::Debug {
            let _ = writeln!(sout, "[DEBUG] basepath: {}", self.basepath);
        }

        // parfilename is checked earlier, because it is used by basepath.

        // check extrafiles
        for filename in self.rawfilenames.clone() {
            // Originally, all specified files were supposed to exist, or the
            // program would stop with an error message. This was not
            // practical, for example in a directory with files appearing and
            // disappearing (an active download directory). So the new rule
            // is: when a specified file doesn't exist, it is silently
            // skipped.
            if !DiskFile::file_exists(&filename) {
                let _ = writeln!(sout, "Ignoring non-existent source file: {filename}");
            }
            // skip files outside basepath
            else if !filename.starts_with(&self.basepath) {
                let _ = writeln!(sout, "Ignoring out of basepath source file: {filename}");
            } else {
                let filesize = self.filesize_cache.get(&filename);

                // Ignore all 0 byte files
                if filesize == 0 {
                    let _ = writeln!(sout, "Skipping 0 byte file: {filename}");
                } else if self.extrafiles.contains(&filename) {
                    let _ = writeln!(sout, "Skipping duplicate filename: {filename}");
                } else {
                    self.extrafiles.push(filename);
                }
            } //end file exists
        }

        // operation should always be set, but let's be thorough.
        if self.operation == Operation::None {
            let _ = writeln!(
                serr,
                "ERROR: No operation was specified (create, repair, or verify)"
            );
            return false;
        }

        if self.operation != Operation::Create {
            // skipdata is bool and either value is valid.

            // Default skip leaway
            if self.skipdata && self.skipleaway == 0 {
                // Expect to find blocks within +/- 64 bytes of the expected
                // position relative to the last block that was found.
                self.skipleaway = 64;
            }
        }

        // If we a creating, check the other parameters
        if self.operation == Operation::Create {
            // If we are creating, the source files must be given.
            if self.extrafiles.is_empty() {
                // Does the par filename include the ".par2" on the end?
                if self.parfilename.len() > 5
                    && stricmp_bytes(
                        &self.parfilename.as_bytes()[self.parfilename.len() - 5..],
                        b".par2",
                    )
                {
                    // Yes it does.
                    let _ = writeln!(
                        serr,
                        "You must specify a list of files when creating."
                    );
                    return false;
                } else {
                    // No it does not.

                    // In that case check to see if the file exists, and if it
                    // does assume that you wish to create par2 files for it.

                    if DiskFile::file_exists(&self.parfilename)
                        && DiskFile::get_file_size(&self.parfilename) > 0
                    {
                        let parfilename = self.parfilename.clone();
                        self.extrafiles.push(parfilename);
                    } else {
                        // The file does not exist or it is empty.

                        let _ = writeln!(
                            serr,
                            "You must specify a list of files when creating."
                        );
                        return false;
                    }
                }
            }

            // Strip the ".par2" from the end of the filename of the main
            // PAR2 file.
            if self.parfilename.len() > 5
                && stricmp_bytes(
                    &self.parfilename.as_bytes()[self.parfilename.len() - 5..],
                    b".par2",
                )
            {
                self.parfilename.truncate(self.parfilename.len() - 5);
            }

            let mut par2name = self.parfilename.clone();
            par2name.push_str(".par2");
            if DiskFile::file_exists(&par2name) {
                let _ = writeln!(serr, "Par2 file already exists: {}", self.parfilename);
                return false;
            }

            // If neither block count nor block size is specified, derive a
            // default block size from total source data, targeting ~2000
            // blocks (as previous default). This replaces the old hard-coded
            // blockcount=2000 which failed when file count exceeded 2000,
            // and avoids inflated block sizes when file sizes vary widely.
            // The minimum is clamped so that the total source block count
            // stays within the PAR2 limit of 32768.
            if self.blockcount == 0 && self.blocksize == 0 {
                let mut totalsize: u64 = 0;
                for f in &self.extrafiles {
                    totalsize += self.filesize_cache.get(f);
                }

                let mut blocksize = totalsize.div_ceil(2000); // 2000 = target (previous default)
                blocksize = blocksize.max(4);
                blocksize = (blocksize + 3) & !3;

                let mut minblocksize = totalsize.div_ceil(32768); // 32768 = maximum PAR2
                minblocksize = minblocksize.max(4);
                minblocksize = (minblocksize + 3) & !3;

                if blocksize < minblocksize {
                    blocksize = minblocksize;
                }

                self.blocksize = blocksize;
            }

            // If no recovery file size scheme is specified then use Variable
            if self.recoveryfilescheme == Scheme::Unknown {
                self.recoveryfilescheme = Scheme::Variable;
            }

            // Assume a redundancy of 5% if neither redundancy or
            // recoveryblockcount were set.
            if !self.redundancyset && !self.recoveryblockcountset {
                self.redundancy = 5;
                self.redundancyset = true;
            }
        }

        true
    }

    /// Use values like block count to compute the block size.
    /// C++ CommandLine::ComputeBlockSize.
    fn compute_block_size(&mut self, serr: &mut dyn Write) -> bool {
        if self.blocksize == 0 {
            // compute value from blockcount
            let numfiles = self.extrafiles.len();

            if (self.blockcount as usize) < numfiles {
                // The block count cannot be less than the number of files.

                let _ = writeln!(
                    serr,
                    "Block count ({}) cannot be smaller than the number of files({}). ",
                    self.blockcount, numfiles
                );
                return false;
            } else if (self.blockcount as usize) == numfiles {
                // If the block count is the same as the number of files, then
                // the block size is the size of the largest file (rounded up
                // to a multiple of 4).

                let mut largestfilesize: u64 = 0;
                for i in &self.extrafiles {
                    let filesize = self.filesize_cache.get(i);
                    if filesize > largestfilesize {
                        largestfilesize = filesize;
                    }
                }
                self.blocksize = (largestfilesize + 3) & !3;
            } else {
                let mut totalsize: u64 = 0;
                for i in &self.extrafiles {
                    totalsize += self.filesize_cache.get(i).div_ceil(4);
                }

                let blockcount = u64::from(self.blockcount);
                let numfiles = numfiles as u64;
                if blockcount > totalsize {
                    self.blocksize = 4;
                } else {
                    // Absolute lower bound and upper bound on the source
                    // block size that will result in the requested source
                    // block count.
                    let mut lowerbound = totalsize / blockcount;
                    let mut upperbound =
                        (totalsize + blockcount - numfiles - 1) / (blockcount - numfiles);

                    let mut count: u64;
                    let mut size: u64;

                    loop {
                        size = (lowerbound + upperbound) / 2;

                        count = 0;
                        for i in &self.extrafiles {
                            let filesize_quart = self.filesize_cache.get(i).div_ceil(4);
                            count += filesize_quart.div_ceil(size);
                        }
                        if count > blockcount {
                            lowerbound = size + 1;
                            if lowerbound >= upperbound {
                                size = lowerbound;
                                count = 0;
                                for i in &self.extrafiles {
                                    let filesize_quart =
                                        self.filesize_cache.get(i).div_ceil(4);
                                    count += filesize_quart.div_ceil(size);
                                }
                            }
                        } else {
                            upperbound = size;
                        }
                        if lowerbound >= upperbound {
                            break;
                        }
                    }

                    if count > 32768 {
                        let _ = writeln!(
                            serr,
                            "Error calculating block size. cannot be higher than 32768."
                        );
                        return false;
                    } else if count == 0 {
                        let _ =
                            writeln!(serr, "Error calculating block size. cannot be 0.");
                        return false;
                    }

                    self.blocksize = size * 4;
                }
            }
        }

        true
    }

    /// Determine how many recovery blocks to create based on the source block
    /// count and the requested level of redundancy.
    /// C++ CommandLine::ComputeRecoveryBlockCount.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_recovery_block_count(
        serr: &mut dyn Write,
        recoveryblockcount: &mut u32,
        sourceblockcount: u32,
        blocksize: u64,
        firstblock: u32,
        recoveryfilescheme: Scheme,
        recoveryfilecount: u32,
        recoveryblockcountset: bool,
        redundancy: u32,
        redundancysize: u64,
        largestfilesize: u64,
    ) -> bool {
        let mut recoveryfilecount = recoveryfilecount;

        if recoveryblockcountset {
            // no need to assign value.
            // pass through, so that value can be checked below.
        } else if redundancy > 0 {
            // count is the number of input blocks

            // Determine recoveryblockcount
            *recoveryblockcount =
                (sourceblockcount.wrapping_mul(redundancy).wrapping_add(50)) / 100;
        } else if redundancysize > 0 {
            let overhead_per_recovery_file = u64::from(sourceblockcount) * 21;
            let recovery_packet_size = blocksize + 70;
            if recoveryfilecount == 0 {
                let estimatedfilecount: u32 = 15;
                let overhead = u64::from(estimatedfilecount) * overhead_per_recovery_file;
                let estimatedrecoveryblockcount: u32 = if overhead > redundancysize {
                    1 // at least 1
                } else {
                    ((redundancysize - overhead) / recovery_packet_size) as u32
                };

                // recoveryfilecount assigned below.
                if !compute_recovery_file_count(
                    serr,
                    &mut recoveryfilecount,
                    recoveryfilescheme,
                    estimatedrecoveryblockcount,
                    largestfilesize,
                    blocksize,
                ) {
                    return false;
                }
            }

            let overhead = u64::from(recoveryfilecount) * overhead_per_recovery_file;
            if overhead > redundancysize {
                *recoveryblockcount = 1; // at least 1
            } else {
                *recoveryblockcount =
                    ((redundancysize - overhead) / recovery_packet_size) as u32;
            }
        } else {
            let _ = writeln!(serr, "Redundancy and Redundancysize not set.");
            return false;
        }

        // Force valid values if necessary
        if *recoveryblockcount == 0 && redundancy > 0 {
            *recoveryblockcount = 1;
        }

        if *recoveryblockcount > 65536 {
            let _ = writeln!(serr, "Too many recovery blocks requested.");
            return false;
        }

        // Check that the last recovery block number would not be too large
        if firstblock + *recoveryblockcount >= 65536 {
            let _ = writeln!(serr, "First recovery block number is too high.");
            return false;
        }

        true
    }

    /// C++ CommandLine::SetParFilename.
    #[cfg_attr(windows, allow(unused_variables))]
    fn set_par_filename(&mut self, serr: &mut dyn Write, filename: &str) -> bool {
        let mut result = false;

        #[cfg(not(windows))]
        if filename.contains(['*', '?']) {
            let _ = writeln!(serr, "par2 file must not have a wildcard in it.");
            return result;
        }

        // If we are verifying or repairing, the PAR2 file must already exist
        if self.operation != Operation::Create {
            // Find the last '.' in the filename
            if let Some(where_) = filename.rfind('.') {
                // Get what follows the last '.'
                let tail = &filename[where_ + 1..];

                if stricmp(tail, "par2") {
                    self.parfilename = filename.to_string();
                    self.version = Version::Par2;
                } else if stricmp(tail, "par")
                    || (tail.len() == 3
                        && byte_at(tail, 0).eq_ignore_ascii_case(&b'p')
                        && byte_at(tail, 1).is_ascii_digit()
                        && byte_at(tail, 2).is_ascii_digit())
                {
                    self.parfilename = filename.to_string();
                    self.version = Version::Par1;
                }

                if DiskFile::file_exists(filename) {
                    result = true;
                }
            }

            // If we haven't figured out which version of PAR file we are
            // using from the file extension, then presumable the files
            // filename was actually the name of a data file.
            if self.version == Version::Unknown {
                // Check for the existence of a PAR2 of PAR file.
                let par2name = format!("{filename}.par2");
                let par2upper = format!("{filename}.PAR2");
                let parname = format!("{filename}.par");
                let parupper = format!("{filename}.PAR");
                if DiskFile::file_exists(&par2name) {
                    self.version = Version::Par2;
                    self.parfilename = par2name;
                    result = true;
                } else if DiskFile::file_exists(&par2upper) {
                    self.version = Version::Par2;
                    self.parfilename = par2upper;
                    result = true;
                } else if DiskFile::file_exists(&parname) {
                    self.version = Version::Par1;
                    self.parfilename = parname;
                    result = true;
                } else if DiskFile::file_exists(&parupper) {
                    self.version = Version::Par1;
                    self.parfilename = parupper;
                    result = true;
                }
            }
        } else {
            self.parfilename = DiskFile::get_canonical_pathname(filename);
            self.version = Version::Par2;
            result = true;
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::diskfile::DiskFile;
    // The parse results for the create/verify/repair commands depend on the
    // current working directory, and the tests change it. Rust runs tests in
    // parallel threads, so the tests that use TempCwd serialise on the shared
    // CWD_LOCK, mirroring the one-after-another execution of the C++ test
    // program.
    use crate::diskfile::CWD_LOCK;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::MutexGuard;

    /// Numbering the temporary directories keeps parallel test binaries from
    /// colliding with one another.
    static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A temporary current working directory, removed when dropped. While it
    /// is held the process-wide working directory is changed, so the
    /// [`CWD_LOCK`] guard must be kept alive alongside it.
    struct TempCwd {
        original: PathBuf,
        dir: PathBuf,
        _lock: MutexGuard<'static, ()>,
    }

    impl TempCwd {
        fn new(name: &str) -> TempCwd {
            let lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let original = std::env::current_dir().unwrap();
            let id = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "par2rust_commandline_test_{}_{}_{}",
                name,
                std::process::id(),
                id
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            std::env::set_current_dir(&dir).unwrap();
            TempCwd {
                original,
                dir,
                _lock: lock,
            }
        }
    }

    impl Drop for TempCwd {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.original).unwrap();
            for attempt in 0..10 {
                match fs::remove_dir_all(&self.dir) {
                    Ok(()) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(_) if attempt == 9 => panic!("could not remove {:?}", self.dir),
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
                }
            }
        }
    }

    /// C++ test9_helper etc.: split a command string on spaces into argv.
    /// A trailing space yields a trailing empty argument, exactly as the C++
    /// helper's space-to-NUL conversion does.
    fn parse_line(cmdline: &str, cl: &mut CommandLine) -> bool {
        let argv: Vec<&str> = cmdline.split(' ').collect();
        let mut sout: Vec<u8> = Vec::new();
        let mut serr: Vec<u8> = Vec::new();
        cl.parse(&mut sout, &mut serr, &argv)
    }

    /// C++ test9_helper: calls parse and expects it to fail.
    fn parse_fail(cmdline: &str) {
        let mut cl = CommandLine::new();
        assert!(
            !parse_line(cmdline, &mut cl),
            "CommandLine should not have parsed: {cmdline:?}"
        );
    }

    // test5: ComputeRecoveryBlockCount, recoveryblockcountset = true
    #[test]
    fn test5_recovery_block_count_set() {
        let mut serr: Vec<u8> = Vec::new();

        let mut recoveryblockcount = 42u32;
        let success = CommandLine::compute_recovery_block_count(
            &mut serr,
            &mut recoveryblockcount,
            100,
            4,
            0,
            Scheme::Variable,
            0,
            true,
            0,
            0,
            40,
        );
        assert!(success, "ComputeRecoveryBlockCount failed test5.1");
        assert_eq!(
            recoveryblockcount, 42,
            "ComputeRecoveryBlockCount should not overwrite recoveryblockcount"
        );

        let mut recoveryblockcount = 66000u32;
        let success = CommandLine::compute_recovery_block_count(
            &mut serr,
            &mut recoveryblockcount,
            100,
            4,
            0,
            Scheme::Variable,
            0,
            true,
            0,
            0,
            40,
        );
        assert!(!success, "should fail for too many blocks");

        let mut recoveryblockcount = 6000u32;
        let success = CommandLine::compute_recovery_block_count(
            &mut serr,
            &mut recoveryblockcount,
            100,
            4,
            60000,
            Scheme::Variable,
            0,
            true,
            0,
            0,
            40,
        );
        assert!(
            !success,
            "should fail for too high a first block coefficient"
        );
    }

    // test6: ComputeRecoveryBlockCount, redundancy > 0
    #[test]
    fn test6_recovery_block_count_redundancy() {
        let mut serr: Vec<u8> = Vec::new();

        let mut recoveryblockcount = 0u32;
        let success = CommandLine::compute_recovery_block_count(
            &mut serr,
            &mut recoveryblockcount,
            100,
            4,
            0,
            Scheme::Variable,
            0,
            false,
            1,
            0,
            40,
        );
        assert!(success, "ComputeRecoveryBlockCount failed test6.1");
        assert_eq!(recoveryblockcount, 1, "1% of 100 is 1");

        let mut recoveryblockcount = 0u32;
        let success = CommandLine::compute_recovery_block_count(
            &mut serr,
            &mut recoveryblockcount,
            1000,
            4,
            0,
            Scheme::Variable,
            0,
            false,
            5,
            0,
            40,
        );
        assert!(success, "ComputeRecoveryBlockCount failed test6.2");
        assert_eq!(recoveryblockcount, 50, "5% of 1000 is 50");

        let mut recoveryblockcount = 0u32;
        let success = CommandLine::compute_recovery_block_count(
            &mut serr,
            &mut recoveryblockcount,
            10,
            4,
            0,
            Scheme::Variable,
            0,
            false,
            1,
            0,
            40,
        );
        assert!(success, "ComputeRecoveryBlockCount failed test6.3");
        assert_eq!(recoveryblockcount, 1, "1% of 10 is still positive");
    }

    // C++ test7_helper: redundancysize > 0, uniform scheme with the recovery
    // file count already determined. Returns Some(2) when the usage is not
    // within one recovery block of the limit, as the C++ helper reports.
    fn test7_helper(
        sourcefilecount: u32, // not used by compute_recovery_block_count!
        sourceblockcount: u32,
        blocksize: u64,
        redundancysize: u64,
        recoveryfilecount: u32,
    ) -> Option<u8> {
        // overhead from packet headers, formatting, etc.
        // fixed: main packet header, main packet contents
        let overhead_fixed = 76i64;
        // per source file: main packet contents, file desc, file slices header
        let overhead_persourcefile = 236i64; // 216 + filename length
        // per source block: file slices contents
        let overhead_persourceblock = 20i64;
        // per recovery block: recovery block header
        let overhead_perrecoveryblock = 68i64;

        let mut serr: Vec<u8> = Vec::new();
        let mut recoveryblockcount = 0u32;
        let success = CommandLine::compute_recovery_block_count(
            &mut serr,
            &mut recoveryblockcount,
            sourceblockcount,
            blocksize,
            0,
            Scheme::Uniform,
            recoveryfilecount,
            false,
            0,
            redundancysize,
            blocksize,
        );
        assert!(success, "ComputeRecoveryBlockCount failed test7.1");

        let usage_per_recoveryfile = overhead_fixed
            + i64::from(sourcefilecount) * overhead_persourcefile
            + i64::from(sourceblockcount) * overhead_persourceblock;
        let usage = i64::from(recoveryfilecount) * usage_per_recoveryfile
            + i64::from(recoveryblockcount)
                * (overhead_perrecoveryblock + blocksize as i64);
        if usage <= redundancysize as i64 - (overhead_perrecoveryblock + blocksize as i64)
            || usage > redundancysize as i64
        {
            eprintln!(
                "ComputeRecoveryBlockCount {redundancysize} data limit\n   \
                 but usage was {usage} with {recoveryblockcount} blocks"
            );
            return Some(2);
        }
        None
    }

    // test7: redundancysize > 0, scUniform with number of recovery files
    // already determined.
    #[test]
    fn test7_recovery_block_count_redundancysize_uniform() {
        // CD is 10 files, 600 source blocks, 1MB block size
        // Redundancys is 40MB in 5 files.
        assert_eq!(test7_helper(10, 600, 1024 * 1024, 40 * 1024 * 1024, 5), None);
        assert_eq!(test7_helper(1, 600, 1024 * 1024, 40 * 1024 * 1024, 5), None);
        assert_eq!(test7_helper(10, 60, 1024 * 1024, 40 * 1024 * 1024, 5), None);
        assert_eq!(test7_helper(10, 600, 16 * 1024, 40 * 1024 * 1024, 5), None);
        assert_eq!(test7_helper(10, 600, 1024 * 1024, 10 * 1024 * 1024, 5), None);
        assert_eq!(test7_helper(10, 600, 1024 * 1024, 40 * 1024 * 1024, 2), None);

        // DVD is 1 files, 5000 source blocks, 1MB block size
        // Redundancys is 50MB in 5 files.
        assert_eq!(test7_helper(1, 5000, 1024 * 1024, 50 * 1024 * 1024, 5), None);

        // if redundancy size is too small, still have 1 block
        let mut serr: Vec<u8> = Vec::new();
        let mut recoveryblockcount = 0u32;
        let success = CommandLine::compute_recovery_block_count(
            &mut serr,
            &mut recoveryblockcount,
            1000,
            1024,
            0,
            Scheme::Uniform,
            1,
            false,
            0,
            4, // = redundancysize
            1024,
        );
        assert!(success, "ComputeRecoveryBlockCount failed test7.2");
        assert_eq!(
            recoveryblockcount, 1,
            "with small redundancy amount should still return 1"
        );
    }

    // C++ test8_helper: redundancysize > 0, variable scheme with the number
    // of recovery files undetermined.
    fn test8_helper(
        sourcefilecount: u32, // not used by compute_recovery_block_count!
        sourceblockcount: u32,
        blocksize: u64,
        redundancysize: u64,
    ) -> Option<u8> {
        // overhead from packet headers, formatting, etc.
        // fixed: main packet header, main packet contents
        let overhead_fixed = 76i64;
        // per source file: main packet contents, file desc, file slices header
        let overhead_persourcefile = 236i64; // 216 + filename length
        // per source block: file slices contents
        let overhead_persourceblock = 20i64;
        // per recovery bloc: recovery block header
        let overhead_perrecoveryblock = 68i64;

        let mut serr: Vec<u8> = Vec::new();
        let mut recoveryblockcount = 0u32;
        let success = CommandLine::compute_recovery_block_count(
            &mut serr,
            &mut recoveryblockcount,
            sourceblockcount,
            blocksize,
            0,
            Scheme::Variable,
            0, // =recoveryfilecount
            false,
            0,
            redundancysize,
            blocksize,
        );
        assert!(success, "ComputeRecoveryBlockCount failed test8.1");

        let mut recoveryfilecount = 0u32;
        let success = compute_recovery_file_count(
            &mut serr,
            &mut recoveryfilecount,
            Scheme::Variable,
            recoveryblockcount,
            blocksize,
            blocksize,
        );
        assert!(success, "ComputeRecoveryBlockCount failed test8.2");

        let usage_per_recoveryfile = overhead_fixed
            + i64::from(sourcefilecount) * overhead_persourcefile
            + i64::from(sourceblockcount) * overhead_persourceblock;
        let usage = i64::from(recoveryfilecount) * usage_per_recoveryfile
            + i64::from(recoveryblockcount)
                * (overhead_perrecoveryblock + blocksize as i64);
        if usage <= redundancysize as i64 - (overhead_perrecoveryblock + blocksize as i64)
            || usage > redundancysize as i64
        {
            eprintln!(
                "ComputeRecoveryBlockCount {redundancysize} data limit\n   \
                 but usage was {usage} with {recoveryblockcount} blocks"
            );
            return Some(2);
        }
        None
    }

    // test8: redundancysize > 0, scVariable with number of recovery files
    // undetermined.
    #[test]
    fn test8_recovery_block_count_redundancysize_variable() {
        assert_eq!(test8_helper(10, 600, 1024 * 1024, 40 * 1024 * 1024), None);
        assert_eq!(test8_helper(1, 600, 1024 * 1024, 40 * 1024 * 1024), None);
        assert_eq!(test8_helper(10, 60, 1024 * 1024, 40 * 1024 * 1024), None);

        //
        // WARNING: THIS TEST USUALLY FAILS BY A SMALL AMOUNT
        //    The ==1 at the end will make the test fail
        //    if the function doesn't return any value.
        //
        assert_ne!(test8_helper(10, 600, 16 * 1024, 40 * 1024 * 1024), Some(1));

        assert_eq!(test8_helper(10, 600, 1024 * 1024, 10 * 1024 * 1024), None);
        assert_eq!(test8_helper(1, 5000, 1024 * 1024, 50 * 1024 * 1024), None);
    }

    // test9: calls to parse that are expected to fail.
    #[test]
    fn test9_bad_parses() {
        let _cwd = TempCwd::new("test9");
        // create input files, in case they are read.
        fs::write("foo.par2", "dummy par2 file.  Just has to exist.").unwrap();
        fs::write("bar.par2", "dummy par2 file.  Just has to exist.").unwrap();
        fs::write("input1.txt", "commandline_test test9 input1.txt\n").unwrap();
        fs::write("input2.txt", "commandline_test test9 input2.txt\n").unwrap();

        // mistaken call for help, version, etc.
        parse_fail("par2 -?");
        parse_fail("par2 -v");

        // missing operation
        parse_fail("par2");
        parse_fail("par2 repairfoo.par");
        parse_fail("par2 -p repairfoo.par");
        parse_fail("par2 -b100 createfoo.par");

        // missing parfile
        parse_fail("par2 repair");
        parse_fail("par2repair");
        parse_fail("par2 create");
        parse_fail("par2create");
        parse_fail("par2verify");
        parse_fail("par2 verify");
        parse_fail("par2 create ");

        // missing input files for create
        parse_fail("par2 create foo.par2");

        // wrong options for the chosen operation
        parse_fail("par2 create -p foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -N foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -S100 foo.par2 input1.txt input2.txt");
        parse_fail("par2 repair -b100 foo.par2");
        parse_fail("par2 repair -s100 foo.par2");
        parse_fail("par2 repair -r5 foo.par2");
        parse_fail("par2 repair -rg5 foo.par2");
        parse_fail("par2 repair -c100 foo.par2");
        parse_fail("par2 repair -f10 foo.par2");
        parse_fail("par2 repair -u foo.par2");
        parse_fail("par2 repair -l foo.par2");
        parse_fail("par2 repair -n10 foo.par2");
        parse_fail("par2 repair -R foo.par2");

        // bad combinations of options
        parse_fail("par2 create -v -q foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -q -v foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -v -v -q foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -q -q -v foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -b100 -s100 foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -rg5 -c100 foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -u -l foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -n10 -l foo.par2 input1.txt input2.txt");

        // badly formatted options
        parse_fail("par2 create -zzz foo.par2 input1.txt input2.txt");
        parse_fail("par2 create --v foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -s3 foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -m50m foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -rt5 foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -bad foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -sad foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -rad foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -cad foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -fad foo.par2 input1.txt input2.txt");
        parse_fail("par2 create -nad foo.par2 input1.txt input2.txt");
    }

    /// The expected parse results for a create command line; the arguments
    /// of the C++ test10_helper.
    #[derive(Clone)]
    struct Expect {
        noiselevel: NoiseLevel,
        memorylimit: usize,
        basepath: String,
        nthreads: u32,
        filethreads: u32,
        parfilename: String,
        extrafiles: Vec<String>,
        blocksize: u64,
        firstblock: u32,
        recoveryfilescheme: Scheme,
        recoveryfilecount: u32,
        recoveryblockcount: u32,
    }

    /// C++ test10_helper: parse a create command line and check that it
    /// results in the expected settings.
    fn check_create(cmdline: &str, exp: &Expect) {
        let mut cl = CommandLine::new();
        assert!(
            parse_line(cmdline, &mut cl),
            "CommandLine said it was unable to parse {cmdline:?}"
        );
        assert_eq!(cl.version, Version::Par2, "version arg={cmdline:?}");
        assert_eq!(cl.operation, Operation::Create, "operation arg={cmdline:?}");
        assert_eq!(cl.noiselevel, exp.noiselevel, "noiselevel arg={cmdline:?}");
        assert_eq!(cl.memorylimit, exp.memorylimit, "memorylimit arg={cmdline:?}");
        assert_eq!(cl.basepath, exp.basepath, "basepath arg={cmdline:?}");
        assert_eq!(cl.nthreads, exp.nthreads, "nthreads arg={cmdline:?}");
        assert_eq!(cl.filethreads, exp.filethreads, "filethreads arg={cmdline:?}");
        assert_eq!(cl.parfilename, exp.parfilename, "parfilename arg={cmdline:?}");
        assert_eq!(cl.extrafiles, exp.extrafiles, "extrafiles arg={cmdline:?}");
        assert_eq!(cl.blocksize, exp.blocksize, "blocksize arg={cmdline:?}");
        assert_eq!(cl.firstblock, exp.firstblock, "firstblock arg={cmdline:?}");
        assert_eq!(
            cl.recoveryfilescheme, exp.recoveryfilescheme,
            "recoveryfilescheme arg={cmdline:?}"
        );
        assert_eq!(
            cl.recoveryfilecount, exp.recoveryfilecount,
            "recoveryfilecount arg={cmdline:?}"
        );
        assert_eq!(
            cl.recoveryblockcount, exp.recoveryblockcount,
            "recoveryblockcount arg={cmdline:?}"
        );
    }

    // test10: calls to "par2 create".
    #[test]
    fn test10_create_parse() {
        let _cwd = TempCwd::new("test10");
        // create input files, in case they are read.
        let input1_contents = "commandline_test test10 input1.txt\n";
        let input2_contents = "commandline_test test10 input2.txt\n";
        fs::write("input1.txt", input1_contents).unwrap();
        fs::write("input2.txt", input2_contents).unwrap();

        // Call once. The results are used to initialize some defaults.
        let mut defaults = CommandLine::new();
        assert!(
            parse_line("par2 create foo.par2 input1.txt input2.txt", &mut defaults),
            "CommandLine unable to fetch default values."
        );

        // Define the default values. (Using fields makes it more obvious
        // when a non-default value is present in a test.)
        let basepath = defaults.basepath.clone();
        let expect = Expect {
            noiselevel: NoiseLevel::Normal,
            memorylimit: defaults.memorylimit,
            basepath: basepath.clone(),
            nthreads: 0,
            filethreads: FILE_THREADS,
            parfilename: basepath.clone() + "foo", // ".par2" is stripped.
            extrafiles: vec![
                basepath.clone() + "input1.txt",
                basepath.clone() + "input2.txt",
            ],
            blocksize: 4, // tries to make 2000 blocks; this is closest
            firstblock: 0,
            recoveryfilescheme: Scheme::Variable,
            recoveryfilecount: 0,
            recoveryblockcount: 1, // 5% recovery data; this is the minimum
        };

        check_create("par2 create foo.par2 input1.txt input2.txt", &expect);

        // -v
        {
            let mut e = expect.clone();
            e.noiselevel = NoiseLevel::Noisy;
            check_create("par2 create -v foo.par2 input1.txt input2.txt", &e);
        }
        // -v -v
        {
            let mut e = expect.clone();
            e.noiselevel = NoiseLevel::Debug;
            check_create("par2 create -v -v foo.par2 input1.txt input2.txt", &e);
        }
        // -q
        {
            let mut e = expect.clone();
            e.noiselevel = NoiseLevel::Quiet;
            check_create("par2 create -q foo.par2 input1.txt input2.txt", &e);
        }
        // -q -q
        {
            let mut e = expect.clone();
            e.noiselevel = NoiseLevel::Silent;
            check_create("par2 create -q -q foo.par2 input1.txt input2.txt", &e);
        }
        // -m option
        {
            let mut e = expect.clone();
            e.memorylimit = 16 * 1024 * 1024;
            check_create("par2 create -m16 foo.par2 input1.txt input2.txt", &e);
        }
        // -t option
        {
            let mut e = expect.clone();
            e.nthreads = 42;
            check_create("par2 create -t42 foo.par2 input1.txt input2.txt", &e);
        }
        // -T option
        {
            let mut e = expect.clone();
            e.filethreads = 42;
            check_create("par2 create -T42 foo.par2 input1.txt input2.txt", &e);
        }
        // -- option
        check_create("par2 create -- foo.par2 input1.txt input2.txt", &expect);
        {
            let mut e = expect.clone();
            e.parfilename = basepath.clone() + "-foo";
            check_create("par2 create -- -foo.par2 input1.txt input2.txt", &e);
        }
        {
            // a file name that starts with a dash, after the "--"
            fs::write("-input2.txt", "commandline_test test10 -input2.txt\n").unwrap();
            let mut e = expect.clone();
            e.extrafiles = vec![
                basepath.clone() + "input1.txt",
                basepath.clone() + "-input2.txt",
            ];
            check_create("par2 create foo.par2 input1.txt -- -input2.txt", &e);
            let _ = fs::remove_file("-input2.txt");
        }
        // -a option (explicitly set the par filename)
        check_create("par2 create -afoo.par2 input1.txt input2.txt", &expect);

        // -b option: block size is the largest file rounded up to a
        // multiple of 4 bytes.
        let longestfilelen = input1_contents.len().max(input2_contents.len());
        let mut longestfilelen_rounded_up = longestfilelen as u64;
        if longestfilelen_rounded_up % 4 != 0 {
            longestfilelen_rounded_up += 4 - (longestfilelen_rounded_up % 4);
        }
        {
            let mut e = expect.clone();
            e.blocksize = longestfilelen_rounded_up;
            check_create("par2 create -b2 foo.par2 input1.txt input2.txt", &e);
        }
        // -s option
        {
            let mut e = expect.clone();
            e.blocksize = 8;
            check_create("par2 create -s8 foo.par2 input1.txt input2.txt", &e);
        }

        let mut default_sourceblockcount = 0u32;
        for contents in [input1_contents, input2_contents] {
            default_sourceblockcount += (contents.len() / 4) as u32;
            if contents.len() % 4 != 0 {
                default_sourceblockcount += 1;
            }
        }

        // -r option: recoverycount = sourcecount
        {
            let mut e = expect.clone();
            e.recoveryblockcount = default_sourceblockcount;
            check_create("par2 create -r100 foo.par2 input1.txt input2.txt", &e);
        }
        // -rk option: 1024 bytes of recovery data.
        {
            let mut e = expect.clone();
            e.recoveryblockcount = 8;
            check_create("par2 create -rk1 foo.par2 input1.txt input2.txt", &e);
        }
        // -c option
        {
            let mut e = expect.clone();
            e.recoveryblockcount = 42;
            check_create("par2 create -c42 foo.par2 input1.txt input2.txt", &e);
        }
        // -f option
        {
            let mut e = expect.clone();
            e.firstblock = 42;
            check_create("par2 create -f42 foo.par2 input1.txt input2.txt", &e);
        }
        // -u option
        {
            let mut e = expect.clone();
            e.recoveryfilescheme = Scheme::Uniform;
            check_create("par2 create -u foo.par2 input1.txt input2.txt", &e);
        }
        // -l option
        {
            let mut e = expect.clone();
            e.recoveryfilescheme = Scheme::Limited;
            check_create("par2 create -l foo.par2 input1.txt input2.txt", &e);
        }
        // -n option: when the recovery file number is set the scheme is
        // uniform.
        {
            let mut e = expect.clone();
            e.recoveryfilescheme = Scheme::Uniform;
            e.recoveryfilecount = 31;
            check_create("par2 create -n31 foo.par2 input1.txt input2.txt", &e);
        }
    }

    /// The expected parse results for a verify/repair command line; the
    /// arguments of the C++ test11_helper.
    #[derive(Clone)]
    struct ExpectRepair {
        noiselevel: NoiseLevel,
        memorylimit: usize,
        basepath: String,
        nthreads: u32,
        filethreads: u32,
        parfilename: String,
        extrafiles: Vec<String>,
        version: Version,
        operation: Operation,
        purgefiles: bool,
        skipdata: bool,
        skipleaway: u64,
    }

    /// C++ test11_helper: parse a verify/repair command line and check that
    /// it results in the expected settings.
    fn check_repair(cmdline: &str, exp: &ExpectRepair) {
        let mut cl = CommandLine::new();
        assert!(
            parse_line(cmdline, &mut cl),
            "CommandLine said it was unable to parse {cmdline:?}"
        );
        assert_eq!(cl.version, exp.version, "version arg={cmdline:?}");
        assert_eq!(cl.operation, exp.operation, "operation arg={cmdline:?}");
        assert_eq!(cl.noiselevel, exp.noiselevel, "noiselevel arg={cmdline:?}");
        assert_eq!(cl.memorylimit, exp.memorylimit, "memorylimit arg={cmdline:?}");
        assert_eq!(cl.basepath, exp.basepath, "basepath arg={cmdline:?}");
        assert_eq!(cl.nthreads, exp.nthreads, "nthreads arg={cmdline:?}");
        assert_eq!(cl.filethreads, exp.filethreads, "filethreads arg={cmdline:?}");
        assert_eq!(cl.parfilename, exp.parfilename, "parfilename arg={cmdline:?}");
        assert_eq!(cl.extrafiles, exp.extrafiles, "extrafiles arg={cmdline:?}");
        assert_eq!(cl.purgefiles, exp.purgefiles, "purgefiles arg={cmdline:?}");
        if exp.version == Version::Par2 {
            assert_eq!(cl.skipdata, exp.skipdata, "skipdata arg={cmdline:?}");
            assert_eq!(cl.skipleaway, exp.skipleaway, "skipleaway arg={cmdline:?}");
        }
    }

    // test11: calls to repair/verify.
    #[test]
    fn test11_repair_verify_parse() {
        let _cwd = TempCwd::new("test11");
        // create input files, in case they are read.
        fs::write("foo.par2", "dummy par2 file.  Just has to exist.").unwrap();
        fs::write("input1.txt", "commandline_test test11 input1.txt\n").unwrap();
        fs::write("input2.txt", "commandline_test test11 input2.txt\n").unwrap();

        // Call once. The results are used to initialize some defaults.
        let mut defaults = CommandLine::new();
        assert!(
            parse_line("par2 repair foo.par2 input1.txt input2.txt", &mut defaults),
            "CommandLine unable to fetch default values."
        );

        let basepath = defaults.basepath.clone();
        let expect = ExpectRepair {
            noiselevel: NoiseLevel::Normal,
            memorylimit: defaults.memorylimit,
            basepath: basepath.clone(),
            nthreads: 0,
            filethreads: FILE_THREADS,
            parfilename: "foo.par2".to_string(), // relative, ".par2" NOT stripped
            extrafiles: vec![
                basepath.clone() + "input1.txt",
                basepath.clone() + "input2.txt",
            ],
            version: Version::Par2,
            operation: Operation::Repair,
            purgefiles: false,
            skipdata: false,
            skipleaway: 0,
        };

        check_repair("par2 repair foo.par2 input1.txt input2.txt", &expect);

        // -v
        {
            let mut e = expect.clone();
            e.noiselevel = NoiseLevel::Noisy;
            check_repair("par2 repair -v foo.par2 input1.txt input2.txt", &e);
        }
        {
            let mut e = expect.clone();
            e.noiselevel = NoiseLevel::Noisy;
            e.operation = Operation::Verify;
            check_repair("par2 verify -v foo.par2 input1.txt input2.txt", &e);
        }
        // -v -v
        {
            let mut e = expect.clone();
            e.noiselevel = NoiseLevel::Debug;
            check_repair("par2 repair -v -v foo.par2 input1.txt input2.txt", &e);
        }
        // -q
        {
            let mut e = expect.clone();
            e.noiselevel = NoiseLevel::Quiet;
            check_repair("par2 repair -q foo.par2 input1.txt input2.txt", &e);
        }
        // -q -q
        {
            let mut e = expect.clone();
            e.noiselevel = NoiseLevel::Silent;
            check_repair("par2 repair -q -q foo.par2 input1.txt input2.txt", &e);
        }
        // -m
        {
            let mut e = expect.clone();
            e.memorylimit = 16 * 1024 * 1024;
            check_repair("par2 repair -m16 foo.par2 input1.txt input2.txt", &e);
        }
        // -t
        {
            let mut e = expect.clone();
            e.nthreads = 42;
            check_repair("par2 repair -t42 foo.par2 input1.txt input2.txt", &e);
        }
        // -T
        {
            let mut e = expect.clone();
            e.filethreads = 42;
            check_repair("par2 repair -T42 foo.par2 input1.txt input2.txt", &e);
        }
        // --
        check_repair("par2 repair -- foo.par2 input1.txt input2.txt", &expect);
        {
            // a par file name that starts with a dash, after the "--"
            fs::write("-foo.par2", "anything\n").unwrap();
            let mut e = expect.clone();
            e.parfilename = "-foo.par2".to_string();
            check_repair("par2 repair -- -foo.par2 input1.txt input2.txt", &e);
            let _ = fs::remove_file("-foo.par2");
        }
        {
            // a data file name that starts with a dash, after the "--"
            fs::write("-input2.txt", "commandline_test test11 -input2.txt\n").unwrap();
            let mut e = expect.clone();
            e.extrafiles = vec![
                basepath.clone() + "input1.txt",
                basepath.clone() + "-input2.txt",
            ];
            check_repair("par2 repair foo.par2 input1.txt -- -input2.txt", &e);
            let _ = fs::remove_file("-input2.txt");
        }
        // -p
        {
            let mut e = expect.clone();
            e.purgefiles = true;
            check_repair("par2 repair -p foo.par2 input1.txt input2.txt", &e);
        }
        // -p with verify
        {
            let mut e = expect.clone();
            e.operation = Operation::Verify;
            e.purgefiles = true;
            check_repair("par2 verify -p foo.par2 input1.txt input2.txt", &e);
        }
        // -N
        {
            let mut e = expect.clone();
            e.skipdata = true;
            e.skipleaway = 64;
            check_repair("par2 repair -N foo.par2 input1.txt input2.txt", &e);
        }
        // -S
        {
            let mut e = expect.clone();
            e.skipdata = true;
            e.skipleaway = 42;
            check_repair("par2 repair -N -S42 foo.par2 input1.txt input2.txt", &e);
        }

        // remove par2 file
        let _ = fs::remove_file("foo.par2");

        // Create par1 file. Do a few tests.
        fs::write("bar.par", "dummy par1 file.  Just has to exist.").unwrap();
        {
            let mut e = expect.clone();
            e.parfilename = "bar.par".to_string();
            e.version = Version::Par1;
            check_repair("par2 repair bar.par input1.txt input2.txt", &e);

            e.operation = Operation::Verify;
            check_repair("par2 verify bar.par input1.txt input2.txt", &e);
        }
    }

    // test12: calls to print help, version.
    #[test]
    fn test12_help_version_parses() {
        for cmd in [
            "par2 --help",
            "par2 -h",
            "par2create --help",
            "par2 --version",
            "par2 -V",
            "par2 -VV",
            "par2create --version",
        ] {
            let mut cl = CommandLine::new();
            assert!(parse_line(cmd, &mut cl), "CommandLine failed for {cmd:?}");
        }
    }

    // test13: the "-L" (follow symlinks) option. Unix only, like the C++
    // original test.
    #[cfg(unix)]
    #[test]
    fn test13_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let _cwd = TempCwd::new("test13");
        // create an input file and a symlink to it.
        fs::write("test13_input1.txt", "commandline_test test13 input1.txt\n").unwrap();
        let _ = fs::remove_file("test13_link1.txt");
        symlink("test13_input1.txt", "test13_link1.txt").unwrap();

        // -L is only allowed when creating.
        parse_fail("par2 repair -L foo.par2 test13_link1.txt");
        parse_fail("par2 verify -L foo.par2 test13_link1.txt");

        // without -L, a symlink is not accepted as a source file
        // (so create fails because there are no source files).
        parse_fail("par2 create foo.par2 test13_link1.txt");

        // with -L, a symlink to a file is accepted as a source file.
        let mut cl = CommandLine::new();
        assert!(parse_line(
            "par2 create -L foo.par2 test13_link1.txt",
            &mut cl
        ));
        assert!(cl.followlinks, "test13 fail followlinks");
        assert_eq!(
            cl.extrafiles,
            vec![DiskFile::get_canonical_pathname("test13_link1.txt")],
            "test13 fail extrafiles"
        );
    }
}
