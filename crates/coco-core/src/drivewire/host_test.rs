use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

fn value_job(value: u8) -> HostJob {
    Box::new(move |_| Ok(vec![value]))
}

#[test]
fn saturation_returns_the_unaccepted_job_for_retry() {
    let (mut executor, host) = HostExecutor::manual(1);
    executor.submit(value_job(1)).unwrap();

    let SubmitError::Full(job) = executor.submit(value_job(2)).unwrap_err() else {
        panic!("expected full queue");
    };
    let first = host.take_request().unwrap();
    executor.submit(job).unwrap();

    host.complete(first.run()).unwrap();
    assert_eq!(executor.poll().unwrap().result.unwrap(), vec![1]);
    let second = host.take_request().unwrap();
    host.complete(second.run()).unwrap();
    assert_eq!(executor.poll().unwrap().result.unwrap(), vec![2]);
    assert_eq!(executor.diagnostics().backpressure, 1);
}

#[test]
fn cancellation_rejects_late_results_and_changes_request_ids() {
    let (mut executor, host) = HostExecutor::manual(2);
    let ran = Arc::new(AtomicBool::new(false));
    let ran_in_job = Arc::clone(&ran);
    let old_id = executor
        .submit(Box::new(move |_| {
            ran_in_job.store(true, Ordering::Release);
            Ok(vec![1])
        }))
        .unwrap();
    let old = host.take_request().unwrap();
    executor.cancel();
    let new_id = executor.submit(value_job(2)).unwrap();
    assert_ne!(old_id, new_id);

    host.complete(old.run()).unwrap();
    assert!(executor.poll().is_none());
    assert!(!ran.load(Ordering::Acquire));
    assert!(!executor.is_idle());

    let new = host.take_request().unwrap();
    host.complete(new.run()).unwrap();
    assert_eq!(executor.poll().unwrap().id, new_id);
    assert!(executor.is_idle());
    assert_eq!(executor.diagnostics().stale, 1);
    assert_eq!(executor.diagnostics().cancelled, 1);
}

#[test]
fn selective_cancellation_preserves_unrelated_work() {
    let ran = Arc::new(AtomicBool::new(false));
    let ran_in_job = Arc::clone(&ran);
    let (mut executor, host) = HostExecutor::manual(2);
    let cancelled_id = executor
        .submit(Box::new(move |_| {
            ran_in_job.store(true, Ordering::Release);
            Ok(vec![1])
        }))
        .unwrap();
    let kept_id = executor.submit(value_job(2)).unwrap();
    let cancelled = host.take_request().unwrap();
    let kept = host.take_request().unwrap();

    assert!(executor.cancel_request(cancelled_id));
    assert!(!executor.cancel_request(cancelled_id));
    host.complete(cancelled.run()).unwrap();
    host.complete(kept.run()).unwrap();

    let completion = executor.poll().unwrap();
    assert_eq!(completion.id, kept_id);
    assert_eq!(completion.result.unwrap(), vec![2]);
    assert!(!ran.load(Ordering::Acquire));
    assert!(executor.is_idle());
    assert_eq!(executor.diagnostics().cancelled, 1);
}

#[test]
fn selective_cancellation_suppresses_an_already_queued_result() {
    let (mut executor, host) = HostExecutor::manual(1);
    let id = executor.submit(value_job(1)).unwrap();
    let completion = host.take_request().unwrap().run();
    host.complete(completion).unwrap();

    assert!(executor.cancel_request(id));
    assert!(executor.poll().is_none());
    assert!(executor.is_idle());
}

#[test]
fn manual_completion_backpressure_returns_the_result() {
    let (mut executor, host) = HostExecutor::manual(2);
    executor.submit(value_job(1)).unwrap();
    executor.submit(value_job(2)).unwrap();
    let first = host.take_request().unwrap().run();
    let second = host.take_request().unwrap().run();
    host.complete(first).unwrap();
    host.complete(second).unwrap();

    executor.submit(value_job(3)).unwrap();
    let third = host.take_request().unwrap().run();
    let third = host.complete(third).unwrap_err();
    assert_eq!(executor.poll().unwrap().result.unwrap(), vec![1]);
    host.complete(third).unwrap();
}

#[test]
fn total_outstanding_is_bounded_when_manual_host_holds_requests() {
    let (mut executor, host) = HostExecutor::manual(1);
    let mut held = Vec::new();
    for value in 0..3 {
        executor.submit(value_job(value)).unwrap();
        held.push(host.take_request().unwrap());
    }

    assert!(matches!(
        executor.submit(value_job(3)),
        Err(SubmitError::Full(_))
    ));
    assert_eq!(executor.diagnostics().outstanding, 3);
    assert_eq!(executor.diagnostics().backpressure, 1);
    drop(held);
}

#[test]
fn foreign_and_duplicate_completions_cannot_underflow_outstanding() {
    let (mut old_executor, old_host) = HostExecutor::manual(1);
    old_executor.submit(value_job(1)).unwrap();
    let foreign = old_host.take_request().unwrap().run();

    let (mut executor, host) = HostExecutor::manual(2);
    host.complete(foreign).unwrap();
    assert!(executor.poll().is_none());
    assert_eq!(executor.diagnostics().outstanding, 0);

    executor.submit(value_job(2)).unwrap();
    let completion = host.take_request().unwrap().run();
    let duplicate_id = completion.id;
    host.complete(completion).unwrap();
    host.complete(HostCompletion {
        id: duplicate_id,
        result: Ok(vec![2]),
    })
    .unwrap();
    assert!(executor.poll().is_some());
    assert!(executor.poll().is_none());
    assert_eq!(executor.diagnostics().outstanding, 0);
    assert_eq!(executor.diagnostics().stale, 2);
}

#[test]
fn suspend_and_stop_invalidate_work_and_reject_submissions() {
    let (mut executor, host) = HostExecutor::manual(2);
    executor.submit(value_job(1)).unwrap();
    let suspended = host.take_request().unwrap();
    executor.suspend();
    assert!(matches!(
        executor.submit(value_job(2)),
        Err(SubmitError::Stopped(_))
    ));
    host.complete(suspended.run()).unwrap();
    assert!(executor.poll().is_none());

    executor.resume();
    executor.submit(value_job(3)).unwrap();
    let stopped = host.take_request().unwrap();
    executor.stop();
    host.complete(stopped.run()).unwrap();
    assert!(executor.poll().is_none());
    assert_eq!(executor.diagnostics().state, HostState::Stopped);
}

#[test]
fn oversized_responses_become_bounded_errors() {
    let (mut executor, host) = HostExecutor::manual(1);
    executor
        .submit(Box::new(|_| Ok(vec![0; MAX_HOST_RESPONSE_BYTES + 1])))
        .unwrap();
    let request = host.take_request().unwrap();
    host.complete(request.run()).unwrap();

    assert_eq!(
        executor.poll().unwrap().result,
        Err(HostError::ResponseTooLarge {
            size: MAX_HOST_RESPONSE_BYTES + 1,
            max: MAX_HOST_RESPONSE_BYTES,
        })
    );
}

#[test]
fn independent_worker_completes_while_another_is_delayed() {
    let (release_tx, release_rx) = mpsc::channel();
    let mut delayed = HostExecutor::new();
    delayed
        .submit(Box::new(move |_| {
            release_rx.recv().unwrap();
            Ok(vec![1])
        }))
        .unwrap();

    let mut ready = HostExecutor::new();
    ready.submit(value_job(2)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let completion = loop {
        if let Some(completion) = ready.poll() {
            break completion;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(completion.result.unwrap(), vec![2]);
    release_tx.send(()).unwrap();
}

#[test]
fn panicking_job_returns_error_and_worker_continues() {
    let mut executor = HostExecutor::new();
    executor
        .submit(Box::new(|_| panic!("host job failure")))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let completion = loop {
        if let Some(completion) = executor.poll() {
            break completion;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(completion.result, Err(HostError::Panicked));
    assert!(executor.is_idle());

    executor.submit(value_job(7)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let completion = loop {
        if let Some(completion) = executor.poll() {
            break completion;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(completion.result.unwrap(), vec![7]);
}

#[test]
fn dropping_executor_does_not_join_running_worker() {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut executor = HostExecutor::new();
    executor
        .submit(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(Vec::new())
        }))
        .unwrap();
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let start = Instant::now();
    drop(executor);
    assert!(start.elapsed() < Duration::from_millis(100));
    release_tx.send(()).unwrap();
}

#[test]
fn dropping_executor_cancels_a_held_request() {
    let ran = Arc::new(AtomicBool::new(false));
    let ran_in_job = Arc::clone(&ran);
    let (mut executor, host) = HostExecutor::manual(1);
    executor
        .submit(Box::new(move |_| {
            ran_in_job.store(true, Ordering::Release);
            Ok(Vec::new())
        }))
        .unwrap();
    let request = host.take_request().unwrap();

    drop(executor);
    assert_eq!(request.run().result, Err(HostError::Cancelled));
    assert!(!ran.load(Ordering::Acquire));
}
