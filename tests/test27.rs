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

//! Port of the C++ `tests/test27` script: par2 with a full path through
//! a symbolic link to a subfolder. The PowerShell suite of the C++
//! project marks this test as skipped on Windows, so this port does the
//! same.

mod common;

use common::run_dir;

#[test]
fn test27_full_path_through_symlink() {
    if cfg!(windows) {
        eprintln!("test27 is skipped on Windows, as it is in the C++ suite");
        return;
    }
    let run = run_dir("test27");
    run.create_dir("folder/subfolder");
    #[cfg(unix)]
    std::os::unix::fs::symlink("folder/subfolder", run.path("tmp"))
        .expect("could not create a symbolic link");

    let tmp = run.path("tmp");
    std::fs::write(tmp.join("some-file"), b"file contents\n").expect("could not write some-file");
    let full = tmp
        .join("some-file")
        .to_string_lossy()
        .into_owned();
    let result = run.par2_in(&tmp, &["create", full.as_str()]);
    assert!(result.success(), "Failed to create parchive:\n{}", result.out);

    std::fs::write(tmp.join("some-file"), b"corrupted contents\n")
        .expect("could not write some-file");
    let result = run.par2_in(&tmp, &["repair", "some-file"]);
    assert!(result.success(), "Failed to repair:\n{}", result.out);
}
