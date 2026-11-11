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

//! Port of the C++ `tests/test18` script: create PAR 2.0 recovery data
//! for an archive and repair it after truncation.

mod common;

use common::run_dir;

#[test]
fn test18_create_and_repair_truncated_archive() {
    let run = run_dir("test18");
    let archive = common::data_dir().join("flatdata.tar.gz");
    std::fs::copy(&archive, run.path("flatdata.tar.gz")).expect("could not copy the archive");

    let result = run.par2(&["c", "recovery", "flatdata.tar.gz"]);
    assert!(result.success(), "Creating PAR 2.0 data failed");

    // "Truncate" the archive, the way the scripts use `dd bs=1983
    // count=1`, which copies the whole file when it is that small.
    run.rename("flatdata.tar.gz", "flatdata.tar.gz-orig");
    let data = run.read("flatdata.tar.gz-orig");
    let keep = data.len().min(1983);
    run.write("flatdata.tar.gz", &data[..keep]);
    run.remove("flatdata.tar.gz-orig");

    // Pass every file in the directory, as the `recovery.par2 ./*` of the
    // scripts does.
    let files = run.glob("");
    let mut args: Vec<&str> = vec!["r", "recovery.par2"];
    args.extend(files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "PAR 2.0 repair failed");
}
