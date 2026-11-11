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

//! Port of the C++ `tests/test28` script: ensure the silent noise level
//! of `-qq` keeps standard output empty.

mod common;

use common::run_dir;

#[test]
fn test28_silent_noise_level() {
    let run = run_dir("test28");
    run.extract("flatdata.tar.gz");

    let data_files = run.glob("test-*.data");

    let mut args: Vec<&str> = vec!["c", "-a", "newtest", "-qq"];
    args.extend(data_files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "create failed:\n{}", result.out);
    assert!(result.out.is_empty(), "create with -qq produced output to stdout");

    let mut args: Vec<&str> = vec!["v", "-qq", "newtest"];
    args.extend(data_files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "verify failed:\n{}", result.out);
    assert!(result.out.is_empty(), "verify with -qq produced output to stdout");

    let mut args: Vec<&str> = vec!["r", "-qq", "newtest"];
    args.extend(data_files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert!(result.success(), "repair failed:\n{}", result.out);
    assert!(result.out.is_empty(), "repair with -qq produced output to stdout");

    // Creating again must fail because the par2 files already exist,
    // with the complaint on standard error only.
    let mut args: Vec<&str> = vec!["c", "-a", "newtest", "-qq"];
    args.extend(data_files.iter().map(String::as_str));
    let result = run.par2(&args);
    assert_ne!(result.code, 0, "second create succeeded but shouldn't have");
    assert!(result.out.is_empty(), "second create with -qq produced output to stdout");
    assert!(!result.err.is_empty(), "second create with -qq did not produce output to stderr");
}
