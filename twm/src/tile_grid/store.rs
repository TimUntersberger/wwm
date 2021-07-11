use log::{error, info};
use std::{fs, path::PathBuf};

const TEMPLATE: &'static str = "\n\n\n\n\n\n\n\n\n\n\n"; // TODO: check for OS compatability?
const PINNED_INDEX: usize = 10;
const PINNED_VISIBLE: &'static str = "v";
const PINNED_INVISIBLE: &'static str = "n";
const EMPTY: &'static str = "";

pub struct Store {}
pub struct StoredData {
    pub grids: Vec::<String>,
    pub are_pinned_visible: bool,
    pub pinned_windows: Vec::<i32>,
}

impl Store {
    fn get_path() -> PathBuf {
        #[allow(unused_mut)]
        let mut path: PathBuf = ["./log"].iter().collect();
        #[cfg(not(debug_assertions))]
        {
            path = dirs::config_dir().expect("Failed to get config directory");

            path.push("nog");
        }

        path.push("workspaces.grid");
        path
    }
    pub fn save(id: i32, grid: String) {
        info!("Saving {} {}", id, grid);
        Store::write_to_file((id - 1) as usize, grid);
    }
    pub fn save_pinned(pinned: Vec::<&i32>, is_visible: bool) {
        let visible = if is_visible { PINNED_VISIBLE } else { PINNED_INVISIBLE };
        let pinned = pinned.iter()
                           .map(|x| x.to_string())
                           .collect::<Vec::<_>>()
                           .join("|");
        let pinned = format!("{}|{}", visible, pinned);
        info!("Saving Pinned IDs {}", pinned);
        Store::write_to_file(PINNED_INDEX, pinned);
    }
    pub fn load() -> StoredData {
        let data = match fs::read_to_string(Store::get_path()) {
                       Ok(f) => f,
                       Err(_) => TEMPLATE.into(),
                   }
                   .split("\n")
                   .map(|x| x.to_string())
                   .collect::<Vec::<_>>();

        let (grids, other_data) = data.split_at(10);
        let empty = EMPTY.into();
        let stored_pinned = other_data.get(0).unwrap_or(&empty);
        
        let split_pinned = stored_pinned.split("|").collect::<Vec::<_>>();
        let (are_pinned_visible, pinned_ids) = split_pinned.split_at(1);
        let are_pinned_visible = are_pinned_visible.get(0).unwrap_or(&PINNED_INVISIBLE);
        let pinned_windows = pinned_ids.into_iter()
                                       .map(|x| x.parse::<i32>()) 
                                       .filter(|x| x.is_ok())
                                       .map(|x| x.unwrap())
                                       .collect();

        StoredData {
            grids: Vec::from(grids),
            are_pinned_visible: *are_pinned_visible == PINNED_VISIBLE,
            pinned_windows,
        }
    }
    fn write_to_file(index: usize, new_value: String) {
        let file = match fs::read_to_string(Store::get_path()) {
            Ok(f) => f,
            Err(_) => TEMPLATE.into(),
        };
        let file: String = file
            .split("\n")
            .into_iter()
            .enumerate()
            .map(|(i, old_value)| if i == index { &new_value } else { old_value })
            .collect::<Vec<_>>()
            .join("\n");

        match fs::write(Store::get_path(), file) {
            Err(e) => error!("Error storing grid {:?}", e),
            _ => (),
        }
    }
}
