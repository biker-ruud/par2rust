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

//! Port of the C++ `tests/test40` script: take the file list on
//! standard input.

mod common;

use common::run_dir;

#[test]
fn test40_filelist_from_stdin() {
    let run = run_dir("test40");
    run.extract("flatdata.tar.gz");

    let filelist = std::fs::read(common::data_dir().join("flatdata-filelist.txt"))
        .expect("could not read the file list");
    let result = run.par2_stdin(&["c", "recovery.par2", "@"], &filelist);
    assert!(result.success(), "failed to create parchive from stdin filelist:\n{}", result.out);

    let result = run.par2(&["r", "recovery.par2"]);
    assert!(result.success(), "failed to verify parchive created from stdin filelist:\n{}", result.out);
}
