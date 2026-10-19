/// How many rows a driver will materialise for one query. A result set is
/// pulled into memory to be rendered, so an unbounded `SELECT *` against a
/// large table would otherwise be a straightforward way to exhaust it --
/// see "Support large result sets efficiently" in CLAUDE.md. Drivers stop
/// reading at this point and report the result as truncated.
pub const MAX_ROWS: usize = 10_000;

/// Ceiling on the text a single result may hold, whatever the row cap says:
/// 10,000 rows of a few KB each (JSON or text columns) is hundreds of MB
/// that a grid will never show. Drivers stop reading at this point too and
/// report the result as truncated.
pub const MAX_RESULT_BYTES: usize = 64 * 1024 * 1024;

tokio::task_local! {
    static FETCH_ROWS: usize;
}

/// How many rows the query running in this task may materialise: whatever
/// [`with_max_rows`] set (the server asks for far fewer than [`MAX_ROWS`] by
/// default -- reading rows nobody looks at is where the time and the memory
/// go), else [`MAX_ROWS`] (the TUI, tests). Drivers call this instead of
/// reading the constant.
pub fn max_rows() -> usize {
    FETCH_ROWS.try_with(|n| *n).unwrap_or(MAX_ROWS)
}

/// Runs `fut` with the row cap set to `rows` (clamped to `1..=MAX_ROWS * 10`).
pub async fn with_max_rows<F: std::future::Future>(rows: usize, fut: F) -> F::Output {
    FETCH_ROWS.scope(rows.clamp(1, MAX_ROWS * 10), fut).await
}

/// Counts what a streaming driver has kept so far, and says when to stop:
/// at [`max_rows`] rows or [`MAX_RESULT_BYTES`] of text, whichever is first.
pub struct RowBudget {
    rows: usize,
    bytes: usize,
    max_rows: usize,
}

impl Default for RowBudget {
    fn default() -> Self {
        Self {
            rows: 0,
            bytes: 0,
            max_rows: max_rows(),
        }
    }
}

impl RowBudget {
    /// Whether one more row still fits (call *before* reading it; a driver
    /// reads one row past the cap only to learn that it was truncated).
    pub fn is_full(&self) -> bool {
        self.rows >= self.max_rows || self.bytes >= MAX_RESULT_BYTES
    }

    /// Records a row that was kept.
    pub fn keep(&mut self, row: &[String]) {
        self.rows += 1;
        self.bytes += row.iter().map(|cell| cell.len() + 24).sum::<usize>();
    }
}
