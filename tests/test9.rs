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

//! Port of the C++ `tests/test9` script: repair renamed files
//! using PAR 2.0 data.

mod common;

use common::run_dir;

#[test]
fn test9_repair_renamed_files() {
    let run = run_dir("test9");
    run.extract("flatdata.tar.gz");
    run.extract("flatdata-par2files.tar.gz");

    // Rename nine of the ten data files, the way the scripts do.
    let renames = [
        (1, "rename4"),
        (2, "rename5"),
        (3, "rename6"),
        (4, "rename7"),
        (5, "rename9"),
        (6, "rename8"),
        (7, "rename3"),
        (8, "rename1"),
        (9, "rename2"),
    ];
    for (index, target) in renames {
        run.rename(&format!("test-{index}.data"), target);
    }

    // Pass the renamed files along with the par2 file, as the `rename*`
    // expansion does in the scripts.
    let renamed = run.glob("rename");
    let mut args: Vec<&str> = vec!["r", "testdata.par2"];
    args.extend(renamed.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(
        result.success(),
        "Initial PAR 2.0 verification and rename failed"
    );

    for (index, _) in renames {
        assert!(
            run.exists(&format!("test-{index}.data")),
            "rename failed for test-{index}.data"
        );
    }
}
