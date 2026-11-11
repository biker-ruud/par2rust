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

//! Port of the C++ `tests/test22` script: par2 can be run from any
//! starting dir, creating files in a far away folder with `-B`.

mod common;

use common::run_dir;

#[test]
fn test22_run_from_any_starting_dir_create() {
    let run = run_dir("test22");
    let faraway = run.path("in/a/folder/far/far/away");
    run.create_dir("in/a/folder/far/far/away");
    run.create_dir("rundir");
    run.extract_in("flatdata.tar.gz", &faraway);

    let mut data_files: Vec<String> = Vec::new();
    for name in run.list_dir("in/a/folder/far/far/away") {
        if name.ends_with(".data") {
            data_files.push(faraway.join(name).to_string_lossy().into_owned());
        }
    }
    let par2file = faraway.join("test.par2").to_string_lossy().into_owned();
    let base = faraway.to_string_lossy().into_owned();

    let rundir = run.path("rundir");
    let bopt = format!("-B{base}");
    let mut args: Vec<&str> = vec!["c", "-r2", bopt.as_str(), par2file.as_str()];
    args.extend(data_files.iter().map(String::as_str));
    let result = run.par2_in(&rundir, &args);
    assert!(result.success(), "create of PAR 2.0 files failed:\n{}", result.out);
}
