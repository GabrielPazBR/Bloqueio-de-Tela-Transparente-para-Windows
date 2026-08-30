pub fn run<T, W, C>(work: W, complete: C)
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
    C: FnOnce(T) + Send + 'static,
{
    std::thread::spawn(move || complete(work()));
}

#[cfg(test)]
mod tests {
    use super::run;
    use std::sync::{Arc, Condvar, Mutex, mpsc};
    use std::time::Duration;

    #[test]
    fn work_does_not_block_the_calling_thread() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_gate = gate.clone();
        let (sender, receiver) = mpsc::channel();

        run(
            move || {
                let (lock, condition) = &*worker_gate;
                let ready = lock.lock().expect("estado do teste");
                let _guard = condition
                    .wait_while(ready, |ready| !*ready)
                    .expect("espera do teste");
                42
            },
            move |value| sender.send(value).expect("resultado do teste"),
        );

        assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
        let (lock, condition) = &*gate;
        *lock.lock().expect("estado do teste") = true;
        condition.notify_one();
        assert_eq!(receiver.recv_timeout(Duration::from_secs(1)), Ok(42));
    }
}
