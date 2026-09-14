use super::*;

const TEST_OPERATION_INDEX: u64 = 7;
const TEST_DURATION: Duration = Duration::from_millis(125);
const TEST_ELAPSED: Duration = Duration::from_secs(2);

fn attempt(outcome: Result<(), String>) -> fixtures::OperationAttempt {
    fixtures::OperationAttempt {
        operation: fixtures::PerformedOperation {
            name: "test-operation",
            cycle: Some(2),
            cycle_step: Some(1),
            scroll: None,
        },
        duration: TEST_DURATION,
        outcome,
    }
}

#[test]
fn operation_event_records_exact_duration_and_success() {
    let event = operation_event(
        &attempt(Ok(())),
        TEST_OPERATION_INDEX,
        TEST_ELAPSED,
        true,
        false,
    );

    assert_eq!(event["name"], "test-operation");
    assert_eq!(event["operation"], TEST_OPERATION_INDEX);
    assert_eq!(event["duration_seconds"], TEST_DURATION.as_secs_f64());
    assert_eq!(event["success"], true);
    assert_eq!(event["outcome"], "success");
    assert_eq!(event["measurement_elapsed_seconds"], 2.0);
    assert_eq!(event["vm_live"], true);
    assert_eq!(event["suspended"], false);
}

#[test]
fn operation_event_records_error_outcome() {
    const ERROR: &str = "operation failed";
    let event = operation_event(
        &attempt(Err(ERROR.into())),
        TEST_OPERATION_INDEX,
        TEST_ELAPSED,
        false,
        true,
    );

    assert_eq!(event["duration_seconds"], TEST_DURATION.as_secs_f64());
    assert_eq!(event["success"], false);
    assert_eq!(event["outcome"], ERROR);
}
