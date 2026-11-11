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

//! Port of the C++ `tests/test19` script: repairing a file whose front
//! was trimmed needs a skip leeway as large as the trim.

mod common;

use common::random_bytes;
use common::run_dir;

#[test]
fn test19_skip_leeway() {
    let run = run_dir("test19");
    run.write("datafile", &random_bytes(1024));

    let result = run.par2(&["c", "-s200", "-c1", "recovery", "datafile"]);
    assert!(result.success(), "Creating PAR 2.0 data failed");

    // Trim the first 100 bytes off the data file.
    let data = run.read("datafile");
    run.write("datafile", &data[100..]);

    // With a skip leeway of 99 the repair must fail.
    let result = run.par2(&["r", "-N", "-S99", "-vv", "recovery.par2"]);
    assert_ne!(
        result.code, 0,
        "Repair should not be possible with skip leaway set to 99"
    );

    // With a skip leeway of 100 the repair must succeed.
    let result = run.par2(&["r", "-N", "-S100", "-vv", "recovery.par2"]);
    assert_eq!(
        result.code, 0,
        "Repair should be possible with skip leaway set to 100"
    );
}
