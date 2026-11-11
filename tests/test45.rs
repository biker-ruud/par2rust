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

//! Port of the C++ `tests/test45` script: scanning the blocks in
//! parallel finds the same data as a single thread does.

mod common;

use common::console_lines;
use common::counted_bytes;
use common::run_dir;

#[test]
fn test45_gap_search_threaded() {
    let run = run_dir("test45");

    // A data file of 64 blocks of 1024 bytes.
    run.write("data.bin", &counted_bytes(1024, 64));
    let result = run.par2(&["c", "-q", "-s1024", "-c20", "test.par2", "data.bin"]);
    assert!(result.success(), "Could not create PAR2 files:\n{}", result.out);

    run.copy("data.bin", "data.bin.orig");

    // A gap holding nothing: block 3 is corrupt.
    run.patch("data.bin", 3072, b"XXXXXXXXXXXXXXXX");

    // A gap holding data: shift blocks 10 to 19 forward by half a block,
    // so that they are still in the file but not where they are expected.
    let mut bytes = run.read("data.bin");
    let region = bytes[10240..20992].to_vec();
    bytes[10752..21504].copy_from_slice(&region);
    run.write("data.bin", &bytes);

    // A single thread leaves the whole file to the byte at a time search.
    // Several threads check each block where it is expected first. Both
    // must find the same data.
    let sequential = run.par2(&["v", "-t1", "test.par2"]);
    let parallel = run.par2(&["v", "-t4", "test.par2"]);
    assert_eq!(
        sequential.code, parallel.code,
        "Exit code differed: -t1 gave {} and -t4 gave {}",
        sequential.code, parallel.code
    );

    let block_lines = |out: &str| -> String {
        console_lines(out)
            .lines()
            .filter(|line| line.contains("data blocks"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let sequential_blocks = block_lines(&sequential.out);
    let parallel_blocks = block_lines(&parallel.out);
    assert_eq!(
        sequential_blocks, parallel_blocks,
        "Scanning the blocks in parallel found different data"
    );
    assert!(
        parallel_blocks.contains("Found 62 of 64 data blocks"),
        "Expected 62 of 64 blocks to be found:\n{parallel_blocks}"
    );

    // The repair must still work when the blocks were scanned in parallel.
    let repair = run.par2(&["r", "-q", "-t4", "test.par2"]);
    assert_eq!(repair.code, 0, "Repair failed:\n{}", repair.out);
    assert!(
        run.files_equal("data.bin", "data.bin.orig"),
        "Repaired file does not match the original"
    );
}
