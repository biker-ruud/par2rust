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

//! Port of the C++ `tests/test11` script: create par2 files using 100
//! blocks and repair them while removing files.

mod common;

use common::run_dir;

#[test]
fn test11_100_blocks() {
    let run = run_dir("test11");
    run.extract("100blocks.tar.gz");

    let files = run.glob("");
    let mut args: Vec<&str> = vec!["c", "-b100", "testdata.par2"];
    args.extend(files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "construction of files using PAR 2.0 failed");

    // Repair after removing three files, passing the rest as the
    // `*` expansion does.
    run.remove("file");
    run.remove("file1");
    run.remove("file3");
    let files = run.glob("");
    let mut args: Vec<&str> = vec!["r", "testdata.par2"];
    args.extend(files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "repair of 100 blocks par2files failed");

    // Repair again after removing one more file.
    run.remove("file5");
    let files = run.glob("");
    let mut args: Vec<&str> = vec!["r", "testdata.par2"];
    args.extend(files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "repair of 100 blocks par2files failed");
}
