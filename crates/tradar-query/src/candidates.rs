//! What completion offers for one connection: the driver's own vocabulary
//! (`QueryDriver::keywords`) plus the connected schema's table and column
//! names, and how a typed prefix is matched against them. No UI here -- the
//! TUI's popup and `tradar-server`'s `complete` both build on it.

use crate::{SchemaInfo, same_table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    Keyword,
    Table,
    Column,
}

impl CandidateKind {
    /// The one-letter tag shown beside a suggestion, so "id" as a column
    /// is distinguishable from "id" as a table.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Keyword => "k",
            Self::Table => "t",
            Self::Column => "c",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub text: String,
    pub kind: CandidateKind,
    /// `text.to_ascii_lowercase()`, computed once at build time rather
    /// than inside `matches()` -- which runs on every keystroke, over
    /// every candidate, and previously redid this same lowercasing twice
    /// per match (once to filter, once more in the sort key).
    text_lower: String,
}

impl Candidate {
    pub fn new(text: impl Into<String>, kind: CandidateKind) -> Self {
        let text = text.into();
        let text_lower = text.to_ascii_lowercase();
        Self {
            text,
            kind,
            text_lower,
        }
    }
}

/// Everything completable for the current connection, built once when the
/// screen opens rather than per keystroke.
#[derive(Debug, Default, Clone)]
pub struct CompletionSource {
    candidates: Vec<Candidate>,
    /// Kept alongside `candidates` (which flattens/dedupes across tables)
    /// for `matches_in_context`, which needs to know *which* table a
    /// column belongs to and which columns carry an FK -- neither survives
    /// the flattening above.
    schema: Vec<SchemaInfo>,
}

impl CompletionSource {
    pub fn new(keywords: &[&str], schema: &[SchemaInfo]) -> Self {
        let mut candidates: Vec<Candidate> = Vec::new();
        for name in keywords {
            candidates.push(Candidate::new(*name, CandidateKind::Keyword));
        }
        // The same column name usually appears in several tables; offering
        // it once keeps the list short. A set rather than an `.any()` scan
        // over the growing candidate list, which would make this quadratic
        // in the number of columns for a schema with many of them.
        let mut seen_columns: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in schema {
            candidates.push(Candidate::new(entry.name.clone(), CandidateKind::Table));
            for column in &entry.columns {
                if seen_columns.insert(column.name.clone()) {
                    candidates.push(Candidate::new(column.name.clone(), CandidateKind::Column));
                }
            }
        }
        Self {
            candidates,
            schema: schema.to_vec(),
        }
    }

    /// `matches`, but adjusted for the SQL context the cursor is sitting
    /// in -- `alias.` completes to that table's own columns instead of the
    /// flattened global list, and a table name typed right after `JOIN`
    /// ranks a table with an FK to one already in the query first. Falls
    /// back to `matches` for `CompletionContext::None`, and for a
    /// `TableColumns`/`JoinTarget` whose table doesn't resolve against the
    /// connected schema (nothing more specific to offer than the flat
    /// list).
    pub fn matches_in_context(
        &self,
        prefix: &str,
        context: &crate::CompletionContext,
    ) -> Vec<Candidate> {
        let needle = prefix.to_ascii_lowercase();
        match context {
            crate::CompletionContext::TableColumns { table } => {
                let Some(entry) = self.schema.iter().find(|e| same_table(table, &e.name)) else {
                    return self.matches(prefix);
                };
                let mut matches: Vec<Candidate> = entry
                    .columns
                    .iter()
                    .filter(|c| {
                        let lower = c.name.to_ascii_lowercase();
                        lower.starts_with(&needle) && lower != needle
                    })
                    .map(|c| Candidate::new(c.name.clone(), CandidateKind::Column))
                    .collect();
                matches.sort_by(|a, b| a.text_lower.cmp(&b.text_lower));
                matches
            }
            crate::CompletionContext::JoinTarget { known_tables } => {
                // A table is "related" if it has an FK to a known table,
                // or a known table has an FK to it -- either direction is
                // a real join condition, and there's no reason to only
                // offer one side of it.
                let related = |entry: &SchemaInfo| -> bool {
                    entry.columns.iter().any(|c| {
                        c.foreign_key
                            .as_ref()
                            .is_some_and(|fk| known_tables.iter().any(|t| same_table(t, &fk.table)))
                    }) || known_tables.iter().any(|t| {
                        self.schema
                            .iter()
                            .find(|e| same_table(t, &e.name))
                            .is_some_and(|kt| {
                                kt.columns.iter().any(|c| {
                                    c.foreign_key
                                        .as_ref()
                                        .is_some_and(|fk| same_table(&fk.table, &entry.name))
                                })
                            })
                    })
                };
                let mut matches: Vec<(bool, Candidate)> = self
                    .schema
                    .iter()
                    .filter(|e| {
                        let lower = e.name.to_ascii_lowercase();
                        lower.starts_with(&needle) && lower != needle
                    })
                    // A table already in the FROM/JOIN list isn't worth
                    // suggesting again -- a deliberate self-join still
                    // works, just by typing the full name rather than
                    // picking it from the popup.
                    .filter(|e| !known_tables.iter().any(|t| same_table(t, &e.name)))
                    .map(|e| {
                        (
                            related(e),
                            Candidate::new(e.name.clone(), CandidateKind::Table),
                        )
                    })
                    .collect();
                matches.sort_by(|(a_related, a), (b_related, b)| {
                    (!a_related, &a.text_lower).cmp(&(!b_related, &b.text_lower))
                });
                matches.into_iter().map(|(_, c)| c).collect()
            }
            crate::CompletionContext::MongoMethod => {
                // Narrowed, not just re-ranked: a collection or field name
                // is never a legal completion here, so there's no reason
                // to show one and bump the method name you're actually
                // typing further down the list.
                let mut matches: Vec<Candidate> = self
                    .candidates
                    .iter()
                    .filter(|c| {
                        c.kind == CandidateKind::Keyword
                            && c.text_lower.starts_with(&needle)
                            && c.text_lower != needle
                    })
                    .cloned()
                    .collect();
                matches.sort_by(|a, b| a.text_lower.cmp(&b.text_lower));
                matches
            }
            crate::CompletionContext::SchemaQualifiedTable { schema } => {
                let mut matches: Vec<Candidate> = self
                    .schema
                    .iter()
                    .filter_map(|e| {
                        let (entry_schema, table) = e.name.split_once('.')?;
                        if !entry_schema.eq_ignore_ascii_case(schema) {
                            return None;
                        }
                        let lower = table.to_ascii_lowercase();
                        (lower.starts_with(&needle) && lower != needle)
                            .then(|| Candidate::new(table, CandidateKind::Table))
                    })
                    .collect();
                // `schema` didn't resolve against any connected entry's own
                // qualifier (an unknown keyspace, or this driver doesn't
                // qualify names at all) -- same "nothing more specific to
                // offer than the flat list" fallback every other context
                // here uses.
                if matches.is_empty() {
                    return self.matches(prefix);
                }
                matches.sort_by(|a, b| a.text_lower.cmp(&b.text_lower));
                matches
            }
            crate::CompletionContext::None => self.matches(prefix),
        }
    }

    /// Matches for `prefix`, best first. Case-insensitive, because SQL is
    /// written in both cases and nobody wants to be told `select` doesn't
    /// match `SELECT`.
    ///
    /// Schema names rank above keywords: the keyword list is fixed and
    /// small, so the thing you can't guess -- your own table and column
    /// names -- is what's worth putting under the cursor.
    pub fn matches(&self, prefix: &str) -> Vec<Candidate> {
        if prefix.is_empty() {
            return Vec::new();
        }
        let needle = prefix.to_ascii_lowercase();
        let mut matches: Vec<&Candidate> = self
            .candidates
            .iter()
            .filter(|c| c.text_lower.starts_with(&needle) && c.text_lower != needle)
            .collect();
        matches.sort_by_key(|c| {
            let rank = match c.kind {
                CandidateKind::Table => 0,
                CandidateKind::Column => 1,
                CandidateKind::Keyword => 2,
            };
            (rank, c.text_lower.clone())
        });
        matches.into_iter().cloned().collect()
    }
}
