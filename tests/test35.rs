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

//! Port of the C++ `tests/test35` script: verify symbolic link handling.
//! The PowerShell suite of the C++ project marks this test as skipped on
//! Windows, so this port does the same.

mod common;

#[test]
fn test35_symlink_handling() {
    #[cfg(unix)]
    symlink_handling();
    #[cfg(not(unix))]
    eprintln!("test35 is skipped on Windows, as it is in the C++ suite");
}

#[cfg(unix)]
fn symlink_handling() {
    use std::os::unix::fs::symlink;

    use common::run_dir;

    let run = run_dir("test35");
    run.write_str("test_file", "01234567890abcdef");
    symlink(run.path("test_file"), run.path("test_link")).unwrap();
    run.create_dir("test_dir");
    symlink(run.path("test_file"), run.path("test_dir/test_link")).unwrap();
    symlink(run.path("test_dir"), run.path("dir_link")).unwrap();

    // Links are not followed for non primary files.
    let result = run.par2(&["c", "test_primary", "test_link"]);
    assert_ne!(result.code, 0, "followed non-primary link");
    let result = run.par2(&["c", "test_primary", "-R", "test_dir"]);
    assert_ne!(result.code, 0, "followed link in directory");
    let result = run.par2(&["c", "test_primary", "-R", "dir_link"]);
    assert_ne!(result.code, 0, "followed linked directory");

    // A primary file being a link calculates data for the real file.
    run.copy("test_file", "test_file.bak");
    let result = run.par2(&["c", "-r10", "test_link"]);
    assert!(result.success(), "calculating link data failed:\n{}", result.out);
    let result = run.par2(&["v", "test_link"]);
    assert!(result.success(), "verifying good link data failed:\n{}", result.out);

    // Damage the real file.
    let mut data = run.read("test_file");
    data.push(b'!');
    run.write("test_file", &data);
    let result = run.par2(&["v", "test_link"]);
    assert_ne!(result.code, 0, "verifying damaged link data succeeded");

    // Repair via the link, which becomes the repaired file.
    let result = run.par2(&["r", "test_link"]);
    assert!(result.success(), "repairing linked data failed:\n{}", result.out);
    assert!(
        run.files_equal("test_link", "test_file.bak"),
        "comparison after repairing linked data failed"
    );

    // Dead links fail.
    symlink("NOT_EXISTING", run.path("dead_link")).unwrap();
    let result = run.par2(&["c", "dead_link"]);
    assert_ne!(result.code, 0, "calculating dead link data succeeded");
    let result = run.par2(&["v", "dead_link"]);
    assert_ne!(result.code, 0, "verifying dead link data succeeded");
    let result = run.par2(&["r", "dead_link"]);
    assert_ne!(result.code, 0, "repairing dead link data succeeded");
}
