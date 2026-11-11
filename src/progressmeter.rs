//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2003 Peter Brian Clements
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

//! Displays a percentage of progress on a stream, refreshing in place at most
//! every 50 milliseconds so that the output stays readable.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const PRINT_INTERVAL_MS: u64 = 50;

// Sentinel for "nothing printed yet", so the first change always prints.
const NEVER_PRINTED: u64 = u64::MAX;

pub struct ProgressMeter {
    // stream for output (for commandline, this is cout)
    sout: Arc<Mutex<Box<dyn Write + Send>>>,
    // message to display alongside percentage
    message: String,
    // pre-computed multiplier to convert progress value into a percentage*10
    scale: f32,
    // last known progress value
    current: AtomicU64,
    // last time progress was outputted, in milliseconds since the meter was
    // created
    printed: AtomicU64,
    start: Instant,
}

impl ProgressMeter {
    pub fn new(sout: Box<dyn Write + Send>, message: &str, total: u64) -> ProgressMeter {
        ProgressMeter::new_shared(Arc::new(Mutex::new(sout)), message, total)
    }

    /// Build a meter on a stream already shared with other meters or with
    /// the caller, so that several of them can write to one stream.
    pub fn new_shared(
        sout: Arc<Mutex<Box<dyn Write + Send>>>,
        message: &str,
        total: u64,
    ) -> ProgressMeter {
        ProgressMeter {
            sout,
            message: message.to_string(),
            scale: if total != 0 { 1000.0f32 / total as f32 } else { 0.0f32 },
            current: AtomicU64::new(0),
            printed: AtomicU64::new(NEVER_PRINTED),
            start: Instant::now(),
        }
    }

    fn calc_thousandths(&self, val: u64) -> u32 {
        (self.scale * val as f32 + 0.5f32) as u32
    }

    fn print_fraction(&self, oldval: u64, newval: u64) -> bool {
        // if the displayed value won't change, don't print
        let newfraction = self.calc_thousandths(newval);
        if self.calc_thousandths(oldval) == newfraction {
            return false;
        }

        // check if enough time has passed
        let lastprinted = self.printed.load(Ordering::Relaxed);
        let now = self.start.elapsed().as_millis() as u64;

        // if enough time has passed, print the current progress, and update
        // the time record
        if lastprinted == NEVER_PRINTED
            || now - lastprinted >= PRINT_INTERVAL_MS
            || newfraction == 1000
        {
            if let Ok(mut sout) = self.sout.lock() {
                let _ = write!(
                    sout,
                    "{}{}.{:01}%\r",
                    self.message,
                    newfraction / 10,
                    newfraction % 10
                );
                let _ = sout.flush();
            }
            self.printed.store(now, Ordering::Relaxed);
            return true;
        }
        false
    }

    /// NOTE: update() doesn't always update current value, so don't mix it
    /// with add()
    pub fn update(&self, newval: u64) {
        let oldval = self.current.load(Ordering::Relaxed);
        if self.print_fraction(oldval, newval) {
            self.current.store(newval, Ordering::Relaxed);
        }
    }

    pub fn add(&self, amount: u64) {
        let newval = self.current.fetch_add(amount, Ordering::Relaxed) + amount;
        self.print_fraction(newval - amount, newval);
    }

    /// print a line whilst progress is still running
    pub fn print_line(&self, line: &str) {
        let val = self.current.load(Ordering::Relaxed);
        let fraction = self.calc_thousandths(val);
        if let Ok(mut sout) = self.sout.lock() {
            let _ = write!(
                sout,
                "{:>w$}\r{}\n{}{}.{:01}%\r",
                "",
                line,
                self.message,
                fraction / 10,
                fraction % 10,
                w = self.message.len() + 6
            );
            let _ = sout.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Result;
    use std::sync::Arc;

    #[derive(Clone)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn contents(buf: &SharedBuf) -> String {
        String::from_utf8(buf.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn add_prints_percentage() {
        let buf = SharedBuf(Arc::new(Mutex::new(Vec::new())));
        let meter = ProgressMeter::new(Box::new(buf.clone()), "Hash: ", 1000);

        meter.add(500);
        let out = contents(&buf);
        assert!(out.contains("Hash: 50.0%\r"), "got: {out}");

        meter.add(500);
        let out = contents(&buf);
        assert!(out.contains("100.0%"), "got: {out}");
    }

    #[test]
    fn unchanged_fraction_does_not_print() {
        let buf = SharedBuf(Arc::new(Mutex::new(Vec::new())));
        let meter = ProgressMeter::new(Box::new(buf.clone()), "x", 1000);

        meter.add(1);
        let after_first = buf.0.lock().unwrap().len();
        meter.add(0);
        assert_eq!(buf.0.lock().unwrap().len(), after_first);
    }

    #[test]
    fn update_stores_only_when_printing() {
        let buf = SharedBuf(Arc::new(Mutex::new(Vec::new())));
        let meter = ProgressMeter::new(Box::new(buf.clone()), "x", 1000);

        meter.update(250);
        assert!(contents(&buf).contains("25.0%"));
        assert_eq!(meter.current.load(Ordering::Relaxed), 250);
    }

    #[test]
    fn print_line_shows_line_and_current_fraction() {
        let buf = SharedBuf(Arc::new(Mutex::new(Vec::new())));
        let meter = ProgressMeter::new(Box::new(buf.clone()), "abc", 1000);
        meter.add(1000);

        buf.0.lock().unwrap().clear();
        meter.print_line("a note");
        let out = contents(&buf);
        assert!(out.contains("a note\n"), "got: {out}");
        assert!(out.ends_with("abc100.0%\r"), "got: {out}");
        // the line is right aligned over the message plus the percentage
        assert!(out.starts_with("         \r"), "got: {out}");
    }

    #[test]
    fn zero_total_never_prints_a_fraction() {
        let buf = SharedBuf(Arc::new(Mutex::new(Vec::new())));
        let meter = ProgressMeter::new(Box::new(buf.clone()), "x", 0);
        meter.add(12345);
        assert!(contents(&buf).is_empty());
    }
}
