//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See https://parchive.sourceforge.net for details of PAR 2.0.
//
//  Ported 1:1 from par2cmdline src/utf8.h / src/utf8.cpp.
//
//  Copyright (c) 2024-2025 Denis <denis@nzbget.com>
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

//! UTF-8 <-> wide (UTF-16) path conversion, Windows only.
//!
//! False if the string is not well formed, leaving out untouched. Otherwise out
//! holds the conversion, and a path longer than [`MAX_DIR_PATH`] has gained a
//! `\\?\` or `\\?\UNC` prefix so that the Win32 calls accept it. A string that
//! is not valid UTF-8 is decoded with the ANSI code page instead.

#![cfg(windows)]

use std::ffi::CString;

pub const MAX_ARGS: i32 = 128;
pub const MAX_DIR_PATH: usize = 248;

/// The wide string counterpart of C++ `std::wstring`: a sequence of UTF-16
/// code units.
pub type WString = Vec<u16>;

const CP_ACP: u32 = 0;
const CP_UTF8: u32 = 65001;
const MB_ERR_INVALID_CHARS: u32 = 0x0000_0008;
const WC_ERR_INVALID_CHARS: u32 = 0x0000_0080;

#[allow(non_snake_case)]
extern "system" {
    fn MultiByteToWideChar(
        code_page: u32,
        dw_flags: u32,
        lp_multi_bs_str: *const u8,
        cb_multi_bs_str: i32,
        lp_wide_char_str: *mut u16,
        c_wide_char_str: i32,
    ) -> i32;
    fn WideCharToMultiByte(
        code_page: u32,
        dw_flags: u32,
        lp_wide_char_str: *const u16,
        c_wide_char_str: i32,
        lp_multi_bs_str: *mut u8,
        cb_multi_bs_str: i32,
        lp_default_char: *const u8,
        lp_used_default_char: *mut i32,
    ) -> i32;
}

fn apply_long_path_prefix(wpath: &mut WString) {
    const PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const UNC: &[u16] = &[b'\\' as u16, b'\\' as u16];

    if wpath.len() <= MAX_DIR_PATH || wpath.windows(PREFIX.len()).any(|w| w == PREFIX) {
        return;
    }

    if wpath.starts_with(UNC) {
        let mut out = wide(r"\\?\UNC");
        out.extend_from_slice(&wpath[1..]);
        *wpath = out;
    } else {
        let mut out = wide(r"\\?\");
        out.extend_from_slice(wpath);
        *wpath = out;
    }
}

fn decode(codepage: u32, bytes: &[u8], out: &mut WString) -> bool {
    let length = bytes.len() as i32;
    let required = unsafe {
        MultiByteToWideChar(
            codepage,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            length,
            std::ptr::null_mut(),
            0,
        )
    };
    if required <= 0 {
        return false;
    }

    let mut wide = vec![0u16; required as usize];
    let done = unsafe {
        MultiByteToWideChar(
            codepage,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            length,
            wide.as_mut_ptr(),
            required,
        )
    };
    if done <= 0 {
        return false;
    }

    *out = wide;
    true
}

pub fn utf8_to_wide(bytes: &[u8], out: &mut WString) -> bool {
    if bytes.is_empty() {
        out.clear();
        return true;
    }

    let mut wpath: WString = Vec::new();
    if !decode(CP_UTF8, bytes, &mut wpath) && !decode(CP_ACP, bytes, &mut wpath) {
        return false;
    }

    apply_long_path_prefix(&mut wpath);

    *out = wpath;
    true
}

pub fn wide_to_utf8(w: &[u16], out: &mut String) -> bool {
    if w.is_empty() {
        out.clear();
        return true;
    }

    let length = w.len() as i32;
    let required = unsafe {
        WideCharToMultiByte(
            CP_UTF8,
            WC_ERR_INVALID_CHARS,
            w.as_ptr(),
            length,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
        )
    };
    if required <= 0 {
        return false;
    }

    let mut utf8 = vec![0u8; required as usize];
    let done = unsafe {
        WideCharToMultiByte(
            CP_UTF8,
            WC_ERR_INVALID_CHARS,
            w.as_ptr(),
            length,
            utf8.as_mut_ptr(),
            required,
            std::ptr::null(),
            std::ptr::null_mut(),
        )
    };
    if done <= 0 {
        return false;
    }

    match String::from_utf8(utf8) {
        Ok(s) => {
            *out = s;
            true
        }
        Err(_) => false,
    }
}

/// Converts the wide command line arguments to UTF-8, dropping the ones that
/// cannot be converted.
pub struct WideToUtf8ArgsAdapter {
    argv: Vec<CString>,
    argc: i32,
}

impl WideToUtf8ArgsAdapter {
    pub fn new(wargv: &[Option<&[u16]>]) -> WideToUtf8ArgsAdapter {
        let mut argc = wargv.len() as i32;

        if argc > MAX_ARGS {
            eprintln!(
                "Too many arguments ({}/{}).\nOnly {} will be processed.",
                wargv.len(),
                MAX_ARGS,
                MAX_ARGS
            );

            argc = MAX_ARGS;
        }

        let mut argv: Vec<CString> = Vec::new();
        for (i, &arg) in wargv.iter().enumerate().take(argc as usize) {
            let Some(w) = arg else {
                eprintln!(
                    "Invalid argument: encountered nullptr in wargv.\nSkipping argument {i}."
                );
                continue;
            };

            let mut arg_str = String::new();
            if !wide_to_utf8(w, &mut arg_str) {
                eprintln!(
                    "Failed to convert wide to UTF-8 string.\nSkipping argument {i}."
                );
                continue;
            }

            // A C string ends at the first NUL, just like the memcpy'd buffer
            // of the C++ version.
            let cstr = match CString::new(arg_str) {
                Ok(cstr) => cstr,
                Err(err) => {
                    let position = err.nul_position();
                    let bytes = err.into_vec();
                    CString::new(&bytes[..position]).unwrap()
                }
            };
            argv.push(cstr);
        }

        WideToUtf8ArgsAdapter {
            argc: argv.len() as i32,
            argv,
        }
    }

    pub fn get_utf8_args(&self) -> &[CString] {
        &self.argv
    }

    /// The number of arguments in [`WideToUtf8ArgsAdapter::get_utf8_args`],
    /// which is less than the number passed in when an argument could not be
    /// used.
    pub fn get_argc(&self) -> i32 {
        self.argc
    }
}

fn wide(s: &str) -> WString {
    s.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WC_NO_BEST_FIT_CHARS: u32 = 0x0000_0400;

    fn w(s: &str) -> WString {
        wide(s)
    }

    #[test]
    fn test1_empty() {
        let empty_string: &[u8] = b"";
        let expected_wide = w("");
        let mut actual_wide = WString::new();
        assert!(utf8_to_wide(empty_string, &mut actual_wide));

        let expected_utf8 = String::new();
        let mut actual_utf8 = String::new();
        assert!(wide_to_utf8(&expected_wide, &mut actual_utf8));

        assert_eq!(actual_wide, expected_wide);
        assert_eq!(actual_utf8, expected_utf8);
    }

    #[test]
    fn test2_ascii() {
        let ascii_string: &[u8] = b"Hello, World!";
        let expected_wide = w("Hello, World!");
        let mut actual_wide = WString::new();
        assert!(utf8_to_wide(ascii_string, &mut actual_wide));

        let expected_utf8 = "Hello, World!";
        let mut actual_utf8 = String::new();
        assert!(wide_to_utf8(&expected_wide, &mut actual_utf8));

        assert_eq!(actual_wide, expected_wide);
        assert_eq!(actual_utf8, expected_utf8);
    }

    #[test]
    fn test3_non_ascii() {
        // "Привет, мир!"
        let non_ascii_string: &[u8] = b"\xD0\x9F\xD1\x80\xD0\xB8\xD0\xB2\xD0\xB5\xD1\x82, \xD0\xBC\xD0\xB8\xD1\x80!";
        let expected_wide = w("Привет, мир!");
        let mut actual_wide = WString::new();
        assert!(utf8_to_wide(non_ascii_string, &mut actual_wide));

        let mut actual_utf8 = String::new();
        assert!(wide_to_utf8(&expected_wide, &mut actual_utf8));

        assert_eq!(actual_wide, expected_wide);
        assert_eq!(actual_utf8.as_bytes(), non_ascii_string);
    }

    #[test]
    fn test4_special_chars() {
        let special_chars_string: &[u8] = b"This string has: !@#$%^&*()_+=-`~[]{}:;'<>,.?/";
        let expected_wide = w("This string has: !@#$%^&*()_+=-`~[]{}:;'<>,.?/");
        let mut actual_wide = WString::new();
        assert!(utf8_to_wide(special_chars_string, &mut actual_wide));

        let expected_utf8 = "This string has: !@#$%^&*()_+=-`~[]{}:;'<>,.?/";
        let mut actual_utf8 = String::new();
        assert!(wide_to_utf8(&expected_wide, &mut actual_utf8));

        assert_eq!(actual_wide, expected_wide);
        assert_eq!(actual_utf8, expected_utf8);
    }

    #[test]
    fn test5_multi_lang() {
        // "Привет! こんにちは世界! 안녕하세요!"
        let multi_lang_string: &[u8] = b"\xD0\x9F\xD1\x80\xD0\xB8\xD0\xB2\xD0\xB5\xD1\x82! \xE3\x81\x93\xE3\x82\x93\xE3\x81\xAB\xE3\x81\xA1\xE3\x81\xAF\xE4\xB8\x96\xE7\x95\x8C! \xEC\x95\x88\xEB\x85\x95\xED\x95\x98\xEC\x84\xB8\xEC\x9A\x94!";
        let expected_wide = w("Привет! こんにちは世界! 안녕하세요!");
        let mut actual_wide = WString::new();
        assert!(utf8_to_wide(multi_lang_string, &mut actual_wide));

        let mut actual_utf8 = String::new();
        assert!(wide_to_utf8(&expected_wide, &mut actual_utf8));

        assert_eq!(actual_wide, expected_wide);
        assert_eq!(actual_utf8.as_bytes(), multi_lang_string);
    }

    #[test]
    fn test6_adapter_empty() {
        let adapter = WideToUtf8ArgsAdapter::new(&[]);
        let utf8_args = adapter.get_utf8_args();

        assert_eq!(adapter.get_argc(), 0);
        assert!(utf8_args.is_empty());
    }

    #[test]
    fn test7_adapter_three_args() {
        // L"Привет", L"мир", L"!"
        let a = w("Привет");
        let b = w("мир");
        let c = w("!");
        let wargv: [Option<&[u16]>; 3] = [Some(&a), Some(&b), Some(&c)];
        let adapter = WideToUtf8ArgsAdapter::new(&wargv);
        let utf8_args = adapter.get_utf8_args();

        assert_eq!(adapter.get_argc(), 3);
        for (i, &arg) in wargv.iter().enumerate() {
            let mut converted = String::new();
            assert!(wide_to_utf8(arg.unwrap(), &mut converted));
            assert_eq!(utf8_args[i].to_str().unwrap(), converted);
        }
    }

    #[test]
    fn test8_adapter_skips_nulls() {
        let a = w("arg1");
        let c = w("arg3");
        let wargv: [Option<&[u16]>; 3] = [Some(&a), None, Some(&c)];
        let adapter = WideToUtf8ArgsAdapter::new(&wargv);
        let utf8_args = adapter.get_utf8_args();

        assert_eq!(utf8_args[0].to_str().unwrap(), "arg1");
        assert_eq!(utf8_args[1].to_str().unwrap(), "arg3");
        assert_eq!(adapter.get_argc(), 2);
    }

    #[test]
    fn test9_long_path_prefix() {
        let mut shortpath = WString::new();
        assert!(utf8_to_wide(b"C:\\short", &mut shortpath));
        assert_eq!(shortpath, w("C:\\short"));

        let mut longpath: Vec<u8> = b"C:\\".to_vec();
        longpath.resize(3 + MAX_DIR_PATH, b'a');
        let mut widelong = WString::new();
        assert!(utf8_to_wide(&longpath, &mut widelong));
        let mut expected = wide(r"\\?\C:\");
        expected.resize(7 + MAX_DIR_PATH, b'a' as u16);
        assert_eq!(widelong, expected);

        let mut uncpath: Vec<u8> = b"\\\\server\\".to_vec();
        uncpath.resize(9 + MAX_DIR_PATH, b'b');
        let mut wideunc = WString::new();
        assert!(utf8_to_wide(&uncpath, &mut wideunc));
        let mut expected = wide(r"\\?\UNC\server\");
        expected.resize(15 + MAX_DIR_PATH, b'b' as u16);
        assert_eq!(wideunc, expected);
    }

    #[test]
    fn test10_accented_filename() {
        // "café.dat"
        let utf8: &[u8] = b"caf\xC3\xA9.dat";

        let mut converted = WString::new();
        assert!(utf8_to_wide(utf8, &mut converted));

        assert_eq!(converted, w("café.dat"));
    }

    #[test]
    fn test11_ansi_codepage_roundtrip() {
        let wide = w("café.dat");

        let mut used_default: i32 = 0;
        let mut ansi = [0u8; 64];
        let length = unsafe {
            WideCharToMultiByte(
                CP_ACP,
                WC_NO_BEST_FIT_CHARS,
                wide.as_ptr(),
                wide.len() as i32,
                ansi.as_mut_ptr(),
                ansi.len() as i32,
                std::ptr::null(),
                &mut used_default,
            )
        };

        // nothing to check when the ANSI code page cannot hold the name
        if length <= 0 || used_default != 0 {
            return;
        }

        let mut converted = WString::new();
        assert!(utf8_to_wide(&ansi[..length as usize], &mut converted));

        assert_eq!(converted, wide);
    }
}
