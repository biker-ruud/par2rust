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

//! Port of the C++ `tests/test15` script: repairing the files of the
//! par2-0.6.8 crash data set should succeed (issue #35).

mod common;

use common::run_dir;

#[test]
fn test15_repair_issue35() {
    let run = run_dir("test15");
    run.extract("par2-0.6.8-crash.tar.gz");

    let result = run.par2_in(
        &run.path("par2-0.6.8-crash"),
        &["r", "pack-ea5f7f848340980493ed39f5b7173d956c680e43.par2"],
    );
    assert!(result.success(), "repair files using PAR 2.0 failed");
}
