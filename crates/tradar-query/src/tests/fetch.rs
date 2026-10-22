use super::*;

#[tokio::test]
async fn the_row_budget_stops_at_the_row_cap_or_the_byte_cap() {
    // Without a scope: the library cap.
    assert_eq!(max_rows(), MAX_ROWS);

    with_max_rows(3, async {
        assert_eq!(max_rows(), 3);
        let mut budget = RowBudget::default();
        for _ in 0..3 {
            assert!(!budget.is_full());
            budget.keep(&["x".to_string()]);
        }
        assert!(budget.is_full(), "three rows is the cap");
    })
    .await;

    with_max_rows(1_000, async {
        let mut budget = RowBudget::default();
        budget.keep(&["x".repeat(MAX_RESULT_BYTES)]);
        assert!(budget.is_full(), "one huge row already fills the byte cap");
    })
    .await;
}
