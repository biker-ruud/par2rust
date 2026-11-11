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

//! Port of the C++ `tests/test31` script: bug 150, files in the root
//! folder must be included when creating recursively.

mod common;

use common::run_dir;

#[test]
fn test31_root_files_with_recursion() {
    let run = run_dir("test31");
    run.extract("smallsubdirdata.tar.gz");

    // Files before and after the subdirs in the wildcard expansion.
    run.write_str("aaaa-test.data", "file in rootpath\n");
    run.write_str("test-a.data", "another file in rootpath\n");

    // The first name of the sorted expansion becomes the par2 file name.
    let items = run.list_dir(".");
    let mut args: Vec<&str> = vec!["create", "-R"];
    args.extend(items.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "Recursive creation of PAR 2.0 files failed:\n{}", result.out);

    // The first file in the wildcard expansion is used as par filename so
    // it is not covered for repair, hence removing it is fine.
    run.remove("aaaa-test.data");

    let result = run.par2(&["verify", "aaaa-test.data.par2"]);
    assert!(result.success(), "verify of rootpath inclusion failed:\n{}", result.out);
}
