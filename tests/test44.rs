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

//! Port of the C++ `tests/test44` script: par2 create works with more
//! than 2000 input files.

mod common;

use common::run_dir;

#[test]
fn test44_more_than_2000_files() {
    let run = run_dir("test44");
    run.create_dir("manyfiles");

    // Generate 2001 non-empty data files (each 512 bytes), the way the
    // script uses `printf '%0512d'`.
    for i in 0..2001u32 {
        run.write(&format!("manyfiles/file_{i:04}.data"), format!("{i:0512}").as_bytes());
    }

    // Windows limits the command line to 32767 characters, so run from
    // the data directory and pass the files by relative name; the tool
    // canonicalises them to the same full paths the script passes.
    let datadir = run.path("manyfiles");
    let base = datadir.to_string_lossy().into_owned();
    let par2file = datadir.join("test.par2").to_string_lossy().into_owned();
    let bopt = format!("-B{base}");
    let names: Vec<String> = (0..2001u32).map(|i| format!("file_{i:04}.data")).collect();
    let mut args: Vec<&str> = vec!["c", bopt.as_str(), par2file.as_str()];
    args.extend(names.iter().map(String::as_str));
    let result = run.par2_in(&datadir, &args);
    assert!(result.success(), "create with >2000 files failed:\n{}", result.out);
}
