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

//! Port of the C++ `tests/test20` script: repair a file that was split
//! into two files.

mod common;

use common::random_bytes;
use common::run_dir;

#[test]
fn test20_repair_split_file() {
    let run = run_dir("test20");
    let data = random_bytes(2000);
    run.write("myfile.dat", &data);

    let result = run.par2(&["c", "-s1000", "-c0", "recovery", "myfile.dat"]);
    assert!(result.success(), "Creating PAR 2.0 data failed");

    // Split the file into its two halves and remove the original.
    run.write("myfile.dat.001", &data[..1000]);
    run.write("myfile.dat.002", &data[1000..]);
    run.remove("myfile.dat");

    // Pass every file in the directory, as the `recovery.par2 ./*` of the
    // scripts does.
    let files = run.glob("");
    let mut args: Vec<&str> = vec!["r", "recovery.par2"];
    args.extend(files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "PAR 2.0 repair failed");
}
