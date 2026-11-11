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

//! Port of the C++ `tests/test5rk` script: create 100 percent recovery
//! data with a byte sized recovery setting and a fixed file count.

mod common;

use common::run_dir;

#[test]
fn test5rk_byte_sized_recovery() {
    let run = run_dir("test5rk");
    run.extract("flatdata.tar.gz");

    // About the "magic -rk1302": par2 c -r100 creates PAR2 files of in
    // sum 1302436 bytes, so do we approximately (-rk1302). To also test
    // ISSUE-80 we also use "-n2".
    let data_files: Vec<String> = (0..10).map(|i| format!("test-{i}.data")).collect();
    let mut args: Vec<&str> = vec!["c", "-rk1302", "-b190", "-n2", "-a", "newtest"];
    args.extend(data_files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "Creating PAR 2.0 data failed:\n{}", result.out);

    // Check that -n2 created exactly 2 vol files.
    let vol_files = run.glob("newtest.vol");
    let vol_files: Vec<String> = vol_files
        .into_iter()
        .filter(|name| name.ends_with("par2"))
        .collect();
    assert_eq!(
        vol_files.len(),
        2,
        "File count option -n2 did not work. Expected 2, got {}",
        vol_files.len()
    );

    // Save originals and delete all data files.
    for i in 0..10 {
        run.copy(&format!("test-{i}.data"), &format!("test-{i}.data.orig"));
    }
    for name in run.glob("test-*.data") {
        run.remove(&name);
    }
    assert!(!run.exists("test-0.data"), "File deletion did not work");

    let result = run.par2(&["r", "newtest.par2"]);
    assert!(result.success(), "Full Repair using PAR 2.0 failed:\n{}", result.out);

    for i in 0..10 {
        assert!(
            run.files_equal(&format!("test-{i}.data"), &format!("test-{i}.data.orig")),
            "Repaired file test-{i}.data does not match original"
        );
    }
}
