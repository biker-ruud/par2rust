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

//! Port of the C++ `tests/test42` script: oversized source block counts
//! in a recovery set must be rejected without crashing.

mod common;

use common::run_dir;

#[test]
fn test42_block_count_wrap() {
    let run = run_dir("test42");
    run.extract("block-count-wrap.tar.gz");

    let result = run.par2(&["r", "-q", "recovery.par2"]);
    assert_ne!(result.code, 0, "Repair accepted oversized source block counts");
    assert!(
        result.code <= 128,
        "Repair terminated by signal instead of rejecting malformed PAR2 data"
    );
    assert!(
        result.err.contains("Too many source blocks in recovery set"),
        "Expected oversized block count rejection:\n{}",
        result.err
    );
}
