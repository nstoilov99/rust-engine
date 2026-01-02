//! Input tokenizer and argument parsing
//!
//! Handles parsing raw input strings into command and arguments,
//! with support for quoted strings.

/// Parsed command input
#[derive(Debug, Clone)]
pub struct ParsedInput {
    /// Command name (first token)
    pub command: String,
    /// Arguments (remaining tokens)
    pub args: Vec<String>,
}

/// Parse raw input string into command and arguments
///
/// Handles:
/// - Basic whitespace splitting
/// - Quoted strings (single and double quotes)
/// - Empty input (returns None)
pub fn parse_input(input: &str) -> Option<ParsedInput> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    let tokens = tokenize(input);
    if tokens.is_empty() {
        return None;
    }

    let command = tokens[0].clone();
    let args = tokens[1..].to_vec();

    Some(ParsedInput { command, args })
}

/// Tokenize input respecting quotes
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';

    for c in input.chars() {
        match c {
            '"' | '\'' if !in_quotes => {
                in_quotes = true;
                quote_char = c;
            }
            c if c == quote_char && in_quotes => {
                in_quotes = false;
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Helper functions for argument parsing within commands
pub mod args {
    /// Parse a boolean argument ("true", "false", "1", "0", "yes", "no")
    pub fn parse_bool(s: &str) -> Result<bool, String> {
        match s.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(format!("Invalid boolean: '{}'", s)),
        }
    }

    /// Parse an integer argument
    pub fn parse_int<T: std::str::FromStr>(s: &str) -> Result<T, String>
    where
        T::Err: std::fmt::Display,
    {
        s.parse()
            .map_err(|e| format!("Invalid integer '{}': {}", s, e))
    }

    /// Parse a float argument
    pub fn parse_float<T: std::str::FromStr>(s: &str) -> Result<T, String>
    where
        T::Err: std::fmt::Display,
    {
        s.parse()
            .map_err(|e| format!("Invalid float '{}': {}", s, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let result = parse_input("help").unwrap();
        assert_eq!(result.command, "help");
        assert!(result.args.is_empty());
    }

    #[test]
    fn test_parse_with_args() {
        let result = parse_input("echo hello world").unwrap();
        assert_eq!(result.command, "echo");
        assert_eq!(result.args, vec!["hello", "world"]);
    }

    #[test]
    fn test_parse_quoted() {
        let result = parse_input("echo \"hello world\"").unwrap();
        assert_eq!(result.command, "echo");
        assert_eq!(result.args, vec!["hello world"]);
    }

    #[test]
    fn test_parse_empty() {
        assert!(parse_input("").is_none());
        assert!(parse_input("   ").is_none());
    }
}
