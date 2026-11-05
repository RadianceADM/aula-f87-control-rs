//! Runs keyboard I/O off the UI thread, one job at a time; results come back
//! to the app on the UI thread in order (through the bridge).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::thread;

use windows_reactor::ApplicationContext;

use crate::app::{App, Msg};
use crate::bridge;

type Job = Box<dyn FnOnce() + Send>;

pub struct Worker {
    tx: mpsc::Sender<Job>,
}

impl Worker {
    pub fn new() -> Worker {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("aula-keyboard".into())
            .spawn(move || {
                for job in rx {
                    let _ = catch_unwind(AssertUnwindSafe(job));
                }
            })
            .expect("worker thread");
        Worker { tx }
    }

    pub fn submit<R: Send + 'static>(
        &self,
        job: impl FnOnce() -> R + Send + 'static,
        done: impl FnOnce(&mut App, &ApplicationContext<App>, R) + Send + 'static,
    ) {
        let _ = self.tx.send(Box::new(move || {
            let result = job();
            bridge::post(Msg::Done(Box::new(move |app, ctx| done(app, ctx, result))));
        }));
    }

    /// Fire and forget.
    pub fn run(&self, job: impl FnOnce() + Send + 'static) {
        let _ = self.tx.send(Box::new(job));
    }
}
