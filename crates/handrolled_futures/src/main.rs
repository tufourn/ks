use std::{
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

#[allow(unused)]
use std::marker::PhantomPinned;

const TIMEOUT_MILLIS: u64 = 100;

#[tokio::main]
async fn main() {
    println!("---");
    {
        println!("foo() returned {}", foo().await);
        println!("foo_desugared() returned {}", foo_desugared().await);
    }
    println!("---");
    {
        let start = Instant::now();
        let result = bar().await;
        let elapsed = start.elapsed();
        println!("bar() returned {} after {:.3?}", result, elapsed);
    }
    {
        let start = Instant::now();
        let result = bar_desugared().await;
        let elapsed = start.elapsed();
        println!("bar_desugared() returned {} after {:.3?}", result, elapsed);
    }
    println!("---");
    {
        let start = Instant::now();
        let result = baz().await;
        let elapsed = start.elapsed();
        println!("baz() returned {} after {:.3?}", result, elapsed);
    }
    {
        let start = Instant::now();
        let result = baz_desugared().await;
        let elapsed = start.elapsed();
        println!("baz_desugared() returned {} after {:.3?}", result, elapsed);
    }
    println!("---");
    println!(
        "unsound_unpin_await() returned {}",
        unsound_unpin_await().await
    );
    println!("---");
    unsound_unpin_manual_poll();
    println!("---");
    unsound_unpin_ub();
    println!("---");
}

// --- trivial future that's instantly ready
async fn foo() -> i32 {
    69
}

fn foo_desugared() -> impl Future<Output = i32> {
    FooFuture
}

struct FooFuture;

impl Future for FooFuture {
    type Output = i32;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(69)
    }
}

// --- simple future with an await/yield point
async fn bar() -> i32 {
    tokio::time::sleep(Duration::from_millis(TIMEOUT_MILLIS)).await;
    69
}

fn bar_desugared() -> impl Future<Output = i32> {
    BarFuture {
        state: BarState::Start,
    }
}

struct BarFuture {
    state: BarState,
}

enum BarState {
    Start,
    Sleeping { sleep: tokio::time::Sleep },
    Done,
}

impl Future for BarFuture {
    type Output = i32;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Safety: not moving data out of this &mut
        let this: &mut BarFuture = unsafe { self.as_mut().get_unchecked_mut() };
        loop {
            match this.state {
                BarState::Start => {
                    let sleep = tokio::time::sleep(Duration::from_millis(TIMEOUT_MILLIS));
                    this.state = BarState::Sleeping { sleep };
                }
                BarState::Sleeping { ref mut sleep } => {
                    // Safety: sleep is not moved
                    let inner = unsafe { Pin::new_unchecked(sleep) };
                    match inner.poll(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(_) => {
                            this.state = BarState::Done;
                            return Poll::Ready(69);
                        }
                    }
                }
                BarState::Done => panic!("futures should not be polled after completion"),
            }
        }
    }
}

// future with self references
async fn baz() -> i32 {
    let s = String::from("420");
    let s_ref: &String = &s; // borrow created before .await
    tokio::time::sleep(Duration::from_millis(TIMEOUT_MILLIS)).await;
    let s_i32: i32 = s_ref.parse().unwrap(); // borrow used after .await
    s_i32 * 100 + 69
}

fn baz_desugared() -> impl Future<Output = i32> {
    BazFuture {
        state: BazState::Start,
    }
}

struct BazFuture {
    state: BazState,
}

enum BazState {
    Start,
    Waiting {
        #[allow(dead_code)]
        s: String,
        // Sleep is !Unpin, so BazState and BazFuture is also !Unpin
        // s_ptr is created after s is placed in the enum variant,
        // and since BazFuture is pinned (and !Unpin), s won't move again,
        // keeping s_ptr valid
        s_ptr: *const String,
        sleep: tokio::time::Sleep,
    },
    Done,
}

impl Future for BazFuture {
    type Output = i32;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Safety: not moving data out of this &mut
        let this = unsafe { self.as_mut().get_unchecked_mut() };

        // we'd break the pinning contract if we do something like this
        //
        // let other = BazFuture {
        //     state: BazState::Start,
        // };
        // let _ = std::mem::replace(this, other);

        loop {
            match this.state {
                BazState::Start => {
                    let s = String::from("420");
                    let sleep = tokio::time::sleep(Duration::from_millis(TIMEOUT_MILLIS));

                    // we'd get UB if we do this, because s may be moved during the
                    // construction of state, and s_ptr would then point to the wrong location
                    //
                    // let s_ptr = &s as *const String;
                    // this.state = BazState::Waiting { s, s_ptr, sleep };

                    // must assign s_ptr to point to s after the state is constructed
                    // to ensure s_ptr points to the final location of s
                    this.state = BazState::Waiting {
                        s,
                        s_ptr: std::ptr::null(), // dummy, to be reassigned to point to s
                        sleep,
                    };
                    if let BazState::Waiting { s, s_ptr, .. } = &mut this.state {
                        *s_ptr = s as *const String;
                    } else {
                        unreachable!("state must be BazState::Waiting")
                    }
                }
                BazState::Waiting {
                    s_ptr,
                    ref mut sleep,
                    ..
                } => {
                    // Safety: sleep is not moved
                    let sleep = unsafe { Pin::new_unchecked(sleep) };
                    match sleep.poll(cx) {
                        Poll::Ready(_) => {
                            // Safety: s_ptr is a pointer to the string s
                            let s_ref: &String = unsafe { &*s_ptr };
                            let s_i32: i32 = s_ref.parse().unwrap();
                            this.state = BazState::Done;
                            return Poll::Ready(s_i32 * 100 + 69);
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
                BazState::Done => panic!("futures should not be polled after completion"),
            }
        }
    }
}

// future that unsoundly implements Unpin, leading to UB
// futures that contain self references must be !Unpin so its pointers stay valid
struct BazFutureUnpin {
    state: BazStateUnpin,
    // we can use PhantomPinned to make this struct !Unpin
    // and the compiler will disallow us from shooting ourself in the foot
    // _marker: PhantomPinned,
}

enum BazStateUnpin {
    Start,
    Waiting {
        s: String,
        s_ptr: *const String,
        yield_once: YieldOnce,
    },
    Done,
}

impl Future for BazFutureUnpin {
    type Output = i32;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Pin<&mut T> is the same as &mut T, if T is Unpin
        // because BazFutureUnpin naturally (and unsoundly) implements Unpin, we can get a &mut without unsafe
        let this: &mut Self = self.as_mut().get_mut();
        loop {
            match this.state {
                BazStateUnpin::Start => {
                    let s = String::from("420");
                    let yield_once = YieldOnce(false);
                    this.state = BazStateUnpin::Waiting {
                        s,
                        s_ptr: std::ptr::null(),
                        yield_once,
                    };
                    if let BazStateUnpin::Waiting { s, s_ptr, .. } = &mut this.state {
                        *s_ptr = s as *const String;
                    }
                }
                BazStateUnpin::Waiting {
                    s_ptr,
                    ref mut yield_once,
                    ..
                } => {
                    let yield_once = Pin::new(yield_once);
                    match yield_once.poll(cx) {
                        Poll::Ready(_) => {
                            // this is the only unsafe block for this future
                            let s_ref: &String = unsafe { &*s_ptr };
                            let s_i32: i32 = s_ref.parse().unwrap();
                            this.state = BazStateUnpin::Done;
                            return Poll::Ready(s_i32 * 100 + 69);
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
                BazStateUnpin::Done => {
                    panic!("futures should not be polled after completion")
                }
            }
        }
    }
}

// a future that returns Pending on first poll() and returns Ready(()) on the second poll
struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();

    // first poll always returns Pending, second poll returns Ready
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // this compiles, because YieldOnce is Unpin
        let _this: &mut Self = self.as_mut().get_mut();

        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

async fn unsound_unpin_await() -> i32 {
    BazFutureUnpin {
        state: BazStateUnpin::Start,
    }
    .await
}

// this works, because we're not swapping out the future
fn unsound_unpin_manual_poll() {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);

    let mut fut = BazFutureUnpin {
        state: BazStateUnpin::Start,
    };

    let mut p = Pin::new(&mut fut);

    let first_poll = p.as_mut().poll(&mut cx);
    assert!(matches!(first_poll, Poll::Pending));

    let second_poll = p.as_mut().poll(&mut cx);
    assert!(matches!(second_poll, Poll::Ready(_)));

    if let Poll::Ready(res) = second_poll {
        println!("unsound_unpin_manual_poll() returned {res}");
    }
}

// we can get UB without any unsafe here, because we've violated the pinning contract
#[allow(dead_code)]
#[forbid(unsafe_code)]
fn unsound_unpin_ub() {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);

    let mut fut1 = BazFutureUnpin {
        state: BazStateUnpin::Start,
    };
    let mut fut2 = BazFutureUnpin {
        state: BazStateUnpin::Start,
    };

    // poll both: establishes s_ptr in each
    let p1 = Pin::new(&mut fut1).poll(&mut cx);
    let p2 = Pin::new(&mut fut2).poll(&mut cx);
    assert!(matches!(p1, Poll::Pending));
    assert!(matches!(p2, Poll::Pending));

    println!("before swap:");
    println!("  &fut1 = {:p}", &fut1);
    println!("  &fut2 = {:p}", &fut2);
    if let BazStateUnpin::Waiting { s, s_ptr, .. } = &fut1.state {
        println!("  fut1.s     = {:p}", s as *const String);
        println!("  fut1.s_ptr = {:p} (points to fut1.s)", *s_ptr);
    }
    if let BazStateUnpin::Waiting { s, s_ptr, .. } = &fut2.state {
        println!("  fut2.s     = {:p}", s as *const String);
        println!("  fut2.s_ptr = {:p} (points to fut2.s)", *s_ptr);
    }

    std::mem::swap(&mut fut1, &mut fut2);

    println!("after swap:");
    println!("  &fut1 = {:p} (remains unchanged)", &fut1);
    println!("  &fut2 = {:p} (remains unchanged)", &fut2);
    if let BazStateUnpin::Waiting { s, s_ptr, .. } = &fut1.state {
        println!("  fut1.s     = {:p}", s as *const String);
        println!("  fut1.s_ptr = {:p} (still points to fut2.s)", *s_ptr);
    }
    if let BazStateUnpin::Waiting { s, s_ptr, .. } = &fut2.state {
        println!("  fut2.s     = {:p}", s as *const String);
        println!("  fut2.s_ptr = {:p} (still points to fut1.s)", *s_ptr);
    }

    // fut1.s_ptr now dangles into fut2's memory
    if let Poll::Ready(res) = Pin::new(&mut fut1).poll(&mut cx) {
        println!("fut1 returned {res}");
    }

    // fut2.s_ptr now dangles into fut1's memory
    // fut1 has already transitioned to Done, fut1.s have been dropped,
    // so fut2.s_ptr points to garbage
    if let Poll::Ready(res) = Pin::new(&mut fut2).poll(&mut cx) {
        println!("fut2 returned {res}");
    }
}
