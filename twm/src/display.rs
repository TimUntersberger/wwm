use crate::{
    bar::Bar,
    config::Config,
    renderer,
    system::DisplayId,
    system::SystemResult,
    system::{api, Rectangle},
    task_bar,
    tile_grid::store::Store,
    tile_grid::TileGrid,
};
use std::cmp::Ordering;
use task_bar::{Taskbar, TaskbarPosition};

#[derive(Default, Debug, Clone)]
pub struct Display {
    pub id: DisplayId,
    pub grids: Vec<TileGrid>,
    pub focused_grid_id: Option<i32>,
    pub dpi: u32,
    pub rect: Rectangle,
    pub taskbar: Option<Taskbar>,
    pub appbar: Option<Bar>,
}

impl Display {
    pub fn height(&self) -> i32 {
        self.rect.height()
    }
    pub fn width(&self) -> i32 {
        self.rect.width()
    }
    pub fn is_primary(&self) -> bool {
        self.rect.left == 0 && self.rect.top == 0
    }
    pub fn get_rect(&self) -> Rectangle {
        api::get_display_rect(self.id)
    }
    pub fn working_area_height(&self, config: &Config) -> i32 {
        let tb_height = self
            .taskbar
            .clone()
            .map(|tb| match tb.get_position() {
                // Should probably handle the error at some point instead of just unwraping
                TaskbarPosition::Top | TaskbarPosition::Bottom => {
                    tb.window.get_rect().unwrap().height()
                }
                _ => 0,
            })
            .unwrap_or(0);

        self.height()
            - if config.remove_task_bar { 0 } else { tb_height }
            - if config.display_app_bar {
                config.bar.height
            } else {
                0
            }
    }
    pub fn working_area_width(&self, config: &Config) -> i32 {
        let tb_width = self
            .taskbar
            .clone()
            .map(|tb| match tb.get_position() {
                // Should probably handle the error at some point instead of just unwraping
                TaskbarPosition::Left | TaskbarPosition::Right => {
                    tb.window.get_rect().unwrap().width()
                }
                _ => 0,
            })
            .unwrap_or(0);

        self.width() - if config.remove_task_bar { 0 } else { tb_width }
    }
    pub fn working_area_top(&self, config: &Config) -> i32 {
        let offset = self
            .taskbar
            .clone()
            .map(|t| match t.get_position() {
                // Should probably handle the error at some point instead of just unwraping
                TaskbarPosition::Top => t.window.get_rect().unwrap().height(),
                _ => 0,
            })
            .unwrap_or(0);

        self.rect.top
            + if config.display_app_bar {
                config.bar.height
            } else {
                0
            }
            + offset
    }
    pub fn working_area_left(&self) -> i32 {
        let offset = self
            .taskbar
            .clone()
            .map(|t| match t.get_position() {
                // Should probably handle the error at some point instead of just unwraping
                TaskbarPosition::Left => t.window.get_rect().unwrap().width(),
                _ => 0,
            })
            .unwrap_or(0);

        self.rect.left + offset
    }
    pub fn get_grid_by_id(&self, id: i32) -> Option<&TileGrid> {
        self.grids.iter().find(|g| g.id == id)
    }
    /// A grid is considered being active when it either has focus or contains one or more tiles
    pub fn get_active_grids(&self) -> Vec<&TileGrid> {
        self.grids
            .iter()
            .filter(|g| self.focused_grid_id == Some(g.id) || !g.is_empty())
            .collect()
    }
    pub fn get_grid_by_id_mut(&mut self, id: i32) -> Option<&mut TileGrid> {
        self.grids.iter_mut().find(|g| g.id == id)
    }
    pub fn get_focused_grid(&self) -> Option<&TileGrid> {
        self.focused_grid_id.and_then(|id| self.get_grid_by_id(id))
    }
    pub fn get_focused_grid_mut(&mut self) -> Option<&mut TileGrid> {
        self.focused_grid_id
            .and_then(move |id| self.get_grid_by_id_mut(id))
    }
    pub fn refresh_grid(&self, config: &Config) -> SystemResult {
        if let Some(g) = self.get_focused_grid() {
            g.draw_grid(self, config)?;

            Store::save(g.id, g.to_string());
        }

        Ok(())
    }

    pub fn remove_grid_by_id(&mut self, id: i32) -> Option<TileGrid> {
        self.grids
            .iter()
            .enumerate()
            .find(|(_, g)| g.id == id)
            .map(|(i, _)| i)
            .map(|i| self.grids.remove(i))
    }

    /// Returns true if the workspace was found and false if it wasn't
    pub fn focus_workspace(&mut self, config: &Config, id: i32) -> SystemResult<bool> {
        if let Some(grid) = self.get_grid_by_id_mut(id) {
            if grid.fullscreen_id.is_some() {
                grid.focused_id = grid.fullscreen_id;
            }
            if !grid.focused_id.is_some() {
                grid.focus_last_tile(); // ensures a tile is focused on the current grid
            }
        }

        if let Some(grid) = self.get_grid_by_id(id) {
            grid.draw_grid(self, config)?;
            grid.show()?;
        } else {
            return Ok(false);
        }

        if self.focused_grid_id != Some(id) {
            if let Some(grid) = self.get_focused_grid() {
                grid.hide();
            }
        }

        self.focused_grid_id = Some(id);

        Ok(true)
    }
    pub fn new(id: DisplayId) -> Self {
        let mut display = Display::default();

        display.dpi = api::get_display_dpi(id);
        display.id = id;
        display.rect = display.get_rect();

        display
    }
}

pub fn init(config: &Config) -> Vec<Display> {
    let mut displays = api::get_displays();
    let taskbars = api::get_taskbars();

    for d in displays.iter_mut() {
        for tb in &taskbars {
            let display = tb.window.get_display().unwrap();
            if display.id == d.id {
                d.taskbar = Some(tb.clone());
            }
        }
    }

    if !config.multi_monitor {
        displays = displays
            .iter_mut()
            .filter(|d| d.is_primary())
            .map(|d| d.to_owned())
            .collect();
    }

    displays.sort_by(|x, y| {
        let ordering = y.rect.left.cmp(&x.rect.left);

        if ordering == Ordering::Equal {
            return y.rect.top.cmp(&x.rect.top);
        }

        ordering
    });

    for i in 1..11 {
        let monitor = config
            .workspace_settings
            .iter()
            .find(|s| s.id == i)
            .map(|s| s.monitor)
            .unwrap_or(-1);

        let grid = TileGrid::new(i, renderer::NativeRenderer);

        if let Some(d) = displays.get_mut((monitor - 1) as usize) {
            d.grids.push(grid);
        } else {
            for d in displays.iter_mut() {
                if d.is_primary() {
                    d.grids.push(grid);
                    break;
                }
            }
        }
    }

    displays

    // task_bar::update_task_bars();
}
