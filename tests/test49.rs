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

//! Port of the C++ `tests/test49` script: checking the 16k hash of a
//! file, which is checked without being asked for.

mod common;

use common::run_dir;

#[test]
fn test49_hash16k_mismatch() {
    let run = run_dir("test49");
    run.extract("hash16k-mismatch.tar.gz");

    // Every block of the file matches its verification entry, but the
    // 16k hash recorded for it does not. The file is damaged rather than
    // unreadable, so the exit code is checked rather than just being
    // non-zero, which an unrecognised option would also give.
    for threads in [None, Some(1), Some(4)] {
        let option = threads.map(|n| format!("-t{n}"));
        let mut args: Vec<&str> = vec!["v", "-q"];
        if let Some(option) = option.as_deref() {
            args.push(option);
        }
        args.push("recovery.par2");
        let verify = run.par2(&args);
        assert_eq!(verify.code, 1, "verify{option:?} gave {}, expected 1", verify.code);
    }
}
