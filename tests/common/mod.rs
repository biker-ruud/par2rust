//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
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

//! Shared helpers for the integration tests, which are the port of the
//! shell and PowerShell scripts in the C++ `tests` directory. Each test
//! runs the `par2rust` binary in its own scratch directory, the way the
//! scripts run `par2` in a `run<testname>` directory, and the test data
//! archives come from `tests/data`.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The `par2rust` binary built for this test run, the way the scripts use
/// `$PARBINARY`.
pub fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_par2rust")
}

/// The directory holding the test data, the way the scripts use `$TESTDATA`.
pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
}

/// The outcome of one run of the binary.
pub struct RunResult {
    pub code: i32,
    pub out: String,
    pub err: String,
}

impl RunResult {
    pub fn success(&self) -> bool {
        self.code == 0
    }
}

/// A scratch directory a test runs in, made fresh by [`run_dir`].
pub struct Run {
    pub dir: PathBuf,
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Creates a fresh scratch directory for the named test under the build
/// directory, removing any leftovers from an earlier run.
pub fn run_dir(testname: &str) -> Run {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("tests")
        .join(format!("run{testname}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("could not clear the run directory");
    }
    std::fs::create_dir_all(&dir).expect("could not create the run directory");
    Run { dir }
}

impl Run {
    /// Runs the binary with the arguments in the scratch directory and
    /// captures both of its output streams.
    pub fn par2(&self, args: &[&str]) -> RunResult {
        let out = Command::new(binary())
            .args(args)
            .current_dir(&self.dir)
            .output()
            .expect("could not run the par2rust binary");
        run_result(&out)
    }

    /// Runs the binary with the arguments in another directory, for the
    /// tests that change into a subdirectory of the scratch directory.
    pub fn par2_in(&self, dir: &Path, args: &[&str]) -> RunResult {
        let out = Command::new(binary())
            .args(args)
            .current_dir(dir)
            .output()
            .expect("could not run the par2rust binary");
        run_result(&out)
    }

    /// Runs the binary feeding the bytes to its standard input, for the
    /// `@` file list read from stdin.
    pub fn par2_stdin(&self, args: &[&str], stdin: &[u8]) -> RunResult {
        let mut child = Command::new(binary())
            .args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("could not run the par2rust binary");
        child
            .stdin
            .as_mut()
            .expect("stdin was not piped")
            .write_all(stdin)
            .expect("could not write to stdin");
        let out = child.wait_with_output().expect("could not run the binary");
        run_result(&out)
    }

    /// Runs the system `tar` extracting the archive in the scratch directory,
    /// the way the scripts use `tar -xzf`.
    pub fn extract(&self, archive: &str) {
        self.extract_in(archive, &self.dir);
    }

    /// Extracts the archive in another directory, for the tests that
    /// unpack the data files into a subdirectory of the scratch directory.
    pub fn extract_in(&self, archive: &str, dest: &Path) {
        let archive = data_dir().join(archive);
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(dest)
            .status()
            .expect("could not run tar");
        assert!(status.success(), "could not extract {}", archive.display());
    }

    /// The path of a file in the scratch directory.
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn exists(&self, name: &str) -> bool {
        self.path(name).exists()
    }

    pub fn write(&self, name: &str, data: &[u8]) {
        std::fs::write(self.path(name), data).expect("could not write a file");
    }

    pub fn write_str(&self, name: &str, data: &str) {
        self.write(name, data.as_bytes());
    }

    pub fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.path(name)).expect("could not read a file")
    }

    pub fn read_str(&self, name: &str) -> String {
        String::from_utf8_lossy(&self.read(name)).into_owned()
    }

    pub fn copy(&self, from: &str, to: &str) {
        std::fs::copy(self.path(from), self.path(to))
            .expect("could not copy a file");
    }

    /// Copies a file from the test data directory into the scratch
    /// directory, the way the scripts use `cp "$TESTDATA/x" ./`.
    pub fn copy_data(&self, name: &str, dest: &str) {
        std::fs::copy(data_dir().join(name), self.path(dest))
            .expect("could not copy a data file");
    }

    pub fn remove(&self, name: &str) {
        let path = self.path(name);
        if path.is_dir() {
            std::fs::remove_dir_all(path).expect("could not remove a directory");
        } else if path.exists() {
            std::fs::remove_file(path).expect("could not remove a file");
        }
    }

    pub fn rename(&self, from: &str, to: &str) {
        std::fs::rename(self.path(from), self.path(to)).expect("could not rename a file");
    }

    pub fn create_dir(&self, name: &str) {
        std::fs::create_dir_all(self.path(name)).expect("could not create a directory");
    }

    /// The names of the entries in a directory of the scratch directory,
    /// sorted, standing in for the `*` expansion of the scripts.
    pub fn list_dir(&self, name: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.path(name))
            .expect("could not read a directory")
            .map(|entry| entry.expect("could not read a directory entry").file_name())
            .filter_map(|name| name.into_string().ok())
            .collect();
        names.sort();
        names
    }

    pub fn metadata_len(&self, name: &str) -> u64 {
        std::fs::metadata(self.path(name))
            .expect("could not stat a file")
            .len()
    }

    /// Truncates a file to the length, the way the scripts use
    /// `dd count=1` to cut a file short.
    pub fn truncate(&self, name: &str, len: u64) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(self.path(name))
            .expect("could not open a file to truncate");
        file.set_len(len).expect("could not truncate a file");
    }

    /// Keeps the last `len` bytes of a file, the way the scripts use
    /// `tail -c` to cut the front off a file.
    pub fn keep_tail(&self, name: &str, len: u64) {
        let mut data = self.read(name);
        let keep = len as usize;
        if keep < data.len() {
            data.drain(0..data.len() - keep);
        }
        self.write(name, &data);
    }

    /// Overwrites bytes part way into a file, the way the scripts use
    /// `dd conv=notrunc` to corrupt a file.
    pub fn patch(&self, name: &str, offset: u64, bytes: &[u8]) {
        use std::io::Seek;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(self.path(name))
            .expect("could not open a file to patch");
        file.seek(std::io::SeekFrom::Start(offset))
            .expect("could not seek in a file");
        file.write_all(bytes).expect("could not write to a file");
    }

    /// True when both files exist and hold the same bytes, the way the
    /// scripts use `cmp -s`.
    pub fn files_equal(&self, one: &str, other: &str) -> bool {
        let one = std::fs::read(self.path(one));
        let other = std::fs::read(self.path(other));
        match (one, other) {
            (Ok(one), Ok(other)) => one == other,
            _ => false,
        }
    }

    /// The names in the scratch directory matching a `*` pattern, sorted,
    /// the way the shell expands `test-*.data`. Only `*` and a literal
    /// prefix and suffix are understood, which is all the scripts use.
    /// A pattern without `*` lists every name holding it as a prefix.
    pub fn glob(&self, pattern: &str) -> Vec<String> {
        let (prefix, suffix) = match pattern.split_once('*') {
            Some((prefix, suffix)) => (prefix, suffix),
            None => (pattern, ""),
        };
        let mut names: Vec<String> = std::fs::read_dir(&self.dir)
            .expect("could not read the run directory")
            .map(|entry| entry.expect("could not read a directory entry").file_name())
            .filter_map(|name| name.into_string().ok())
            .filter(|name| {
                name.len() >= prefix.len() + suffix.len()
                    && name.starts_with(prefix)
                    && (suffix.is_empty() || name.ends_with(suffix))
            })
            .collect();
        names.sort();
        names
    }
}

/// A file of `length` pseudo random bytes, standing in for the
/// `/dev/urandom` the scripts read from. The bytes only need to be
/// different from each other, so a simple counter stream will do.
pub fn random_bytes(length: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(length);
    let mut state: u32 = 0x1234_5678;
    for _ in 0..length {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        data.push((state >> 16) as u8);
    }
    data
}

/// A file of `count` records of `record` bytes each, holding the record
/// number written as decimal digits, the way the scripts build data files
/// with `printf`.
pub fn counted_bytes(count: usize, record: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(count * record);
    for i in 0..count {
        let text = i.to_string();
        let zeros = record - text.len().min(record);
        data.extend(std::iter::repeat(b'0').take(zeros));
        data.extend(text.as_bytes());
    }
    data
}

/// A file of `count` records each holding a digit followed by the zero
/// padded record number, the way the scripts build the data files of the
/// parallel reading tests with `printf '%s%063d'`.
pub fn numbered_records(count: usize, record: usize, digit: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(count * record);
    for i in 0..count {
        data.push(digit);
        let text = i.to_string();
        let zeros = record - 1 - text.len().min(record - 1);
        data.extend(std::iter::repeat(b'0').take(zeros));
        data.extend(text.as_bytes());
    }
    data
}

/// Reduces the raw standard output of a run to the lines a terminal would
/// show, the way the scripts fold the `\r` rewritten progress line away.
pub fn console_lines(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut lines = String::new();
    for line in text.split('\n') {
        let line = match line.rfind('\r') {
            Some(at) => &line[at + 1..],
            None => line,
        };
        lines.push_str(line);
        lines.push('\n');
    }
    lines
}

fn run_result(out: &Output) -> RunResult {
    RunResult {
        code: out.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&out.stdout).into_owned(),
        err: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}
