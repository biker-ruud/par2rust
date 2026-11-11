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

//! A fixed set of equally sized buffers which the threads using them take from
//! and give back to. They all come from one allocation, so that a thread which
//! holds several of them, or every one of them, costs no more than the pool
//! does and none of them is made and thrown away as the demand for them moves.

use std::sync::{Condvar, Mutex};

pub struct BufferPool {
    slab: Vec<u8>,                  // every buffer, in one allocation
    buffers: usize,
    buffersize: usize,
    unused: Mutex<Vec<usize>>,      // the buffers which are not in use
    given: Condvar,                 // a buffer has been given back
}

impl BufferPool {
    pub fn new() -> BufferPool {
        BufferPool {
            slab: Vec::new(),
            buffers: 0,
            buffersize: 0,
            unused: Mutex::new(Vec::new()),
            given: Condvar::new(),
        }
    }

    /// Makes count buffers of size bytes, none of them in use. Every buffer
    /// taken from an earlier set is given up, so no thread may still hold one.
    ///
    /// `&mut self` is what makes "no thread may still hold one" true: nothing
    /// else can hold a reference into the pool while it is being remade.
    pub fn reset(&mut self, count: usize, size: usize) {
        self.slab = vec![0u8; count * size];

        self.buffers = count;
        self.buffersize = size;

        let unused = self.unused.get_mut().unwrap();
        unused.clear();
        unused.reserve(count);
        for index in 0..count {
            unused.push(index);
        }
    }

    pub fn count(&self) -> usize {
        self.buffers
    }

    pub fn size(&self) -> usize {
        self.buffersize
    }

    /// The buffer a taken index stands for.
    ///
    /// # Safety
    ///
    /// The caller must have taken `index` from this pool and not given it
    /// back, so that this is the only handle to that buffer: taking one index
    /// twice, or using it after `give`, lets two handles write the same memory.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn at(&self, index: usize) -> &mut [u8] {
        debug_assert!(index < self.buffers);
        debug_assert!(!self.slab.is_empty());
        std::slice::from_raw_parts_mut(
            self.slab.as_ptr().add(index * self.buffersize) as *mut u8,
            self.buffersize,
        )
    }

    /// Takes a buffer, and returns whether there was one which was not in use.
    pub fn try_take(&self) -> Option<usize> {
        let mut unused = self.unused.lock().unwrap();

        unused.pop()
    }

    /// Takes a buffer, waiting for one to be given back if every one of them
    /// is in use.
    pub fn take(&self) -> usize {
        let mut unused = self.unused.lock().unwrap();

        while unused.is_empty() {
            unused = self.given.wait(unused).unwrap();
        }

        unused.pop().unwrap()
    }

    pub fn give(&self, index: usize) {
        {
            let mut unused = self.unused.lock().unwrap();
            unused.push(index);
        }

        self.given.notify_one();
    }
}

impl Default for BufferPool {
    fn default() -> BufferPool {
        BufferPool::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    // A pool hands out every buffer it was made with, and no more than that
    #[test]
    fn test1_pool_hands_out_every_buffer_and_no_more() {
        let count = 8usize;

        let mut pool = BufferPool::new();
        pool.reset(count, 1024);

        assert_eq!(pool.count(), count);
        assert_eq!(pool.size(), 1024);

        let mut taken: HashSet<usize> = HashSet::new();
        for i in 0..count {
            let index = pool
                .try_take()
                .unwrap_or_else(|| panic!("test1: only {i} of {count} buffers could be taken"));

            assert!(
                taken.insert(index),
                "test1: buffer {index} was handed out twice"
            );
        }

        assert!(
            pool.try_take().is_none(),
            "test1: a buffer was taken from a pool with none left"
        );

        // Giving one back makes exactly one available again
        pool.give(*taken.iter().next().unwrap());

        assert!(
            pool.try_take().is_some(),
            "test1: the buffer which was given back could not be taken"
        );

        assert!(
            pool.try_take().is_none(),
            "test1: two buffers came back from one Give"
        );
    }

    // The buffers do not overlap, and each is the size the pool was made with
    #[test]
    fn test2_buffers_do_not_overlap() {
        let count = 6usize;
        let size = 512usize;

        let mut pool = BufferPool::new();
        pool.reset(count, size);

        let mut taken = Vec::new();
        for i in 0..count {
            let index = pool
                .try_take()
                .unwrap_or_else(|| panic!("test2: only {i} buffers could be taken"));
            taken.push(index);
        }

        // Writing the whole of every buffer
        for (i, &index) in taken.iter().enumerate() {
            let buffer = unsafe { pool.at(index) };
            assert_eq!(buffer.len(), size);
            buffer.fill((i + 1) as u8);
        }

        // did not disturb any other one
        for (i, &index) in taken.iter().enumerate() {
            let buffer = unsafe { pool.at(index) };
            for (byte, &value) in buffer.iter().enumerate() {
                assert_eq!(
                    value,
                    (i + 1) as u8,
                    "test2: buffer {i} byte {byte} was overwritten"
                );
            }
        }
    }

    // Take waits for a buffer when every one of them is in use
    #[test]
    fn test3_take_waits_for_a_buffer() {
        let mut pool = BufferPool::new();
        pool.reset(1, 64);

        let held = pool
            .try_take()
            .expect("test3: the only buffer could not be taken");

        let pool = Arc::new(pool);
        let waiting = Arc::new(AtomicBool::new(true));
        let got = Arc::new(AtomicUsize::new(usize::MAX));

        let (tx, rx) = std::sync::mpsc::channel();
        let t_pool = Arc::clone(&pool);
        let t_waiting = Arc::clone(&waiting);
        let t_got = Arc::clone(&got);

        let taker = std::thread::spawn(move || {
            let index = t_pool.take();
            t_got.store(index, Ordering::SeqCst);
            t_waiting.store(false, Ordering::SeqCst);
            let _ = tx.send(index);
        });

        // The buffer is still held, so the other thread cannot have been given
        // one. Finding it still waiting after a short while says that Take held
        // it up rather than that it had yet to call Take at all.
        std::thread::sleep(Duration::from_millis(20));

        assert!(
            waiting.load(Ordering::SeqCst),
            "test3: Take returned while every buffer was in use"
        );
        assert!(rx.try_recv().is_err());

        pool.give(held);
        taker.join().unwrap();

        assert_eq!(
            got.load(Ordering::SeqCst),
            held,
            "test3: Take gave a buffer which was not the one given back"
        );
    }

    // Buffers taken and given back by many threads at once are never held twice
    #[test]
    fn test4_buffers_are_never_held_twice() {
        let count = 4usize;
        let threads = 8usize;
        let rounds = 500usize;

        let mut pool = BufferPool::new();
        pool.reset(count, 64);
        let pool = Arc::new(pool);

        let holders: Vec<AtomicI32> = (0..count).map(|_| AtomicI32::new(0)).collect();
        let clashes = AtomicI32::new(0);

        std::thread::scope(|scope| {
            for _ in 0..threads {
                let pool = Arc::clone(&pool);
                let holders = &holders;
                let clashes = &clashes;
                scope.spawn(move || {
                    for round in 0..rounds {
                        let index = pool.take();

                        if holders[index].fetch_add(1, Ordering::SeqCst) != 0 {
                            clashes.fetch_add(1, Ordering::SeqCst);
                        }

                        // The buffer is this thread's alone to write to
                        let buffer = unsafe { pool.at(index) };
                        buffer.fill((round & 0xff) as u8);

                        holders[index].fetch_sub(1, Ordering::SeqCst);
                        pool.give(index);
                    }
                });
            }
        });

        assert_eq!(
            clashes.load(Ordering::SeqCst),
            0,
            "test4: a buffer was held by two threads at once {} times",
            clashes.load(Ordering::SeqCst)
        );

        // Every buffer came back
        for i in 0..count {
            assert!(
                pool.try_take().is_some(),
                "test4: only {i} of {count} buffers were given back"
            );
        }
    }

    // A pool can be made again with a different set of buffers
    #[test]
    fn test5_pool_can_be_remade() {
        let mut pool = BufferPool::new();

        pool.reset(2, 128);

        assert!(
            pool.try_take().is_some(),
            "test5: the first set of buffers could not be taken"
        );

        pool.reset(5, 32);

        assert_eq!(pool.count(), 5);
        assert_eq!(pool.size(), 32);

        for i in 0..5 {
            assert!(
                pool.try_take().is_some(),
                "test5: only {i} of the new buffers could be taken"
            );
        }

        assert!(
            pool.try_take().is_none(),
            "test5: a buffer was taken from a pool with none left"
        );
    }
}
