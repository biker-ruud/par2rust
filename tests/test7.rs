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

//! Port of the C++ `tests/test7` script: repair two files in subdirs
//! using PAR 2.0 data generated on Windows.

mod common;

use common::run_dir;

#[test]
fn test7_repair_subdir_files_win_paths() {
    let run = run_dir("test7");
    run.extract("subdirdata.tar.gz");
    run.extract("subdirdata-par2files-win.tar.gz");

    run.copy("subdir1\\test-2.data", "subdir1\\test-2.data.orig");
    run.copy("subdir2\\test-7.data", "subdir2\\test-7.data.orig");
    run.remove("subdir1\\test-2.data");
    run.remove("subdir2\\test-7.data");

    let result = run.par2(&["r", "testdata.par2"]);
    assert!(result.success(), "Reconstruction of two files using PAR 2.0 failed");

    assert!(
        run.files_equal("subdir1\\test-2.data", "subdir1\\test-2.data.orig"),
        "Repaired file subdir1/test-2.data does not match original"
    );
    assert!(
        run.files_equal("subdir2\\test-7.data", "subdir2\\test-7.data.orig"),
        "Repaired file subdir2/test-7.data does not match original"
    );
}
