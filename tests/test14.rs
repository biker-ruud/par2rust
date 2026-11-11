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

//! Port of the C++ `tests/test14` script: repair a file where one byte
//! got removed at the beginning.

mod common;

use common::run_dir;

#[test]
fn test14_repair_byte_removed_at_beginning() {
    let run = run_dir("test14");
    run.extract("flatdata.tar.gz");

    let data_files: Vec<String> = (0..10).map(|i| format!("test-{i}.data")).collect();
    let mut args: Vec<&str> = vec!["c", "testdata.par2"];
    args.extend(data_files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "creating repair files using PAR 2.0 failed");

    // Cut the first byte off test-1.data, the way the scripts use
    // `tail -c 177173`.
    run.rename("test-1.data", "test-1.data-correct");
    run.write("test-1.data", &run.read("test-1.data-correct")[1..]);

    let result = run.par2(&["r", "testdata.par2"]);
    assert!(result.success(), "repair files using PAR 2.0 failed");
}
