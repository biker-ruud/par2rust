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

//! Port of the C++ `tests/test37` script: subdir data par2 creation
//! with a file list read from a file.

mod common;

use common::run_dir;

#[test]
fn test37_filelist_from_file_subdir() {
    let run = run_dir("test37");
    run.extract("subdirdata.tar.gz");
    run.copy_data("subdirdata-filelist.txt", "subdirdata-filelist.txt");

    let result = run.par2(&["c", "recovery.par2", "@subdirdata-filelist.txt"]);
    assert!(result.success(), "failed to create parchive from filelist:\n{}", result.out);

    let result = run.par2(&["v", "recovery.par2"]);
    assert!(result.success(), "failed to verify parchive created from filelist:\n{}", result.out);
}
