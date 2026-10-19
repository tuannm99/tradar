//! Takes a query string, hands it to the active `QueryDriver`, and
//! normalizes the result for the TUI to render. No database-specific logic
//! lives here. Implements `Session`: `submit_query` is the synchronous
//! command a `Screen` calls, which spawns the actual driver IO and reports
//! back through an internal channel that `tick()` drains. `tick()` also
//! fires a background `QueryDriver::ping()` every `PING_INTERVAL`, so
//! `alive()` reflects a dropped connection without the user having to run a
//! query into it first.

use std::sync::Arc;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use tradar_connector_spi::Session;
use tradar_core::action::{Action, Component};
use tradar_core::storage::SavedConnection;

use crate::components::query_screen::QueryScreenComponent;
use crate::query_driver::{QueryDriver, QueryResult, SchemaInfo};

/// Outcomes are drained at most this many at a time per `tick()` call, so a
/// connector that somehow floods this channel can never starve rendering.
/// In practice at most one outcome is ever in flight per `QueryEngine`
/// (there is no query pipelining), but the bound is kept for consistency
/// with the rest of the `Session::tick()` contract.
const MAX_DRAIN_PER_TICK: usize = 64;

/// How often `tick()` fires a background `QueryDriver::ping()`. The TUI
/// otherwise has no way to notice a connection dropped except a query
/// failing -- see "Trạng thái connection" in docs/backlog/features-batch-2026-08-14.md. Fifteen
/// seconds is often enough to catch a drop within the time it'd take to
/// notice by hand, and rare enough not to matter against a database with
/// connection logging or a low idle-connection limit.
const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// How often the results pane's running-query spinner glyph advances --
/// shared with `components::results`'s own spinner-frame calculation so
/// `tick()` knows the coarsest redraw cadence that still animates it
/// smoothly, rather than requesting a redraw on every ~50ms input-poll
/// cycle regardless of whether the glyph would actually look different.
pub(crate) const SPINNER_FRAME_MS: u128 = 80;

pub enum QueryOutcome {
    Completed { result: QueryResult },
    Failed { error: String },
}

struct TaggedOutcome {
    epoch: u64,
    outcome: QueryOutcome,
}

pub struct QueryEngine {
    driver: Arc<dyn QueryDriver>,
    connection: SavedConnection,
    schema: Result<Vec<SchemaInfo>, String>,
    history: Vec<String>,
    epoch: u64,
    pending: bool,
    /// The in-flight query's task, kept so it can be aborted. `None`
    /// whenever nothing is running.
    running: Option<tokio::task::JoinHandle<()>>,
    /// When the in-flight query was submitted, for the results pane's
    /// elapsed-time readout. `tokio::time::Instant` for the same reason as
    /// `last_ping` -- a paused clock in tests advances it.
    running_since: Option<tokio::time::Instant>,
    /// Which `SPINNER_FRAME_MS`-sized tick of `running_since` was last
    /// reported as a change, so `tick()` only asks for a redraw when the
    /// spinner glyph would actually look different -- not on every ~50ms
    /// input-poll cycle it happens to be called on while a query runs.
    last_reported_spinner_frame: Option<u128>,
    last_outcome: Option<QueryOutcome>,
    outcome_tx: UnboundedSender<TaggedOutcome>,
    outcome_rx: UnboundedReceiver<TaggedOutcome>,
    /// Whether the last background ping succeeded. Starts `true`: the
    /// connect that produced this engine already proved the backend was
    /// reachable, so there is nothing to doubt until a ping actually fails.
    alive: bool,
    /// `tokio::time::Instant` rather than `std::time::Instant` so a paused
    /// clock in tests advances it -- see the ping tests below.
    last_ping: tokio::time::Instant,
    ping_in_flight: bool,
    ping_tx: UnboundedSender<bool>,
    ping_rx: UnboundedReceiver<bool>,
}

impl QueryEngine {
    pub fn new(
        driver: Arc<dyn QueryDriver>,
        connection: SavedConnection,
        schema: Result<Vec<SchemaInfo>, String>,
    ) -> Self {
        let (outcome_tx, outcome_rx) = mpsc::unbounded_channel();
        let (ping_tx, ping_rx) = mpsc::unbounded_channel();
        Self {
            driver,
            connection,
            schema,
            history: Vec::new(),
            epoch: 0,
            pending: false,
            running: None,
            running_since: None,
            last_reported_spinner_frame: None,
            last_outcome: None,
            outcome_tx,
            outcome_rx,
            alive: true,
            last_ping: tokio::time::Instant::now(),
            ping_in_flight: false,
            ping_tx,
            ping_rx,
        }
    }

    /// The driver this engine runs queries through, for a headless host
    /// (`tradar-server`) that talks to it directly instead of through this
    /// engine's tick-driven submit/outcome loop.
    pub fn driver(&self) -> Arc<dyn QueryDriver> {
        Arc::clone(&self.driver)
    }

    pub fn connection(&self) -> &SavedConnection {
        &self.connection
    }

    pub fn schema(&self) -> &Result<Vec<SchemaInfo>, String> {
        &self.schema
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// How long the in-flight query has been running, for the results
    /// pane's elapsed-time readout. `None` when nothing is running.
    pub fn elapsed_running(&self) -> Option<std::time::Duration> {
        self.running_since.map(|since| since.elapsed())
    }

    /// Whether the most recent background ping succeeded -- see
    /// `PING_INTERVAL`. Not query-side: an editor left idle for minutes with
    /// a dropped connection still shows this without the user having to run
    /// anything.
    pub fn alive(&self) -> bool {
        self.alive
    }

    /// Whether this driver currently holds an open transaction -- see
    /// `QueryDriver::in_transaction`.
    pub fn in_transaction(&self) -> bool {
        self.driver.in_transaction()
    }

    /// Fires a `ping()` in the background, unless one is already in flight.
    /// Guarded by `ping_in_flight` rather than firing unconditionally on
    /// every interval tick, so a slow or hanging backend can't pile up
    /// concurrent pings.
    fn fire_ping(&mut self) {
        self.ping_in_flight = true;
        self.last_ping = tokio::time::Instant::now();
        let driver = Arc::clone(&self.driver);
        let tx = self.ping_tx.clone();
        tokio::spawn(async move {
            let alive = driver.ping().await.is_ok();
            let _ = tx.send(alive);
        });
    }

    /// The statements in `text`, per this driver's own rules -- see
    /// `QueryDriver::split_statements`.
    pub fn split_statements(&self, text: &str) -> Vec<crate::query_driver::Statement> {
        self.driver.split_statements(text)
    }

    /// The driver's own completion vocabulary -- see
    /// `QueryDriver::keywords`.
    pub fn keywords(&self) -> &'static [&'static str] {
        self.driver.keywords()
    }

    pub fn export_curl(&self, query: &str) -> Option<String> {
        self.driver.export_curl(query)
    }

    /// The table `query` reads from, when this driver can say -- see
    /// `QueryDriver::edit_source`.
    pub fn edit_source(&self, query: &str) -> Option<String> {
        self.driver.edit_source(query)
    }

    /// This driver's statement for `edit` -- see `QueryDriver::edit_sql`.
    pub fn edit_sql(&self, edit: &crate::query_driver::RowEdit) -> Option<String> {
        self.driver.edit_sql(edit)
    }

    /// This driver's statement for `op` -- see `QueryDriver::table_ddl`.
    pub fn table_ddl(&self, op: &crate::query_driver::TableDesignerOp) -> Option<String> {
        self.driver.table_ddl(op)
    }

    /// Whether the migrations panel can run here -- see
    /// `QueryDriver::supports_migrations`.
    pub fn supports_migrations(&self) -> bool {
        self.driver.supports_migrations()
    }

    /// The key column(s) for `source` -- see `QueryDriver::edit_key_columns`.
    pub fn edit_key_columns(&self, source: &str) -> Option<Vec<String>> {
        self.driver.edit_key_columns(source)
    }

    /// A skeleton statement for `op` against the schema entry named
    /// `name`, restricted to `columns` -- see `Component::crud_snippet`.
    /// `None` when `name` isn't a known entry (schema failed to load, or
    /// it's stale) or the driver has nothing to say for it.
    pub fn crud_snippet(
        &self,
        name: &str,
        op: tradar_core::action::CrudOp,
        columns: &[String],
    ) -> Option<String> {
        let schema = self.schema.as_ref().ok()?;
        let entry = schema.iter().find(|entry| entry.name == name)?;
        self.driver.crud_snippet(entry, op, columns)
    }

    /// Spawns the actual query execution and returns immediately -- the
    /// synchronous command a `Screen` calls from `handle_key_event`. Bumps
    /// an internal epoch so a reply from a superseded call (unreachable
    /// today since queries don't pipeline, but kept for the same reason as
    /// `MAX_DRAIN_PER_TICK`) is dropped instead of overwriting a newer one.
    pub fn submit_query(&mut self, query: String) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.pending = true;
        self.running_since = Some(tokio::time::Instant::now());
        self.last_reported_spinner_frame = None;
        self.history.push(query.clone());

        let driver = Arc::clone(&self.driver);
        let tx = self.outcome_tx.clone();
        self.running = Some(tokio::spawn(async move {
            let outcome = match driver.execute(&query).await {
                Ok(result) => QueryOutcome::Completed { result },
                Err(e) => QueryOutcome::Failed {
                    error: e.to_string(),
                },
            };
            let _ = tx.send(TaggedOutcome { epoch, outcome });
        }));
    }

    /// Spawns `driver.browse_entry(&entry)` -- the browse sidebar's Enter
    /// action, reported back through the same outcome channel/epoch as
    /// `submit_query`. Unlike `submit_query`, this does **not** push into
    /// `history`: a sidebar click isn't a command the user typed and might
    /// want to recall with Ctrl+R.
    /// The literal command `submit_browse(entry)` will run, for the browse
    /// sidebar to echo -- see `QueryDriver::browse_command`.
    pub fn browse_command(&self, entry: &SchemaInfo) -> Option<String> {
        self.driver.browse_command(entry)
    }

    pub fn submit_browse(&mut self, entry: SchemaInfo) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.pending = true;
        self.running_since = Some(tokio::time::Instant::now());
        self.last_reported_spinner_frame = None;

        let driver = Arc::clone(&self.driver);
        let tx = self.outcome_tx.clone();
        self.running = Some(tokio::spawn(async move {
            let outcome = match driver.browse_entry(&entry).await {
                Ok(result) => QueryOutcome::Completed { result },
                Err(e) => QueryOutcome::Failed {
                    error: e.to_string(),
                },
            };
            let _ = tx.send(TaggedOutcome { epoch, outcome });
        }));
    }

    /// Runs `statements` one after another, reporting the last result --
    /// or the first failure, since carrying on after an error would apply
    /// half a script and hide which half.
    pub fn submit_all(&mut self, statements: Vec<String>) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.pending = true;
        self.running_since = Some(tokio::time::Instant::now());
        self.last_reported_spinner_frame = None;
        for statement in &statements {
            self.history.push(statement.clone());
        }

        let driver = Arc::clone(&self.driver);
        let tx = self.outcome_tx.clone();
        self.running = Some(tokio::spawn(async move {
            let total = statements.len();
            let mut last = None;
            let mut failure = None;
            for (index, statement) in statements.into_iter().enumerate() {
                match driver.execute(&statement).await {
                    Ok(result) => last = Some(result),
                    Err(e) => {
                        failure = Some(format!("statement {} of {total} failed: {e}", index + 1));
                        break;
                    }
                }
            }
            let outcome = match (failure, last) {
                (Some(error), _) => QueryOutcome::Failed { error },
                (None, Some(result)) => QueryOutcome::Completed { result },
                (None, None) => QueryOutcome::Failed {
                    error: "nothing to run".to_string(),
                },
            };
            let _ = tx.send(TaggedOutcome { epoch, outcome });
        }));
    }

    /// Abandons the running query, if any. Aborting the task drops the
    /// driver future -- on its own, not a reliable cancel (dropping a
    /// connection mid-query doesn't make most databases stop running it
    /// quickly; Postgres in particular keeps going until it next tries to
    /// write to the socket). Also fires `QueryDriver::cancel_query` --
    /// best-effort, driver-specific, fire-and-forget since this method is
    /// sync -- for the drivers that can reach into the database and really
    /// stop it (Postgres, SQLite); every other driver's default no-op
    /// leaves behavior exactly as before this existed. The epoch bump means
    /// a reply that was already in flight is ignored rather than landing
    /// after the fact.
    pub fn cancel(&mut self) -> bool {
        let Some(handle) = self.running.take() else {
            return false;
        };
        handle.abort();
        let driver = Arc::clone(&self.driver);
        tokio::spawn(async move {
            let _ = driver.cancel_query().await;
        });
        self.epoch += 1;
        self.pending = false;
        self.running_since = None;
        self.last_reported_spinner_frame = None;
        self.last_outcome = None;
        true
    }

    /// The most recent outcome for the in-flight query, if `tick()` picked
    /// one up since the last call. Consumes it -- calling twice in a row
    /// without an intervening `tick()` returns `None` the second time.
    pub fn take_outcome(&mut self) -> Option<QueryOutcome> {
        self.last_outcome.take()
    }
}

impl Session for QueryEngine {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn tick(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..MAX_DRAIN_PER_TICK {
            match self.outcome_rx.try_recv() {
                Ok(tagged) if tagged.epoch == self.epoch => {
                    self.pending = false;
                    self.running = None;
                    self.running_since = None;
                    self.last_reported_spinner_frame = None;
                    self.last_outcome = Some(tagged.outcome);
                    changed = true;
                }
                Ok(_stale) => {}
                Err(_) => break,
            }
        }

        // At most one ping is ever in flight, so a single non-blocking
        // check is enough -- unlike the outcome channel above, there's
        // nothing to drain in a loop.
        if let Ok(alive) = self.ping_rx.try_recv() {
            self.ping_in_flight = false;
            if alive != self.alive {
                self.alive = alive;
                changed = true;
            }
        }
        if !self.ping_in_flight && self.last_ping.elapsed() >= PING_INTERVAL {
            self.fire_ping();
        }

        // Redraw while a query is running, not just when an outcome lands
        // -- the results pane's spinner/elapsed-time readout has nothing
        // else to drive its animation. Gated on the spinner's own frame
        // advancing (not every tick) so a slow query doesn't force a
        // redraw on every ~50ms input-poll cycle when the glyph would
        // look identical to the last one drawn.
        if self.pending
            && let Some(since) = self.running_since
        {
            let frame = since.elapsed().as_millis() / SPINNER_FRAME_MS;
            if self.last_reported_spinner_frame != Some(frame) {
                self.last_reported_spinner_frame = Some(frame);
                changed = true;
            }
        }

        changed
    }

    fn build_screen(
        self: Box<Self>,
        action_tx: UnboundedSender<Action>,
        restore: Option<&str>,
    ) -> Box<dyn Component> {
        let mut screen = QueryScreenComponent::new(*self, action_tx);
        screen
            .query_editor
            .set_vim_enabled(tradar_core::config::vim_mode());
        if let Some(text) = restore {
            screen.query_editor.set_text(text);
        }
        Box::new(screen)
    }
}

#[cfg(test)]
mod tests;
