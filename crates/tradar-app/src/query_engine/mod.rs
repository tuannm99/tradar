//! Takes a query string, hands it to the active `Driver`, and normalizes
//! the result for the TUI to render. No database-specific logic lives here.

use crate::drivers::{Driver, QueryResult, SchemaInfo};

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

    pub async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        self.driver.list_schema().await
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
        schema: Vec<SchemaInfo>,
    }

    #[async_trait]
    impl Driver for FakeDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(self.schema.clone())
        }

        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Ok(self.result.clone())
        }
    }

    #[tokio::test]
    async fn run_delegates_to_the_active_driver() {
        let driver = FakeDriver {
            result: QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            },
            schema: Vec::new(),
        };
        let mut engine = QueryEngine::new(Box::new(driver));

        let result = engine.run("SELECT id FROM users").await.unwrap();

        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            }
        );
    }

    #[tokio::test]
    async fn run_appends_the_query_to_history() {
        let driver = FakeDriver {
            result: QueryResult::Table {
                columns: Vec::new(),
                rows: Vec::new(),
            },
            schema: Vec::new(),
        };
        let mut engine = QueryEngine::new(Box::new(driver));

        engine.run("SELECT 1").await.unwrap();
        engine.run("SELECT 2").await.unwrap();

        assert_eq!(engine.history(), &["SELECT 1", "SELECT 2"]);
    }

    #[tokio::test]
    async fn list_schema_delegates_to_the_active_driver() {
        let driver = FakeDriver {
            result: QueryResult::Table {
                columns: Vec::new(),
                rows: Vec::new(),
            },
            schema: vec![SchemaInfo {
                name: "users".to_string(),
            }],
        };
        let engine = QueryEngine::new(Box::new(driver));

        let schema = engine.list_schema().await.unwrap();

        assert_eq!(
            schema,
            vec![SchemaInfo {
                name: "users".to_string()
            }]
        );
    }
}
