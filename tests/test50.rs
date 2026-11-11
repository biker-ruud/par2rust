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

//! Port of the C++ `tests/test50` script: what the tool prints is what
//! was recorded in `test50.expected`.

mod common;

use common::numbered_records;
use common::run_dir;
use common::RunResult;

/// Record what a run prints, so that a change to it has to be made
/// deliberately. The progress indicator rewrites its line with `\r`, so
/// each line is reduced to what was left on it, which is what a console
/// shows.
fn add_run(actual: &mut String, args: &[&str], result: &RunResult) {
    actual.push_str(&format!("$ par2 {}\n", args.join(" ")));
    actual.push_str(&format!("exit {}\n", result.code));

    let mut text = result.out.replace("\r\n", "\n");
    if text.ends_with('\n') {
        text.pop();
    }
    if !text.is_empty() {
        for line in text.split('\n') {
            let folded = match line.rfind('\r') {
                Some(at) => &line[at + 1..],
                None => line,
            };
            actual.push_str(folded);
            actual.push('\n');
        }
    }
    actual.push('\n');
}

#[test]
fn test50_recorded_output() {
    let run = run_dir("test50");
    let mut actual = String::new();

    // Three data files of four blocks of 1024 bytes. Each file holds
    // different data so that a block cannot be matched against the wrong
    // file.
    for n in 1..=3u8 {
        run.write(&format!("data{n}.bin"), &numbered_records(64, 64, b'0' + n));
    }

    let args = ["c", "-T1", "-s1024", "-c4", "-n1", "test.par2", "data1.bin", "data2.bin", "data3.bin"];
    let result = run.par2(&args);
    add_run(&mut actual, &args, &result);

    let args = ["v", "-T1", "test.par2"];
    let result = run.par2(&args);
    add_run(&mut actual, &args, &result);

    // One block of data2.bin is corrupt, which the recovery blocks can
    // replace.
    run.patch("data2.bin", 2048, b"XXXXXXXXXXXXXXXX");

    let args = ["v", "-T1", "test.par2"];
    let result = run.par2(&args);
    add_run(&mut actual, &args, &result);

    let args = ["r", "-T1", "test.par2"];
    let result = run.par2(&args);
    add_run(&mut actual, &args, &result);

    // More blocks are lost than there are recovery blocks to replace
    // them.
    run.remove("data3.bin");
    for off in [0u64, 1024, 2048, 3072] {
        run.patch("data1.bin", off, b"XXXXXXXXXXXXXXXX");
    }

    let args = ["v", "-T1", "test.par2"];
    let result = run.par2(&args);
    add_run(&mut actual, &args, &result);

    let recorded = std::fs::read_to_string(common::data_dir().join("test50.expected"))
        .expect("could not read the recorded output")
        .replace("\r\n", "\n");

    if actual != recorded {
        // Show the first difference to make a failure readable.
        let first_difference = actual
            .lines()
            .zip(recorded.lines())
            .position(|(got, want)| got != want)
            .unwrap_or_else(|| actual.lines().count().min(recorded.lines().count()));
        panic!(
            "The tool printed something other than what was recorded, first difference at line {}:\n\
             got:  {:?}\n\
             want: {:?}",
            first_difference + 1,
            actual.lines().nth(first_difference).unwrap_or("<eof>"),
            recorded.lines().nth(first_difference).unwrap_or("<eof>"),
        );
    }
}
