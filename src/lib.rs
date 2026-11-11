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

//! par2rust - a PAR 2.0 compatible file verification and repair tool,
//! ported 1:1 from par2cmdline (C++) to Rust.

pub mod backends;
pub mod bufferpool;
pub mod commandline;
pub mod crc;
pub mod creatorpacket;
pub mod criticalpacket;
pub mod datablock;
pub mod descriptionpacket;
pub mod diskfile;
pub mod filechecksummer;
pub mod galois;
pub mod hasher;
pub mod letype;
pub mod libpar2;
pub mod mainpacket;
pub mod md5;
pub mod noiselevel;
pub mod par1fileformat;
pub mod par1repairer;
pub mod par1repairersourcefile;
pub mod par2creator;
pub mod par2creatorsourcefile;
pub mod par2fileformat;
pub mod par2repairer;
pub mod par2repairersourcefile;
pub mod processor;
pub mod progressmeter;
pub mod recoverypacket;
pub mod reedsolomon;
pub mod reference_processor;
pub mod taskpool;
pub mod utf8;
pub mod verificationhashtable;
pub mod verificationpacket;
