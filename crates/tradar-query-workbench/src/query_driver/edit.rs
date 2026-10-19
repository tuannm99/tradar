/// A change to one row the user already has on screen, described in terms
/// of the grid rather than any dialect's syntax: the screen fills this in
/// from what's selected, and the driver turns it into a statement it can
/// run (`QueryDriver::edit_sql`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowEdit {
    pub table: String,
    /// Column/value pairs that identify the row -- its primary key, taken
    /// from the row as fetched.
    pub key: Vec<(String, String)>,
    pub change: RowChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowChange {
    SetValue {
        column: String,
        value: String,
    },
    /// Several columns of the one row in a single statement (the grid's
    /// `:w` sends a row's changed cells together, so the row changes
    /// atomically and costs one statement instead of one per cell).
    SetValues(Vec<(String, String)>),
    DeleteRow,
}

/// Turns a `RowEdit` into standard SQL, shared by the SQL connectors for
/// the same reason as `SQL_KEYWORDS`: both need the same answer and neither
/// may depend on the other.
///
/// Values are always written as string literals. Postgres and SQLite both
/// coerce `'1'` to a number where a number is wanted, so one shape covers
/// every column type without this code having to know any of them -- the
/// single exception is the literal `NULL`, which is written unquoted
/// because that is exactly how the grid renders a null.
pub fn build_sql_edit(edit: &RowEdit) -> String {
    let table = quote_identifier(&edit.table);
    let where_clause = edit
        .key
        .iter()
        .map(|(column, value)| {
            if value == "NULL" {
                // `= NULL` is never true; a key column can't be null
                // anyway, so this only shows up if the grid is lying about
                // which columns are the key.
                format!("{} IS NULL", quote_identifier(column))
            } else {
                format!("{} = {}", quote_identifier(column), sql_literal(value))
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ");

    match &edit.change {
        RowChange::SetValue { column, value } => format!(
            "UPDATE {table} SET {} = {} WHERE {where_clause}",
            quote_identifier(column),
            sql_literal(value)
        ),
        RowChange::SetValues(pairs) => format!(
            "UPDATE {table} SET {} WHERE {where_clause}",
            pairs
                .iter()
                .map(|(column, value)| format!(
                    "{} = {}",
                    quote_identifier(column),
                    sql_literal(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        RowChange::DeleteRow => format!("DELETE FROM {table} WHERE {where_clause}"),
    }
}

/// `INSERT` for a new row: `values` are the columns to set (an omitted
/// column takes its default); same literal rules as `build_sql_edit` --
/// everything a string literal except the unquoted `NULL`.
pub fn build_sql_insert(table: &str, values: &[(String, String)]) -> String {
    let columns = values
        .iter()
        .map(|(column, _)| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let literals = values
        .iter()
        .map(|(_, value)| sql_literal(value))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {} ({columns}) VALUES ({literals})",
        quote_identifier(table)
    )
}

/// One column in a `TableDesignerOp::CreateTable`'s definition -- the
/// table-designer form's own fields (name/type free text, nullable/primary
/// key toggles), not yet turned into SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewColumn {
    pub name: String,
    /// The dialect's own type name, typed as-is (`INTEGER`, `VARCHAR(255)`,
    /// `TIMESTAMP`...) -- no validation or mapping, since there is no type
    /// list that's both complete and correct across every SQL dialect this
    /// might grow to cover.
    pub type_name: String,
    pub nullable: bool,
    pub primary_key: bool,
}

/// A schema change requested through the table-designer overlay -- see
/// `Component::open_table_designer`/`TableDesignRequest`. Unlike
/// `TableDesignRequest` (table/column names only, so it can travel through
/// the driver-agnostic `Component` trait), this already carries every field
/// the *form* collected, ready for `build_table_ddl`/`QueryDriver::table_ddl`
/// to turn into one dialect's actual statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableDesignerOp {
    AddColumn {
        table: String,
        column: String,
        type_name: String,
        nullable: bool,
        /// A raw SQL expression (`0`, `now()`, `'x'`...), not a value this
        /// code quotes for you -- same reasoning as `NewColumn::type_name`:
        /// there's no reliable way to guess whether a typed default is a
        /// number, a string, or a function call, so it's sent through
        /// exactly as typed. `None` omits `DEFAULT` entirely.
        default: Option<String>,
    },
    DropColumn {
        table: String,
        column: String,
    },
    RenameTable {
        table: String,
        new_name: String,
    },
    CreateTable {
        table: String,
        columns: Vec<NewColumn>,
    },
}

/// Turns a `TableDesignerOp` into standard DDL -- shared by the SQL
/// connectors that opt into the table designer, same reason as
/// `build_sql_edit`: both need the same answer and neither may depend on
/// the other. A driver that hasn't been taught to run this yet (everything
/// but Postgres, so far -- see `QueryDriver::table_ddl`) never calls it.
pub fn build_table_ddl(op: &TableDesignerOp) -> String {
    match op {
        TableDesignerOp::AddColumn {
            table,
            column,
            type_name,
            nullable,
            default,
        } => {
            let mut sql = format!(
                "ALTER TABLE {} ADD COLUMN {} {}",
                quote_identifier(table),
                quote_identifier(column),
                type_name.trim()
            );
            if !nullable {
                sql.push_str(" NOT NULL");
            }
            if let Some(default) = default.as_deref().filter(|d| !d.trim().is_empty()) {
                sql.push_str(" DEFAULT ");
                sql.push_str(default.trim());
            }
            sql
        }
        TableDesignerOp::DropColumn { table, column } => format!(
            "ALTER TABLE {} DROP COLUMN {}",
            quote_identifier(table),
            quote_identifier(column)
        ),
        TableDesignerOp::RenameTable { table, new_name } => format!(
            "ALTER TABLE {} RENAME TO {}",
            quote_identifier(table),
            quote_identifier(new_name)
        ),
        TableDesignerOp::CreateTable { table, columns } => {
            let column_lines: Vec<String> = columns
                .iter()
                .map(|c| {
                    let mut line = format!("{} {}", quote_identifier(&c.name), c.type_name.trim());
                    // A primary key is never null by definition -- stating
                    // it again as a separate `NOT NULL` would be redundant,
                    // not wrong, but this reads closer to how a hand-written
                    // `CREATE TABLE` states it.
                    if c.primary_key {
                        line.push_str(" PRIMARY KEY");
                    } else if !c.nullable {
                        line.push_str(" NOT NULL");
                    }
                    line
                })
                .collect();
            format!(
                "CREATE TABLE {} (\n  {}\n)",
                quote_identifier(table),
                column_lines.join(",\n  ")
            )
        }
    }
}

/// Double-quotes an identifier, one dotted part at a time so a
/// schema-qualified `public.users` stays two identifiers rather than
/// becoming one strangely named table. `pub`: CQL's own identifier
/// quoting is the same ANSI double-quote convention, so
/// `tradar-connector-cassandra`'s own edit-statement builder reuses this
/// directly instead of keeping a second copy (connector crates can't
/// depend on each other, but every one already depends on this crate).
pub fn quote_identifier(name: &str) -> String {
    name.split('.')
        .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn sql_literal(value: &str) -> String {
    if value == "NULL" {
        return "NULL".to_string();
    }
    format!("'{}'", value.replace('\'', "''"))
}
