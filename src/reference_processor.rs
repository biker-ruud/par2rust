//  This file is part of par2cmdline (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2026 Michael Nightingale
//
//  par2cmdline is free software; you can redistribute it and/or modify
//  it under the terms of the GNU General Public License as published by
//  the Free Software Foundation; either version 2 of the License, or
//  (at your option) any later version.
//
//  par2cmdline is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU General Public License for more details.
//
//  You should have received a copy of the GNU General Public License
//  along with this program; if not, write to the Free Software
//  Foundation, Inc., 59 Temple Place, Suite 330, Boston, MA  02111-1307  USA

//! Multiplies each input block by the matrix on `numthreads` threads, which
//! stay alive from one submission to the next, and keeps the accumulated
//! output blocks in one buffer. `add_input` queues a submission and returns,
//! the block being multiplied on the thread this holds while the caller
//! reads the next one.
//!
//! The C++ original hands the queue raw pointers into the caller's buffers,
//! which the caller keeps until the future resolves. This port copies each
//! submission into reference counted buffers instead, which keeps the queue
//! safe without any `unsafe` of its own; the caller contract is unchanged,
//! only cheaper to satisfy.

use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use crate::galois::Spec16;
use crate::processor::Processor;
use crate::reedsolomon::ReedSolomon;
use crate::taskpool::{Batch, TaskError, TaskPool};

// The accumulated output blocks, one buffer shared between the thread which
// multiplies and the caller which reads it back.
struct SharedBuffer {
    data: UnsafeCell<Vec<u8>>,
}

// The writes of one pass happen on the task pool threads, and the caller
// reads the buffer only once the queue has drained, so no two accesses
// overlap.
unsafe impl Sync for SharedBuffer {}

impl SharedBuffer {
    fn new(len: usize) -> SharedBuffer {
        SharedBuffer {
            data: UnsafeCell::new(vec![0u8; len]),
        }
    }

    fn bytes(&self) -> &[u8] {
        unsafe { &*self.data.get() }
    }

    fn zero(&self) {
        unsafe {
            let v = &mut *self.data.get();
            v.fill(0);
        }
    }
}

// Points at the buffer for the task pool threads, which write disjoint
// ranges of it.
#[derive(Clone, Copy)]
struct ChunkWriter {
    base: NonNull<u8>,
    maxlength: usize,
}

unsafe impl Send for ChunkWriter {}
unsafe impl Sync for ChunkWriter {}

impl ChunkWriter {
    // Hands out the address of one chunk. The task pool threads write
    // disjoint chunks, so the caller may turn the address into a mutable
    // slice for the length of its task and no two such slices overlap.
    fn chunk(&self, index: usize) -> *mut u8 {
        unsafe { self.base.as_ptr().add(index * self.maxlength) }
    }
}

// One input block waiting to be multiplied, or being multiplied.
struct Submission {
    data: Arc<[u8]>,
    factors: Arc<[u16]>,
    processed: Sender<()>,
}

// The buffers and threads of one run of the processor, replaced whole by
// init.
struct PassState {
    maxlength: usize,
    outputcount: usize,
    buffer: Arc<SharedBuffer>,
    pool: TaskPool,
}

struct Inner {
    rs: Arc<ReedSolomon<Spec16>>,
    numthreads: u32,

    pass: Mutex<Option<Arc<PassState>>>,
    currentlength: AtomicUsize,

    queue: Mutex<VecDeque<Submission>>, // submissions in flight, oldest first
    stopping: AtomicBool,
    haswork: Condvar, // a submission has been added
    drained: Condvar, // the queue has emptied
}

pub struct ReferenceProcessor {
    inner: Arc<Inner>,
    worker: Option<JoinHandle<()>>, // multiplies the submitted blocks
}

impl ReferenceProcessor {
    pub fn new(rs: Arc<ReedSolomon<Spec16>>, numthreads: u32) -> ReferenceProcessor {
        let inner = Arc::new(Inner {
            rs,
            numthreads,
            pass: Mutex::new(None),
            currentlength: AtomicUsize::new(0),
            queue: Mutex::new(VecDeque::new()),
            stopping: AtomicBool::new(false),
            haswork: Condvar::new(),
            drained: Condvar::new(),
        });

        let worker = {
            let inner = Arc::clone(&inner);
            thread::spawn(move || Self::serve(inner))
        };

        ReferenceProcessor {
            inner,
            worker: Some(worker),
        }
    }

    // Multiplies each submitted block in turn until the processor is
    // dropped.
    fn serve(inner: Arc<Inner>) {
        loop {
            let (data, factors) = {
                let mut guard = inner.queue.lock().unwrap();

                while !inner.stopping.load(Ordering::SeqCst) && guard.is_empty() {
                    guard = inner.haswork.wait(guard).unwrap();
                }

                if inner.stopping.load(Ordering::SeqCst) {
                    return;
                }

                // A submission is left at the front of the queue while it
                // runs, so that the queue only empties once it has.
                let submission = guard.front().unwrap();
                (Arc::clone(&submission.data), Arc::clone(&submission.factors))
            };

            let pass = Arc::clone(inner.pass.lock().unwrap().as_ref().unwrap());
            let length = data.len();

            let failure = std::panic::catch_unwind(AssertUnwindSafe(|| {
                Self::multiply(&inner, &pass, data, factors, length)
            }));

            let processed = {
                let mut guard = inner.queue.lock().unwrap();
                guard.pop_front().unwrap().processed
            };

            inner.drained.notify_one();

            // A receiver which fails to receive says the multiplication
            // failed, which is what an unready future means in C++.
            if matches!(failure, Ok(Ok(()))) {
                let _ = processed.send(());
            }
        }
    }

    fn multiply(
        inner: &Arc<Inner>,
        pass: &Arc<PassState>,
        data: Arc<[u8]>,
        factors: Arc<[u16]>,
        length: usize,
    ) -> Result<(), TaskError> {
        let writer = ChunkWriter {
            base: NonNull::new(unsafe { (*pass.buffer.data.get()).as_mut_ptr() }).unwrap(),
            maxlength: pass.maxlength,
        };
        let rs = Arc::clone(&inner.rs);

        let batch = Batch::new();
        let body = move |index: usize| {
            // Each task writes its own chunk, so this mutable slice does
            // not overlap the slices of the other tasks.
            let chunk =
                unsafe { std::slice::from_raw_parts_mut(writer.chunk(index), length) };
            rs.multiply_add(factors[index], length, &data, chunk);
        };

        pass.pool.submit(&batch, 0, pass.outputcount, body);
        pass.pool.wait(&batch)
    }
}

impl Processor for ReferenceProcessor {
    fn init(&mut self, maxlength: usize, outputcount: u32) -> bool {
        let outputcount = outputcount as usize;
        let threads = (self.inner.numthreads as usize).min(outputcount).max(1);

        let pass = PassState {
            maxlength,
            outputcount,
            buffer: Arc::new(SharedBuffer::new(maxlength * outputcount)),
            pool: TaskPool::new(threads),
        };

        // Replacing the old pass joins the threads it was running, which
        // init only does between runs.
        *self.inner.pass.lock().unwrap() = Some(Arc::new(pass));
        self.inner.currentlength.store(maxlength, Ordering::Relaxed);

        true
    }

    fn set_chunk_length(&mut self, length: usize) {
        self.inner.currentlength.store(length, Ordering::Relaxed);
    }

    fn reset_output(&mut self) {
        self.inner.pass.lock().unwrap().as_ref().unwrap().buffer.zero();
    }

    fn wait_for_add(&mut self) {}

    fn add_input(
        &mut self,
        data: &[u8],
        inputindex: u32,
        factors: Option<&[u16]>,
    ) -> Receiver<()> {
        let _ = inputindex;

        // The reference implementation takes neither offer, so it is always
        // given the factors it multiplies by.
        let factors = factors.expect("the reference implementation needs the matrix factors");

        let (processed, ready) = channel();

        {
            let mut guard = self.inner.queue.lock().unwrap();
            guard.push_back(Submission {
                data: Arc::from(data),
                factors: Arc::from(factors),
                processed,
            });
        }

        self.inner.haswork.notify_one();

        ready
    }

    fn end_input(&mut self) {
        let mut guard = self.inner.queue.lock().unwrap();
        while !guard.is_empty() {
            guard = self.inner.drained.wait(guard).unwrap();
        }
    }

    fn peek_output(&mut self, index: u32) -> Option<&[u8]> {
        let guard = self.inner.pass.lock().unwrap();
        let pass = guard.as_ref()?;

        let currentlength = self.inner.currentlength.load(Ordering::Relaxed);
        let start = index as usize * pass.maxlength;

        // The buffer lives as long as this borrow of self: only init
        // replaces it, and init needs &mut self, which the borrow checker
        // forbids until the borrow ends. The worker writes the buffer only
        // between add_input and end_input, and the caller reads it after
        // end_input.
        let base = unsafe { (*pass.buffer.data.get()).as_ptr() };
        Some(unsafe { std::slice::from_raw_parts(base.add(start), currentlength) })
    }

    fn get_output(&mut self, index: u32, out: &mut [u8]) -> bool {
        let guard = self.inner.pass.lock().unwrap();
        let pass = match guard.as_ref() {
            Some(pass) => pass,
            None => return false,
        };

        let currentlength = self.inner.currentlength.load(Ordering::Relaxed);
        let start = index as usize * pass.maxlength;
        out[..currentlength].copy_from_slice(&pass.buffer.bytes()[start..start + currentlength]);

        true
    }
}

impl Drop for ReferenceProcessor {
    fn drop(&mut self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        self.inner.haswork.notify_one();

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
