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

//! Port of the C++ `tests/test29` script: issue 190, repairing a single
//! bit flip in a 9 MB file.

mod common;

use common::run_dir;

#[test]
fn test29_bitflip_repair() {
    let run = run_dir("test29");
    run.extract("bug190.tar.gz");

    // Generate the par2 files from the original good file.
    run.copy("9MBones_crc_ok_orig", "9MBones_crc_ok");
    let result = run.par2(&["c", "-m500", "-r30", "-n1", "-v", "9MBones_crc_ok"]);
    assert!(result.success(), "create failed:\n{}", result.out);

    // Replace it with the bit flipped bad copy.
    run.copy("9MBones_crc_ok_bad", "9MBones_crc_ok");
    let result = run.par2(&["repair", "9MBones_crc_ok.par2"]);
    assert!(result.success(), "bitflip repair failed:\n{}", result.out);
}
