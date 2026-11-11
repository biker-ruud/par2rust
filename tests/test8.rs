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

//! Port of the C++ `tests/test8` script: create PAR 2.0 data for files
//! in subdirs, then verify after moving the folder.

mod common;

use common::run_dir;

#[test]
fn test8_create_in_subdirs_verify_after_move() {
    let run = run_dir("test8");
    run.extract("subdirdata.tar.gz");

    // Move the subdirs into a source1 folder, the way the scripts do.
    run.create_dir("source1");
    std::fs::rename(run.path("subdir1"), run.path("source1\\subdir1")).expect("could not move subdir1");
    std::fs::rename(run.path("subdir2"), run.path("source1\\subdir2")).expect("could not move subdir2");

    // Create the par2 files from within source1 with the subdirs as arguments.
    let items = run.list_dir("source1");
    let mut args: Vec<&str> = vec!["c", "-R", "testdata.par2"];
    args.extend(items.iter().map(String::as_str));
    let result = run.par2_in(&run.path("source1"), &args);
    assert!(result.success(), "Construction of files using PAR 2.0 failed");

    // Move the source folder and verify from within it.
    std::fs::rename(run.path("source1"), run.path("source2")).expect("could not move source1");
    let result = run.par2_in(&run.path("source2"), &["v", "testdata.par2"]);
    assert!(
        result.success(),
        "Verification of files using PAR 2.0 failed after moving the folder:\n{}",
        result.out
    );
}
