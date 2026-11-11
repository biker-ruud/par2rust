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

//! Port of the C++ `tests/test4` script: repair two missing files
//! using PAR 2.0 data.

mod common;

use common::run_dir;

#[test]
fn test4_repair_two_missing_files() {
    let run = run_dir("test4");
    run.extract("flatdata.tar.gz");
    run.extract("flatdata-par2files.tar.gz");

    run.copy("test-0.data", "test-0.data.orig");
    run.copy("test-5.data", "test-5.data.orig");
    run.remove("test-0.data");
    run.remove("test-5.data");

    let result = run.par2(&["r", "testdata.par2"]);
    assert!(result.success(), "Reconstruction of two files using PAR 2.0 failed");

    assert!(
        run.files_equal("test-0.data", "test-0.data.orig"),
        "Repaired file test-0.data does not match original"
    );
    assert!(
        run.files_equal("test-5.data", "test-5.data.orig"),
        "Repaired file test-5.data does not match original"
    );
}
