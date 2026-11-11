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

//! Port of the C++ `tests/test11` script: repair a file whose recorded
//! size is larger than the data the packets cover (read beyond EOF).

mod common;

use common::run_dir;

#[test]
fn test11_read_beyond_eof() {
    let run = run_dir("test11_readbeyondeof");
    run.extract("readbeyondeof.tar.gz");

    // Shorten the data file, the way the scripts use `dd bs=113579 count=1`.
    run.rename("test.data", "test.data-correct");
    run.write("test.data", &run.read("test.data-correct")[..113579]);

    let result = run.par2(&["r", "test.par2"]);
    assert!(result.success(), "repair files using PAR 2.0 failed");
}
