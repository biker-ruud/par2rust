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

//! Port of the C++ `tests/test5` script: create and repair 100% loss
//! using PAR 2.0 data.

mod common;

use common::run_dir;

#[test]
fn test5_create_and_repair_100_percent_loss() {
    let run = run_dir("test5");
    run.extract("flatdata.tar.gz");

    let data_files: Vec<String> = (0..10).map(|i| format!("test-{i}.data")).collect();
    let mut args: Vec<&str> = vec!["c", "-r100", "-b190", "-a", "newtest"];
    args.extend(data_files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "Creating PAR 2.0 data failed");

    for i in 0..10 {
        run.copy(&format!("test-{i}.data"), &format!("test-{i}.data.orig"));
    }
    for name in run.glob("test-*.data") {
        run.remove(&name);
    }

    let result = run.par2(&["r", "newtest.par2"]);
    assert!(result.success(), "Full Repair using PAR 2.0 failed");

    for i in 0..10 {
        assert!(
            run.files_equal(&format!("test-{i}.data"), &format!("test-{i}.data.orig")),
            "Repaired file test-{i}.data does not match original"
        );
    }
}
