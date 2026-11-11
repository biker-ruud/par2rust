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

//! Port of the C++ `tests/test25` script: par2 with a full path.

mod common;

use common::run_dir;

#[test]
fn test25_full_path() {
    let run = run_dir("test25");
    run.write_str("some-file", "file contents\n");

    let full = run.path("some-file").to_string_lossy().into_owned();
    let result = run.par2(&["create", full.as_str()]);
    assert!(result.success(), "Failed to create parchive:\n{}", result.out);

    run.write_str("some-file", "corrupted contents\n");
    let result = run.par2(&["repair", "some-file"]);
    assert!(result.success(), "Failed to repair:\n{}", result.out);
}
