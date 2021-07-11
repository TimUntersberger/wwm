use regex::Regex;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Action {
    /// Forces nog to manage the window
    Manage,
    /// Forces nog to pin the window
    Pin,
    /// Nog will check the other rules to see whether to manage the window
    Validate,
    /// Forces nog to not manage the window
    Ignore,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "{}", match self {
            Self::Manage => "manage",
            Self::Pin=> "pin",
            Self::Validate => "validate",
            Self::Ignore => "ignore",
        })
    }
}

impl std::str::FromStr for Action {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "manage" => Self::Manage,
            "pin" => Self::Pin,
            "validate" => Self::Validate,
            "ignore" => Self::Ignore,
            x => return Err(format!("{} is not a valid action for a rule", x))
        })
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub pattern: Regex,
    pub has_custom_titlebar: bool,
    pub action: Action,
    pub chromium: bool,
    pub firefox: bool,
    pub workspace_id: i32,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            pattern: Regex::new("").unwrap(),
            has_custom_titlebar: false,
            action: Action::Validate,
            chromium: false,
            firefox: false,
            workspace_id: -1,
        }
    }
}
