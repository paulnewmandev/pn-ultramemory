// SPDX-License-Identifier: Apache-2.0
//! An ordered parallel map with bounded look-ahead: the engine of parallel indexing.
//!
//! [`ordered_parallel`] runs a function over the items `0..n` on several threads and hands the
//! results to a consumer **in item order** on the calling thread, so that what the consumer does
//! (storing files in batches) is identical whatever the number of threads.
//!
//! * **Work distribution.** Threads pull the next item from one atomic counter.
//! * **Bounded memory.** A thread does not start an item that is too far ahead of what the consumer
//!   has taken, counted in items and in the summed weights of the items in flight. The item the
//!   consumer waits for is always allowed, so the pipeline cannot deadlock.
//! * **The caller works too.** While no result is ready, the calling thread computes items itself,
//!   so a run with one thread spawns nothing and a failure to spawn a worker only costs speed.
//! * **Panics are results.** A panic inside the work function is caught and delivered to the
//!   consumer as an error message for that item; it never hangs or crashes the run.
//! * **Early exit.** When the consumer fails, the remaining items are abandoned and the error is
//!   returned once the running items finish.

use std::any::Any;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

/// How far ahead of the consumer the threads may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Lookahead {
    /// The most items between the one the consumer waits for and the one a thread starts.
    pub(super) items: usize,
    /// The most summed weight of the items between them.
    pub(super) weight: u64,
}

/// What the consumer waits for and what is waiting for it.
struct State<T> {
    /// Finished items that the consumer has not taken yet, by index.
    ready: BTreeMap<usize, Result<T, String>>,
    /// The number of items the consumer has taken.
    consumed: usize,
    /// Set when the consumer failed, so that threads stop.
    abort: bool,
}

/// Everything the threads share.
struct Shared<'a, T> {
    /// The next item nobody has claimed.
    next: AtomicUsize,
    /// The results and the position of the consumer.
    state: Mutex<State<T>>,
    /// Signalled when a result arrives, when the consumer advances and when the run is aborted.
    changed: Condvar,
    /// The number of items.
    count: usize,
    /// `prefix[i]` is the summed weight of the items before `i`.
    prefix: Vec<u64>,
    /// How far ahead the threads may run.
    lookahead: Lookahead,
    /// The function that computes one item.
    work: &'a (dyn Fn(usize) -> T + Sync),
}

impl<T> Shared<'_, T> {
    /// Locks the state, recovering from a poisoned lock instead of failing.
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits for a signal, recovering from a poisoned lock instead of failing.
    fn wait<'g>(&self, guard: MutexGuard<'g, State<T>>) -> MutexGuard<'g, State<T>> {
        self.changed
            .wait(guard)
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether item `index` may start while the consumer waits for item `consumed`.
    fn within_window(&self, index: usize, consumed: usize) -> bool {
        if index <= consumed {
            return true;
        }
        let ahead = self.prefix[index] - self.prefix[consumed];
        index - consumed < self.lookahead.items && ahead <= self.lookahead.weight
    }

    /// Computes one item, turning a panic into the message of the panic.
    fn compute(&self, index: usize) -> Result<T, String> {
        catch_unwind(AssertUnwindSafe(|| (self.work)(index)))
            .map_err(|payload| panic_message(&*payload))
    }

    /// Stores a finished item and wakes whoever waits.
    fn publish(&self, index: usize, result: Result<T, String>) {
        self.lock().ready.insert(index, result);
        self.changed.notify_all();
    }

    /// The loop of a spawned thread: claim, wait for the window, compute, publish.
    fn run_worker(&self) {
        loop {
            let index = self.next.fetch_add(1, Ordering::SeqCst);
            if index >= self.count {
                return;
            }
            {
                let mut state = self.lock();
                while !state.abort && !self.within_window(index, state.consumed) {
                    state = self.wait(state);
                }
                if state.abort {
                    return;
                }
            }
            let result = self.compute(index);
            self.publish(index, result);
        }
    }

    /// Tells every thread to stop.
    fn abort(&self) {
        self.lock().abort = true;
        self.changed.notify_all();
    }
}

/// The best message that can be recovered from the payload of a panic.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned())
}

/// What the consumer loop does next.
enum Step<T> {
    /// Hand this result to the consumer.
    Deliver(Result<T, String>),
    /// Compute this item on the calling thread.
    Compute(usize),
    /// Nothing to do until something changes.
    Wait,
}

/// Runs `work` for every item of `weights` on `threads` threads (the calling thread included) and
/// gives each result to `consume` in item order.
///
/// `weights[i]` is the weight of item `i` for the look-ahead limit, and its length is the number of
/// items. A panic in `work` reaches `consume` as `Err(message)` for that item. If `consume` fails,
/// its error is returned and the items that have not started are skipped.
///
/// # Errors
/// Returns the first error `consume` returns.
pub(super) fn ordered_parallel<T, E>(
    weights: &[u64],
    threads: usize,
    lookahead: Lookahead,
    work: &(dyn Fn(usize) -> T + Sync),
    mut consume: impl FnMut(usize, Result<T, String>) -> Result<(), E>,
) -> Result<(), E>
where
    T: Send,
{
    let count = weights.len();
    let mut prefix = Vec::with_capacity(count + 1);
    let mut sum = 0_u64;
    prefix.push(0);
    for weight in weights {
        sum = sum.saturating_add(*weight);
        prefix.push(sum);
    }
    let shared = Shared {
        next: AtomicUsize::new(0),
        state: Mutex::new(State {
            ready: BTreeMap::new(),
            consumed: 0,
            abort: false,
        }),
        changed: Condvar::new(),
        count,
        prefix,
        lookahead,
        work,
    };
    let helpers = threads.max(1).min(count.max(1)) - 1;
    thread::scope(|scope| {
        for _ in 0..helpers {
            let builder = thread::Builder::new()
                .name("pn-ultramemory-index".to_owned())
                .stack_size(WORKER_STACK_BYTES);
            if builder.spawn_scoped(scope, || shared.run_worker()).is_err() {
                break;
            }
        }
        let outcome = consume_in_order(&shared, &mut consume);
        if outcome.is_err() {
            shared.abort();
        }
        outcome
    })
}

/// The stack of a spawned thread: larger than the default, because parsers can nest deeply.
const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

/// The loop of the calling thread: deliver results in order, and compute items while waiting.
fn consume_in_order<T, E>(
    shared: &Shared<'_, T>,
    consume: &mut impl FnMut(usize, Result<T, String>) -> Result<(), E>,
) -> Result<(), E> {
    let mut position = 0;
    while position < shared.count {
        let step = {
            let mut state = shared.lock();
            if let Some(result) = state.ready.remove(&position) {
                state.consumed = position + 1;
                Step::Deliver(result)
            } else {
                let candidate = shared.next.load(Ordering::SeqCst);
                let claimed = candidate < shared.count
                    && shared.within_window(candidate, position)
                    && shared
                        .next
                        .compare_exchange(
                            candidate,
                            candidate + 1,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_ok();
                if claimed {
                    Step::Compute(candidate)
                } else {
                    drop(shared.wait(state));
                    Step::Wait
                }
            }
        };
        match step {
            Step::Deliver(result) => {
                shared.changed.notify_all();
                consume(position, result)?;
                position += 1;
            }
            Step::Compute(index) => {
                let result = shared.compute(index);
                shared.publish(index, result);
            }
            Step::Wait => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Lookahead, ordered_parallel};

    /// A limit generous enough not to matter.
    const WIDE: Lookahead = Lookahead {
        items: usize::MAX,
        weight: u64::MAX,
    };

    /// A small deterministic amount of busy work that varies with the item.
    fn busy(index: usize) -> usize {
        let mut value = index.wrapping_mul(2_654_435_761) % 1000;
        for _ in 0..(value % 200) {
            value = value.wrapping_mul(31).wrapping_add(7) % 100_003;
        }
        value
    }

    /// Results arrive in item order whatever the number of threads and however uneven the work.
    #[test]
    fn results_arrive_in_order() {
        for threads in [1, 2, 3, 8, 32] {
            let weights = vec![1_u64; 500];
            let mut seen = Vec::new();
            let outcome: Result<(), ()> = ordered_parallel(
                &weights,
                threads,
                WIDE,
                &|index| (index, busy(index)),
                |position, result| {
                    let (index, _) = result.expect("no panic");
                    assert_eq!(index, position);
                    seen.push(index);
                    Ok(())
                },
            );
            assert!(outcome.is_ok());
            assert_eq!(seen, (0..500).collect::<Vec<_>>(), "threads {threads}");
        }
    }

    /// No items at all is a successful, empty run.
    #[test]
    fn empty_input_is_fine() {
        let outcome: Result<(), ()> =
            ordered_parallel(&[], 8, WIDE, &|index: usize| index, |_, _| Ok(()));
        assert!(outcome.is_ok());
    }

    /// The look-ahead limit keeps the threads within a fixed number of items of the consumer.
    #[test]
    fn lookahead_bounds_the_work_in_flight() {
        let consumed = AtomicUsize::new(0);
        let worst = AtomicUsize::new(0);
        let weights = vec![1_u64; 400];
        let limit = Lookahead {
            items: 6,
            weight: u64::MAX,
        };
        let outcome: Result<(), ()> = ordered_parallel(
            &weights,
            8,
            limit,
            &|index| {
                let ahead = index.saturating_sub(consumed.load(Ordering::SeqCst));
                worst.fetch_max(ahead, Ordering::SeqCst);
                busy(index)
            },
            |_, _| {
                consumed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        );
        assert!(outcome.is_ok());
        assert!(worst.load(Ordering::SeqCst) <= 7, "{worst:?}");
        assert_eq!(consumed.load(Ordering::SeqCst), 400);
    }

    /// The weight limit also bounds the look-ahead, and a heavy item alone still makes progress.
    #[test]
    fn weight_limit_still_makes_progress() {
        let weights: Vec<u64> = (0..100)
            .map(|i| if i % 10 == 0 { 1_000_000 } else { 10 })
            .collect();
        let limit = Lookahead {
            items: 1000,
            weight: 100,
        };
        let mut count = 0;
        let outcome: Result<(), ()> =
            ordered_parallel(&weights, 4, limit, &|index| index, |_, _| {
                count += 1;
                Ok(())
            });
        assert!(outcome.is_ok());
        assert_eq!(count, 100);
    }

    /// A panic in the work function reaches the consumer as an error for that item only.
    #[test]
    fn a_panic_is_reported_for_its_item() {
        let weights = vec![1_u64; 50];
        let mut failed = Vec::new();
        let outcome: Result<(), ()> = ordered_parallel(
            &weights,
            4,
            WIDE,
            &|index| {
                assert!(index % 17 != 3, "boom {index}");
                index
            },
            |position, result| {
                if let Err(message) = result {
                    assert!(message.contains("boom"), "{message}");
                    failed.push(position);
                }
                Ok(())
            },
        );
        assert!(outcome.is_ok());
        assert_eq!(failed, vec![3, 20, 37]);
    }

    /// An error from the consumer stops the run and is returned, without hanging.
    #[test]
    fn a_consumer_error_stops_the_run() {
        let weights = vec![1_u64; 10_000];
        let started = AtomicUsize::new(0);
        let outcome: Result<(), usize> = ordered_parallel(
            &weights,
            8,
            Lookahead {
                items: 16,
                weight: u64::MAX,
            },
            &|index| {
                started.fetch_add(1, Ordering::SeqCst);
                busy(index)
            },
            |position, _| if position == 5 { Err(position) } else { Ok(()) },
        );
        assert_eq!(outcome, Err(5));
        assert!(started.load(Ordering::SeqCst) < 200);
    }
}
