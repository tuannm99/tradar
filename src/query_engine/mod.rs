//! Takes a query string, hands it to the active `Driver`, and normalizes
//! the result for the TUI to render. No database-specific logic lives here.

use crate::drivers::{Driver, QueryResult};

pub struct QueryEngine {
    driver: Box<dyn Driver>,
    history: Vec<String>,
}

impl QueryEngine {
    pub fn new(driver: Box<dyn Driver>) -> Self {
        Self {
            driver,
            history: Vec::new(),
        }
    }

    pub async fn run(&mut self, query: &str) -> anyhow::Result<QueryResult> {
        let result = self.driver.execute(query).await?;
        self.history.push(query.to_string());
        Ok(result)
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::{Driver, QueryResult, SchemaInfo};
    use async_trait::async_trait;

    struct FakeDriver {
        result: QueryResult,
    }

    #[async_trait]
    impl Driver for FakeDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }

        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Ok(QueryResult {
                columns: self.result.columns.clone(),
                rows: self.result.rows.clone(),
            })
        }
    }

    #[tokio::test]
    async fn run_delegates_to_the_active_driver() {
        let driver = FakeDriver {
            result: QueryResult {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            },
        };
        let mut engine = QueryEngine::new(Box::new(driver));

        let result = engine.run("SELECT id FROM users").await.unwrap();

        assert_eq!(result.columns, vec!["id"]);
        assert_eq!(result.rows, vec![vec!["1".to_string()]]);
    }

    #[tokio::test]
    async fn run_appends_the_query_to_history() {
        let driver = FakeDriver {
            result: QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
            },
        };
        let mut engine = QueryEngine::new(Box::new(driver));

        engine.run("SELECT 1").await.unwrap();
        engine.run("SELECT 2").await.unwrap();

        assert_eq!(engine.history(), &["SELECT 1", "SELECT 2"]);
    }
}
