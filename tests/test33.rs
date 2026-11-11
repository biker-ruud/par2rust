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

//! Port of the C++ `tests/test33` script: rename-only mode with files
//! that were renamed.

mod common;

use common::run_dir;

#[test]
fn test33_rename_only_mode() {
    let run = run_dir("test33");
    run.extract("flatdata.tar.gz");
    run.extract("flatdata-par2files.tar.gz");

    // Save the original files.
    for i in 0..3 {
        run.copy(&format!("test-{i}.data"), &format!("test-{i}.data.orig"));
    }

    // Rename some files, simulating accidental renaming.
    run.rename("test-0.data", "renamed-file-a.data");
    run.rename("test-1.data", "renamed-file-b.data");
    run.rename("test-2.data", "renamed-file-c.data");

    // Verify fails without the renamed files.
    let result = run.par2(&["v", "testdata.par2"]);
    let output = format!("{}{}", result.out, result.err);
    assert!(
        output.contains("missing"),
        "Verification should report missing files:\n{output}"
    );

    // Repair with rename-only mode, passing the renamed files as extras.
    let result = run.par2(&[
        "r",
        "-O",
        "testdata.par2",
        "renamed-file-a.data",
        "renamed-file-b.data",
        "renamed-file-c.data",
    ]);
    assert!(result.success(), "Repair with rename-only mode failed:\n{}", result.out);

    // The original files are restored with their correct names.
    for i in 0..3 {
        assert!(
            run.files_equal(&format!("test-{i}.data"), &format!("test-{i}.data.orig")),
            "Repaired file test-{i}.data does not match original"
        );
    }
}
