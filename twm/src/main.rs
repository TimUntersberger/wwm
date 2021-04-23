#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
extern crate num_derive;
#[macro_use]
extern crate strum_macros;
#[macro_use]
extern crate interpreter;

use bar::component::{self, Component, ComponentText};
use config::{rule::Rule, workspace_setting::WorkspaceSetting, Config};
use crossbeam_channel::select;
use direction::Direction;
use display::Display;
use event::Event;
use event::EventChannel;
use hot_reload::update_config;
use interpreter::{Dynamic, Function, Interpreter, Module, RuntimeError};
use itertools::Itertools;
use keybindings::{keybinding::Keybinding, KbManager};
use log::debug;
use log::{error, info};
use parking_lot::{deadlock, Mutex};
use popup::Popup;
use regex::Regex;
use split_direction::SplitDirection;
use std::fs::ReadDir;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::{mem, thread, time::Duration};
use std::{process, sync::atomic::AtomicBool, sync::Arc};
use system::NativeWindow;
use system::{DisplayId, SystemResult, SystemError, WinEventListener, WindowId};
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
mod hot_reload;
mod keybindings;
mod logging;
mod message_loop;
mod nogscript;
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
    pub config: Config,
    pub work_mode: bool,
    pub displays: Vec<Display>,
    pub event_channel: EventChannel,
    pub keybindings_manager: KbManager,
    pub additonal_rules: Vec<Rule>,
    pub window_event_listener: WinEventListener,
    pub workspace_id: i32,
}

impl Default for AppState {
    fn default() -> Self {
        let config = Config::default();
        Self {
            work_mode: true,
            displays: time!("initializing displays", display::init(&config)),
            keybindings_manager: KbManager::new(
                config.keybindings.clone(),
                config.mode_handlers.clone(),
                config.allow_right_alt,
            ),
            event_channel: EventChannel::default(),
            additonal_rules: Vec::new(),
            window_event_listener: WinEventListener::default(),
            workspace_id: 1,
            config,
        }
    }
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            work_mode: config.work_mode,
            displays: display::init(&config),
            keybindings_manager: KbManager::new(
                config.keybindings.clone(),
                config.mode_handlers.clone(),
                config.allow_right_alt,
            ),
            event_channel: EventChannel::default(),
            additonal_rules: Vec::new(),
            window_event_listener: WinEventListener::default(),
            workspace_id: 1,
            config,
        }
    }
    pub fn init(&mut self, config: Config) {
        self.config = config;
        self.work_mode = self.config.work_mode;
        self.displays = display::init(&self.config);
        self.keybindings_manager = KbManager::new(
            self.config.keybindings.clone(),
            self.config.mode_handlers.clone(),
            self.config.allow_right_alt,
        );
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
            grid.hide();

            let config = self.config.clone();
            let new_display = self.get_display_by_idx_mut(monitor).unwrap();
            let id = grid.id;

            new_display.grids.push(grid);
            new_display.focus_workspace(&config, id)?;
            self.workspace_id = id;
        }

        self.refresh_visible_grids()?;

        Ok(())
    }

    pub fn move_workspace_to_workspace(&mut self, workspace_id: i32) -> SystemResult {
        let is_empty = self
            .get_grid_by_id(workspace_id)
            .map_or(false, |g| g.is_empty());
        let current_id = self.workspace_id.clone();
        let current_grid_exists = self.get_current_grid().is_some();
        if is_empty && current_grid_exists && current_id != workspace_id {
            Store::save(current_id, "".into());
            let mut empty_grid = TileGrid::new(current_id, renderer::NativeRenderer);
            let source = self.get_current_grid_mut().unwrap();
            source.hide();
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

            self.refresh_visible_grids()?;
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
            self.get_grid_by_id_mut(id).unwrap().push(window);
            self.change_workspace(id, false);
        });

        self.refresh_visible_grids()?;

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
                this.change_workspace(*id, false);
            }
        } else {
            // otherwise just focus first workspace
            this.change_workspace(1, false);
        }

        info!("Registering windows event handler");
        this.window_event_listener.start(&this.event_channel);

        let kb = this.keybindings_manager.clone();

        drop(this);

        kb.enter_work_mode();

        Ok(())
    }

    pub fn leave_work_mode(state_arc: Arc<Mutex<AppState>>) -> SystemResult {
        let mut this = state_arc.lock();
        this.window_event_listener.stop();
        this.keybindings_manager.leave_work_mode();

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
                display.refresh_grid(&config);
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
                display.refresh_grid(&config);
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

    pub fn toggle_mode(&mut self, mode: String) {
        if self.keybindings_manager.get_mode() == Some(mode.clone()) {
            info!("Disabling {} mode", mode);
            self.keybindings_manager.leave_mode();
        } else {
            info!("Enabling {} mode", mode);
            self.keybindings_manager.enter_mode(&mode);
        }
    }

    pub fn get_workspace_settings(&self, id: i32) -> Option<&WorkspaceSetting> {
        self.config.workspace_settings.iter().find(|s| s.id == id)
    }

    pub fn is_workspace_visible(&self, id: i32) -> bool {
        self.displays
            .iter()
            .find(|d| d.focused_grid_id == Some(id))
            .is_some()
    }

    pub fn change_workspace(&mut self, id: i32, _force: bool) {
        let config = self.config.clone();
        let current = self.get_current_display().id;
        if let Some(d) = self.find_grid_display_mut(id) {
            let new = d.id;
            d.focus_workspace(&config, id);
            self.workspace_id = id;
            self.redraw_app_bars();
            if current != new {
                self.get_display_by_id(current)
                    .map(|d| d.refresh_grid(&config));
            }
        }
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

    pub fn refresh_visible_grids(&mut self) -> SystemResult {
        let config = self.config.clone();
        self.displays
            .iter_mut()
            .map(|d| d.refresh_grid(&config))
            .collect::<Result<(), SystemError>>()
    }

    pub fn refresh_current_grid(&mut self) -> SystemResult {
        let config = self.config.clone();
        let display = self.get_current_display_mut();
        display.refresh_grid(&config)?;

        Ok(())
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

fn parse_config(
    state_arc: Arc<Mutex<AppState>>,
    callbacks_arc: Arc<Mutex<Vec<Function>>>,
    interpreter_arc: Arc<Mutex<Interpreter>>,
) -> Result<Config, String> {
    callbacks_arc.lock().clear();
    let mut config = Config::default();

    config.bar.use_default_components(state_arc.clone());

    let config = Arc::new(Mutex::new(config));
    let mut interpreter = Interpreter::new();

    let is_init_inner = Arc::new(AtomicBool::new(true));
    let is_init_inner2 = is_init_inner.clone();
    let is_init = move || is_init_inner2.load(std::sync::atomic::Ordering::SeqCst);

    interpreter.debug = true;
    interpreter.source_locations = interpreter_arc.lock().source_locations.clone();
    let root = nogscript::lib::create_root_module(
        is_init,
        state_arc.clone(),
        callbacks_arc.clone(),
        interpreter_arc.clone(),
        config.clone(),
    );
    interpreter.add_module(root);

    let mut config_path: PathBuf = dirs::config_dir().unwrap_or_default();
    config_path.push("nog");
    let mut plugins_path = get_plugins_path().unwrap_or_default();

    config.lock().path = config_path.clone();
    interpreter.source_locations.push(config_path.clone());

    if !config_path.exists() {
        debug!("nog folder doesn't exist yet. Creating the folder");
        std::fs::create_dir(config_path.clone()).map_err(|e| e.to_string())?;
    }

    config.lock().plugins_path = plugins_path.clone();

    interpreter.source_locations.push(plugins_path.clone());

    config_path.push("config.ns");

    if !config_path.exists() {
        debug!("config file doesn't exist yet. Creating the file");
        if let Ok(mut file) = std::fs::File::create(config_path.clone()) {
            debug!("Initializing config with default values");
            // file.write_all(include_bytes!("../../../assets/default_config.nog"))
            //     .map_err(|e| e.to_string())?;
        }
    }

    debug!("Running config file");

    interpreter.execute_file(config_path)?;

    is_init_inner.store(false, std::sync::atomic::Ordering::SeqCst);

    *interpreter_arc.lock() = interpreter;

    let cfg = config.lock();

    Ok(cfg.clone())
}

fn run(
    state_arc: Arc<Mutex<AppState>>,
    callbacks_arc: Arc<Mutex<Vec<Function>>>,
    interpreter_arc: Arc<Mutex<Interpreter>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let receiver = state_arc.lock().event_channel.receiver.clone();
    let sender = state_arc.lock().event_channel.sender.clone();

    info!("Starting hot reloading of config");
    config::hot_reloading::start(state_arc.clone());

    startup::set_launch_on_startup(state_arc.lock().config.launch_on_startup);

    os_specific_setup(state_arc.clone());

    info!("Listening for keybindings");
    state_arc
        .lock()
        .keybindings_manager
        .start(state_arc.clone());

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
                    Event::Keybinding(kb) => {
                        debug!("Received keybinding {:?}", kb);
                        sender.send(Event::CallCallback { idx: kb.callback_id, is_mode_callback: false } ).unwrap();
                        Ok(())
                    },
                    Event::ConfigError(err) => {
                        error!("{}", err.message(&interpreter_arc.lock().program()));

                        Ok(())
                    }
                    Event::CallCallback { idx, is_mode_callback } => {
                        let cb = callbacks_arc.lock().get(idx).unwrap().clone();
                        if let Err(e) = cb.invoke(&mut interpreter_arc.lock(), vec![]) {
                            state_arc.lock().event_channel.sender.send(Event::ConfigError(e)).unwrap();
                        }
                        if is_mode_callback {
                            state_arc.lock().keybindings_manager.sender.send(keybindings::ChanMessage::ModeCbExecuted);
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
                        match parse_config(state_arc.clone(), callbacks_arc.clone(), interpreter_arc.clone()) {
                            Ok(new_config) => update_config(state_arc.clone(), new_config),
                            Err(e) => {
                                sender.send(Event::NewPopup(Popup::new_error(vec![e])));
                                Ok(())
                            }

                        }
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

fn get_plugins_path() -> Result<PathBuf, String> {
    let mut plugins_path: PathBuf = dirs::config_dir().unwrap_or_default();
    plugins_path.push("nog");
    plugins_path.push("plugins");

    if !plugins_path.exists() {
        debug!("plugins folder doesn't exist yet. Creating the folder");
        std::fs::create_dir(plugins_path.clone()).map_err(|e| e.to_string())?;
    }

    Ok(plugins_path)
}

fn get_plugins_path_iter() -> Result<ReadDir, String> {
    Ok(get_plugins_path()?.read_dir().unwrap())
}

/// Fill source_locations of interpreter with plugin paths
fn load_plugin_source_locations(i: &mut Interpreter) {
    if let Ok(dirs) = get_plugins_path_iter() {
        for dir in dirs {
            if let Ok(dir) = dir {
                let mut path = dir.path();
                path.push("plugin");
                i.source_locations.push(path);
            }
        }
    }
}

fn main() {
    std::env::set_var("RUST_BACKTRACE", "1");
    logging::setup().expect("Failed to setup logging");

    let state_arc = Arc::new(Mutex::new(AppState::default()));
    let callbacks_arc: Arc<Mutex<Vec<Function>>> = Arc::new(Mutex::new(Vec::new()));
    let mut interpreter = Interpreter::new();

    load_plugin_source_locations(&mut interpreter);

    let interpreter_arc = Arc::new(Mutex::new(interpreter));

    {
        let mut config = parse_config(
            state_arc.clone(),
            callbacks_arc.clone(),
            interpreter_arc.clone(),
        )
        .map_err(|e| {
            let state_arc = state_arc.clone();
            Popup::error(vec![e], state_arc);
        })
        .unwrap_or_else(|_| {
            let mut config = Config::default();
            config.bar.use_default_components(state_arc.clone());
            config
        });

        state_arc.lock().init(config)
    }

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
    if let Err(e) = run(
        state_arc.clone(),
        callbacks_arc.clone(),
        interpreter_arc.clone(),
    ) {
        error!("An error occured {:?}", e);
        if let Err(e) = on_quit(&mut arc.lock()) {
            error!("Something happend when cleaning up. {}", e);
        }
    }
}
