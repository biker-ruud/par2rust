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

//! Port of the C++ `tests/test23` script: par2 can be run from any
//! starting dir, verifying files in a far away folder with `-B`.

mod common;

use common::run_dir;

#[test]
fn test23_run_from_any_starting_dir_verify() {
    let run = run_dir("test23");
    let faraway = run.path("in/a/folder/far/far/away");
    run.create_dir("in/a/folder/far/far/away");
    run.create_dir("rundir");
    run.extract_in("flatdata.tar.gz", &faraway);
    run.extract_in("flatdata-par2files.tar.gz", &faraway);

    let base = faraway.to_string_lossy().into_owned();
    let par2file = faraway.join("testdata.par2").to_string_lossy().into_owned();
    let rundir = run.path("rundir");
    let result = run.par2_in(&rundir, &["v", "-B", base.as_str(), par2file.as_str()]);
    assert!(result.success(), "verify of PAR 2.0 files failed:\n{}", result.out);
}
