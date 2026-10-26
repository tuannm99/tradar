//! MongoDB connector: a minimal shell-subset parser for the literal shape
//! `db.<collection>.<method>(<json-args>)` with optional chained cursor
//! calls (`.find(...).sort(...).limit(...).skip(...)`, `.count()`), not a
//! real JS engine. Anything outside that shape — `$where`, arbitrary
//! expressions, a chain method this doesn't recognize — is rejected with a
//! clear error rather than guessed at. Two mongosh shell helpers are
//! special-cased on top of that: `use <db>` switches which database
//! `db.<collection>...` targets, and `show dbs`/`show databases` lists
//! every database on the server.

use std::sync::Arc;

use async_trait::async_trait;

use tradar_query::QueryDriver;

mod cheatsheet;
mod convert;
mod db_helpers;
mod driver;
mod keywords;
mod relax;
mod run;
mod shell;
mod snippets;

use driver::*;

#[cfg(feature = "tui")]
mod tui;
#[cfg(feature = "tui")]
pub use tui::connector;
// The crate's tests drive the TUI path too (`QueryEngine`, `SavedConnection`).
#[cfg(all(test, feature = "tui"))]
#[allow(unused_imports)]
use tui::*;

/// Opens a connected driver for `tradar-server` -- no TUI types involved.
pub fn opener() -> Box<dyn tradar_query::DriverOpener> {
    Box::new(Opener)
}

struct Opener;

#[async_trait]
impl tradar_query::DriverOpener for Opener {
    fn id(&self) -> &'static str {
        "mongo"
    }

    async fn open(&self, target: &str) -> anyhow::Result<Arc<dyn QueryDriver>> {
        let mut driver = MongoDriver::new(target);
        tradar_query::with_connect_timeout(target, driver.connect()).await?;
        Ok(Arc::new(driver))
    }
}

#[cfg(test)]
mod tests;
