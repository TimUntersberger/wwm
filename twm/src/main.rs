#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
extern crate num_derive;
#[macro_use]
extern crate strum_macros;

use config::{rule::Rule, workspace_setting::WorkspaceSetting, Config};
use crossbeam_channel::select;
use direction::Direction;
use display::Display;
use event::Event;
use event::EventChannel;
use itertools::Itertools;
use keybindings::{keybinding::Keybinding, KeybindingsMessage, keybinding::KeybindingKind};
use log::debug;
use log::{error, info};
use lua::{setup_lua_rt, LuaRuntime};
use parking_lot::{deadlock, Mutex};
use split_direction::SplitDirection;
use std::path::PathBuf;
use std::process::Command;
use std::{fmt::Debug, fs::ReadDir};
use std::{mem, thread, time::Duration};
use std::{process, sync::Arc};
use system::NativeWindow;
use system::{DisplayId, SystemResult, WinEventListener, WindowId};
use task_bar::Taskbar;
use tile_grid::{store::Store, TileGrid};
use win_event_handler::{win_event::WinEvent, win_event_type::WinEventType};
use window::Window;

pub const NOG_BAR_NAME: &'static str = "nog_bar";
pub const NOG_POPUP_NAME: &'static str = "nog_popup";

#[macro_use]
#[allow(unused_macros)]
mod macros {
    /// logs the amount of time it took to execute the passed expression
    macro_rules! time {
        ($name: expr, $expr: expr) => {{
            let timer = std::time::Instant::now();
            let temp = $expr;
            log::debug!("{} took {:?}", $name, timer.elapsed());
            temp
        }};
    }
    /// sleeps for the given milliseconds
    macro_rules! sleep {
        ($ms: expr) => {
            std::thread::sleep(std::time::Duration::from_millis($ms))
        };
    }
    /// only runs the code if this is compiled on windows
    ///
    /// usage
    /// ```rust
    /// windows! {
    ///     // only runs on windows
    /// }
    /// ```
    /// TODO: correctly implement this
    macro_rules! windows {
        ($( $stmt:stmt )*) => {
            #[cfg(target_os = "windows")]
            {
                $(
                    $stmt
                )*
            };
        }
    }
    /// This macro either gets the Ok(..) value of the first expression or returns the second
    /// expression.
    macro_rules! fail_silent_with {
        ($expr: expr, $value: expr) => {
            match $expr {
                Ok(r) => r,
                Err(m) => return $value,
            };
        };
    }
    /// This macro either gets the Ok(..) value of the first expression or returns the second
    /// expression.
    /// This also prints the error using log::error
    macro_rules! fail_with {
        ($expr: expr, $value: expr) => {
            match $expr {
                Ok(r) => r,
                Err(m) => {
                    error!("{}", m);
                    return $value;
                }
            };
        };
    }
    /// This macro either gets the Ok(..) value of the passed expression or returns an Ok(()).
    /// This also prints the error using log::error
    macro_rules! fail {
        ($expr: expr) => {
            match $expr {
                Ok(r) => r,
                Err(m) => {
                    error!("{}", m);
                    return Ok(());
                }
            };
        };
    }
}

mod bar;
mod config;
mod direction;
mod display;
mod event;
mod event_handler;
mod keybindings;
mod keyboardhook;
mod logging;
mod lua;
mod message_loop;
// mod nogscript;
mod popup;
mod renderer;
mod split_direction;
mod startup;
mod system;
mod task_bar;
mod tile;
mod tile_grid;
mod tray;
mod update;
mod util;
mod win_event_handler;
mod window;

#[derive(Debug, Clone)]
pub struct AppState {
    pub lua_rt: LuaRuntime,
    pub config: Config,
    pub work_mode: bool,
    pub displays: Vec<Display>,
    pub event_channel: EventChannel,
    pub additonal_rules: Vec<Rule>,
    pub window_event_listener: WinEventListener,
    pub workspace_id: i32,
}

impl Default for AppState {
    fn default() -> Self {
        let config = Config::default();
        Self {
            work_mode: true,
            lua_rt: LuaRuntime::new(),
            displays: time!("initializing displays", display::init(&config)),
            event_channel: EventChannel::default(),
            additonal_rules: Vec::new(),
            window_event_listener: WinEventListener::default(),
            workspace_id: 1,
            config,
        }
    }
}

impl AppState {
    pub fn init(&mut self, state_arc: Arc<Mutex<AppState>>) {
        self.work_mode = self.config.work_mode;
        self.displays = display::init(&self.config);
    }

    /// TODO: maybe rename this function
    pub fn cleanup(&mut self) -> SystemResult {
        for d in self.displays.iter_mut() {
            for grid in d.grids.iter_mut() {
                grid.cleanup()?;
            }
        }

        Ok(())
    }

    pub fn install_plugin(&mut self, name: String) -> SystemResult {
        let url = format!("https://www.github.com/{}", &name);
        let mut path = self.config.plugins_path.clone();
        path.push(name.split("/").join("_"));

        if path.exists() {
            debug!("{} is already installed", name);
        } else {
            debug!("Installing {} from {}", name, url);
            Command::new("git")
                .arg("clone")
                .arg(&url)
                .arg(&path)
                .spawn()
                .unwrap()
                .wait()
                .unwrap();

            path.push("plugin");
        }

        Ok(())
    }

    pub fn update_plugins(&mut self) -> SystemResult {
        if let Ok(dirs) = get_plugins_path_iter() {
            for dir in dirs {
                if let Ok(dir) = dir {
                    let name = dir.file_name().to_str().unwrap().to_string();

                    let mut path = self.config.plugins_path.clone();
                    path.push(&name);

                    let name = name.split("_").join("/");
                    let url = format!("https://www.github.com/{}", name);

                    let output = Command::new("git")
                        .arg("rev-parse")
                        .arg("--is-inside-work-tree")
                        .current_dir(&path)
                        .output()
                        .unwrap();

                    let is_git_repo = output.stdout.iter().map(|&x| x as char).count() != 0;

                    if !is_git_repo {
                        debug!("{} is not a git repo", name);
                        continue;
                    }

                    let output = Command::new("git")
                        .arg("rev-list")
                        .arg("HEAD...origin/master")
                        .arg("--count")
                        .current_dir(&path)
                        .output()
                        .unwrap();

                    let has_updates =
                        output.stdout.iter().map(|&x| x as char).collect::<String>() != "0\n";

                    if has_updates {
                        debug!("Updating {}", name);
                        Command::new("git")
                            .arg("pull")
                            .arg(&url)
                            .spawn()
                            .unwrap()
                            .wait()
                            .unwrap();
                    } else {
                        debug!("{} is up to date", &name);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn uninstall_plugin(&mut self, name: String) -> SystemResult {
        let mut path = self.config.plugins_path.clone();
        path.push(name.split("/").join("_"));

        if path.exists() {
            debug!("Uninstalling {}", name);
            if let Err(e) = std::fs::remove_dir_all(path) {
                error!("Failed to remove plugin: {}", e.to_string());
            }
        } else {
            debug!("{} is not installed", name);
        }
        Ok(())
    }

    pub fn get_plugins(&self) -> SystemResult<Vec<String>> {
        let mut list: Vec<String> = Vec::new();

        if let Ok(dirs) = get_plugins_path_iter() {
            for dir in dirs {
                if let Ok(dir) = dir {
                    list.push(dir.path().to_str().unwrap().into());
                }
            }
        }

        Ok(list)
    }

    pub fn create_popup(&mut self) -> SystemResult {
        Ok(())
    }

    pub fn get_window_title(&mut self) -> SystemResult<String> {
        Ok(self
            .get_current_grid()
            .and_then(|g| g.get_focused_window())
            .and_then(|w| w.get_title().ok())
            .unwrap_or_default())
    }

    //TODO: Make this work at runtime after initilization
    pub fn add_keybinding(&mut self, kb: Keybinding) {
        self.config.add_keybinding(kb.clone());
    }

    pub fn emit_change_workspace(&mut self, id: i32) -> SystemResult {
        self.event_channel
            .sender
            .send(Event::ChangeWorkspace(id, true));

        Ok(())
    }

    pub fn emit_lua_rt_error(&mut self, msg: &str) {
        self.event_channel
            .sender
            .send(Event::LuaRuntimeError(mlua::Error::RuntimeError(
                msg.to_string(),
            )));
    }

    pub fn move_workspace_to_monitor(&mut self, monitor: i32) -> SystemResult {
        if self.get_display_by_idx_mut(monitor).is_none() {
            error!("Monitor with id {} doesn't exist", monitor);
            return Ok(());
        }
        let display = self.get_current_display_mut();

        if let Some(grid) = display
            .focused_grid_id
            .and_then(|id| display.remove_grid_by_id(id))
        {
            let config = self.config.clone();
            let new_display = self.get_display_by_idx_mut(monitor).unwrap();
            let id = grid.id;

            new_display.grids.push(grid);
            new_display.grids.sort_by_key(|g| g.id);
            new_display.focus_workspace(&config, id)?;
            self.workspace_id = id;
        }

        Ok(())
    }

    pub fn move_workspace_to_workspace(&mut self, workspace_id: i32) -> SystemResult {
        let is_empty = self
            .get_grid_by_id(workspace_id)
            .map_or(false, |g| g.is_empty());
        let current_id = self.workspace_id.clone();
        let current_grid_exists = self.get_current_grid().is_some();
        if is_empty && current_grid_exists && current_id != workspace_id {
            let mut empty_grid = TileGrid::new(current_id, renderer::NativeRenderer);
            let source = self.get_current_grid_mut().unwrap();
            source.id = workspace_id;
            mem::swap(source, &mut empty_grid);
            let target = self.get_grid_by_id_mut(workspace_id).unwrap();
            target.id = current_id;
            mem::swap(target, &mut empty_grid);

            let config = self.config.clone();
            if let Some(display) = self.find_grid_display_mut(workspace_id) {
                display.focus_workspace(&config, workspace_id)?;
                self.workspace_id = workspace_id;
            }
        }

        Ok(())
    }

    pub fn minimize_window(&mut self) -> SystemResult {
        let config = self.config.clone();
        let grid = self.get_current_grid_mut().unwrap();

        grid.modify_focused_window(|window| {
            window.minimize()?;
            window.cleanup()
        })?;

        grid.close_focused();

        let display = self.get_current_display_mut();
        display.refresh_grid(&config)?;

        Ok(())
    }

    pub fn close_window(&mut self) -> SystemResult {
        if popup::is_visible() {
            return popup::close();
        }

        let config = self.config.clone();
        let grid = self.get_current_grid_mut().unwrap();

        grid.modify_focused_window(|window| {
            window.cleanup()?;
            window.close()
        })?;

        grid.close_focused();

        let display = self.get_current_display_mut();
        display.refresh_grid(&config)?;

        Ok(())
    }

    pub fn ignore_window(&mut self) -> SystemResult {
        if let Some(window) = self.get_current_grid().unwrap().get_focused_window() {
            let mut rule = Rule::default();

            let process_name = window.get_process_name();
            let pattern = format!("^{}$", process_name);

            debug!("Adding rule with pattern {}", pattern);

            rule.pattern = regex::Regex::new(&pattern).expect("Failed to build regex");
            rule.manage = false;

            self.additonal_rules.push(rule);

            self.toggle_floating();
        }

        Ok(())
    }

    pub fn move_window_to_workspace(&mut self, id: i32) -> SystemResult {
        let grid = self.get_current_grid_mut().unwrap();
        let window = grid.pop();

        window.map(|window| {
            if let Some(target_grid) = self.get_grid_by_id_mut(id) {
                window.hide();
                target_grid.push(window);
                Store::save(id, target_grid.to_string());
            }
        });

        let config = self.config.clone();
        let display = self.get_current_display_mut();
        display.refresh_grid(&config)?;

        Ok(())
    }

    pub fn toggle_fullscreen(&mut self) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();
        display.get_focused_grid_mut().unwrap().toggle_fullscreen();
        display.refresh_grid(&config)?;

        Ok(())
    }

    pub fn enter_work_mode(state_arc: Arc<Mutex<AppState>>) -> SystemResult {
        let mut this = state_arc.lock();
        if this.config.remove_task_bar {
            info!("Hiding taskbar");
            this.hide_taskbars();
        }

        if this.config.display_app_bar {
            drop(this);
            bar::create::create(state_arc.clone());
            this = state_arc.lock();
        }

        let mut focused_workspaces = Vec::<i32>::new();
        let remove_title_bar = this.config.remove_title_bar;
        let use_border = this.config.use_border;
        let stored_grids: Vec<String> = Store::load();
        let rules = this.config.rules.clone();
        let additional_rules = this.additonal_rules.clone();
        for display in this.displays.iter_mut() {
            for grid in display.grids.iter_mut() {
                if let Some(stored_grid) = stored_grids.get((grid.id - 1) as usize) {
                    grid.from_string(stored_grid);
                    Store::save(grid.id, grid.to_string());

                    if let Err(e) = grid.modify_windows(|window| {
                        let rules = rules.iter().chain(additional_rules.iter()).collect();
                        window.set_matching_rule(rules);
                        window.init(remove_title_bar, use_border)?;

                        Ok(())
                    }) {
                        error!("Error while initializing window {:?}", e);
                    }
                }

                grid.hide(); // hides all the windows just loaded into the grid
            }

            if let Some(id) = display.focused_grid_id {
                focused_workspaces.push(id);
            }
        }

        if !focused_workspaces.is_empty() {
            // re-focus to show each display's focused workspace
            for id in focused_workspaces.iter().rev() {
                this.change_workspace(*id, false)?;
            }
        } else {
            // otherwise just focus first workspace
            this.change_workspace(1, false)?;
        }

        info!("Registering windows event handler");
        this.window_event_listener.start(&this.event_channel);

        let tx = this.event_channel.sender.clone();

        drop(this);

        tx.send(Event::UpdateKeybindings);

        Ok(())
    }

    pub fn leave_work_mode(state_arc: Arc<Mutex<AppState>>) -> SystemResult {
        let mut this = state_arc.lock();
        let tx = this.event_channel.sender.clone();
        tx.send(Event::UpdateKeybindings);
        drop(this);
        let mut this = state_arc.lock();
        this.window_event_listener.stop();

        popup::cleanup()?;

        if this.config.display_app_bar {
            drop(this);
            bar::close_all(state_arc.clone());
            this = state_arc.lock();
        }

        if this.config.remove_task_bar {
            this.show_taskbars();
        }

        this.cleanup()?;
        Ok(())
    }

    pub fn toggle_work_mode(state_arc: Arc<Mutex<AppState>>) -> SystemResult {
        let mut this = state_arc.lock();
        this.work_mode = !this.work_mode;

        if !this.work_mode {
            drop(this);
            Self::leave_work_mode(state_arc)?;
        } else {
            drop(this);
            Self::enter_work_mode(state_arc)?;
        }

        Ok(())
    }

    pub fn swap(&mut self, direction: Direction) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();

        if let Some(grid) = display.get_focused_grid_mut() {
            if !config.ignore_fullscreen_actions || !grid.is_fullscreened() {
                grid.swap_focused(direction);
                display.refresh_grid(&config)?;
            }
        }

        Ok(())
    }

    pub fn swap_columns_and_rows(&mut self) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();

        if let Some(grid) = display.get_focused_grid_mut() {
            if !config.ignore_fullscreen_actions || !grid.is_fullscreened() {
                grid.swap_columns_and_rows();
                display.refresh_grid(&config)?;
            }
        }

        Ok(())
    }

    pub fn move_in(&mut self, direction: Direction) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();

        if let Some(grid) = display.get_focused_grid_mut() {
            if !config.ignore_fullscreen_actions || !grid.is_fullscreened() {
                grid.move_focused_in(direction);
                display.refresh_grid(&config)?;
            }
        }

        Ok(())
    }

    pub fn move_out(&mut self, direction: Direction) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();

        if let Some(grid) = display.get_focused_grid_mut() {
            if !config.ignore_fullscreen_actions || !grid.is_fullscreened() {
                grid.move_focused_out(direction);
                display.refresh_grid(&config)?;
            }
        }

        Ok(())
    }

    pub fn focus(&mut self, direction: Direction) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();

        if let Some(grid) = display.get_focused_grid_mut() {
            if !config.ignore_fullscreen_actions || !grid.is_fullscreened() {
                grid.focus(direction)?;
                display.refresh_grid(&config)?;
            }
        }

        Ok(())
    }

    pub fn resize(&mut self, direction: Direction, amount: i32) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();

        if let Some(grid) = display.get_focused_grid_mut() {
            if !config.ignore_fullscreen_actions || !grid.is_fullscreened() {
                grid.trade_size_with_neighbor(grid.focused_id, direction, amount);
                info!("Resizing in the direction {:?} by {}", direction, amount);

                display.refresh_grid(&config)?;
            }
        }
        Ok(())
    }

    pub fn set_split_direction(&mut self, direction: SplitDirection) -> SystemResult {
        let display = self.get_current_display_mut();
        if let Some(grid) = display.get_focused_grid_mut() {
            grid.next_axis = direction;
        }
        Ok(())
    }

    pub fn toggle_floating(&mut self) -> SystemResult {
        let config = self.config.clone();

        let window =
            NativeWindow::get_foreground_window().expect("Failed to get foreground window");
        let current_workspace_id = self.workspace_id;
        let grid = self.find_grid_containing_window(window.id);

        if let Some(grid) = grid {
            // don't do anything if focused window isn't on current grid
            if grid.id == current_workspace_id {
                if let Some(mut w) = grid.remove_by_window_id(window.id) {
                    debug!("Unmanaging window '{}' | {}", w.title, w.id);
                    w.cleanup();
                    if let Some(d) = self.find_grid_display(current_workspace_id) {
                        d.refresh_grid(&config);
                    }
                }
            }
        } else {
            self.event_channel
                .sender
                .clone()
                .send(Event::WinEvent(WinEvent {
                    typ: WinEventType::Show(true),
                    window,
                }))
                .expect("Failed to send WinEvent");
        }

        Ok(())
    }

    pub fn reset_column(&mut self) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();

        if let Some(g) = display.get_focused_grid_mut() {
            g.reset_column();
        }
        display.refresh_grid(&config)?;

        Ok(())
    }

    pub fn reset_row(&mut self) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();

        if let Some(g) = display.get_focused_grid_mut() {
            g.reset_row();
        }
        display.refresh_grid(&config)?;

        Ok(())
    }

    pub fn get_workspace_settings(&self, id: i32) -> Option<&WorkspaceSetting> {
        self.config.workspaces.iter().find(|s| s.id == id)
    }

    pub fn is_workspace_visible(&self, id: i32) -> bool {
        self.displays
            .iter()
            .find(|d| d.focused_grid_id == Some(id))
            .is_some()
    }

    pub fn change_workspace(&mut self, id: i32, _force: bool) -> SystemResult {
        let config = self.config.clone();
        let current = self.get_current_display().id;
        if let Some(d) = self.find_grid_display_mut(id) {
            let new = d.id;
            d.focus_workspace(&config, id)?;
            self.workspace_id = id;
            self.redraw_app_bars();
            if current != new {
                self.get_display_by_id(current)
                    .map(|d| d.refresh_grid(&config));
            }
        }

        Ok(())
    }

    pub fn get_ws_text(&mut self, id: i32) -> String {
        self.config
            .workspaces
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.text.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or(format!(" {} ", id.to_string()))
    }

    pub fn get_focused_ws_of_display(&self, display_id: i32) -> Option<i32> {
        self.get_display_by_id(DisplayId(display_id))
            .and_then(|d| d.get_focused_grid())
            .map(|ws| ws.id)
    }

    pub fn get_focused_win_of_display(&self, display_id: i32) -> Option<i32> {
        self.get_display_by_id(DisplayId(display_id))
            .and_then(|d| d.get_focused_grid())
            .and_then(|g| g.get_focused_window())
            .map(|w| w.id.0)
    }

    pub fn redraw_app_bars(&self) {
        debug!("Sending redraw-app-bar event");
        self.event_channel
            .sender
            .send(Event::RedrawAppBar)
            .expect("Failed to send redraw-app-bar event");
    }

    pub fn get_display_by_id(&self, id: DisplayId) -> Option<&Display> {
        self.displays.iter().find(|d| d.id == id)
    }

    pub fn get_display_by_id_mut(&mut self, id: DisplayId) -> Option<&mut Display> {
        self.displays.iter_mut().find(|d| d.id == id)
    }

    pub fn get_display_by_idx(&self, idx: i32) -> Option<&Display> {
        let x: usize = if idx == -1 {
            0
        } else {
            std::cmp::max(self.displays.len() - (idx as usize), 0)
        };

        self.displays.get(x)
    }

    pub fn get_display_by_idx_mut(&mut self, idx: i32) -> Option<&mut Display> {
        if idx > self.displays.len() as i32 {
            return None;
        }

        let x: usize = if idx == -1 {
            0
        } else {
            self.displays.len() - idx as usize
        };

        self.displays.get_mut(x)
    }

    pub fn get_taskbars(&self) -> Vec<&Taskbar> {
        self.displays
            .iter()
            .map(|d| d.taskbar.as_ref())
            .filter(|x| x.is_some())
            .map(|x| x.unwrap())
            .collect()
    }

    /// Returns the display containing the grid
    pub fn find_grid_display(&self, id: i32) -> Option<&Display> {
        for d in self.displays.iter() {
            if let Some(_) = d.grids.iter().find(|g| g.id == id) {
                return Some(d);
            }
        }
        None
    }

    /// Returns the display containing the grid
    pub fn find_grid_display_mut(&mut self, id: i32) -> Option<&mut Display> {
        for d in self.displays.iter_mut() {
            if let Some(_) = d.grids.iter().find(|g| g.id == id) {
                return Some(d);
            }
        }
        None
    }

    /// Returns the grid containing the window and its corresponding tile
    pub fn find_grid_containing_window(&mut self, id: WindowId) -> Option<&mut TileGrid> {
        for d in self.displays.iter_mut() {
            for g in d.grids.iter_mut() {
                if g.contains(id) {
                    return Some(g);
                }
            }
        }
        None
    }

    pub fn get_taskbars_mut(&mut self) -> Vec<&mut Taskbar> {
        self.displays
            .iter_mut()
            .map(|d| d.taskbar.as_mut())
            .filter(|x| x.is_some())
            .map(|x| x.unwrap())
            .collect()
    }

    pub fn show_taskbars(&self) {
        for tb in self.get_taskbars() {
            tb.window.show();
        }
    }

    pub fn hide_taskbars(&self) {
        // have to hide the taskbars in a specific order for it to work (I know like wtf)

        // first hide primary display
        for d in &self.displays {
            if d.is_primary() {
                if let Some(tb) = &d.taskbar {
                    tb.window.hide();
                }
                break;
            }
        }

        // then the other ones
        for d in &self.displays {
            if !d.is_primary() {
                if let Some(tb) = &d.taskbar {
                    tb.window.hide();
                }
            }
        }
    }

    pub fn get_current_display_mut(&mut self) -> &mut Display {
        let workspace_id = self.workspace_id;
        self.displays
            .iter_mut()
            .find(|d| d.grids.iter().any(|g| g.id == workspace_id))
            .unwrap()
    }

    pub fn get_current_display(&self) -> &Display {
        self.displays
            .iter()
            .find(|d| d.grids.iter().any(|g| g.id == self.workspace_id))
            .unwrap()
    }

    pub fn get_current_grid_mut(&mut self) -> Option<&mut TileGrid> {
        self.get_grid_by_id_mut(self.workspace_id)
    }

    pub fn get_current_grid(&self) -> Option<&TileGrid> {
        self.get_grid_by_id(self.workspace_id)
    }

    pub fn get_grids_mut(&mut self) -> Vec<&mut TileGrid> {
        self.displays
            .iter_mut()
            .map(|d| d.grids.iter_mut())
            .flatten()
            .collect()
    }

    pub fn get_grids(&self) -> Vec<&TileGrid> {
        self.displays
            .iter()
            .map(|d| d.grids.iter())
            .flatten()
            .collect()
    }

    pub fn get_grid_by_id_mut(&mut self, id: i32) -> Option<&mut TileGrid> {
        self.get_grids_mut().into_iter().find(|g| g.id == id)
    }

    pub fn get_grid_by_id(&self, id: i32) -> Option<&TileGrid> {
        self.get_grids().into_iter().find(|g| g.id == id)
    }
}

fn on_quit(state: &mut AppState) -> SystemResult {
    os_specific_cleanup();

    state.cleanup()?;

    popup::cleanup();

    if state.config.remove_task_bar {
        state.show_taskbars();
    }

    state.window_event_listener.stop();

    process::exit(0);
}

#[cfg(target_os = "windows")]
fn os_specific_cleanup() {
    if let Some(window) = tray::WINDOW.lock().as_ref() {
        tray::remove_icon(window.id.into());
    }
}

#[cfg(target_os = "windows")]
fn os_specific_setup(state: Arc<Mutex<AppState>>) {
    info!("Creating tray icon");
    tray::create(state);
}

fn run(state_arc: Arc<Mutex<AppState>>) -> Result<(), Box<dyn std::error::Error>> {
    let receiver = state_arc.lock().event_channel.receiver.clone();
    let sender = state_arc.lock().event_channel.sender.clone();

    if state_arc.lock().config.enable_hot_reloading {
        info!("Starting hot reloading of config");
        config::hot_reloading::start(state_arc.clone());
    }

    startup::set_launch_on_startup(state_arc.lock().config.launch_on_startup);

    os_specific_setup(state_arc.clone());

    info!("Listening for keybindings");
    let kb_tx = keybindings::listen(state_arc.clone());

    if state_arc.lock().config.work_mode {
        AppState::enter_work_mode(state_arc.clone())?;
    }

    loop {
        select! {
            recv(receiver) -> maybe_msg => {
                let msg = maybe_msg.unwrap();
                let _ = match msg {
                    Event::NewPopup(mut p) => {
                        p.create(state_arc.clone())?;
                        Ok(())
                    },
                    Event::ToggleAppbar(display_id) => {
                        let window = state_arc
                            .clone()
                            .lock()
                            .get_display_by_id(display_id)
                            .and_then(|d| d.appbar.as_ref())
                            .map(|bar| bar.window.get_native_window());

                        if let Some(win) = window {
                            if win.is_visible() {
                                println!("before");
                                win.hide();
                                println!("after");
                            } else {
                                win.show();
                            }
                        }
                        Ok(())
                    },
                    Event::UpdateKeybindings => {
                        kb_tx.send(KeybindingsMessage::UpdateKeybindings);
                        Ok(())
                    }
                    Event::Keybinding(kb) => {
                        debug!("Received keybinding {:?}", kb);
                        sender.send(Event::CallCallback { idx: kb.callback_id } ).unwrap();
                        Ok(())
                    },
                    Event::LuaRuntimeError(err) => {
                        error!("{}", lua::get_err_msg(&err));

                        Ok(())
                    }
                    Event::CallCallback { idx } => {
                        let rt = state_arc.lock().lua_rt.clone();
                        let res = rt.with_lua(|lua| {
                            LuaRuntime::get_callback(lua, idx)?.call::<_, ()>(())
                        });

                        if let Err(e) = res {
                            sender.send(Event::LuaRuntimeError(e));
                        }

                        Ok(())
                    },
                    Event::RedrawAppBar => {
                        let windows = state_arc.lock().displays.iter().map(|d| d.appbar.as_ref()).flatten().map(|b| b.window.clone()).collect::<Vec<Window>>();

                        for window in windows {
                            window.redraw();
                        }

                        Ok(())
                    },
                    Event::WinEvent(ev) => event_handler::winevent::handle(&mut state_arc.lock(), ev),
                    Event::Exit => {
                        on_quit(&mut state_arc.lock())?;
                        break;
                    },
                    Event::ReloadConfig => {
                        info!("Reloading Config");
                        let rt = state_arc.lock().lua_rt.clone();
                        run_config(&rt);

                        Ok(())
                    },
                    Event::UpdateBarSections(display_id, left, center, right) => {
                        let mut state = state_arc.lock();
                        for d in state.displays.iter_mut() {
                            if d.id == display_id {
                                if let Some(bar) = d.appbar.as_mut() {
                                    bar.left = left;
                                    bar.center = center;
                                    bar.right = right;
                                    break;
                                }
                            }
                        }
                        Ok(())
                    },
                    Event::ChangeWorkspace(id, force) => {
                        state_arc.lock().change_workspace(id, force);
                        Ok(())
                    }
                }.map_err(|e| {
                    error!("{:?}", e);
                    crate::system::win::api::print_last_error();
                });
            }
        }
    }

    Ok(())
}

fn get_config_path() -> PathBuf {
    let mut path: PathBuf = dirs::config_dir().unwrap_or_default();
    path.push("nog");
    path
}

fn get_runtime_path() -> PathBuf {
    #[cfg(debug_assertions)] // dev
    {
        let mut path: PathBuf = std::env::current_exe().unwrap();
        path.pop();
        path.pop();
        path.pop();
        path.push("twm");
        path.push("runtime");
        path
    }
    #[cfg(not(debug_assertions))] // prod
    {
        let mut path: PathBuf = dirs::data_dir().unwrap_or_default();
        path.push("nog");
        path.push("runtime");
        path
    }
}

fn get_plugins_path() -> Result<PathBuf, String> {
    let mut path: PathBuf = get_config_path();
    path.push("plugins");

    if !path.exists() {
        debug!("plugins folder doesn't exist yet. Creating the folder");
        std::fs::create_dir(path.clone()).map_err(|e| e.to_string())?;
    }

    Ok(path)
}

fn get_plugins_path_iter() -> Result<ReadDir, String> {
    Ok(get_plugins_path()?.read_dir().unwrap())
}

fn run_config(rt: &LuaRuntime) {
    let mut path = get_config_path();
    path.push("config");

    if !path.exists() {
        info!("config folder doesn't exist yet. Creating the folder");
        std::fs::create_dir(path.clone()).unwrap();
    }

    path.push("init.lua");

    if !path.exists() {
        info!("Config file is missing. Creating default config");
        std::fs::write(&path, include_bytes!("../../assets/default_config.lua")).unwrap();
    }

    rt.run_file(path);

    debug!("config execution finished");
}

fn main() {
    std::env::set_var("RUST_BACKTRACE", "1");
    logging::setup().expect("Failed to setup logging");

    info!("Config: {:?}", get_config_path());
    info!("Runtime: {:?}", get_runtime_path());

    let state_arc = Arc::new(Mutex::new(AppState::default()));

    debug!("Setting up lua runtime");
    setup_lua_rt(state_arc.clone());

    let rt = state_arc.lock().lua_rt.clone();
    run_config(&rt);
    rt.disable_setup().unwrap();

    info!("Initializing Application");
    state_arc.lock().init(state_arc.clone());
    info!("Initialized Application");

    let arc = state_arc.clone();

    thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(5));
        let deadlocks = deadlock::check_deadlock();
        if deadlocks.is_empty() {
            continue;
        }

        debug!("{} deadlocks detected", deadlocks.len());
        for (i, threads) in deadlocks.iter().enumerate() {
            debug!("Deadlock #{}", i);
            for t in threads {
                debug!("Thread Id {:#?}", t.thread_id());
                debug!("{:#?}", t.backtrace());
            }
        }

        on_quit(&mut arc.lock()).unwrap();
    });

    info!("");

    let arc = state_arc.clone();
    ctrlc::set_handler(move || {
        if let Err(e) = on_quit(&mut arc.lock()) {
            error!("Something happend when cleaning up. {}", e);
        }
    })
    .unwrap();

    let arc = state_arc.clone();
    if let Err(e) = run(state_arc.clone()) {
        error!("An error occured {:?}", e);
        if let Err(e) = on_quit(&mut arc.lock()) {
            error!("Something happend when cleaning up. {}", e);
        }
    }
}
