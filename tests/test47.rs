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

//! Port of the C++ `tests/test47` script: reading several files at once
//! finds the same data as reading one at a time.

mod common;

use common::console_lines;
use common::numbered_records;
use common::run_dir;

#[test]
fn test47_multi_file_reading() {
    let run = run_dir("test47");

    // Four data files of 16 blocks of 1024 bytes. Each file holds
    // different data so that a block cannot be matched against the wrong
    // file.
    for n in 1..=4u8 {
        run.write(&format!("data{n}.bin"), &numbered_records(256, 64, b'0' + n));
        run.copy(&format!("data{n}.bin"), &format!("data{n}.bin.orig"));
    }

    let result = run.par2(&[
        "c", "-q", "-s1024", "-c20", "test.par2", "data1.bin", "data2.bin", "data3.bin", "data4.bin",
    ]);
    assert!(result.success(), "Could not create PAR2 files:\n{}", result.out);

    // data1.bin is left alone, so it is matched entirely where it is
    // expected.

    // data2.bin has a gap holding nothing: block 3 is corrupt.
    run.patch("data2.bin", 3072, b"XXXXXXXXXXXXXXXX");

    // data3.bin has a gap holding data: blocks 5 onwards are shifted
    // forward by half a block.
    let mut bytes = run.read("data3.bin");
    let region = bytes[5120..15360].to_vec();
    bytes[5632..15872].copy_from_slice(&region);
    run.write("data3.bin", &bytes);

    // data4.bin is truncated, so it cannot be a perfect match on length
    // alone.
    run.truncate("data4.bin", 12288);

    // Reading one file at a time gives each file every thread for its
    // blocks. Reading four at once shares those threads out between them.
    // Both must find the same data.
    let onefile = run.par2(&["v", "-T1", "test.par2"]);
    let allfiles = run.par2(&["v", "-T4", "test.par2"]);
    assert_eq!(
        onefile.code, allfiles.code,
        "Exit code differed: -T1 gave {} and -T4 gave {}",
        onefile.code, allfiles.code
    );

    let target_lines = |out: &str| -> String {
        let folded = console_lines(out);
        let mut lines: Vec<&str> = folded.lines().filter(|line| line.contains("Target:")).collect();
        lines.sort_unstable();
        lines.join("\n")
    };
    let onefile_blocks = target_lines(&onefile.out);
    let allfiles_blocks = target_lines(&allfiles.out);
    assert_eq!(
        onefile_blocks, allfiles_blocks,
        "Reading several files at once found different data"
    );
    assert!(
        allfiles_blocks.contains("data1.bin\" - found"),
        "Expected data1.bin to be found intact:\n{allfiles_blocks}"
    );
    assert!(
        allfiles_blocks.contains("data2.bin\" - damaged. Found 15 of 16 data blocks"),
        "Expected 15 of 16 blocks in data2.bin:\n{allfiles_blocks}"
    );

    // The repair must still work when the files were read at the same time.
    let repair = run.par2(&["r", "-q", "-T4", "test.par2"]);
    assert_eq!(repair.code, 0, "Repair failed:\n{}", repair.out);
    for n in 1..=4 {
        assert!(
            run.files_equal(&format!("data{n}.bin"), &format!("data{n}.bin.orig")),
            "Repaired data{n}.bin does not match the original"
        );
    }
}
