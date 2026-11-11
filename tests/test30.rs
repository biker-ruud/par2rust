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

//! Port of the C++ `tests/test30` script: issue 128, a 0 byte file in
//! the directory must not upset verify or repair.

mod common;

use common::run_dir;

#[test]
fn test30_zero_byte_file() {
    let run = run_dir("test30");
    run.extract("flatdata.tar.gz");
    run.extract("bug128-parfiles.tar.gz");

    // Check with a 0 byte file present.
    run.write("test-a.data", b"");

    let result = run.par2(&["verify", "recovery.par2"]);
    assert!(result.success(), "0 byte file verify failed:\n{}", result.out);

    run.remove("test-a.data");

    let result = run.par2(&["repair", "recovery.par2"]);
    assert!(result.success(), "0 byte file repair failed:\n{}", result.out);
}
