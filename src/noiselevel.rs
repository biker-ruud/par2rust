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

//! Port of the NoiseLevel enum from par2cmdline's libpar2.h.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub enum NoiseLevel {
    /// Unknown (use some basic default)
    Unknown = 0,
    /// Absolutely no output (other than errors)
    Silent,
    /// Bare minimum of output
    Quiet,
    /// Normal level of output
    Normal,
    /// Lots of output
    Noisy,
    /// Extra debugging information
    Debug,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test1_ordering() {
        assert!(NoiseLevel::Silent < NoiseLevel::Quiet);
        assert!(NoiseLevel::Quiet < NoiseLevel::Normal);
        assert!(NoiseLevel::Normal < NoiseLevel::Noisy);
        assert!(NoiseLevel::Noisy < NoiseLevel::Debug);
        assert_eq!(NoiseLevel::Unknown as usize, 0);
    }
}
