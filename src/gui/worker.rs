//! Runs keyboard I/O off the UI thread, one job at a time; results come back
//! to the UI thread in order.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::thread;

use gtk::glib;

type Job = Box<dyn FnOnce() + Send>;

pub struct Worker {
    tx: mpsc::Sender<Job>,
}

impl Worker {
    pub fn new() -> Worker {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::spawn(move || {
            for job in rx {
                let _ = catch_unwind(AssertUnwindSafe(job));
            }
        });
        Worker { tx }
    }

    pub fn submit<R: Send + 'static>(&self, job: impl FnOnce() -> R + Send + 'static, done: impl FnOnce(R) + 'static) {
        let (tx, rx) = async_channel::bounded(1);
        let _ = self.tx.send(Box::new(move || {
            let _ = tx.send_blocking(job());
        }));
        glib::spawn_future_local(async move {
            if let Ok(result) = rx.recv().await {
                done(result);
            }
        });
    }

    /// Fire and forget.
    pub fn run(&self, job: impl FnOnce() + Send + 'static) {
        let _ = self.tx.send(Box::new(job));
    }
}
