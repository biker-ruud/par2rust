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

//! Port of the C++ `tests/test32` script: bug 205, par2 files must be
//! created even when the target folder already exists.

mod common;

use common::run_dir;

#[test]
fn test32_create_into_existing_folder() {
    let run = run_dir("test32");
    run.extract("subdirdata.tar.gz");
    run.create_dir("par2");

    let items = run.list_dir(".");
    let mut args: Vec<&str> = vec!["create", "-vv", "-a", "par2/disk1.par2", "-n31", "-R", "-v", "-B./"];
    args.extend(items.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "Recursive creation of PAR 2.0 files failed:\n{}", result.out);

    let result = run.par2(&["verify", "-B./", "par2/disk1.par2"]);
    assert!(result.success(), "verify failed:\n{}", result.out);
}
