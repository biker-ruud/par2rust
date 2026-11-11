//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2003 Peter Brian Clements
//  Copyright (c) 2019 Michael D. Nahas
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

//! Port of par2cmdline's par2cmdline.cpp (the main function).

use std::io::Write;
use std::sync::{Arc, Mutex};

use par2rust::backends::Backends;
use par2rust::commandline::{CommandLine, Operation, Version};
use par2rust::libpar2;
use par2rust::libpar2::Result;
use par2rust::par1repairer::SharedSout;

fn main() {
    // The C++ wmain receives the command line as wide characters and
    // converts it to UTF-8 itself; on Windows std::env::args() already
    // hands over the arguments decoded from the wide command line.
    let argv: Vec<String> = std::env::args().collect();
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();

    let result = run(&args);

    std::process::exit(result as i32);
}

/// The body of the C++ main. The C++ original also sets the Windows console
/// output code page to UTF-8 here; that is left out of this port.
fn run(argv: &[&str]) -> Result {
    // We only output through the streams handed to the library, mirroring
    // the C++ original which only uses cout and cerr here.
    let sout: SharedSout = Arc::new(Mutex::new(Box::new(std::io::stdout())));
    let mut stdout = std::io::stdout();
    let mut serr = std::io::stderr();

    let mut commandline = CommandLine::new();

    let mut result = Result::InvalidCommandLineArguments;

    if commandline.parse(&mut stdout, &mut serr, argv) {
        // Which operation was selected
        match commandline.operation {
            Operation::Create => {
                // Create recovery data
                result = libpar2::par2create(
                    &sout,
                    &mut serr,
                    commandline.noiselevel,
                    commandline.memorylimit,
                    &commandline.basepath,
                    commandline.nthreads,
                    commandline.filethreads,
                    &commandline.parfilename,
                    &commandline.extrafiles,
                    commandline.blocksize,
                    commandline.firstblock,
                    commandline.recoveryfilescheme,
                    commandline.recoveryfilecount,
                    commandline.recoveryblockcount,
                    Backends::default(),
                );
            }
            Operation::Verify | Operation::Repair => {
                // Verify or Repair damaged files
                match commandline.version {
                    Version::Par1 => {
                        result = libpar2::par1repair(
                            &sout,
                            &mut serr,
                            commandline.noiselevel,
                            commandline.memorylimit,
                            commandline.nthreads,
                            commandline.parfilename.clone(),
                            &commandline.extrafiles,
                            commandline.operation == Operation::Repair,
                            commandline.purgefiles,
                        );
                    }
                    Version::Par2 => {
                        result = libpar2::par2repair(
                            &sout,
                            &mut serr,
                            commandline.noiselevel,
                            commandline.memorylimit,
                            &commandline.basepath,
                            commandline.nthreads,
                            commandline.filethreads,
                            commandline.parfilename.clone(),
                            &commandline.extrafiles,
                            commandline.operation == Operation::Repair,
                            commandline.purgefiles,
                            commandline.renameonly,
                            commandline.skipdata,
                            commandline.skipleaway,
                            commandline.fullhash,
                            Backends::default(),
                        );
                    }
                    Version::Unknown => {}
                }
            }
            Operation::None => {
                result = Result::Success;
            }
        }
    }

    // Flush the output streams before exiting, as returning from main
    // would do in the C++ original.
    let _ = stdout.flush();
    let _ = serr.flush();

    result
}
