use chrono::{DateTime, Duration, Local, TimeZone};
use pest::Parser;
use pest_derive::Parser;
use regex::Regex;
use std::fmt;

use crate::logcat::LogEntry;

#[derive(Parser)]
#[grammar = "filter.pest"]
struct FilterParser;

/// A filter key parsed from user input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Tag,
    Package,
    Process,
    Message,
    Level,
    Age,
    Is,
    Name,
}

impl Key {
    fn from_str(s: &str) -> Option<Key> {
        match s {
            "tag" => Some(Key::Tag),
            "package" => Some(Key::Package),
            "process" => Some(Key::Process),
            "message" => Some(Key::Message),
            "level" => Some(Key::Level),
            "age" => Some(Key::Age),
            "is" => Some(Key::Is),
            "name" => Some(Key::Name),
            _ => None,
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Key::Tag => "tag",
            Key::Package => "package",
            Key::Process => "process",
            Key::Message => "message",
            Key::Level => "level",
            Key::Age => "age",
            Key::Is => "is",
            Key::Name => "name",
        };
        write!(f, "{}", s)
    }
}

/// A single predicate: `key:value` or `-key~:value`.
#[derive(Clone, Debug)]
pub struct Predicate {
    pub key: Key,
    pub value: String,
    pub negate: bool,
    pub regex: bool,
}

/// Filter expression AST.
///
/// Evaluated against each `LogEntry` to determine visibility.
/// `AgeSince` is a special node whose cutoff is refreshed each tick.
#[derive(Clone, Debug)]
pub enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Pred(Predicate),
    /// Match entries newer than the stored cutoff time.
    /// The cutoff is updated each tick to `now - duration`.
    AgeSince {
        cutoff: DateTime<Local>,
    },
}

/// Intermediate token produced during compilation.
#[derive(Debug, Clone)]
enum FilterToken {
    Predicate {
        key: Key,
        value: String,
        negate: bool,
        regex: bool,
    },
    And,
    Or,
    OpenParen,
    CloseParen,
}

/// Compile a filter query string into an `Expr`.
///
/// Returns `Err(String)` with a human-readable error if parsing or compilation fails.
pub fn compile(input: &str, msgs: &crate::i18n::Messages) -> Result<Expr, String> {
    // Empty filter matches everything
    let trimmed = input.trim();
    if trimmed.is_empty() {
        // Return a predicate that always matches
        return Ok(Expr::Pred(Predicate {
            key: Key::Name,
            value: String::new(),
            negate: false,
            regex: false,
        }));
    }

    let pairs = FilterParser::parse(Rule::query, trimmed)
        .map_err(|e| msgs.parse_error(&e.to_string()))?;

    let mut tokens: Vec<FilterToken> = Vec::new();

    for pair in pairs {
        collect_tokens(pair, &mut tokens)?;
    }



    // Stage 1: merge adjacent same-key non-negated predicates into OR groups.
    let tokens = merge_same_key_or(tokens);

    // Stage 2: apply implicit AND between remaining adjacent non-operator tokens.
    let tokens = apply_implicit_and(tokens);

    // Stage 3: build AST from token stream with precedence (& binds tighter than |).
    build_ast(&tokens, 0, msgs).map(|(expr, _)| expr)
}

/// Recursively collect tokens from pest pairs.
fn collect_tokens(pair: pest::iterators::Pair<Rule>, tokens: &mut Vec<FilterToken>) -> Result<(), String> {
    match pair.as_rule() {
        Rule::query | Rule::term => {
            for inner in pair.into_inner() {
                collect_tokens(inner, tokens)?;
            }
        }
        Rule::or_expr => {
            let mut first = true;
            for inner in pair.into_inner() {
                if !first {
                    tokens.push(FilterToken::Or);
                }
                collect_tokens(inner, tokens)?;
                first = false;
            }
        }
        Rule::and_expr => {
            // Just recurse — implicit AND is handled by apply_implicit_and later.
            // (Pest does not expose `&` nor space as separate inner pairs.)
            for inner in pair.into_inner() {
                collect_tokens(inner, tokens)?;
            }
        }
        Rule::bare_word => {
            let word = pair.as_str().to_string();
            tokens.push(FilterToken::Predicate {
                key: Key::Message,
                value: word,
                negate: false,
                regex: false,
            });
        }
        Rule::predicate => {
            let mut key: Option<Key> = None;
            let _negate = false;
            let mut regex = false;
            let mut value = String::new();

            // The predicate's parent (not_expr) handles negation, but the `-`
            // is a separate rule. We check for negation in the not_expr handler.
            // Here we extract the sub-parts of the predicate itself.
            for inner in pair.into_inner() {
                match inner.as_rule() {
                    Rule::key => {
                        key = Key::from_str(inner.as_str());
                    }
                    Rule::regex_modifier => {
                        regex = true;
                    }
                    Rule::quoted_value => {
                        value = extract_value(inner);
                    }
                    _ => {}
                }
            }

            let key = key.ok_or_else(|| "未知过滤器键".to_string())?;

            tokens.push(FilterToken::Predicate {
                key,
                value,
                negate: false, // negation handled by parent not_expr
                regex,
            });
        }
        Rule::not_expr => {
            // Pest consumes the `-` literal as part of pattern matching,
            // but it does NOT appear as a separate inner pair.
            // Detect negation by checking if the full matched text starts with `-`.
            let is_negated = pair.as_str().starts_with('-');
            let inner_pairs: Vec<_> = pair.clone().into_inner().collect();

            let mut sub_tokens: Vec<FilterToken> = Vec::new();
            for inner in &inner_pairs {
                collect_tokens((*inner).clone(), &mut sub_tokens)?;
            }

            if is_negated {
                for token in sub_tokens {
                    match token {
                        FilterToken::Predicate { key, value, negate: _, regex } => {
                            tokens.push(FilterToken::Predicate {
                                key, value, negate: true, regex,
                            });
                        }
                        other => tokens.push(other),
                    }
                }
            } else {
                tokens.extend(sub_tokens);
            }
        }
        Rule::quoted_value | Rule::unquoted_value | Rule::quoted_inner => {
            // Handled by predicate
        }
        Rule::key | Rule::regex_modifier => {
            // Handled by predicate
        }
        Rule::WHITESPACE | Rule::EOI => {}
    }
    Ok(())
}

/// Extract the string value from a quoted_value pair.
fn extract_value(pair: pest::iterators::Pair<Rule>) -> String {
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::quoted_inner => return inner.as_str().to_string(),
            Rule::unquoted_value => return inner.as_str().to_string(),
            _ => {}
        }
    }
    String::new()
}

/// Merge adjacent same-key non-negated predicates into implicit OR.
///
/// Rule: consecutive predicates with the same key, all non-negated,
/// without intervening operators → combine into OR group.
fn merge_same_key_or(tokens: Vec<FilterToken>) -> Vec<FilterToken> {
    let mut result: Vec<FilterToken> = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let token = &tokens[i];

        if let FilterToken::Predicate { key, negate, .. } = token
            && !negate {
                // Collect run of same-key non-negated predicates
                let run_key = key.clone();
                let mut run: Vec<FilterToken> = Vec::new();
                while i < tokens.len() {
                    if let FilterToken::Predicate { key: k, negate: n, .. } = &tokens[i] {
                        if k == &run_key && !n {
                            run.push(tokens[i].clone());
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                if run.len() == 1 {
                    result.push(run.into_iter().next().unwrap());
                } else {
                    // Merge into OR group: push tokens with OR separators
                    let run_len = run.len();
                    for (j, tok) in run.into_iter().enumerate() {
                        if j > 0 {
                            result.push(FilterToken::Or);
                        }
                        result.push(tok);
                    }
                    // Wrap in parentheses for correct precedence
                    result.insert(result.len() - (2 * run_len - 1), FilterToken::OpenParen);
                    result.push(FilterToken::CloseParen);
                }
                continue;
            }

        result.push(tokens[i].clone());
        i += 1;
    }

    result
}

/// Apply implicit AND between adjacent tokens that aren't separated by an operator.
fn apply_implicit_and(tokens: Vec<FilterToken>) -> Vec<FilterToken> {
    let mut result: Vec<FilterToken> = Vec::new();

    for (i, token) in tokens.iter().enumerate() {
        result.push(token.clone());

        if i + 1 < tokens.len() {
            let current_is_value = matches!(
                token,
                FilterToken::Predicate { .. } | FilterToken::CloseParen
            );
            let next_is_value = matches!(
                &tokens[i + 1],
                FilterToken::Predicate { .. } | FilterToken::OpenParen
            );
            let current_is_not_operator = !matches!(token, FilterToken::And | FilterToken::Or | FilterToken::OpenParen);
            let next_is_not_operator = !matches!(&tokens[i + 1], FilterToken::And | FilterToken::Or | FilterToken::CloseParen);

            if current_is_value && next_is_value {
                result.push(FilterToken::And);
            } else if current_is_not_operator && next_is_not_operator
                && !matches!(token, FilterToken::CloseParen)
                && !matches!(&tokens[i + 1], FilterToken::OpenParen)
            {
                // Need implicit AND between e.g. a predicate and the next predicate/paren
            }
        }
    }

    result
}

/// Build AST from token stream using recursive descent with precedence.
/// Precedence: & binds tighter than |
fn build_ast(tokens: &[FilterToken], pos: usize, msgs: &crate::i18n::Messages) -> Result<(Expr, usize), String> {
    let (mut left, mut pos) = build_and(tokens, pos, msgs)?;

    while pos < tokens.len() {
        match &tokens[pos] {
            FilterToken::Or => {
                pos += 1;
                let (right, new_pos) = build_and(tokens, pos, msgs)?;
                left = Expr::Or(Box::new(left), Box::new(right));
                pos = new_pos;
            }
            FilterToken::CloseParen => {
                break;
            }
            _ => break,
        }
    }

    Ok((left, pos))
}

fn build_and(tokens: &[FilterToken], pos: usize, msgs: &crate::i18n::Messages) -> Result<(Expr, usize), String> {
    let (mut left, mut pos) = build_primary(tokens, pos, msgs)?;

    while pos < tokens.len() {
        match &tokens[pos] {
            FilterToken::And => {
                pos += 1;
                let (right, new_pos) = build_primary(tokens, pos, msgs)?;
                left = Expr::And(Box::new(left), Box::new(right));
                pos = new_pos;
            }
            FilterToken::Or | FilterToken::CloseParen => break,
            _ => break,
        }
    }

    Ok((left, pos))
}

fn build_primary(tokens: &[FilterToken], pos: usize, msgs: &crate::i18n::Messages) -> Result<(Expr, usize), String> {
    if pos >= tokens.len() {
        return Err(msgs.unexpected_token.to_string());
    }

    match &tokens[pos] {
        FilterToken::Predicate { key, value, negate, regex } => {
            let expr = make_predicate(key, value, *negate, *regex, msgs)?;
            Ok((expr, pos + 1))
        }
        FilterToken::OpenParen => {
            let (expr, pos) = build_ast(tokens, pos + 1, msgs)?;
            if pos < tokens.len() && matches!(&tokens[pos], FilterToken::CloseParen) {
                Ok((expr, pos + 1))
            } else {
                Err(msgs.missing_close_paren.to_string())
            }
        }
        _ => Err(format!("{}: {:?}", msgs.unexpected_token, tokens[pos])),
    }
}
fn make_predicate(key: &Key, value: &str, negate: bool, regex: bool, msgs: &crate::i18n::Messages) -> Result<Expr, String> {
    match key {
        Key::Age => {
            let duration = parse_duration(value, msgs)?;
            let cutoff = Local::now() - duration;
            Ok(if negate {
                Expr::Pred(Predicate {
                    key: Key::Age,
                    value: value.to_string(),
                    negate: true,
                    regex: false,
                })
            } else {
                Expr::AgeSince {
                    cutoff,
                }
            })
        }
        Key::Name => {
            // name: is a no-op — always matches
            Ok(Expr::Pred(Predicate {
                key: Key::Name,
                value: value.to_string(),
                negate,
                regex,
            }))
        }
        _ => {
            Ok(Expr::Pred(Predicate {
                key: key.clone(),
                value: value.to_string(),
                negate,
                regex,
            }))
        }
    }
}

/// Parse a human-readable duration string like `5m`, `30s`, `1h`, `2d`.
fn parse_duration(s: &str, msgs: &crate::i18n::Messages) -> Result<Duration, String> {
    try_parse_duration(s).ok_or_else(|| {
        if s.is_empty() {
            msgs.age_needs_value.to_string()
        } else {
            msgs.invalid_age_value(s)
        }
    })
}

/// Parse a duration string, returning None on failure (no i18n).
fn try_parse_duration(s: &str) -> Option<Duration> {
    if s.is_empty() {
        return None;
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    match unit {
        "s" => Some(Duration::seconds(num)),
        "m" => Some(Duration::minutes(num)),
        "h" => Some(Duration::hours(num)),
        "d" => Some(Duration::days(num)),
        _ => None,
    }
}

impl Expr {
    /// Evaluate this expression against a log entry.
    pub fn evaluate(&self, entry: &LogEntry) -> bool {
        match self {
            Expr::And(left, right) => {
                left.evaluate(entry) && right.evaluate(entry)
            }
            Expr::Or(left, right) => {
                left.evaluate(entry) || right.evaluate(entry)
            }
            Expr::Pred(p) => evaluate_predicate(p, entry),
            Expr::AgeSince { cutoff, .. } => {
                if let chrono::MappedLocalTime::Single(ts) =
                    chrono::Local.from_local_datetime(&entry.timestamp)
                {
                    ts >= *cutoff
                } else {
                    false
                }
            }
        }
    }
}

fn evaluate_predicate(p: &Predicate, entry: &LogEntry) -> bool {
    let result = match p.key {
        Key::Tag => {
            let tag = &entry.tag;
            if p.regex {
                match Regex::new(&p.value) {
                    Ok(re) => re.is_match(tag),
                    Err(_) => false,
                }
            } else {
                tag.contains(&p.value)
            }
        }
        Key::Package | Key::Process => {
            // Use resolved package name if available, fall back to PID string.
            let target = entry.package.as_deref().unwrap_or("");
            if target.is_empty() {
                // No package resolved yet — match against PID as fallback
                let pid_str = entry.pid.to_string();
                if p.regex {
                    Regex::new(&p.value).is_ok_and(|re| re.is_match(&pid_str))
                } else {
                    pid_str.contains(&p.value)
                }
            } else if p.regex {
                Regex::new(&p.value).is_ok_and(|re| re.is_match(target))
            } else {
                target.contains(&p.value)
            }
        }
        Key::Message => {
            let msg = &entry.message;
            if p.regex {
                match Regex::new(&p.value) {
                    Ok(re) => re.is_match(msg),
                    Err(_) => false,
                }
            } else {
                msg.contains(&p.value)
            }
        }
        Key::Level => {
            // Level filtering uses >= semantics (e.g., level:INFO matches INFO and above)
            let target = match p.value.to_uppercase().as_str() {
                "V" | "VERBOSE" => crate::logcat::LogLevel::Verbose,
                "D" | "DEBUG" => crate::logcat::LogLevel::Debug,
                "I" | "INFO" => crate::logcat::LogLevel::Info,
                "W" | "WARN" | "WARNING" => crate::logcat::LogLevel::Warn,
                "E" | "ERROR" => crate::logcat::LogLevel::Error,
                "F" | "FATAL" => crate::logcat::LogLevel::Fatal,
                "S" | "SILENT" => crate::logcat::LogLevel::Fatal,
                _ => return false,
            };
            entry.level as u8 >= target as u8
        }
        Key::Age => {
            // Age predicates (negated) compare entry timestamp to a cutoff
            if let Some(duration) = try_parse_duration(&p.value) {
                let cutoff = Local::now() - duration;
                if let chrono::MappedLocalTime::Single(ts) =
                    chrono::Local.from_local_datetime(&entry.timestamp)
                {
                    ts >= cutoff
                } else {
                    false
                }
            } else {
                false
            }
        }
        Key::Is => {
            match p.value.as_str() {
                "crash" => {
                    entry.message.contains("FATAL EXCEPTION")
                        || entry.message.contains("AndroidRuntime")
                }
                "stacktrace" => {
                    // Stack trace continuation lines start with whitespace + "at "
                    entry.message.starts_with("at ")
                        || (entry.message.starts_with('\t') && entry.message.contains("at "))
                }
                _ => false,
            }
        }
        Key::Name => {
            // name: is a no-op — always matches
            true
        }
    };

    if p.negate { !result } else { result }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logcat::LogLevel;

    fn make_entry(tag: &str, msg: &str, level: LogLevel) -> LogEntry {
        LogEntry {
            timestamp: chrono::Local::now().naive_local(),
            pid: 1234,
            tid: 5678,
            level,
            tag: tag.to_string(),
            message: msg.to_string(),
            package: None,
        }
    }

    #[test]
    fn test_tag_filter() {
        let expr = compile("tag:MyTag").unwrap();
        let entry = make_entry("MyTag", "hello", LogLevel::Info);
        assert!(expr.evaluate(&entry));

        let entry2 = make_entry("Other", "hello", LogLevel::Info);
        assert!(!expr.evaluate(&entry2));
    }

    #[test]
    fn test_negated_tag() {
        let expr = compile("-tag:MyTag").unwrap();
        let entry = make_entry("MyTag", "hello", LogLevel::Info);
        assert!(!expr.evaluate(&entry));

        let entry2 = make_entry("Other", "hello", LogLevel::Info);
        assert!(expr.evaluate(&entry2));
    }

    #[test]
    fn test_and_precedence() {
        // & binds tighter than |
        // tag:foo | level:ERROR & tag:bar  ≡  tag:foo | (level:ERROR & tag:bar)
        let expr = compile("tag:foo | level:ERROR & tag:bar").unwrap();
        let entry = make_entry("foo", "msg", LogLevel::Info);
        assert!(expr.evaluate(&entry), "tag:foo should match (left of |)");

        let entry2 = make_entry("bar", "msg", LogLevel::Error);
        assert!(expr.evaluate(&entry2), "level:ERROR & tag:bar should match");

        let entry3 = make_entry("bar", "msg", LogLevel::Info);
        assert!(!expr.evaluate(&entry3), "tag:bar without ERROR should not match");
    }

    #[test]
    fn test_implicit_same_key_or() {
        // tag:foo tag:bar  ≡  (tag:foo | tag:bar)
        let expr = compile("tag:foo tag:bar").unwrap();

        let e1 = make_entry("foo", "msg", LogLevel::Info);
        assert!(expr.evaluate(&e1), "tag:foo should match");

        let e2 = make_entry("bar", "msg", LogLevel::Info);
        assert!(expr.evaluate(&e2), "tag:bar should match");

        let e3 = make_entry("baz", "msg", LogLevel::Info);
        assert!(!expr.evaluate(&e3), "tag:baz should not match");
    }

    #[test]
    fn test_implicit_different_key_and() {
        // tag:foo & level:ERROR → implicit AND between different keys
        // Both must match for the entry to pass
        let expr = compile("tag:foo level:ERROR").unwrap();

        let e1 = make_entry("foo", "msg", LogLevel::Error);
        assert!(expr.evaluate(&e1), "tag:foo & level:ERROR");

        let e2 = make_entry("foo", "msg", LogLevel::Info);
        assert!(!expr.evaluate(&e2), "tag:foo but not ERROR");

        let e3 = make_entry("bar", "msg", LogLevel::Error);
        assert!(!expr.evaluate(&e3), "level:ERROR but not tag:foo");
    }

    #[test]
    fn test_level_geq() {
        let expr = compile("level:INFO").unwrap();
        assert!(expr.evaluate(&make_entry("T", "msg", LogLevel::Info)));
        assert!(expr.evaluate(&make_entry("T", "msg", LogLevel::Warn)));
        assert!(expr.evaluate(&make_entry("T", "msg", LogLevel::Error)));
        assert!(!expr.evaluate(&make_entry("T", "msg", LogLevel::Debug)));
        assert!(!expr.evaluate(&make_entry("T", "msg", LogLevel::Verbose)));
    }

    #[test]
    fn test_negated_regex() {
        let expr = compile("-tag~:My.*Tag").unwrap();
        let e1 = make_entry("MyTestTag", "msg", LogLevel::Info);
        assert!(!expr.evaluate(&e1));

        let e2 = make_entry("Something", "msg", LogLevel::Info);
        assert!(expr.evaluate(&e2));
    }

    #[test]
    fn test_age_filter() {
        use chrono::{Local, TimeDelta};
        // Create entries at various ages
        let now = Local::now().naive_local();
        let entry_old = LogEntry {
            timestamp: now - TimeDelta::minutes(10),
            pid: 0, tid: 0,
            level: LogLevel::Info,
            tag: "T".into(),
            message: "old".into(),
            package: None,
        };
        let entry_recent = LogEntry {
            timestamp: now - TimeDelta::minutes(2),
            pid: 0, tid: 0,
            level: LogLevel::Info,
            tag: "T".into(),
            message: "recent".into(),
            package: None,
        };

        let expr = compile("age:5m").unwrap();
        assert!(!expr.evaluate(&entry_old), "10min old should not pass age:5m");
        assert!(expr.evaluate(&entry_recent), "2min old should pass age:5m");
    }

    #[test]
    fn test_is_stacktrace() {
        let expr = compile("is:stacktrace").unwrap();
        let entry = make_entry("T", "\tat com.example.Foo.bar(Foo.java:42)", LogLevel::Info);
        assert!(expr.evaluate(&entry));

        let entry2 = make_entry("T", "normal message", LogLevel::Info);
        assert!(!expr.evaluate(&entry2));
    }

    #[test]
    fn test_is_crash() {
        let expr = compile("is:crash").unwrap();
        let entry = make_entry("AndroidRuntime", "FATAL EXCEPTION: main", LogLevel::Error);
        assert!(expr.evaluate(&entry));

        let entry2 = make_entry("T", "normal", LogLevel::Info);
        assert!(!expr.evaluate(&entry2));
    }

    #[test]
    fn test_invalid_filter_error() {
        assert!(compile("tag:").is_err());
        assert!(compile("").is_ok()); // empty = match all
    }

    #[test]
    fn test_quoted_value() {
        let expr = compile("tag:\"hello world\"").unwrap();
        let entry = make_entry("hello world", "msg", LogLevel::Info);
        assert!(expr.evaluate(&entry));
    }

    #[test]
    fn test_name_noop() {
        let expr = compile("name:MySavedFilter").unwrap();
        let entry = make_entry("AnyTag", "any msg", LogLevel::Info);
        assert!(expr.evaluate(&entry), "name: should be a no-op");
    }
}
