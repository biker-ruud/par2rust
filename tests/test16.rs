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

//! Port of the C++ `tests/test16` script: files outside the par2 base
//! directory are ignored (issues #34 and #36).

mod common;

use common::run_dir;

#[test]
fn test16_ignore_files_outside_basedir() {
    let run = run_dir("test16");
    run.extract("subdirdata.tar.gz");

    let result = run.par2_in(
        &run.path("subdir1"),
        &[
            "c",
            "-r2",
            "-a",
            "test.par2",
            "test-0.data",
            "test-1.data",
            "test-2.data",
            "test-3.data",
            "test-4.data",
            "..\\subdir2\\test-5.data",
            "..\\subdir2\\test-6.data",
            "..\\subdir2\\test-7.data",
            "..\\subdir2\\test-8.data",
            "..\\subdir2\\test-9.data",
        ],
    );
    assert!(result.success(), "creating files using PAR 2.0 failed");

    let output = format!("{}{}", result.out, result.err);
    assert!(
        output.contains("Ignoring"),
        "there were no files ignored:\n{output}"
    );
}
