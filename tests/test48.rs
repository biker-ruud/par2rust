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

//! Port of the C++ `tests/test48` script: checking the hash of the
//! whole of a file when asked to.

mod common;

use common::run_dir;

#[test]
fn test48_full_hash_mismatch() {
    let run = run_dir("test48");
    run.extract("full-hash-mismatch.tar.gz");

    // Every block of the file matches its verification entry, so the
    // blocks alone say the file is intact.
    let blocks = run.par2(&["v", "-q", "recovery.par2"]);
    assert_eq!(blocks.code, 0, "Expected the file to verify when only its blocks are checked");

    // The whole file hash recorded for it does not match, which only
    // shows up when asked for. The exit code is checked rather than just
    // being non-zero, which an unrecognised option would also give.
    for threads in [None, Some(1), Some(4)] {
        let option = threads.map(|n| format!("-t{n}"));
        let mut args: Vec<&str> = vec!["v", "-q"];
        if let Some(option) = option.as_deref() {
            args.push(option);
        }
        args.extend(["--full-hash", "recovery.par2"]);
        let whole = run.par2(&args);
        assert_eq!(whole.code, 1, "--full-hash{option:?} gave {}, expected 1", whole.code);
    }
}
