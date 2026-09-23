//! The overlay behind the navigator's `a`/`x`/`R`/`n` -- add a column, drop
//! one, rename a table, or create a new one. Same "type it, show the exact
//! statement, confirm before running" shape as `row_edit`: a schema change
//! is a write you can't undo any more than a row edit can, so nothing runs
//! until approved, and what's approved is exactly what runs.
//!
//! Not a `Component`: like `RowEditComponent`, it's driven directly by
//! `QueryScreenComponent`, which owns the driver that can actually build
//! the statement (`QueryEngine::table_ddl`) -- this overlay only collects
//! the fields and hands a `TableDesignerOp` back for the host to try.
//!
//! v1 scope (see `docs/backlog/table-designer.md`): Postgres only, and only
//! these four operations -- constraint/index/FK changes are a later round.
//! `Add column`/`Create table`'s type field is free text, not a picker:
//! there is no type list that's both complete and correct across dialects,
//! so whatever's typed goes straight into the statement, same reasoning as
//! `NewColumn::type_name`'s own doc comment.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, TextInput};

use crate::query_driver::{NewColumn, TableDesignerOp};

pub enum TableDesignerOutcome {
    Cancelled,
    /// The fields are complete -- the host builds the statement via
    /// `QueryEngine::table_ddl` and calls `show_statement`/`show_problem`
    /// with the result.
    OpReady(TableDesignerOp),
    /// The user approved the shown statement: run it.
    Confirmed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddColumnField {
    Name,
    Type,
    Nullable,
    Default,
}

impl AddColumnField {
    const ORDER: [Self; 4] = [Self::Name, Self::Type, Self::Nullable, Self::Default];

    fn next(self) -> Self {
        match self {
            Self::Name => Self::Type,
            Self::Type => Self::Nullable,
            Self::Nullable => Self::Default,
            Self::Default => Self::Name,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Name => Self::Default,
            Self::Type => Self::Name,
            Self::Nullable => Self::Type,
            Self::Default => Self::Nullable,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Column",
            Self::Type => "Type",
            Self::Nullable => "Nullable",
            Self::Default => "Default",
        }
    }
}

struct AddColumnForm {
    field: AddColumnField,
    name: TextInput,
    type_name: TextInput,
    nullable: bool,
    default: TextInput,
    error: Option<String>,
}

impl AddColumnForm {
    fn new() -> Self {
        Self {
            field: AddColumnField::Name,
            name: TextInput::new(""),
            type_name: TextInput::new(""),
            // Nullable by default: a NOT NULL column added to a table that
            // already has rows needs a DEFAULT to succeed, and guessing one
            // is worse than asking the user to opt in.
            nullable: true,
            default: TextInput::new(""),
            error: None,
        }
    }

    fn build_op(&mut self, table: &str) -> Option<TableDesignerOp> {
        if self.name.text().trim().is_empty() || self.type_name.text().trim().is_empty() {
            self.error = Some("column name and type are required".to_string());
            return None;
        }
        let default = self.default.text().trim().to_string();
        Some(TableDesignerOp::AddColumn {
            table: table.to_string(),
            column: self.name.text().trim().to_string(),
            type_name: self.type_name.text().trim().to_string(),
            nullable: self.nullable,
            default: (!default.is_empty()).then_some(default),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewColumnField {
    Name,
    Type,
    Nullable,
    PrimaryKey,
}

impl NewColumnField {
    const ORDER: [Self; 4] = [Self::Name, Self::Type, Self::Nullable, Self::PrimaryKey];

    fn next(self) -> Self {
        match self {
            Self::Name => Self::Type,
            Self::Type => Self::Nullable,
            Self::Nullable => Self::PrimaryKey,
            Self::PrimaryKey => Self::Name,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Name => Self::PrimaryKey,
            Self::Type => Self::Name,
            Self::Nullable => Self::Type,
            Self::PrimaryKey => Self::Nullable,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Column",
            Self::Type => "Type",
            Self::Nullable => "Nullable",
            Self::PrimaryKey => "Primary key",
        }
    }
}

struct NewColumnForm {
    field: NewColumnField,
    name: TextInput,
    type_name: TextInput,
    nullable: bool,
    primary_key: bool,
}

impl NewColumnForm {
    fn new() -> Self {
        Self {
            field: NewColumnField::Name,
            name: TextInput::new(""),
            type_name: TextInput::new(""),
            nullable: true,
            primary_key: false,
        }
    }

    /// `None` when there's nothing worth committing (both fields still
    /// blank) -- what lets `Confirm` finish a table without the last
    /// column needing its own explicit "commit" keypress, while an empty
    /// leftover form after a real column doesn't turn into a phantom
    /// nameless one.
    fn to_column(&self) -> Option<NewColumn> {
        let name = self.name.text().trim().to_string();
        let type_name = self.type_name.text().trim().to_string();
        if name.is_empty() && type_name.is_empty() {
            return None;
        }
        Some(NewColumn {
            name,
            type_name,
            nullable: self.nullable,
            primary_key: self.primary_key,
        })
    }
}

enum Stage {
    AddColumn {
        table: String,
        form: AddColumnForm,
    },
    RenameTable {
        table: String,
        input: TextInput,
        error: Option<String>,
    },
    CreateTableName {
        input: TextInput,
        error: Option<String>,
    },
    CreateTableColumns {
        table: String,
        columns: Vec<NewColumn>,
        form: NewColumnForm,
        error: Option<String>,
    },
    /// Showing the built statement, waiting for `y`.
    Confirm(String),
    /// Nothing can run, and why. Dismiss only.
    Blocked(String),
}

pub struct TableDesignerComponent {
    title: String,
    stage: Stage,
}

impl TableDesignerComponent {
    pub fn add_column(table: String) -> Self {
        Self {
            title: format!("Add column to {table}"),
            stage: Stage::AddColumn {
                table,
                form: AddColumnForm::new(),
            },
        }
    }

    pub fn rename_table(table: String) -> Self {
        Self {
            title: format!("Rename table {table}"),
            stage: Stage::RenameTable {
                input: TextInput::new(&table),
                table,
                error: None,
            },
        }
    }

    pub fn create_table() -> Self {
        Self {
            title: "Create table".to_string(),
            stage: Stage::CreateTableName {
                input: TextInput::new(""),
                error: None,
            },
        }
    }

    /// Skips straight to approving (or refusing) a pre-built result -- used
    /// for `DropColumn`, which has no fields of its own to collect (the
    /// navigator already named the table and column), and for any op the
    /// host discovers up front the driver can't run at all.
    pub fn confirm(title: String, result: Result<String, String>) -> Self {
        Self {
            title,
            stage: match result {
                Ok(sql) => Stage::Confirm(sql),
                Err(reason) => Stage::Blocked(reason),
            },
        }
    }

    /// Moves from collecting fields to approving the statement built from
    /// them.
    pub fn show_statement(&mut self, statement: String) {
        self.stage = Stage::Confirm(statement);
    }

    /// Reports a failure to build the statement, in place, rather than
    /// closing and leaving no sign of what happened.
    pub fn show_problem(&mut self, reason: String) {
        self.stage = Stage::Blocked(reason);
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<TableDesignerOutcome> {
        if code == KeyCode::Esc {
            return Some(TableDesignerOutcome::Cancelled);
        }

        match &mut self.stage {
            Stage::AddColumn { table, form } => {
                let key = KeyPress::new(code, modifiers);
                let mut pending = None;
                if let Resolution::Command(command) =
                    keymap().resolve(Context::Prompt, &mut pending, key)
                {
                    match command {
                        Command::NextField => {
                            form.field = form.field.next();
                            return None;
                        }
                        Command::PrevField => {
                            form.field = form.field.prev();
                            return None;
                        }
                        Command::Confirm => {
                            return form.build_op(table).map(TableDesignerOutcome::OpReady);
                        }
                        _ => {}
                    }
                }
                if form.field == AddColumnField::Nullable {
                    if matches!(
                        code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
                    ) {
                        form.nullable = !form.nullable;
                    }
                    return None;
                }
                let input = match form.field {
                    AddColumnField::Name => &mut form.name,
                    AddColumnField::Type => &mut form.type_name,
                    AddColumnField::Default => &mut form.default,
                    AddColumnField::Nullable => unreachable!("handled above"),
                };
                if input.handle_key_event(code, modifiers) {
                    form.error = None;
                }
                None
            }
            Stage::RenameTable {
                table,
                input,
                error,
            } => {
                if code == KeyCode::Enter {
                    let new_name = input.text().trim().to_string();
                    if new_name.is_empty() {
                        *error = Some("new name must not be empty".to_string());
                        return None;
                    }
                    return Some(TableDesignerOutcome::OpReady(
                        TableDesignerOp::RenameTable {
                            table: table.clone(),
                            new_name,
                        },
                    ));
                }
                if input.handle_key_event(code, modifiers) {
                    *error = None;
                }
                None
            }
            Stage::CreateTableName { input, error } => {
                if code == KeyCode::Enter {
                    let table = input.text().trim().to_string();
                    if table.is_empty() {
                        *error = Some("table name must not be empty".to_string());
                        return None;
                    }
                    self.stage = Stage::CreateTableColumns {
                        table,
                        columns: Vec::new(),
                        form: NewColumnForm::new(),
                        error: None,
                    };
                    return None;
                }
                if input.handle_key_event(code, modifiers) {
                    *error = None;
                }
                None
            }
            Stage::CreateTableColumns {
                table,
                columns,
                form,
                error,
            } => {
                let key = KeyPress::new(code, modifiers);
                let mut pending = None;
                if let Resolution::Command(command) = keymap().resolve_in(
                    &[Context::TableDesigner, Context::Prompt],
                    &mut pending,
                    key,
                ) {
                    match command {
                        Command::NextField => {
                            form.field = form.field.next();
                            return None;
                        }
                        Command::PrevField => {
                            form.field = form.field.prev();
                            return None;
                        }
                        Command::TableDesignerCommitColumn => {
                            match form.to_column() {
                                Some(column) => {
                                    columns.push(column);
                                    *form = NewColumnForm::new();
                                    *error = None;
                                }
                                None => {
                                    *error = Some("column name and type are required".to_string())
                                }
                            }
                            return None;
                        }
                        Command::Confirm => {
                            if let Some(column) = form.to_column() {
                                columns.push(column);
                                *form = NewColumnForm::new();
                            }
                            if columns.is_empty() {
                                *error = Some("at least one column is required".to_string());
                                return None;
                            }
                            return Some(TableDesignerOutcome::OpReady(
                                TableDesignerOp::CreateTable {
                                    table: table.clone(),
                                    columns: columns.clone(),
                                },
                            ));
                        }
                        _ => {}
                    }
                }
                if form.field == NewColumnField::Nullable {
                    if matches!(
                        code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
                    ) {
                        form.nullable = !form.nullable;
                    }
                    return None;
                }
                if form.field == NewColumnField::PrimaryKey {
                    if matches!(
                        code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
                    ) {
                        form.primary_key = !form.primary_key;
                    }
                    return None;
                }
                let input = match form.field {
                    NewColumnField::Name => &mut form.name,
                    NewColumnField::Type => &mut form.type_name,
                    NewColumnField::Nullable | NewColumnField::PrimaryKey => {
                        unreachable!("handled above")
                    }
                };
                if input.handle_key_event(code, modifiers) {
                    *error = None;
                }
                None
            }
            // A plain `y`, the same answer the connection picker's delete
            // and row-edit's confirm ask for -- and anything else cancels,
            // so a stray key can never write to the database.
            Stage::Confirm(statement) => match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    Some(TableDesignerOutcome::Confirmed(statement.clone()))
                }
                _ => Some(TableDesignerOutcome::Cancelled),
            },
            Stage::Blocked(_) => match code {
                KeyCode::Enter | KeyCode::Char(_) => Some(TableDesignerOutcome::Cancelled),
                _ => None,
            },
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let block = ui::panel(&self.title, true);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let next = keymap()
            .binding_for(Context::Prompt, Command::NextField)
            .unwrap_or_default();
        let confirm = keymap()
            .binding_for(Context::Prompt, Command::Confirm)
            .unwrap_or_default();
        let cancel = keymap()
            .binding_for(Context::Prompt, Command::Cancel)
            .unwrap_or_default();

        let mut lines: Vec<Line> = Vec::new();
        match &self.stage {
            Stage::AddColumn { form, .. } => {
                for field in AddColumnField::ORDER {
                    lines.push(field_line(
                        field.label(),
                        field == form.field,
                        match field {
                            AddColumnField::Name => Some(&form.name),
                            AddColumnField::Type => Some(&form.type_name),
                            AddColumnField::Default => Some(&form.default),
                            AddColumnField::Nullable => None,
                        },
                        (field == AddColumnField::Nullable).then_some(form.nullable),
                        theme,
                    ));
                }
                push_footer(
                    &mut lines,
                    &form.error,
                    &format!(
                        "{next} next field · ←/→ toggle nullable · {confirm} build · {cancel} cancel"
                    ),
                    theme,
                );
            }
            Stage::RenameTable { input, error, .. } => {
                lines.push(Line::from(input.spans(true)));
                push_footer(
                    &mut lines,
                    error,
                    &format!("{confirm} build · {cancel} cancel"),
                    theme,
                );
            }
            Stage::CreateTableName { input, error } => {
                lines.push(Line::from(Span::styled(
                    "Table name:",
                    Style::default().fg(theme.text_dim),
                )));
                lines.push(Line::from(input.spans(true)));
                push_footer(
                    &mut lines,
                    error,
                    &format!("{confirm} next: columns · {cancel} cancel"),
                    theme,
                );
            }
            Stage::CreateTableColumns {
                table,
                columns,
                form,
                error,
            } => {
                lines.push(Line::from(Span::styled(
                    format!("Table: {table}"),
                    Style::default().fg(theme.text_dim),
                )));
                for column in columns {
                    let mut flags = Vec::new();
                    if column.primary_key {
                        flags.push("pk");
                    }
                    if !column.nullable {
                        flags.push("not null");
                    }
                    let suffix = if flags.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", flags.join(", "))
                    };
                    lines.push(Line::from(Span::styled(
                        format!("  {} {}{suffix}", column.name, column.type_name),
                        Style::default().fg(theme.text),
                    )));
                }
                lines.push(Line::from(""));
                for field in NewColumnField::ORDER {
                    lines.push(field_line(
                        field.label(),
                        field == form.field,
                        match field {
                            NewColumnField::Name => Some(&form.name),
                            NewColumnField::Type => Some(&form.type_name),
                            NewColumnField::Nullable | NewColumnField::PrimaryKey => None,
                        },
                        match field {
                            NewColumnField::Nullable => Some(form.nullable),
                            NewColumnField::PrimaryKey => Some(form.primary_key),
                            _ => None,
                        },
                        theme,
                    ));
                }
                let add_column = keymap()
                    .binding_for(Context::TableDesigner, Command::TableDesignerCommitColumn)
                    .unwrap_or_default();
                push_footer(
                    &mut lines,
                    error,
                    &format!(
                        "{next} next field · ←/→ toggle · {add_column} add column · {confirm} create table · {cancel} cancel"
                    ),
                    theme,
                );
            }
            Stage::Confirm(statement) => {
                lines.push(Line::from(Span::styled(
                    "This will run:",
                    Style::default().fg(theme.text_dim),
                )));
                lines.push(Line::from(Span::styled(
                    statement.clone(),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "y to run, any other key to cancel",
                    Style::default().fg(theme.warning),
                )));
            }
            Stage::Blocked(reason) => {
                lines.push(Line::from(Span::styled(
                    reason.clone(),
                    Style::default().fg(theme.error),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "esc to dismiss",
                    Style::default().fg(theme.text_dim),
                )));
            }
        }

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

fn field_line(
    label: &str,
    focused: bool,
    input: Option<&TextInput>,
    toggle: Option<bool>,
    theme: &tradar_core::theme::Theme,
) -> Line<'static> {
    let label_style = if focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text_dim)
    };
    let mut spans = vec![Span::styled(format!("{label:<12}"), label_style)];
    if let Some(input) = input {
        spans.extend(input.spans(focused));
    } else if let Some(value) = toggle {
        let style = if focused {
            ui::selection_style()
        } else {
            Style::default().fg(theme.text)
        };
        spans.push(Span::styled(if value { " yes " } else { " no " }, style));
    }
    Line::from(spans)
}

fn push_footer(
    lines: &mut Vec<Line<'static>>,
    error: &Option<String>,
    hint: &str,
    theme: &tradar_core::theme::Theme,
) {
    lines.push(Line::from(""));
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(theme.error),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            hint.to_string(),
            Style::default().fg(theme.text_dim),
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_str(component: &mut TableDesignerComponent, text: &str) {
        for c in text.chars() {
            component.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    #[test]
    fn add_column_requires_a_name_and_type_before_confirming() {
        let mut component = TableDesignerComponent::add_column("users".to_string());

        let outcome = component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(
            outcome.is_none(),
            "nothing is ready until name and type are filled in"
        );
        let Stage::AddColumn { form, .. } = &component.stage else {
            panic!("expected AddColumn");
        };
        assert!(form.error.is_some());
    }

    #[test]
    fn add_column_builds_the_op_from_every_field() {
        let mut component = TableDesignerComponent::add_column("users".to_string());
        type_str(&mut component, "nickname");
        component.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        type_str(&mut component, "TEXT");
        component.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        // Nullable field: toggle off.
        component.handle_key_event(KeyCode::Right, KeyModifiers::NONE);
        component.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        type_str(&mut component, "'anon'");

        let outcome = component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        match outcome {
            Some(TableDesignerOutcome::OpReady(TableDesignerOp::AddColumn {
                table,
                column,
                type_name,
                nullable,
                default,
            })) => {
                assert_eq!(table, "users");
                assert_eq!(column, "nickname");
                assert_eq!(type_name, "TEXT");
                assert!(!nullable);
                assert_eq!(default.as_deref(), Some("'anon'"));
            }
            other => panic!(
                "expected AddColumn op ready: {other:?}",
                other = debug_outcome(&other)
            ),
        }
    }

    #[test]
    fn shift_tab_cycles_backwards_through_add_column_fields() {
        let mut component = TableDesignerComponent::add_column("users".to_string());

        component.handle_key_event(KeyCode::BackTab, KeyModifiers::NONE);

        let Stage::AddColumn { form, .. } = &component.stage else {
            panic!("expected AddColumn");
        };
        assert_eq!(
            form.field,
            AddColumnField::Default,
            "wraps to the last field"
        );
    }

    #[test]
    fn rename_table_prefills_the_current_name() {
        let component = TableDesignerComponent::rename_table("users".to_string());

        let Stage::RenameTable { input, .. } = &component.stage else {
            panic!("expected RenameTable");
        };
        assert_eq!(input.text(), "users");
    }

    #[test]
    fn rename_table_confirms_into_an_op() {
        let mut component = TableDesignerComponent::rename_table("users".to_string());
        // Clear the prefilled name and type a new one.
        for _ in 0..5 {
            component.handle_key_event(KeyCode::Backspace, KeyModifiers::NONE);
        }
        type_str(&mut component, "accounts");

        let outcome = component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        match outcome {
            Some(TableDesignerOutcome::OpReady(TableDesignerOp::RenameTable {
                table,
                new_name,
            })) => {
                assert_eq!(table, "users");
                assert_eq!(new_name, "accounts");
            }
            other => panic!(
                "expected RenameTable op ready: {other:?}",
                other = debug_outcome(&other)
            ),
        }
    }

    #[test]
    fn rename_table_refuses_an_empty_name() {
        let mut component = TableDesignerComponent::rename_table("users".to_string());
        for _ in 0..5 {
            component.handle_key_event(KeyCode::Backspace, KeyModifiers::NONE);
        }

        let outcome = component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(outcome.is_none());
        let Stage::RenameTable { error, .. } = &component.stage else {
            panic!("expected RenameTable");
        };
        assert!(error.is_some());
    }

    #[test]
    fn create_table_moves_from_naming_to_columns_on_confirm() {
        let mut component = TableDesignerComponent::create_table();
        type_str(&mut component, "accounts");

        component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let Stage::CreateTableColumns { table, columns, .. } = &component.stage else {
            panic!("expected CreateTableColumns");
        };
        assert_eq!(table, "accounts");
        assert!(columns.is_empty());
    }

    #[test]
    fn create_table_refuses_an_empty_table_name() {
        let mut component = TableDesignerComponent::create_table();

        let outcome = component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(outcome.is_none());
        assert!(matches!(component.stage, Stage::CreateTableName { .. }));
    }

    #[test]
    fn committing_a_column_adds_it_and_resets_the_form_for_the_next_one() {
        let mut component = TableDesignerComponent::create_table();
        type_str(&mut component, "accounts");
        component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        type_str(&mut component, "id");
        component.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        type_str(&mut component, "INTEGER");

        component.handle_key_event(KeyCode::Char('a'), KeyModifiers::CONTROL);

        let Stage::CreateTableColumns { columns, form, .. } = &component.stage else {
            panic!("expected CreateTableColumns");
        };
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].name, "id");
        assert_eq!(columns[0].type_name, "INTEGER");
        assert!(form.name.is_empty(), "the form resets for the next column");
    }

    #[test]
    fn confirming_create_table_also_commits_whatever_column_is_still_in_the_form() {
        let mut component = TableDesignerComponent::create_table();
        type_str(&mut component, "accounts");
        component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        type_str(&mut component, "id");
        component.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        type_str(&mut component, "INTEGER");

        let outcome = component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        match outcome {
            Some(TableDesignerOutcome::OpReady(TableDesignerOp::CreateTable {
                table,
                columns,
            })) => {
                assert_eq!(table, "accounts");
                assert_eq!(columns.len(), 1);
                assert_eq!(columns[0].name, "id");
            }
            other => panic!(
                "expected CreateTable op ready: {other:?}",
                other = debug_outcome(&other)
            ),
        }
    }

    #[test]
    fn create_table_refuses_to_finish_with_no_columns_at_all() {
        let mut component = TableDesignerComponent::create_table();
        type_str(&mut component, "accounts");
        component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let outcome = component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(outcome.is_none());
        let Stage::CreateTableColumns { error, .. } = &component.stage else {
            panic!("expected CreateTableColumns");
        };
        assert!(error.is_some());
    }

    #[test]
    fn the_statement_has_to_be_approved_with_y_before_it_runs() {
        let mut component = TableDesignerComponent::confirm(
            "Drop column".to_string(),
            Ok("ALTER TABLE t DROP COLUMN c".to_string()),
        );

        match component.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE) {
            Some(TableDesignerOutcome::Confirmed(sql)) => {
                assert_eq!(sql, "ALTER TABLE t DROP COLUMN c")
            }
            _ => panic!("expected the statement to be confirmed"),
        }
    }

    #[test]
    fn any_other_key_cancels_rather_than_running_the_statement() {
        let mut component = TableDesignerComponent::confirm(
            "Drop column".to_string(),
            Ok("ALTER TABLE t DROP COLUMN c".to_string()),
        );

        assert!(matches!(
            component.handle_key_event(KeyCode::Char('n'), KeyModifiers::NONE),
            Some(TableDesignerOutcome::Cancelled)
        ));
    }

    #[test]
    fn a_blocked_op_says_why_and_only_dismisses_on_a_real_key() {
        let mut component = TableDesignerComponent::confirm(
            "Add column".to_string(),
            Err("this connection doesn't support the table designer".to_string()),
        );

        assert!(
            component
                .handle_key_event(KeyCode::Up, KeyModifiers::NONE)
                .is_none()
        );
        assert!(matches!(
            component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE),
            Some(TableDesignerOutcome::Cancelled)
        ));
    }

    #[test]
    fn esc_cancels_from_every_stage() {
        let mut component = TableDesignerComponent::add_column("users".to_string());

        assert!(matches!(
            component.handle_key_event(KeyCode::Esc, KeyModifiers::NONE),
            Some(TableDesignerOutcome::Cancelled)
        ));
    }

    // `TableDesignerOutcome` has no `Debug`/`PartialEq` (it carries
    // `TableDesignerOp`, itself not compared anywhere else), so failing
    // assertions describe the outcome by hand instead of deriving it just
    // for test output.
    fn debug_outcome(outcome: &Option<TableDesignerOutcome>) -> &'static str {
        match outcome {
            None => "None",
            Some(TableDesignerOutcome::Cancelled) => "Some(Cancelled)",
            Some(TableDesignerOutcome::OpReady(_)) => "Some(OpReady(_))",
            Some(TableDesignerOutcome::Confirmed(_)) => "Some(Confirmed(_))",
        }
    }
}
