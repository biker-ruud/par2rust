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

//! Port of the C++ `tests/test34` script: rename-only mode must skip
//! files that are damaged rather than match them.

mod common;

use common::run_dir;

#[test]
fn test34_rename_only_skips_damaged() {
    let run = run_dir("test34");
    run.extract("flatdata.tar.gz");
    run.extract("flatdata-par2files.tar.gz");

    // Save the original file.
    run.copy("test-0.data", "test-0.data.orig");

    // Delete the original and create a damaged version with a new name.
    run.remove("test-0.data");
    run.copy("test-0.data.orig", "renamed-damaged.data");
    run.patch("renamed-damaged.data", 0, &[0u8; 100]);

    // With rename-only mode the damaged file must be skipped, so the
    // repair is either not possible or restores from recovery data.
    let result = run.par2(&["r", "-O", "testdata.par2", "renamed-damaged.data"]);
    if result.code == 0 {
        assert!(
            run.files_equal("test-0.data", "test-0.data.orig"),
            "Rename-only mode should have skipped the damaged file"
        );
    }
}
