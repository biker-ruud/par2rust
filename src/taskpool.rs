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

//! A fixed set of threads which run work submitted to them from anywhere, so
//! that work submitted from several places at once is shared out between all of
//! them rather than each place being given a share of the threads.
//!
//! Submitting does not wait for the work to run, and a thread which waits for
//! a submission of its own runs queued work while it waits. The pool's own
//! threads wait for nothing but the queue, so a thread which is waiting for its
//! work can never leave the queue unattended.
//!
//! The thread-count helpers of the retired `foreach_parallel.h` live here too:
//! the pool replaces the runner, and these outlived it.

use std::any::Any;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

/// The largest number of threads a requested thread count is taken up to. Each
/// thread costs a stack and a share of the buffers a loop divides between them.
pub const MAX_THREAD_COUNT: u32 = 256;

/// The number of threads to use when no particular number was asked for.
pub fn default_threads() -> u32 {
    std::thread::available_parallelism()
        .map(|count| count.get() as u32)
        .unwrap_or(1)
        .max(1)
}

/// The number of threads to use for a requested count, where 0 asks for the
/// default.
pub fn resolve_threads(requested: u32) -> u32 {
    if requested != 0 {
        requested.min(MAX_THREAD_COUNT)
    } else {
        default_threads()
    }
}

/// The first failure of a body, handed back by [`TaskPool::wait`].
pub type TaskError = Box<dyn Any + Send>;

/// One submission of work. A batch is handed to [`TaskPool::submit`] and then
/// to [`TaskPool::wait`], and may be submitted again once it has been waited
/// for.
pub struct Batch {
    inner: Arc<BatchInner>,
}

struct BatchInner {
    // Locked after the pool's own lock, never before it.
    state: Mutex<BatchState>,
}

struct BatchState {
    body: Option<Arc<dyn Fn(usize) + Send + Sync>>, // the body, owned while queued
    next: usize,                                    // the next value a thread may claim
    end: usize,                                     // one past the last value to run
    remaining: usize, // values which have not finished
    error: Option<TaskError>, // the first failure of the body
}

impl Batch {
    pub fn new() -> Batch {
        Batch {
            inner: Arc::new(BatchInner {
                state: Mutex::new(BatchState {
                    body: None,
                    next: 0,
                    end: 0,
                    remaining: 0,
                    error: None,
                }),
            }),
        }
    }
}

impl Default for Batch {
    fn default() -> Batch {
        Batch::new()
    }
}

pub struct TaskPool {
    inner: Arc<Inner>,
}

struct Inner {
    threads: Mutex<Vec<thread::JoinHandle<()>>>,
    state: Mutex<State>,
    haswork: Condvar,   // a batch has been queued
    batchdone: Condvar, // a batch has finished
}

struct State {
    queue: VecDeque<Arc<BatchInner>>, // batches with values left to claim
    stopping: bool,
}

impl TaskPool {
    pub fn new(numthreads: usize) -> TaskPool {
        let wanted = numthreads.max(1);

        let inner = Arc::new(Inner {
            threads: Mutex::new(Vec::new()),
            state: Mutex::new(State {
                queue: VecDeque::new(),
                stopping: false,
            }),
            haswork: Condvar::new(),
            batchdone: Condvar::new(),
        });

        let mut threads = inner.threads.lock().unwrap();
        for _ in 0..wanted {
            let inner = Arc::clone(&inner);

            // The work runs on however many threads could be started, and on
            // the threads waiting for it
            match thread::Builder::new().spawn(move || Inner::serve(inner)) {
                Ok(handle) => threads.push(handle),
                Err(_) => break,
            }
        }
        drop(threads);

        TaskPool { inner }
    }

    pub fn thread_count(&self) -> usize {
        self.inner.threads.lock().unwrap().len()
    }

    /// Queues the body to be called for every value in `[first, last)` and
    /// returns without waiting for any of them to run. The batch must be
    /// handed to [`TaskPool::wait`] before it is submitted again.
    pub fn submit<F>(&self, batch: &Batch, first: usize, last: usize, body: F)
    where
        F: Fn(usize) + Send + Sync + 'static,
    {
        let mut state = self.inner.state.lock().unwrap();

        {
            let mut bs = batch.inner.state.lock().unwrap();

            bs.body = Some(Arc::new(body));
            bs.next = first;
            bs.end = last;
            bs.remaining = last.saturating_sub(first);
            bs.error = None;
        }

        if last <= first {
            return;
        }

        state.queue.push_back(Arc::clone(&batch.inner));

        drop(state);

        self.inner.haswork.notify_all();

        // A thread waiting for a batch of its own runs whatever is queued
        // while it waits, so it is woken for this one as well as the pool's
        // own threads
        self.inner.batchdone.notify_all();
    }

    /// Runs queued work until the batch has finished. A panic thrown by the
    /// body abandons the rest of that batch and is returned here.
    pub fn wait(&self, batch: &Batch) -> Result<(), TaskError> {
        loop {
            if self.inner.remaining(&batch.inner) == 0 {
                break;
            }

            // Nothing is left to help with when every value has been claimed,
            // and whichever threads hold those claims will finish them
            if !self.inner.claim_one() {
                let guard = self.inner.state.lock().unwrap();

                let finished = {
                    let bs = batch.inner.state.lock().unwrap();
                    bs.remaining == 0
                };

                if !finished {
                    let _guard = self.inner.batchdone.wait(guard).unwrap();
                }
            }
        }

        let mut bs = batch.inner.state.lock().unwrap();

        match bs.error.take() {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }
}

impl Drop for TaskPool {
    fn drop(&mut self) {
        {
            let mut state = self.inner.state.lock().unwrap();
            state.stopping = true;
        }

        self.inner.haswork.notify_all();

        let handles: Vec<_> = self.inner.threads.lock().unwrap().drain(..).collect();
        for handle in handles {
            let _ = handle.join();
        }
    }
}

impl Inner {
    fn serve(inner: Arc<Inner>) {
        loop {
            let mut state = inner.state.lock().unwrap();

            while !state.stopping && state.queue.is_empty() {
                state = inner.haswork.wait(state).unwrap();
            }

            if state.stopping {
                return;
            }

            drop(state);

            inner.claim_one();
        }
    }

    fn remaining(&self, batch: &Arc<BatchInner>) -> usize {
        // The pool's lock is taken before a batch's own, never after it
        let _state = self.state.lock().unwrap();

        batch.state.lock().unwrap().remaining
    }

    /// Runs one value of the batch at the head of the queue and returns
    /// whether there was one. The locks are dropped while the body runs.
    fn claim_one(&self) -> bool {
        let (batch, value, body) = {
            let mut state = self.state.lock().unwrap();

            let Some(batch) = state.queue.front().cloned() else {
                return false;
            };

            let mut bs = batch.state.lock().unwrap();

            let value = bs.next;
            bs.next += 1;

            // A batch is queued only while it has values left for a thread
            // to claim
            if bs.next == bs.end {
                state.queue.pop_front();
            }

            let body = Arc::clone(bs.body.as_ref().expect("a queued batch has a body"));

            drop(bs);

            (batch, value, body)
        };

        let failure = catch_unwind(AssertUnwindSafe(|| body(value))).err();

        let mut state = self.state.lock().unwrap();
        let mut bs = batch.state.lock().unwrap();

        if let Some(failure) = failure {
            if bs.error.is_none() {
                bs.error = Some(failure);
            }

            abandon(&mut state, &batch, &mut bs);
        }

        bs.remaining -= 1;

        if bs.remaining == 0 {
            self.batchdone.notify_all();
        }

        true
    }

}

/// Drops the values of a failed batch which no thread has claimed yet. Called
/// with the pool's lock and the batch's own lock held.
fn abandon(state: &mut State, batch: &Arc<BatchInner>, bs: &mut BatchState) {
    if bs.next == bs.end {
        return;
    }

    if let Some(queued) = state.queue.iter().position(|b| Arc::ptr_eq(b, batch)) {
        state.queue.remove(queued);
    }

    bs.remaining -= bs.end - bs.next;
    bs.next = bs.end;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    // Counts how many times each value in [first, last) was run, and reports
    // any value which was not run exactly once
    fn check_counts(what: &str, counts: &[AtomicUsize], first: usize, last: usize) {
        for (i, count) in counts.iter().enumerate() {
            let expected: usize = if i >= first && i < last { 1 } else { 0 };
            assert_eq!(
                count.load(Ordering::SeqCst),
                expected,
                "{what}: value {i} ran {} times, expected {expected}",
                count.load(Ordering::SeqCst)
            );
        }
    }

    fn counts(size: usize) -> Arc<Vec<AtomicUsize>> {
        Arc::new((0..size).map(|_| AtomicUsize::new(0)).collect())
    }

    // Every value of a batch runs exactly once, whatever the thread count
    #[test]
    fn test1_every_value_runs_exactly_once() {
        let size = 1000usize;

        for numthreads in 1..=8usize {
            let counts = counts(size);

            let pool = TaskPool::new(numthreads);

            let batch = Batch::new();
            let c = Arc::clone(&counts);
            pool.submit(&batch, 0, size, move |value| {
                c[value].fetch_add(1, Ordering::SeqCst);
            });
            pool.wait(&batch).unwrap();

            check_counts("test1", &counts, 0, size);
        }
    }

    // Only the values of the range asked for are run
    #[test]
    fn test2_only_the_range_is_run() {
        let size = 64usize;

        let counts = counts(size);

        let pool = TaskPool::new(4);

        let batch = Batch::new();
        let c = Arc::clone(&counts);
        pool.submit(&batch, 16, 48, move |value| {
            c[value].fetch_add(1, Ordering::SeqCst);
        });
        pool.wait(&batch).unwrap();

        check_counts("test2", &counts, 16, 48);
    }

    // An empty range finishes without running anything
    #[test]
    fn test3_empty_range_runs_nothing() {
        let pool = TaskPool::new(4);

        let runs = Arc::new(AtomicUsize::new(0));
        let batch = Batch::new();

        let r = Arc::clone(&runs);
        pool.submit(&batch, 2, 2, move |_| {
            r.fetch_add(1, Ordering::SeqCst);
        });
        pool.wait(&batch).unwrap();

        let r = Arc::clone(&runs);
        pool.submit(&batch, 3, 1, move |_| {
            r.fetch_add(1, Ordering::SeqCst);
        });
        pool.wait(&batch).unwrap();

        assert_eq!(runs.load(Ordering::SeqCst), 0, "test3: an empty range ran values");
    }

    // A batch can be submitted again once it has been waited for
    #[test]
    fn test4_batch_can_be_resubmitted() {
        let size = 100usize;
        let rounds = 20usize;

        let counts = counts(size);

        let pool = TaskPool::new(4);

        let batch = Batch::new();

        for _ in 0..rounds {
            let c = Arc::clone(&counts);
            pool.submit(&batch, 0, size, move |value| {
                c[value].fetch_add(1, Ordering::SeqCst);
            });
            pool.wait(&batch).unwrap();
        }

        for (i, count) in counts.iter().enumerate() {
            assert_eq!(
                count.load(Ordering::SeqCst),
                rounds,
                "test4: value {i} ran {} times, expected {rounds}",
                count.load(Ordering::SeqCst)
            );
        }
    }

    // Work submitted from many threads at once is shared between them all,
    // even when there are more threads submitting than the pool has
    #[test]
    fn test5_work_from_many_threads_is_shared() {
        let submitters = 8usize;
        let size = 500usize;

        for numthreads in 1..=4usize {
            let counts = counts(submitters * size);

            let pool = TaskPool::new(numthreads);

            std::thread::scope(|scope| {
                for submitter in 0..submitters {
                    let pool = &pool;
                    let counts = Arc::clone(&counts);
                    scope.spawn(move || {
                        let batch = Batch::new();
                        let c = Arc::clone(&counts);
                        pool.submit(&batch, submitter * size, (submitter + 1) * size, move |value| {
                            c[value].fetch_add(1, Ordering::SeqCst);
                        });
                        pool.wait(&batch).unwrap();
                    });
                }
            });

            let total = submitters * size;
            check_counts("test5", &counts, 0, total);
        }
    }

    // A thread waiting for its own batch runs the work which is still queued,
    // so two values which have to run at the same time can, even on a pool of
    // one
    #[test]
    fn test6_waiting_thread_runs_queued_work() {
        let pool = TaskPool::new(1);

        // The work can only be shared with the thread waiting for it if the
        // pool managed to start a thread of its own
        if pool.thread_count() < 1 {
            return;
        }

        let mutex = Arc::new(Mutex::new(0usize));
        let arrived = Arc::new(Condvar::new());
        let timedout = Arc::new(AtomicBool::new(false));

        let batch = Batch::new();

        let m = Arc::clone(&mutex);
        let cv = Arc::clone(&arrived);
        let to = Arc::clone(&timedout);
        pool.submit(&batch, 0, 2, move |_| {
            let mut waiting = m.lock().unwrap();

            *waiting += 1;
            if *waiting == 2 {
                cv.notify_all();
                return;
            }

            while *waiting < 2 {
                let (guard, timeout) =
                    cv.wait_timeout(waiting, Duration::from_secs(30)).unwrap();
                waiting = guard;
                if timeout.timed_out() {
                    // Let the other value out rather than leaving the test
                    // hanging
                    to.store(true, Ordering::SeqCst);
                    *waiting = 2;
                    cv.notify_all();
                    break;
                }
            }
        });
        pool.wait(&batch).unwrap();

        assert!(
            !timedout.load(Ordering::SeqCst),
            "test6: the thread waiting for the batch did not run any of it"
        );
    }

    // A failure of the body is returned by wait, which still returns once
    // every value the batch had out has finished
    //
    // How many values run before the failure is recorded is not fixed: the
    // threads which have already claimed one run it whatever happens, so only
    // the values no thread had reached are abandoned. What is checked here is
    // that the batch finishes rather than hanging, which is what wrong
    // accounting in abandon would cost, and that it is left fit to be
    // submitted again.
    #[test]
    fn test7_failure_is_returned_and_batch_reusable() {
        let size = 200usize;

        for numthreads in 1..=4usize {
            let pool = TaskPool::new(numthreads);

            let runs = Arc::new(AtomicUsize::new(0));
            let r = Arc::clone(&runs);
            let batch = Batch::new();
            pool.submit(&batch, 0, size, move |value| {
                r.fetch_add(1, Ordering::SeqCst);
                if value == 0 {
                    panic!("failed");
                }
            });

            let failure = match pool.wait(&batch) {
                Ok(()) => panic!("test7: the failure was not returned"),
                Err(failure) => failure,
            };

            let is_failed = failure
                .downcast_ref::<&str>()
                .is_some_and(|message| *message == "failed")
                || failure
                    .downcast_ref::<String>()
                    .is_some_and(|message| message == "failed");
            assert!(is_failed, "test7: the failure was not the panic");

            // The batch is usable again, and no longer holds the failure
            runs.store(0, Ordering::SeqCst);
            let r = Arc::clone(&runs);
            pool.submit(&batch, 0, 10, move |_| {
                r.fetch_add(1, Ordering::SeqCst);
            });
            pool.wait(&batch).unwrap();

            assert_eq!(
                runs.load(Ordering::SeqCst),
                10,
                "test7: {} values ran after the failure, expected 10",
                runs.load(Ordering::SeqCst)
            );
        }
    }

    // Several batches from one thread can be in flight at once, and each is
    // waited for on its own
    #[test]
    fn test8_several_batches_in_flight() {
        let size = 200usize;

        let counts = counts(2 * size);

        let pool = TaskPool::new(4);

        let first = Batch::new();
        let second = Batch::new();

        let c = Arc::clone(&counts);
        pool.submit(&first, 0, size, move |value| {
            c[value].fetch_add(1, Ordering::SeqCst);
        });
        let c = Arc::clone(&counts);
        pool.submit(&second, size, 2 * size, move |value| {
            c[value].fetch_add(1, Ordering::SeqCst);
        });

        pool.wait(&second).unwrap();
        pool.wait(&first).unwrap();

        check_counts("test8", &counts, 0, 2 * size);
    }
}
