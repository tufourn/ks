use std::{
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

const TIMEOUT_SEC: u64 = 1;

#[tokio::main]
async fn main() {
    println!("---");
    {
        println!("foo() returned {}", foo().await);
        println!("foo_desugared() returned {}", foo_desugared().await);
    }
    println!("---");
    {
        println!("Calling bar().await");
        let start = Instant::now();
        let result = bar().await;
        let elapsed = start.elapsed();
        println!("bar() returned {} after {:.3?}", result, elapsed);
    }
    println!("---");
    {
        println!("Calling bar_desugared().await");
        let start = Instant::now();
        let result = bar_desugared().await;
        let elapsed = start.elapsed();
        println!("bar_desugared() returned {} after {:.3?})", result, elapsed);
    }
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
    tokio::time::sleep(Duration::from_secs(TIMEOUT_SEC)).await;
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
                    let sleep = tokio::time::sleep(Duration::from_secs(TIMEOUT_SEC));
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
