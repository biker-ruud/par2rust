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

//! Port of the C++ `tests/test21` script: save par files outside of
//! the base directory using `-B`.

mod common;

use common::run_dir;

#[test]
fn test21_parfiles_outside_basedir() {
    let run = run_dir("test21");
    run.create_dir("parfiles");
    run.create_dir("datafiles");

    let datafiles = run.path("datafiles");
    run.extract_in("flatdata.tar.gz", &datafiles);

    let files = run.list_dir("datafiles");
    let mut args: Vec<&str> = vec!["c", "-B", "./", "../parfiles/recovery"];
    args.extend(files.iter().map(String::as_str));
    let result = run.par2_in(&datafiles, &args);
    assert!(result.success(), "Creating PAR 2.0 data failed:\n{}", result.out);

    let result = run.par2_in(&datafiles, &["v", "-B", "./", "../parfiles/recovery.par2"]);
    assert!(result.success(), "Verifying PAR 2.0 data failed:\n{}", result.out);
}
