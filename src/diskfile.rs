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

//! Port of par2cmdline's diskfile.h / diskfile.cpp.
//!
//! C++ selects a Win32 or POSIX implementation at compile time; this port
//! uses `std::fs` so one implementation covers both platforms. Positional
//! reads use `FileExt::read_at` (Windows `ReadFile` with `OVERLAPPED`,
//! POSIX `pread`). Error messages are written to a caller-supplied `serr`
//! stream instead of the C++ member `std::ostream`.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;

/// Positional read: C++ uses `pread` on POSIX and `ReadFile` with an
/// `OVERLAPPED` position on Windows; std exposes these as `read_at`
/// (unix) and `seek_read` (windows).
#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    f.read_at(buf, offset)
}

#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    f.seek_read(buf, offset)
}

/// Positional write: C++ uses `fseek`+`fwrite` on POSIX and
/// `SetFilePointer`+`WriteFile` on Windows.
#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], offset: u64) -> std::io::Result<usize> {
    f.write_at(buf, offset)
}

#[cfg(windows)]
fn write_at(f: &File, buf: &[u8], offset: u64) -> std::io::Result<usize> {
    f.seek_write(buf, offset)
}

/// C++ MAX_LENGTH (libpar2internal.h): 0xffffffff & ~7
pub const MAX_LENGTH: usize = 0xffff_ffff & !7;

/// C++ _MAX_PATH (libpar2internal.h)
pub const MAX_PATH_LEN: usize = if cfg!(windows) { 32767 } else { 4095 };

/// C++ PATHSEP / ALTPATHSEP
#[cfg(windows)]
pub const PATH_SEP: char = '\\';
#[cfg(not(windows))]
pub const PATH_SEP: char = '/';

pub struct DiskFile {
    filename: String,
    filesize: u64,
    offset: u64,
    file: Option<File>,
    exists: bool,
}

impl DiskFile {
    pub fn new() -> DiskFile {
        DiskFile {
            filename: String::new(),
            filesize: 0,
            offset: 0,
            file: None,
            exists: false,
        }
    }

    pub fn file_name(&self) -> &str {
        &self.filename
    }

    pub fn file_size(&self) -> u64 {
        self.filesize
    }

    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }

    pub fn exists(&self) -> bool {
        self.exists
    }

    /// C++ DiskFile::CreateParentDirectory
    pub fn create_parent_directory(pathname: &str, serr: &mut dyn Write) -> bool {
        // do we have a path separator in the filename ?
        if let Some(where_) = pathname.rfind(['/', '\\']) {
            let path = &pathname[..where_];

            // Handle Windows root path (e.g., "C:" or empty path from "\file")
            // If path is empty or is a drive letter (e.g., "C:"), the root already exists
            if path.is_empty() || (cfg!(windows) && path.len() == 2 && path.as_bytes()[1] == b':') {
                return true;
            }

            if Path::new(path).exists() {
                return true; // let the caller deal with non-directories
            }

            if !DiskFile::create_parent_directory(path, serr) {
                return false;
            }

            if let Err(e) = fs::create_dir(path) {
                let _ = writeln!(serr, "Could not create the {path} directory: {e}");
                return false;
            }
        }
        true
    }

    /// C++ DiskFile::Create — create new file on disk with the given size.
    pub fn create(&mut self, filename: &str, filesize: u64, serr: &mut dyn Write) -> bool {
        debug_assert!(self.file.is_none());

        self.filename = filename.to_string();
        self.filesize = filesize;

        if !DiskFile::create_parent_directory(filename, serr) {
            return false;
        }

        // Create the file (C++ CreateFileW uses GENERIC_READ | GENERIC_WRITE)
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(filename)
        {
            Err(e) => {
                let _ = writeln!(serr, "Could not create \"{filename}\": {e}");
                return false;
            }
            Ok(f) => self.file = Some(f),
        }

        if filesize > 0 {
            // Set the end of the file
            if let Err(e) = self.file.as_ref().unwrap().set_len(filesize) {
                let _ = writeln!(serr, "Could not set size of \"{filename}\": {e}");
                self.file = None;
                let _ = fs::remove_file(filename);
                return false;
            }
        }

        self.offset = filesize;

        self.exists = true;
        true
    }

    /// C++ DiskFile::Write with default maxlength = MAX_LENGTH
    pub fn write(&mut self, offset: u64, buffer: &[u8], serr: &mut dyn Write) -> bool {
        self.write_max(offset, buffer, MAX_LENGTH, serr)
    }

    /// C++ DiskFile::Write
    pub fn write_max(&mut self, offset: u64, buffer: &[u8], maxlength: usize, serr: &mut dyn Write) -> bool {
        debug_assert!(self.file.is_some());

        let mut off = offset;
        let mut length = buffer.len();
        let mut pos = 0usize;

        while length > 0 {
            let want = if length > maxlength { maxlength } else { length };

            // Write the data
            match write_at(self.file.as_ref().unwrap(), &buffer[pos..pos + want], off) {
                Err(e) => {
                    let _ = writeln!(
                        serr,
                        "Could not write {length} bytes to \"{}\" at offset {off}: {e}",
                        self.filename
                    );
                    return false;
                }
                Ok(wrote) => {
                    if wrote != want {
                        let _ = writeln!(
                            serr,
                            "INFO: Incomplete write to \"{}\" at offset {off}.  Expected to write {want} bytes and wrote {wrote} bytes.",
                            self.filename
                        );
                    }
                    off += wrote as u64;
                    length -= wrote;
                    pos += wrote;
                }
            }

            if self.filesize < off {
                self.filesize = off;
            }
        }

        self.offset = off;
        true
    }

    /// C++ DiskFile::Open() — reopen the current filename
    pub fn reopen(&mut self, serr: &mut dyn Write) -> bool {
        let filename = self.filename.clone();
        self.open(&filename, serr)
    }

    /// C++ DiskFile::Open(const std::string&) — size comes from disk
    pub fn open(&mut self, filename: &str, serr: &mut dyn Write) -> bool {
        let filesize = DiskFile::get_file_size(filename);
        self.open_with_size(filename, filesize, serr)
    }

    /// C++ DiskFile::Open(const std::string&, u64)
    pub fn open_with_size(&mut self, filename: &str, filesize: u64, _serr: &mut dyn Write) -> bool {
        debug_assert!(self.file.is_none());

        self.filename = filename.to_string();
        self.filesize = filesize;

        match File::open(filename) {
            Err(_) => return false,
            Ok(f) => self.file = Some(f),
        }

        self.offset = 0;
        self.exists = true;

        true
    }

    /// C++ DiskFile::Read with default maxlength = MAX_LENGTH
    pub fn read(&mut self, offset: u64, buffer: &mut [u8], serr: &mut dyn Write) -> bool {
        self.read_max(offset, buffer, MAX_LENGTH, serr)
    }

    /// C++ DiskFile::Read
    pub fn read_max(&mut self, offset: u64, buffer: &mut [u8], maxlength: usize, serr: &mut dyn Write) -> bool {
        debug_assert!(self.file.is_some());

        let mut off = offset;
        let mut length = buffer.len();
        let mut pos = 0usize;

        while length > 0 {
            let want = if length > maxlength { maxlength } else { length };

            // Read from the given position without using the handle's file pointer
            match read_at(self.file.as_ref().unwrap(), &mut buffer[pos..pos + want], off) {
                Err(e) => {
                    let _ = writeln!(
                        serr,
                        "Could not read {length} bytes from \"{}\" at offset {off}: {e}",
                        self.filename
                    );
                    return false;
                }
                Ok(0) => {
                    let _ = writeln!(
                        serr,
                        "Could not read {length} bytes from \"{}\" at offset {off}: unexpected end of file.",
                        self.filename
                    );
                    return false;
                }
                Ok(got) => {
                    if got != want {
                        let _ = writeln!(
                            serr,
                            "Incomplete read from \"{}\" at offset {off}.  Tried to read {want} bytes and received {got} bytes.",
                            self.filename
                        );
                        return false;
                    }
                    off += got as u64;
                    length -= got;
                    pos += got;
                }
            }
        }

        true
    }

    /// C++ DiskFile::Close
    pub fn close(&mut self) {
        self.file = None;
    }

    /// C++ DiskFile::Delete
    pub fn delete(&mut self, serr: &mut dyn Write) -> bool {
        debug_assert!(self.file.is_none());

        if !self.filename.is_empty() && fs::remove_file(&self.filename).is_ok() {
            self.exists = false;
            true
        } else {
            let _ = writeln!(serr, "Cannot delete {}", self.filename);
            false
        }
    }

    /// C++ DiskFile::Rename() — append ".1", ".2", ... until free
    pub fn rename_auto(&mut self, serr: &mut dyn Write) -> bool {
        let mut index: u32 = 0;
        let newname = loop {
            index += 1;
            let candidate = format!("{}.{}", self.filename, index);

            // Check path length against maximum
            if candidate.len() > MAX_PATH_LEN {
                let _ = writeln!(
                    serr,
                    "{} pathlength is more than {MAX_PATH_LEN}.",
                    self.filename
                );
                return false;
            }

            if !Path::new(&candidate).exists() {
                break candidate;
            }
        };

        self.rename_to(&newname, serr)
    }

    /// C++ DiskFile::Rename(std::string)
    pub fn rename_to(&mut self, newname: &str, serr: &mut dyn Write) -> bool {
        debug_assert!(self.file.is_none());

        if fs::rename(&self.filename, newname).is_ok() {
            self.filename = newname.to_string();
            true
        } else {
            let _ = writeln!(
                serr,
                "{} cannot be renamed to {newname}",
                self.filename
            );
            false
        }
    }

    /// C++ DiskFile::GetCanonicalPathname
    pub fn get_canonical_pathname(filename: &str) -> String {
        // Is the supplied path already an absolute one
        let is_abs = filename.starts_with('/')
            || (cfg!(windows)
                && (filename.starts_with('\\')
                    || (filename.len() >= 2 && filename.as_bytes()[1] == b':')));

        let joined = if is_abs {
            filename.to_string()
        } else {
            // Get the current directory
            match std::env::current_dir() {
                Ok(curdir) => format!("{}{}{}", curdir.display(), PATH_SEP, filename),
                Err(_) => return filename.to_string(),
            }
        };

        // Normalize /./ and /../ segments (C++ work/in/out scan). On Windows
        // both separators count; on POSIX a backslash is a literal character.
        let chars: Vec<char> = joined.chars().collect();
        let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
        let mut work: Vec<char> = Vec::with_capacity(chars.len());
        let mut inx = 0usize;
        while inx < chars.len() {
            if is_sep(chars[inx]) {
                if inx + 2 < chars.len()
                    && chars[inx + 1] == '.'
                    && is_sep(chars[inx + 2])
                {
                    // skip the input past /./
                    inx += 2;
                } else if inx + 3 < chars.len()
                    && chars[inx + 1] == '.'
                    && chars[inx + 2] == '.'
                    && is_sep(chars[inx + 3])
                {
                    // backtrack the output if /../ was found on the input
                    inx += 3;
                    if !work.is_empty() {
                        // drop the last component
                        work.pop();
                        while let Some(&c) = work.last() {
                            if is_sep(c) {
                                break;
                            }
                            work.pop();
                        }
                    }
                } else {
                    work.push(PATH_SEP);
                    inx += 1;
                }
            } else {
                work.push(chars[inx]);
                inx += 1;
            }
        }

        if cfg!(windows) {
            // uppercase the drive letter, use backslashes
            if let Some(c0) = work.first_mut() {
                *c0 = c0.to_ascii_uppercase();
            }
            for c in work.iter_mut() {
                if *c == '/' {
                    *c = '\\';
                }
            }
        }
        work.into_iter().collect()
    }

    /// C++ DiskFile::FindFiles with default followlinks = false
    pub fn find_files(path: &str, wildcard: &str, recursive: bool) -> Vec<String> {
        DiskFile::find_files_follow(path, wildcard, recursive, false)
    }

    /// C++ DiskFile::FindFiles
    pub fn find_files_follow(
        path: &str,
        wildcard: &str,
        recursive: bool,
        followlinks: bool,
    ) -> Vec<String> {
        // check path, if not ending with path separator, add one
        let mut path = path.to_string();
        if !path.ends_with(PATH_SEP) {
            path.push(PATH_SEP);
        }
        let mut matches: Vec<String> = Vec::new();

        if wildcard.contains(['*', '?']) {
            if let Ok(entries) = fs::read_dir(&path) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();

                    if name == "." || name == ".." {
                        continue;
                    }

                    if !wildcard_match(wildcard, &name) {
                        continue;
                    }

                    let fn_ = path.clone() + &name;
                    classify_add(&fn_, recursive, followlinks, &mut matches);
                }
            }
        } else {
            let fn_ = path.clone() + wildcard;
            classify_add(&fn_, recursive, followlinks, &mut matches);
        }

        matches
    }

    /// C++ DiskFile::GetFileSize
    pub fn get_file_size(filename: &str) -> u64 {
        match fs::metadata(filename) {
            Ok(st) if st.is_file() => st.len(),
            _ => 0,
        }
    }

    /// C++ DiskFile::FileExists
    pub fn file_exists(filename: &str) -> bool {
        match fs::metadata(filename) {
            Ok(st) => st.is_file(),
            Err(_) => false,
        }
    }

    /// C++ DiskFile::SplitFilename
    pub fn split_filename(filename: &str) -> (String, String) {
        if let Some(where_) = filename.rfind(['/', '\\']) {
            (
                filename[..=where_].to_string(),
                filename[where_ + 1..].to_string(),
            )
        } else {
            (format!(".{PATH_SEP}"), filename.to_string())
        }
    }

    /// C++ DiskFile::SplitRelativeFilename
    pub fn split_relative_filename(filename: &str, basepath: &str) -> String {
        filename[basepath.len()..].to_string()
    }
}

impl Default for DiskFile {
    fn default() -> Self {
        DiskFile::new()
    }
}

/// C++ wildcard matching in FindFiles: split at the first '*' or '?'.
fn wildcard_match(wildcard: &str, name: &str) -> bool {
    let wb = wildcard.as_bytes();
    let where_ = match wb.iter().position(|&c| c == b'*' || c == b'?') {
        Some(w) => w,
        None => return name == wildcard,
    };
    let front = &wildcard[..where_];
    let multiple = wb[where_] == b'*';
    let back = &wildcard[where_ + 1..];

    if multiple {
        name.len() >= wildcard.len()
            && name.starts_with(front)
            && name.ends_with(back)
    } else {
        name.len() == wildcard.len()
            && wildcard
                .bytes()
                .zip(name.bytes())
                .all(|(w, n)| w == b'?' || w == n)
    }
}

/// C++ lstat/stat classification in FindFiles: recurse into directories,
/// collect regular files, collect symlinks to files when following links.
fn classify_add(fn_: &str, recursive: bool, followlinks: bool, matches: &mut Vec<String>) {
    let Ok(st) = fs::symlink_metadata(fn_) else {
        return;
    };
    let ft = st.file_type();
    if ft.is_dir() {
        if recursive {
            matches.extend(DiskFile::find_files_follow(fn_, "*", true, followlinks));
        }
    } else if ft.is_file() {
        matches.push(fn_.to_string());
    } else if followlinks && ft.is_symlink() {
        if let Ok(stt) = fs::metadata(fn_) {
            if stt.is_file() {
                matches.push(fn_.to_string());
            }
        }
    }
}

/// C++ DiskFileMap — tracks which files already have a DiskFile, to avoid
/// processing a file twice. The C++ map holds raw pointers; this port maps
/// filename to the index of the caller-owned DiskFile.
pub struct DiskFileMap {
    diskfilemap: HashMap<String, usize>,
}

impl DiskFileMap {
    pub fn new() -> DiskFileMap {
        DiskFileMap {
            diskfilemap: HashMap::new(),
        }
    }

    pub fn insert(&mut self, filename: &str, index: usize) -> bool {
        debug_assert!(!filename.is_empty());
        // C++ only inserts when the filename is not already present.
        if self.diskfilemap.contains_key(filename) {
            return false;
        }
        self.diskfilemap.insert(filename.to_string(), index);
        true
    }

    pub fn remove(&mut self, filename: &str) {
        debug_assert!(!filename.is_empty());
        self.diskfilemap.remove(filename);
    }

    pub fn find(&self, filename: &str) -> Option<usize> {
        debug_assert!(!filename.is_empty());
        self.diskfilemap.get(filename).copied()
    }
}

impl Default for DiskFileMap {
    fn default() -> Self {
        DiskFileMap::new()
    }
}

/// C++ FileSizeCache
pub struct FileSizeCache {
    cache: HashMap<String, u64>,
}

impl FileSizeCache {
    pub fn new() -> FileSizeCache {
        FileSizeCache {
            cache: HashMap::new(),
        }
    }

    pub fn get(&mut self, filename: &str) -> u64 {
        if let Some(&size) = self.cache.get(filename) {
            return size;
        }

        // go to disk
        let filesize = DiskFile::get_file_size(filename);

        self.cache.insert(filename.to_string(), filesize);
        filesize
    }
}

impl Default for FileSizeCache {
    fn default() -> Self {
        FileSizeCache::new()
    }
}

/// Process-wide lock for tests that change the current working directory.
/// Rust runs tests on parallel threads, so anything that depends on the
/// process cwd (canonical pathnames, command line parsing) must hold this
/// lock for the whole duration of its cwd-dependent work.
#[cfg(test)]
pub(crate) static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Each test gets its own scratch directory (the C++ tests share the
    /// current directory; Rust tests run in parallel).
    fn temp_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("par2rust_diskfile_{}_{}_{}", std::process::id(), n, tag));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn join(dir: &Path, name: &str) -> String {
        format!("{}{}{}", dir.display(), PATH_SEP, name)
    }

    /// Deterministic PRNG standing in for C `rand()`.
    struct Rng(u64);
    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
    }

    // test1: static functions
    #[test]
    fn test1_static_functions() {
        let dir = temp_dir("t1");
        let input1_contents = b"diskfile_test test1 input1.txt";
        let input2_contents = b"diskfile_test test1 input2.txt";
        fs::write(join(&dir, "input1.txt"), input1_contents).unwrap();
        fs::write(join(&dir, "input2.txt"), input2_contents).unwrap();

        assert!(!DiskFile::file_exists(&join(&dir, "definitely_not_here")));
        assert!(DiskFile::file_exists(&join(&dir, "input1.txt")));
        assert_eq!(
            DiskFile::get_file_size(&join(&dir, "input1.txt")),
            input1_contents.len() as u64
        );

        let d = dir.display().to_string();
        let files = DiskFile::find_files(&d, "input1.txt", false);
        assert_eq!(files, vec![join(&dir, "input1.txt")]);

        let mut files = DiskFile::find_files(&d, "input?.txt", false);
        files.sort();
        assert_eq!(
            files,
            vec![join(&dir, "input1.txt"), join(&dir, "input2.txt")]
        );

        // Unix semantics: '?' matches exactly one character (upstream differs
        // on Windows, where FindFirstFile treats a trailing '?' as optional).
        let files = DiskFile::find_files(&d, "input1?.txt", false);
        assert!(files.is_empty(), "FindFiles failed on empty ?");

        let mut files = DiskFile::find_files(&d, "input*.txt", false);
        files.sort();
        assert_eq!(
            files,
            vec![join(&dir, "input1.txt"), join(&dir, "input2.txt")]
        );

        // Upstream's crude matcher (and upstream's own Unix behaviour) finds
        // nothing for these; the upstream test disables those assertions
        // with a TODO. This port pins the matcher's actual behaviour.
        let files = DiskFile::find_files(&d, "input1*.txt", false);
        assert!(files.is_empty());
        let files = DiskFile::find_files(&d, "i*p*t*.txt", false);
        assert!(files.is_empty());

        {
            let (path, name) = DiskFile::split_filename("input1.txt");
            assert_eq!(path, format!(".{PATH_SEP}"));
            assert_eq!(name, "input1.txt");

            let (path, name) =
                DiskFile::split_filename(&format!("dir{PATH_SEP}input1.txt"));
            assert_eq!(path, format!("dir{PATH_SEP}"));
            assert_eq!(name, "input1.txt");

            let (path, name) = DiskFile::split_filename(&format!(
                "multiple{PATH_SEP}dirs{PATH_SEP}input1.txt"
            ));
            assert_eq!(path, format!("multiple{PATH_SEP}dirs{PATH_SEP}"));
            assert_eq!(name, "input1.txt");

            let (path, name) = DiskFile::split_filename(&format!(
                "{PATH_SEP}root_dir{PATH_SEP}input1.txt"
            ));
            assert_eq!(path, format!("{PATH_SEP}root_dir{PATH_SEP}"));
            assert_eq!(name, "input1.txt");
        }

        {
            let name = DiskFile::split_relative_filename(
                &format!("root{PATH_SEP}dir{PATH_SEP}input1.txt"),
                &format!("root{PATH_SEP}dir{PATH_SEP}"),
            );
            assert_eq!(name, "input1.txt");

            let name = DiskFile::split_relative_filename(
                &format!("root{PATH_SEP}dir{PATH_SEP}input1.txt"),
                &format!("root{PATH_SEP}"),
            );
            assert_eq!(name, format!("dir{PATH_SEP}input1.txt"));
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    // test1 (canonical pathname part): needs the process current directory,
    // so it is serialised and chdir'd into a scratch directory.
    #[test]
    fn test1_canonical_pathname() {
        let _guard = CWD_LOCK.lock().unwrap();
        let dir = temp_dir("t1c");
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();

        let path_and_name1 = DiskFile::get_canonical_pathname("input1.txt");
        let (path1, name1) = DiskFile::split_filename(&path_and_name1);
        assert_eq!(path1.clone() + &name1, path_and_name1);
        if cfg!(windows) {
            assert_eq!(&path1[1..2], ":");
            assert_eq!(&path1[2..3], PATH_SEP.to_string());
        } else {
            assert_eq!(&path1[0..1], PATH_SEP.to_string());
        }
        assert_eq!(name1, "input1.txt");

        let path_and_name2 = DiskFile::get_canonical_pathname("input2.txt");
        let (path2, name2) = DiskFile::split_filename(&path_and_name2);
        assert_eq!(path2.clone() + &name2, path_and_name2);
        assert_eq!(path2, path1);
        assert_eq!(name2, "input2.txt");

        // check that ././input1.txt is the same as input1.txt
        let path_and_name3 =
            DiskFile::get_canonical_pathname(&format!(".{PATH_SEP}.{PATH_SEP}input1.txt"));
        assert_eq!(path_and_name3, path_and_name1);

        std::env::set_current_dir(old).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    // test2: non-static functions
    #[test]
    fn test2_object_functions() {
        let dir = temp_dir("t2");
        let mut serr = std::io::sink();
        let input1_contents = b"diskfile_test test1 input1.txt";
        fs::write(join(&dir, "input1.txt"), input1_contents).unwrap();

        // read input1.txt
        {
            let mut diskfile = DiskFile::new();
            assert!(!diskfile.is_open());
            assert!(!diskfile.exists());
            assert!(diskfile.open(&join(&dir, "input1.txt"), &mut serr));
            assert!(diskfile.is_open());
            assert!(diskfile.exists());
            assert_eq!(diskfile.file_name(), join(&dir, "input1.txt"));
            assert_eq!(diskfile.file_size(), input1_contents.len() as u64);

            let mut buffer = vec![0u8; input1_contents.len()];
            assert!(diskfile.read(0, &mut buffer, &mut serr));
            assert_eq!(buffer, input1_contents);

            // random reads
            let mut rng = Rng(345087209);
            let len = input1_contents.len() as u64;
            for _ in 0..100 {
                let offset = rng.below(len - 1);
                let length = 1 + rng.below(len - offset - 1);
                assert!(diskfile.read(
                    offset,
                    &mut buffer[offset as usize..(offset + length) as usize],
                    &mut serr
                ));
                assert_eq!(buffer, input1_contents);
            }

            assert!(diskfile.is_open());
            diskfile.close();
            assert!(!diskfile.is_open());

            // reopen!
            assert!(diskfile.reopen(&mut serr));
            assert!(diskfile.is_open());
            diskfile.close();
            assert!(!diskfile.is_open());
        }

        // create input2.txt, move it to input3.txt, delete it.
        {
            let input2_contents = b"diskfile_test test3 input2.txt is longer";
            let mut diskfile = DiskFile::new();
            assert!(!diskfile.is_open());
            assert!(!diskfile.exists());
            assert!(diskfile.create(&join(&dir, "input2.txt"), input2_contents.len() as u64, &mut serr));
            assert!(diskfile.is_open());
            assert!(diskfile.exists());
            assert_eq!(diskfile.file_size(), input2_contents.len() as u64);
            assert_eq!(diskfile.file_name(), join(&dir, "input2.txt"));
            assert!(diskfile.write(0, input2_contents, &mut serr));

            diskfile.close();
            assert!(!diskfile.is_open());

            // Rename from input2.txt to input3.txt
            let input3 = join(&dir, "input3.txt");
            assert!(diskfile.rename_to(&input3, &mut serr));
            assert!(!DiskFile::file_exists(&join(&dir, "input2.txt")));
            assert_eq!(diskfile.file_name(), input3);
            assert!(diskfile.exists());

            assert!(diskfile.delete(&mut serr));
            assert!(!diskfile.exists());
            assert!(!DiskFile::file_exists(&input3));
        }

        // random write + read
        {
            let input2_contents = b"diskfile_test test3 input2.txt is longer";
            let buffer_len = input2_contents.len();
            let mut rng = Rng(23461119);
            let mut blocksize = 1usize;
            while blocksize < buffer_len {
                {
                    let mut diskfile = DiskFile::new();
                    assert!(diskfile.create(&join(&dir, "input2.txt"), buffer_len as u64, &mut serr));

                    let blockcount = buffer_len.div_ceil(blocksize);
                    let mut blockorder: Vec<usize> = (0..blockcount).collect();
                    // shuffle
                    for i in 0..blockcount.saturating_sub(1) {
                        let other_index = 1 + rng.below((blockcount - (i + 1)) as u64) as usize;
                        blockorder.swap(other_index, i);
                    }
                    // write blocks
                    for &block in &blockorder {
                        let offset = blocksize * block;
                        let write_len =
                            if offset + blocksize > buffer_len { buffer_len - offset } else { blocksize };
                        assert!(diskfile.write(
                            offset as u64,
                            &input2_contents[offset..offset + write_len],
                            &mut serr
                        ));
                    }
                    diskfile.close();
                }

                {
                    let mut diskfile = DiskFile::new();
                    assert!(diskfile.open_with_size(&join(&dir, "input2.txt"), buffer_len as u64, &mut serr));
                    let mut buffer = vec![0u8; buffer_len];
                    assert!(diskfile.read(0, &mut buffer, &mut serr));
                    assert_eq!(buffer, input2_contents);
                }

                fs::remove_file(join(&dir, "input2.txt")).unwrap();
                blocksize *= 2;
            }
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    // test3: DiskFileMap
    #[test]
    fn test3_diskfilemap() {
        let dir = temp_dir("t3");
        let mut serr = std::io::sink();
        let input1_contents = b"diskfile_test test3 input1.txt";
        fs::write(join(&dir, "input1.txt"), input1_contents).unwrap();

        let mut dfm = DiskFileMap::new();
        assert!(dfm.find("input1.txt").is_none());

        let mut df1 = DiskFile::new();
        assert!(df1.open(&join(&dir, "input1.txt"), &mut serr));
        assert!(dfm.insert("input1.txt", 0));
        assert_eq!(dfm.find("input1.txt"), Some(0));

        // duplicate insert fails
        assert!(!dfm.insert("input1.txt", 1));
        assert_eq!(dfm.find("input1.txt"), Some(0));

        dfm.remove("input1.txt");
        assert!(dfm.find("input1.txt").is_none());

        fs::remove_dir_all(&dir).unwrap();
    }

    // test4: FileSizeCache
    #[test]
    fn test4_filesizecache() {
        let dir = temp_dir("t4");
        let input1_contents = b"diskfile_test test3 input1.txt";
        let input2_contents = b"diskfile_test test3 input2.txt is longer";
        fs::write(join(&dir, "input1.txt"), input1_contents).unwrap();
        fs::write(join(&dir, "input2.txt"), input2_contents).unwrap();

        let mut cache = FileSizeCache::new();
        for _ in 0..1000 {
            assert_eq!(cache.get(&join(&dir, "input1.txt")), input1_contents.len() as u64);
            assert_eq!(cache.get(&join(&dir, "input2.txt")), input2_contents.len() as u64);
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    // test5: cannot create a file if one already exists
    #[test]
    fn test5_create_existing_fails() {
        let dir = temp_dir("t5");
        let mut serr = std::io::sink();
        let input1_contents = b"diskfile_test test3 input1.txt";
        fs::write(join(&dir, "input1.txt"), input1_contents).unwrap();

        let mut diskfile = DiskFile::new();
        assert!(!diskfile.create(&join(&dir, "input1.txt"), input1_contents.len() as u64, &mut serr));

        fs::remove_dir_all(&dir).unwrap();
    }

    // test6: Read()/Write() where length > maxlength
    #[test]
    fn test6_maxlength() {
        let dir = temp_dir("t6");
        let mut serr = std::io::sink();
        let input1_contents = b"diskfile_test test6 input1.txt";

        {
            let mut diskfile = DiskFile::new();
            assert!(diskfile.create(&join(&dir, "input1.txt"), input1_contents.len() as u64, &mut serr));
            assert!(diskfile.write_max(0, input1_contents, 2, &mut serr));
            diskfile.close();
        }

        {
            let mut diskfile = DiskFile::new();
            assert!(diskfile.open(&join(&dir, "input1.txt"), &mut serr));
            let mut buffer = vec![0u8; input1_contents.len()];
            assert!(diskfile.read_max(0, &mut buffer, 2, &mut serr));
            assert_eq!(buffer, input1_contents);
            diskfile.close();
            fs::remove_file(join(&dir, "input1.txt")).unwrap();
        }

        let input2_contents = b"diskfile_test test6 input2.txt is longer";

        // try again, writing mid-file with different maxlength.
        {
            let mut diskfile = DiskFile::new();
            assert!(diskfile.create(&join(&dir, "input2.txt"), input2_contents.len() as u64, &mut serr));
            let midpoint = input2_contents.len();
            assert!(diskfile.write_max(midpoint as u64, &input2_contents[midpoint..], 3, &mut serr));
            assert!(diskfile.write_max(0, &input2_contents[..midpoint], 4, &mut serr));
            diskfile.close();
        }

        {
            let mut diskfile = DiskFile::new();
            assert!(diskfile.open(&join(&dir, "input2.txt"), &mut serr));
            let mut buffer = vec![0u8; input2_contents.len()];
            let midpoint = input2_contents.len() - 2;
            assert!(diskfile.read_max(midpoint as u64, &mut buffer[midpoint..], 4, &mut serr));
            assert!(diskfile.read_max(0, &mut buffer[..midpoint], 3, &mut serr));
            assert_eq!(buffer, input2_contents);
            diskfile.close();
            fs::remove_file(join(&dir, "input2.txt")).unwrap();
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    // test7: symlink handling in FindFiles (POSIX only, as upstream)
    #[cfg(unix)]
    #[test]
    fn test7_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("t7");
        let input1_contents = b"diskfile_test test7 input1.txt";
        fs::write(join(&dir, "input1.txt"), input1_contents).unwrap();
        symlink(join(&dir, "input1.txt"), join(&dir, "input1_link.txt")).unwrap();
        symlink(join(&dir, "not_there"), join(&dir, "dangling_link.txt")).unwrap();

        let d = dir.display().to_string();

        // by default (followlinks=false) symlinks are not returned
        let files = DiskFile::find_files(&d, "input1_link.txt", false);
        assert!(files.is_empty(), "FindFiles returned symlink when followlinks=false");

        // with followlinks=true a symlink to a file is returned
        let files = DiskFile::find_files_follow(&d, "input1_link.txt", false, true);
        assert_eq!(files, vec![join(&dir, "input1_link.txt")]);

        // dangling symlinks are never returned
        let files = DiskFile::find_files_follow(&d, "dangling_link.txt", false, true);
        assert!(files.is_empty(), "FindFiles returned dangling symlink");

        fs::remove_dir_all(&dir).unwrap();
    }
}
