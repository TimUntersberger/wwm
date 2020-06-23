use crate::hot_key_manager::{
    key::Key, 
    modifier::Modifier, 
    Direction, 
    Keybinding, 
    KeybindingType
};
use crate::tile_grid::SplitDirection;
use regex::Regex;
use std::io::{Error, ErrorKind};

#[macro_use]
mod macros;

use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Rule {
    pub pattern: Regex,
    pub has_custom_titlebar: bool,
    pub manage: bool,
    pub workspace: i32,
    pub x: i32,
    pub width: i32,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            pattern: Regex::new("").unwrap(),
            has_custom_titlebar: false,
            manage: true,
            workspace: -1,
            x: 0,
            width: 0,
        }
    }
}

pub struct Config {
    pub app_bar_height: i32,
    pub app_bar_bg: i32,
    pub app_bar_font: String,
    pub app_bar_font_size: i32,
    pub app_bar_workspace_bg: i32,
    pub margin: i32,
    pub padding: i32,
    pub remove_title_bar: bool,
    pub remove_task_bar: bool,
    pub display_app_bar: bool,
    pub keybindings: Vec<Keybinding>,
    pub rules: Vec<Rule>,
}

impl Config {
    pub fn new() -> Self {
        Self {
            app_bar_height: 20,
            app_bar_bg: 0x0027242c,
            app_bar_font: String::from("Consolas"),
            app_bar_font_size: 18,
            app_bar_workspace_bg: 0x00161616,
            margin: 0,
            padding: 0,
            remove_title_bar: false,
            remove_task_bar: false,
            display_app_bar: false,
            keybindings: Vec::new(),
            rules: Vec::new(),
        }
    }
}

pub fn load() -> Result<Config, Box<dyn std::error::Error>> {
    let mut pathbuf = match dirs::config_dir() {
        Some(path) => path,
        None => std::path::PathBuf::new(),
    };

    pathbuf.push("wwm");

    if !pathbuf.exists() {
        std::fs::create_dir(pathbuf.clone())?;
    }

    pathbuf.push("config.yaml");

    if !pathbuf.exists() {
        std::fs::File::create(pathbuf.clone())?;
    }

    let path = match pathbuf.to_str() {
        Some(string) => string,
        None => "",
    };

    let file_content = std::fs::read_to_string(path)?;

    let vec_yaml = yaml_rust::YamlLoader::load_from_str(&file_content)?;
    let mut yaml = &yaml_rust::Yaml::Null;
    if !vec_yaml.is_empty() {
        yaml = &vec_yaml[0];
    }

    let mut config = Config::new();

    if let yaml_rust::yaml::Yaml::Hash(hash) = yaml {
        for entry in hash.iter() {
            let (key, value) = entry;
            let config_key = key.as_str().ok_or("Invalid config key")?;

            if_str!(config, config_key, value, app_bar_font);
            if_hex!(config, config_key, value, app_bar_bg);
            if_hex!(config, config_key, value, app_bar_workspace_bg);
            if_i32!(config, config_key, value, app_bar_font_size);
            if_i32!(config, config_key, value, app_bar_height);
            if_i32!(config, config_key, value, margin);
            if_i32!(config, config_key, value, padding);
            if_bool!(config, config_key, value, remove_title_bar);
            if_bool!(config, config_key, value, remove_task_bar);
            if_bool!(config, config_key, value, display_app_bar);

            if config_key == "rules" {
                let rules = value.as_vec().ok_or("rules has to be an array")?;

                for yaml_rule in rules {
                    if let yaml_rust::Yaml::Hash(hash) = yaml_rule {
                        let mut rule = Rule::default();

                        for entry in hash.iter() {
                            let (key, value) = entry;
                            let hash_key = key.as_str().ok_or("Invalid config key")?;

                            if_regex!(rule, hash_key, value, pattern);
                            if_bool!(rule, hash_key, value, has_custom_titlebar);
                            if_bool!(rule, hash_key, value, manage);
                            if_i32!(rule, hash_key, value, workspace);
                            if_i32!(rule, hash_key, value, x);
                            if_i32!(rule, hash_key, value, width);
                        }

                        config.rules.push(rule);
                    }
                }
            }

            if config_key == "keybindings" {
                let bindings = value.as_vec().ok_or("keybindings has to be an array")?;

                for binding in bindings {
                    let typ_str = ensure_str!("keybinding", binding, type);
                    let key_combo = ensure_str!("keybinding", binding, key);
                    let key_combo_parts = key_combo.split("+").collect::<Vec<&str>>();
                    let modifier_count = key_combo_parts.len() - 1;

                    let modifier = key_combo_parts
                        .iter()
                        .take(modifier_count)
                        .map(|x| match *x {
                            "Alt" => Modifier::ALT,
                            "Control" => Modifier::CONTROL,
                            "Shift" => Modifier::SHIFT,
                            "Win" => Modifier::WIN,
                            _ => Modifier::default(),
                        })
                        .fold(Modifier::default(), |mut sum, crr| {
                            sum.insert(crr);

                            sum
                        });

                    let key = key_combo_parts
                        .iter()
                        .last()
                        .and_then(|x| Key::from_str(x).ok())
                        .ok_or("Invalid key")?;

                    let typ =
                        match typ_str {
                            "Shell" => KeybindingType::Shell(
                                ensure_str!("keybinding of type shell", binding, cmd).to_string(),
                            ),
                            "CloseTile" => KeybindingType::CloseTile,
                            "Quit" => KeybindingType::Quit,
                            "ChangeWorkspace" => KeybindingType::ChangeWorkspace(ensure_i32!(
                                "keybinding of type shell",
                                binding,
                                id
                            )),
                            "ToggleFloatingMode" => KeybindingType::ToggleFloatingMode,
                            "Focus" => KeybindingType::Focus(Direction::from_str(ensure_str!(
                                "keybinding of type shell",
                                binding,
                                direction
                            ))?),
                            "Swap" => KeybindingType::Swap(Direction::from_str(ensure_str!(
                                "keybinding of type shell",
                                binding,
                                direction
                            ))?),
                            "Split" => KeybindingType::Split(SplitDirection::from_str(
                                ensure_str!("keybinding of type shell", binding, direction),
                            )?),
                            x => {
                                return Err(Box::new(Error::new(
                                    ErrorKind::InvalidInput,
                                    "unknown type ".to_string() + x,
                                )))
                            }
                        };

                    config.keybindings.push(Keybinding { key, modifier, typ });
                }
            }
        }
    }
    Ok(config)
}
