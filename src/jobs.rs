//! Small helper for running one-shot background jobs and polling results.
use std::sync::mpsc;

pub struct Job<T> {
    rx: Option<mpsc::Receiver<T>>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn<F>(f: F) -> Self
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        Job { rx: Some(rx) }
    }

    /// Returns the result once, without blocking; keeps polling until disconnected.
    pub fn poll(&mut self) -> Option<T> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                None
            }
        }
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }
}
