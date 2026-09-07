//! Parses the results-grid filter text (`ResultsComponent::filter`) into
//! structured conditions -- what lets `col:value` scope a term to one
//! column, and `AND`/`OR` combine several. Kept as its own module rather
//! than inline in `components::results` since `components::filter_conditions`
//! (the panel that lists/deletes conditions) needs the same parsed shape
//! without depending on `results`'s own internals.
//!
//! Syntax, deliberately unquoted and without parentheses (see
//! `docs/backlog/multi-filter.md`): a filter is one or more conditions
//! joined by `AND`/`OR` (case-insensitive, must be its own whitespace-
//! delimited word -- `Andes` or `oracle` are never mistaken for the
//! keyword). `AND` binds tighter than `OR`, same precedence SQL uses, so
//! `a AND b OR c` reads as `(a AND b) OR c` -- a row matches when *any*
//! `OR` group matches, and a group matches when *all* its conditions do.
//! Each condition is either `column:value` (`column` matched
//! case-insensitively against a real column name -- `value` then searched
//! only in that column) or a bare term (`value` searched in every cell,
//! the original single-filter behavior, still exactly what an empty
//! `columns` list -- the `Documents` JSON view, which has no column list
//! at all -- always falls back to). A `column:` prefix that doesn't match
//! any real column name is treated as ordinary bare text instead of an
//! error, so a value that happens to contain a colon (a timestamp, a URL)
//! still searches correctly rather than silently matching nothing.

/// One parsed condition: either scoped to a column (`col:value`) or bare
/// (`value`, checked against every cell). `value_lower` is precomputed once
/// at parse time since matching runs on every row, every redraw -- see
/// `ParsedFilter::matches_row`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterCondition {
    /// `Some((display name, column index))` for a `col:value` condition
    /// whose `col` matched a real column -- the name is kept in its
    /// original case for `render`/the panel, the index precomputed so
    /// matching never has to search `columns` again.
    pub column: Option<(String, usize)>,
    /// As typed, original case -- for `render`/the panel.
    pub value: String,
    value_lower: String,
}

impl FilterCondition {
    fn parse(raw: &str, columns: &[String]) -> Self {
        if let Some((left, right)) = raw.split_once(':')
            && let Some(index) = columns
                .iter()
                .position(|c| c.eq_ignore_ascii_case(left.trim()))
        {
            let value = right.trim().to_string();
            return Self {
                column: Some((columns[index].clone(), index)),
                value_lower: value.to_lowercase(),
                value,
            };
        }
        Self {
            column: None,
            value_lower: raw.to_lowercase(),
            value: raw.to_string(),
        }
    }

    fn matches_row(&self, row: &[String]) -> bool {
        match &self.column {
            Some((_, index)) => row
                .get(*index)
                .is_some_and(|cell| cell.to_lowercase().contains(&self.value_lower)),
            None => row
                .iter()
                .any(|cell| cell.to_lowercase().contains(&self.value_lower)),
        }
    }

    /// Back to filter-text syntax -- the inverse of `parse`, used by
    /// `ParsedFilter::render` to rebuild the filter string after the panel
    /// removes one condition.
    fn render(&self) -> String {
        match &self.column {
            Some((name, _)) => format!("{name}:{}", self.value),
            None => self.value.clone(),
        }
    }
}

/// A filter string parsed into disjunctive-normal form (see the module doc
/// comment for the precedence this gives `AND`/`OR`). An empty/blank filter
/// parses to no groups at all, which `matches_row`/`matches_text` special-
/// case to "everything matches" -- the same behavior an empty filter always
/// had before this module existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFilter {
    groups: Vec<Vec<FilterCondition>>,
}

impl ParsedFilter {
    /// `columns` resolves which conditions are column-scoped -- pass `&[]`
    /// for a result with no column list (the `Documents` JSON view), which
    /// makes every condition fall back to a bare substring term.
    pub fn parse(text: &str, columns: &[String]) -> Self {
        let mut groups: Vec<Vec<FilterCondition>> = Vec::new();
        let mut current_group: Vec<FilterCondition> = Vec::new();
        let mut current_words: Vec<&str> = Vec::new();

        fn flush(words: &mut Vec<&str>, group: &mut Vec<FilterCondition>, columns: &[String]) {
            if !words.is_empty() {
                group.push(FilterCondition::parse(&words.join(" "), columns));
                words.clear();
            }
        }

        for word in text.split_whitespace() {
            if word.eq_ignore_ascii_case("and") && !current_words.is_empty() {
                flush(&mut current_words, &mut current_group, columns);
            } else if word.eq_ignore_ascii_case("or") && !current_words.is_empty() {
                flush(&mut current_words, &mut current_group, columns);
                if !current_group.is_empty() {
                    groups.push(std::mem::take(&mut current_group));
                }
            } else {
                current_words.push(word);
            }
        }
        flush(&mut current_words, &mut current_group, columns);
        if !current_group.is_empty() {
            groups.push(current_group);
        }

        Self { groups }
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Every `OR` group, each an `AND` list of conditions -- what the
    /// filter-conditions panel iterates to render/index into for deletion.
    pub fn groups(&self) -> &[Vec<FilterCondition>] {
        &self.groups
    }

    pub fn matches_row(&self, row: &[String]) -> bool {
        self.is_empty()
            || self
                .groups
                .iter()
                .any(|group| group.iter().all(|c| c.matches_row(row)))
    }

    /// For a result with no cell/column structure (the `Documents` JSON
    /// view) -- `text_lower` is the whole document's JSON, already
    /// lowercased by the caller. Column-scoped conditions can't apply here
    /// (there is no column list to have resolved one against, so `parse`
    /// would already have fallen every condition back to bare), so this
    /// checks `value_lower` as a plain substring the same as a bare
    /// condition would against a table cell.
    pub fn matches_text(&self, text_lower: &str) -> bool {
        self.is_empty()
            || self
                .groups
                .iter()
                .any(|group| group.iter().all(|c| text_lower.contains(&c.value_lower)))
    }

    /// Removes the condition at `(group_index, condition_index)`, dropping
    /// its group entirely if that was the group's last condition. Used by
    /// the filter-conditions panel's delete action; `render()` afterward
    /// gives the new filter text to hand back to `ResultsComponent::set_filter`.
    pub fn without(&self, group_index: usize, condition_index: usize) -> Self {
        let mut groups = self.groups.clone();
        if let Some(group) = groups.get_mut(group_index) {
            if condition_index < group.len() {
                group.remove(condition_index);
            }
            if group.is_empty() {
                groups.remove(group_index);
            }
        }
        Self { groups }
    }

    /// Back to filter-text syntax -- the inverse of `parse`.
    pub fn render(&self) -> String {
        self.groups
            .iter()
            .map(|group| {
                group
                    .iter()
                    .map(FilterCondition::render)
                    .collect::<Vec<_>>()
                    .join(" AND ")
            })
            .collect::<Vec<_>>()
            .join(" OR ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<String> {
        vec!["status".to_string(), "role".to_string()]
    }

    fn row(status: &str, role: &str) -> Vec<String> {
        vec![status.to_string(), role.to_string()]
    }

    #[test]
    fn an_empty_filter_matches_everything() {
        let parsed = ParsedFilter::parse("", &columns());
        assert!(parsed.is_empty());
        assert!(parsed.matches_row(&row("active", "admin")));
    }

    #[test]
    fn a_bare_term_matches_any_cell_case_insensitively_like_before() {
        let parsed = ParsedFilter::parse("ADMIN", &columns());
        assert!(parsed.matches_row(&row("active", "admin")));
        assert!(!parsed.matches_row(&row("active", "viewer")));
    }

    #[test]
    fn a_column_scoped_term_only_checks_that_column() {
        let parsed = ParsedFilter::parse("role:admin", &columns());
        assert!(parsed.matches_row(&row("active", "admin")));
        // "admin" sitting in `status` instead must not match.
        assert!(!parsed.matches_row(&row("admin", "viewer")));
    }

    #[test]
    fn a_column_prefix_that_is_not_a_real_column_falls_back_to_bare_text() {
        // No column named "12", so this searches every cell for "12:30" --
        // a timestamp-shaped value must still be findable.
        let parsed = ParsedFilter::parse("12:30", &columns());
        assert!(parsed.matches_row(&row("12:30", "admin")));
    }

    #[test]
    fn column_matching_is_case_insensitive_on_the_column_name() {
        let parsed = ParsedFilter::parse("STATUS:active", &columns());
        assert!(parsed.matches_row(&row("active", "admin")));
    }

    #[test]
    fn and_requires_every_condition_in_the_group() {
        let parsed = ParsedFilter::parse("status:active AND role:admin", &columns());
        assert!(parsed.matches_row(&row("active", "admin")));
        assert!(!parsed.matches_row(&row("active", "viewer")));
    }

    #[test]
    fn or_requires_only_one_group_to_match() {
        let parsed = ParsedFilter::parse("status:active OR status:pending", &columns());
        assert!(parsed.matches_row(&row("active", "admin")));
        assert!(parsed.matches_row(&row("pending", "admin")));
        assert!(!parsed.matches_row(&row("closed", "admin")));
    }

    #[test]
    fn and_binds_tighter_than_or() {
        // (status:active AND role:admin) OR status:pending
        let parsed =
            ParsedFilter::parse("status:active AND role:admin OR status:pending", &columns());
        assert!(parsed.matches_row(&row("active", "admin")));
        assert!(parsed.matches_row(&row("pending", "viewer")));
        assert!(!parsed.matches_row(&row("active", "viewer")));
    }

    #[test]
    fn and_or_keywords_are_case_insensitive() {
        let parsed = ParsedFilter::parse("status:active and role:admin", &columns());
        assert!(parsed.matches_row(&row("active", "admin")));
        assert!(!parsed.matches_row(&row("active", "viewer")));
    }

    #[test]
    fn a_word_that_merely_contains_and_or_or_is_not_treated_as_the_keyword() {
        // "Andes" / "oracle" must stay literal text, not get chopped into
        // "d" / "acle" by a naive substring split on "and"/"or".
        let parsed = ParsedFilter::parse("Andes", &[]);
        assert_eq!(parsed.groups().len(), 1);
        assert_eq!(parsed.groups()[0].len(), 1);
        assert_eq!(parsed.groups()[0][0].value, "Andes");
    }

    #[test]
    fn a_leading_bare_and_or_or_is_literal_text_not_a_dangling_operator() {
        let parsed = ParsedFilter::parse("and", &[]);
        assert_eq!(parsed.groups().len(), 1);
        assert_eq!(parsed.groups()[0][0].value, "and");
    }

    #[test]
    fn matches_text_combines_conditions_over_the_whole_document() {
        let parsed = ParsedFilter::parse("hanoi AND active", &[]);
        assert!(parsed.matches_text("{\"city\":\"hanoi\",\"status\":\"active\"}"));
        assert!(!parsed.matches_text("{\"city\":\"hanoi\",\"status\":\"closed\"}"));
    }

    #[test]
    fn render_round_trips_a_parsed_filter() {
        let text = "status:active AND role:admin OR status:pending";
        let parsed = ParsedFilter::parse(text, &columns());
        assert_eq!(parsed.render(), text);
    }

    #[test]
    fn without_drops_a_condition_and_its_group_when_it_was_the_last_one() {
        let parsed =
            ParsedFilter::parse("status:active AND role:admin OR status:pending", &columns());

        // Drop "role:admin" (group 0, condition 1) -- the group survives
        // with its one remaining condition.
        let one_left = parsed.without(0, 1);
        assert_eq!(one_left.render(), "status:active OR status:pending");

        // Drop "status:pending" (group 1, condition 0) -- that whole group
        // is now empty and disappears rather than rendering as a dangling
        // "OR".
        let group_gone = parsed.without(1, 0);
        assert_eq!(group_gone.render(), "status:active AND role:admin");
    }

    #[test]
    fn removing_the_only_condition_leaves_an_empty_filter() {
        let parsed = ParsedFilter::parse("status:active", &columns());
        let empty = parsed.without(0, 0);
        assert!(empty.is_empty());
        assert_eq!(empty.render(), "");
    }
}
